//! Finding executables on PATH, the way the daemon and vdirsyncer will.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The first executable regular file named `name` in a directory of this process's PATH.
pub fn on_path(name: &str) -> Option<PathBuf> {
    on(name, std::env::var_os("PATH").as_deref())
}

/// The first executable regular file named `name` in a directory of `path`.
pub fn on(name: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    std::env::split_paths(path?)
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| d.join(name))
        .find(|c| is_executable(c))
}

/// An executable regular file (symlinks followed).
pub fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
