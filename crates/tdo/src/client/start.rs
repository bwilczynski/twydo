//! Starting the daemon on demand: the same executable as `tdo daemon --detach`, which runs
//! the real daemon in a session of its own with its log appended to a file.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use twydo_core::config::Config;
use twydo_daemon::paths;

use super::Target;
use crate::service;

/// `$XDG_STATE_HOME/twydo/daemon-<socket-id>.log`: where a detached daemon's stderr goes.
pub fn log_path(socket: &Path) -> PathBuf {
    Config::with_vdir(PathBuf::new())
        .state_dir()
        .join(format!("daemon-{}.log", paths::socket_id(socket)))
}

/// Opens the log for appending, creating its directory.
pub fn open_log(socket: &Path) -> io::Result<File> {
    let path = log_path(socket);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
}

/// What the service manager last said when it failed to start the daemon in this process, for
/// the final "no daemon answered" message.
static MANAGER_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// The manager's error left by [`start`], if any (not cleared).
pub fn manager_error() -> Option<String> {
    MANAGER_ERROR.lock().ok().and_then(|g| g.clone())
}

/// `journalctl --user -u <unit>` when the daemon is meant to run under a systemd unit that
/// started without error: its stderr goes to the journal, not to the log [`LogMark`] reads.
pub fn journal_hint(t: &Target) -> Option<String> {
    let unit = service::unit_for_current(&t.socket);
    (unit.platform == service::Platform::Systemd && unit.file.exists() && manager_error().is_none())
        .then(|| format!("journalctl --user -u {}", unit.name))
}

/// Starts the daemon for `t`: through the service manager when a unit is installed for its
/// socket, else (or when the manager fails; its error is kept, see [`manager_error`]) with
/// [`detached`].
pub fn start(t: &Target) -> io::Result<()> {
    if let Ok(mut g) = MANAGER_ERROR.lock() {
        *g = None;
    }
    let unit = service::unit_for_current(&t.socket);
    if unit.file.exists() {
        let mgr = service::manager(service::Platform::current());
        match mgr.start(&unit) {
            Ok(()) => return Ok(()),
            Err(e) => {
                if let Ok(mut g) = MANAGER_ERROR.lock() {
                    *g = Some(e.to_string());
                }
            }
        }
    }
    detached(t)
}

/// Runs `tdo daemon --detach` for `t` and waits for it to return (it exits as soon as the
/// daemon process is launched).
pub fn detached(t: &Target) -> io::Result<()> {
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.arg("daemon")
        .arg("--detach")
        .arg("--socket")
        .arg(&t.socket);
    if let Some(v) = &t.vdir {
        cmd.arg("--vdir").arg(v);
    }
    let out = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim();
        Err(io::Error::other(if err.is_empty() {
            format!("tdo daemon --detach: {}", out.status)
        } else {
            err.to_string()
        }))
    }
}

/// Re-runs this executable's arguments `args` as a daemon in its own session (stdin and
/// stdout closed, stderr appended to the log of `socket`) and returns at once.
pub fn spawn_session(args: &[std::ffi::OsString], socket: &Path) -> io::Result<()> {
    let log = open_log(socket)?;
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    // SAFETY: setsid is async-signal-safe and touches no shared state.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    // Not waited for: the daemon outlives this process and is reparented.
    cmd.spawn().map(drop)
}

/// Where the log ended when a connect began, so a failure can quote what the daemon logged
/// since.
pub struct LogMark {
    path: PathBuf,
    len: u64,
}

impl LogMark {
    pub fn take(socket: &Path) -> LogMark {
        let path = log_path(socket);
        let len = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        LogMark { path, len }
    }

    /// The last non-empty line logged since the mark.
    pub fn last_line(&self) -> Option<String> {
        let mut f = File::open(&self.path).ok()?;
        f.seek(SeekFrom::Start(self.len)).ok()?;
        let mut text = String::new();
        f.read_to_string(&mut text).ok()?;
        text.lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .map(|l| l.trim().to_string())
    }
}
