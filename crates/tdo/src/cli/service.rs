//! `tdo daemon install`, `uninstall` and `status`: the login service that keeps the daemon
//! running (daemon design §10.1). The service manager is a seam: the commands take a
//! [`Manager`], and the real one runs `systemctl` / `launchctl` from `PATH`.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::EXIT_ERROR;
use super::daemon::stop_detached;
use crate::client::{self, Probe, Target};
use crate::service::{self, Manager, Platform, Spec};

fn mise_shims() -> PathBuf {
    let base = std::env::var_os("MISE_DATA_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|h| PathBuf::from(h).join(".local/share/mise"))
        })
        .unwrap_or_default();
    base.join("shims")
}

fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

fn fail(msg: impl std::fmt::Display) -> i32 {
    eprintln!("tdo: {msg}");
    EXIT_ERROR
}

pub fn install(target: &Target, exe: Option<&Path>, mgr: &dyn Manager) -> i32 {
    let unit = service::unit_for_current(&target.socket);
    let exe = match exe {
        Some(p) => absolute(p),
        None => match service::stable_exe(std::env::var_os("PATH").as_deref(), &mise_shims()) {
            Ok(p) => p,
            Err(e) => return fail(e),
        },
    };
    let spec = Spec {
        exe,
        socket: unit.socket.clone(),
        default_socket: unit.default_socket,
        vdir: target.vdir.as_deref().map(absolute),
        env: service::service_env(&|k| std::env::var_os(k)),
        log: client::start::log_path(&target.socket),
    };
    let text = service::render(&unit, &spec);

    // An installed unit is stopped through the manager first (socket shutdown would let it
    // restart the daemon, and launchd keeps the job loaded so the next bootstrap would fail).
    // "Not loaded" is fine: the error is ignored.
    let existed = unit.file.exists();
    if existed {
        let _ = mgr.stop(&unit);
    }
    // Then the detached daemon of this instance, so the service's daemon can take the lock;
    // one of another instance is never touched.
    if let Err(e) = stop_detached(target) {
        return fail(e);
    }
    if let Err(e) = write_unit(&unit.file, &text) {
        return fail(format!("cannot write {}: {e}", unit.file.display()));
    }
    if let Err(e) = mgr.reload() {
        return fail(e);
    }
    if let Err(e) = mgr.enable_now(&unit) {
        return fail(e);
    }
    if existed {
        println!("{} was already installed; rewrote it", unit.name);
    } else {
        println!("installed {}", unit.name);
    }
    match mgr.disable_old_timer() {
        Ok(Some(name)) => println!("disabled {name}: the daemon syncs now"),
        Ok(None) => {}
        Err(e) => eprintln!("tdo: cannot disable the old sync timer: {e}"),
    }

    let deadline = Instant::now() + client::START_WAIT;
    loop {
        match client::probe(target) {
            Probe::Running(_) => return 0,
            Probe::WrongInstance(m) => return fail(m),
            Probe::NotRunning if Instant::now() >= deadline => break,
            Probe::NotRunning => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    let see = match unit.platform {
        Platform::Systemd => format!("journalctl --user -u {}", unit.name),
        Platform::Launchd => spec.log.display().to_string(),
    };
    fail(format!("the service did not start; see {see}"))
}

fn write_unit(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, text)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o644))
}

pub fn uninstall(target: &Target, mgr: &dyn Manager) -> i32 {
    let unit = service::unit_for_current(&target.socket);
    if !unit.file.exists() {
        println!("no unit installed");
        return 0;
    }
    let mut code = 0;
    if let Err(e) = mgr.disable_now(&unit) {
        eprintln!("tdo: {e}");
        code = EXIT_ERROR;
    }
    if let Err(e) = fs::remove_file(&unit.file) {
        return fail(format!("cannot remove {}: {e}", unit.file.display()));
    }
    if let Err(e) = mgr.reload() {
        eprintln!("tdo: {e}");
        code = EXIT_ERROR;
    }
    println!("removed {}", unit.name);
    code
}

pub fn status(target: &Target, mgr: &dyn Manager) -> i32 {
    let unit = service::unit_for_current(&target.socket);
    if unit.file.exists() {
        let state = mgr.state(&unit);
        let mut line = format!("unit: installed ({})", unit.file.display());
        match state.enabled {
            Some(true) => line.push_str(", enabled"),
            Some(false) => line.push_str(", not enabled"),
            None => {}
        }
        if state.active == Some(false) {
            line.push_str(", not active");
        }
        println!("{line}");
    } else {
        println!("unit: not installed; daemon won't start at login; run tdo daemon install");
    }
    match client::probe(target) {
        Probe::Running(i) => {
            println!(
                "daemon: running, version {}, pid {}, vdir {}",
                i.version,
                i.pid,
                i.vdir.display()
            );
            0
        }
        Probe::NotRunning => {
            println!("daemon: not running");
            0
        }
        Probe::WrongInstance(m) => {
            println!("daemon: {m}");
            EXIT_ERROR
        }
    }
}
