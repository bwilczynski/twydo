//! `tdo daemon`: the hidden foreground daemon. It loads the configuration (only the daemon
//! reads it), serves the socket until `SIGTERM`/`SIGINT` or `daemon.shutdown`, and exits
//! cleanly.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::client::{self, Target};
use crate::service::{self, Platform};
use serde_json::json;
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::flag;
use twydo_core::compat::Matrix;
use twydo_daemon::client::{Client, ClientError};
use twydo_daemon::protocol::methods::{DaemonInfo, ShutdownParams};
use twydo_daemon::protocol::rpc::ErrorKind;
use twydo_daemon::server::{Daemon, DaemonOptions, StartError};
use twydo_daemon::{EXIT_ALREADY_RUNNING, paths};

/// `--detach`: runs this daemon again, without `--detach`, in a session of its own with its
/// stderr appended to the log, and returns.
fn detach_daemon(
    vdir: Option<&Path>,
    socket: Option<&Path>,
    matrix: Option<&PathBuf>,
    crash_after_commit: bool,
) -> i32 {
    let socket = paths::socket_path(socket);
    let mut args: Vec<std::ffi::OsString> =
        vec!["daemon".into(), "--socket".into(), socket.clone().into()];
    if let Some(v) = vdir {
        args.extend(["--vdir".into(), v.into()]);
    }
    if let Some(m) = matrix {
        args.extend(["--matrix".into(), m.into()]);
    }
    if crash_after_commit {
        args.push("--test-crash-after-commit".into());
    }
    match client::start::spawn_session(&args, &socket) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("tdo: cannot start the daemon: {e}");
            EXIT_ERROR
        }
    }
}

use super::{CliError, EXIT_ERROR, resolve_config};

/// The commands under `tdo daemon` that are clients of a running daemon.
#[derive(clap::Subcommand)]
pub enum DaemonCommand {
    /// Stop the daemon once it is idle, and wait until it is gone
    Stop,
    /// Install the daemon as a login service (systemd user unit or launchd agent)
    Install {
        /// The `tdo` the unit runs (default: a stable path found on PATH)
        #[arg(long, value_name = "PATH")]
        exe: Option<PathBuf>,
    },
    /// Remove the login service
    Uninstall,
    /// Show whether the service is installed and whether the daemon is running
    Status,
    /// Print the daemon's protocol schema (OpenRPC) and exit
    Schema,
    /// Connect (starting the daemon if needed) and print its epoch
    #[command(hide = true)]
    Ping,
}

/// `tdo daemon stop` and `tdo daemon ping`.
pub fn client_command(vdir: Option<&Path>, socket: Option<&Path>, cmd: DaemonCommand) -> i32 {
    let target = Target::from_flags(vdir, socket);
    match cmd {
        DaemonCommand::Ping => {
            let (c, _) = match client::connect(&target, "tdo", None) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("tdo: {e}");
                    return EXIT_ERROR;
                }
            };
            match c.caller().call_timeout::<_, DaemonInfo>(
                "daemon.info",
                json!({}),
                twydo_daemon::client::HELLO_TIMEOUT,
            ) {
                Ok(info) => {
                    println!("{}", info.epoch);
                    0
                }
                Err(e) => {
                    eprintln!("tdo: {e}");
                    EXIT_ERROR
                }
            }
        }
        DaemonCommand::Stop => match stop(&target) {
            Ok(false) => {
                println!("no daemon is running");
                0
            }
            Ok(true) => 0,
            Err(e) => {
                eprintln!("tdo: {e}");
                EXIT_ERROR
            }
        },
        DaemonCommand::Install { exe } => {
            let mgr = service::manager(Platform::current());
            super::service::install(&target, exe.as_deref(), mgr.as_ref())
        }
        DaemonCommand::Uninstall => {
            let mgr = service::manager(Platform::current());
            super::service::uninstall(&target, mgr.as_ref())
        }
        DaemonCommand::Status => {
            let mgr = service::manager(Platform::current());
            super::service::status(&target, mgr.as_ref())
        }
        DaemonCommand::Schema => {
            match serde_json::to_string_pretty(&twydo_daemon::schema::document()) {
                Ok(t) => {
                    println!("{t}");
                    0
                }
                Err(e) => {
                    eprintln!("tdo: {e}");
                    EXIT_ERROR
                }
            }
        }
    }
}

/// `tdo daemon stop`: through the service manager when a unit is installed (it would restart
/// a daemon that merely exited), else [`stop_detached`]. `Ok(false)`: none was running.
pub fn stop(target: &Target) -> Result<bool, String> {
    let unit = service::unit_for_current(&target.socket);
    if !unit.file.exists() {
        return stop_detached(target);
    }
    let was_running = match client::probe(target) {
        client::Probe::WrongInstance(m) => return Err(m),
        client::Probe::NotRunning => false,
        client::Probe::Running(_) => true,
    };
    // The manager is told to stop even when nothing answers: the unit may be between restarts
    // (RestartSec, launchd throttling, startup before the socket binds).
    let mgr = service::manager(Platform::current());
    let failed = mgr.stop(&unit).err();
    if (failed.is_none() || !was_running)
        && client::wait_gone(&target.socket, Duration::from_millis(500))
    {
        return Ok(was_running);
    }
    // The manager stopped nothing, or failed (launchd with the agent not loaded): the daemon
    // was started outside the unit (detached).
    let stopped = stop_detached(target);
    match (failed, stopped) {
        // Report the manager's error only while a daemon still answers.
        (Some(e), Ok(false) | Err(_)) if !client::wait_gone(&target.socket, Duration::ZERO) => {
            Err(format!("cannot stop {}: {e}", unit.name))
        }
        (Some(_), Ok(_)) => Ok(true),
        (_, r) => r,
    }
}

/// Asks the daemon on `target`'s socket to stop once idle and waits until it is gone.
/// `Ok(false)`: none was running. Hello comes first, so another instance's daemon on this
/// socket is never stopped; a protocol mismatch still stops (`daemon.shutdown` is frozen
/// across protocols).
pub fn stop_detached(target: &Target) -> Result<bool, String> {
    let hello = client::hello_params(target, "tdo", None).map_err(|e| e.to_string())?;
    let raw = match Client::connect_raw(&target.socket) {
        Ok(c) => c,
        Err(ClientError::Io(e))
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(false);
        }
        Err(e) => return Err(e.to_string()),
    };
    // `daemon.shutdown` goes on this very connection, after its hello was checked.
    match raw.hello_on(hello) {
        Ok(_) => {}
        Err(ClientError::Rpc(e)) => match e.data.as_ref().map(|d| d.kind) {
            // Relies on the daemon checking the instance before the protocol in hello: a
            // `protocol` refusal means this is our instance's daemon.
            Some(ErrorKind::Protocol) => {}
            _ => return Err(e.message),
        },
        Err(e) => return Err(e.to_string()),
    }
    let sent: Result<serde_json::Value, _> = raw.caller().call_timeout(
        "daemon.shutdown",
        ShutdownParams { when_idle: true },
        twydo_daemon::client::HELLO_TIMEOUT,
    );
    drop(raw);
    // A daemon that is already going may close the connection before its reply is written:
    // that is the answer, too.
    if let Err(e) = sent
        && !matches!(e, ClientError::Closed)
    {
        return Err(e.to_string());
    }
    if client::wait_gone(&target.socket, client::STOP_WAIT) {
        Ok(true)
    } else {
        Err("the daemon is still busy; it stops when its job is done".to_string())
    }
}

pub fn run(
    vdir: Option<&Path>,
    socket: Option<&Path>,
    matrix: Option<&PathBuf>,
    crash_after_commit: bool,
    detach: bool,
) -> i32 {
    if detach {
        return detach_daemon(vdir, socket, matrix, crash_after_commit);
    }
    let cfg = match resolve_config(vdir) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("tdo: {e}");
            return EXIT_ERROR;
        }
    };
    let matrix = match matrix {
        Some(p) => match Matrix::load(p).map_err(CliError::from) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("tdo: {e}");
                return EXIT_ERROR;
            }
        },
        None => Matrix::embedded().clone(),
    };
    let instance = match paths::instance(vdir) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("tdo: {e}");
            return EXIT_ERROR;
        }
    };
    let socket = paths::socket_path(socket);
    let mut opts = DaemonOptions::new(socket.clone());
    opts.matrix = matrix;
    opts.crash_after_commit = crash_after_commit;
    let shown = cfg.vdir.display().to_string();

    let stop = Arc::new(AtomicBool::new(false));
    for sig in [SIGTERM, SIGINT] {
        // Registered first, so a second signal (the flag is already set) ends the process at
        // once with 130 instead of waiting for the teardown.
        if let Err(e) = flag::register_conditional_shutdown(sig, 130, Arc::clone(&stop)) {
            eprintln!("tdo: cannot handle signals: {e}");
            return EXIT_ERROR;
        }
        if let Err(e) = flag::register(sig, Arc::clone(&stop)) {
            eprintln!("tdo: cannot handle signals: {e}");
            return EXIT_ERROR;
        }
    }
    let handle = match Daemon::start(cfg, instance, opts) {
        Ok(h) => h,
        Err(StartError::AlreadyRunning) => {
            eprintln!("tdo: another daemon serves {shown}");
            return EXIT_ALREADY_RUNNING;
        }
        Err(e) => {
            eprintln!("tdo: daemon failed to start: {e}");
            return EXIT_ERROR;
        }
    };
    eprintln!(
        "tdo daemon: serving {shown} on {} (pid {})",
        socket.display(),
        std::process::id()
    );
    // Ends on a signal, or once the daemon stopped itself (`daemon.shutdown`).
    while !stop.load(Ordering::Relaxed) && !handle.is_stopped() {
        std::thread::sleep(Duration::from_millis(100));
    }
    handle.shutdown();
    eprintln!("tdo daemon: stopped");
    0
}
