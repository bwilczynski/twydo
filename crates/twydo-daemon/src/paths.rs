//! Where the daemon's socket and lock live, and which instance it serves. Nothing here reads
//! a file: clients use it without ever touching the configuration.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use twydo_core::config::Config;
use twydo_core::sync::lock::LockFile;

use crate::protocol::methods::Instance;

/// The instance identity of this process: the configuration path and the `--vdir` flag, both
/// made absolute. No file is read or required to exist.
pub fn instance(vdir_flag: Option<&Path>) -> io::Result<Instance> {
    instance_for(&Config::path(), vdir_flag)
}

/// [`instance`] for an explicit configuration path.
pub fn instance_for(config: &Path, vdir_flag: Option<&Path>) -> io::Result<Instance> {
    Ok(Instance {
        config: std::path::absolute(config)?,
        vdir: vdir_flag.map(std::path::absolute).transpose()?,
    })
}

/// The socket to use: `--socket`, else `TWYDO_SOCKET`, else the per-user default.
pub fn socket_path(flag: Option<&Path>) -> PathBuf {
    // SAFETY: geteuid has no preconditions and cannot fail.
    let uid = unsafe { libc::geteuid() };
    socket_path_from(flag, &|k| std::env::var_os(k), uid)
}

/// Pure form of [`socket_path`]. The default is `$XDG_RUNTIME_DIR/twydo/tdo.sock` on Linux,
/// `$TMPDIR/twydo/tdo.sock` on macOS, and `/tmp/twydo-<uid>/tdo.sock` without either variable.
pub fn socket_path_from(
    flag: Option<&Path>,
    env: &dyn Fn(&str) -> Option<OsString>,
    uid: u32,
) -> PathBuf {
    if let Some(f) = flag {
        return f.to_path_buf();
    }
    let non_empty = |k: &str| env(k).filter(|v| !v.is_empty());
    if let Some(s) = non_empty("TWYDO_SOCKET") {
        return PathBuf::from(s);
    }
    let var = if cfg!(target_os = "macos") {
        "TMPDIR"
    } else {
        "XDG_RUNTIME_DIR"
    };
    match non_empty(var) {
        Some(d) => PathBuf::from(d).join("twydo").join("tdo.sock"),
        None => PathBuf::from(format!("/tmp/twydo-{uid}")).join("tdo.sock"),
    }
}

/// A short stable id of a socket path: the first 12 hex digits of the SHA-256 of its absolute
/// path. It names the daemon's log file.
pub fn socket_id(socket: &Path) -> String {
    use sha2::{Digest, Sha256};
    let abs = std::path::absolute(socket).unwrap_or_else(|_| socket.to_path_buf());
    let digest = Sha256::digest(abs.as_os_str().as_encoded_bytes());
    digest.iter().take(6).map(|b| format!("{b:02x}")).collect()
}

/// The single-instance lock of a vdir's state directory, released on drop (or by the OS when
/// the process dies).
pub struct DaemonLock(#[allow(dead_code)] LockFile);

impl DaemonLock {
    /// Takes `<state>/daemon.lock` without waiting; `None` when another daemon holds it.
    pub fn take(state: &Path) -> io::Result<Option<DaemonLock>> {
        Ok(LockFile::take(&state.join("daemon.lock"), true)?.map(DaemonLock))
    }
}
