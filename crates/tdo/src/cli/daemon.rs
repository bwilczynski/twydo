//! `tdo daemon`: the hidden foreground daemon. It loads the configuration (only the daemon
//! reads it), serves the socket until `SIGTERM`/`SIGINT` or `daemon.shutdown`, and exits
//! cleanly.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::flag;
use twydo_core::compat::Matrix;
use twydo_daemon::server::{Daemon, DaemonOptions, StartError};
use twydo_daemon::{EXIT_ALREADY_RUNNING, paths};

use super::{CliError, EXIT_ERROR, resolve_config};

pub fn run(
    vdir: Option<&Path>,
    socket: Option<&Path>,
    matrix: Option<&PathBuf>,
    crash_after_commit: bool,
) -> i32 {
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
