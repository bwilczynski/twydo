//! Exclusive lock files under `<state>/sync/` (the run lock).

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::Path;

/// An exclusive lock on a file, released on drop. It unlocks explicitly rather than relying
/// on close: a child forked by another thread holds a copy of the descriptor until it execs,
/// and would keep the lock alive.
pub struct LockFile(File);

impl LockFile {
    /// Blocks for the lock on `path` (created if missing), or with `try_only` returns `None`
    /// when another process holds it.
    pub fn take(path: &Path, try_only: bool) -> io::Result<Option<LockFile>> {
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        if !try_only {
            f.lock()?;
            return Ok(Some(LockFile(f)));
        }
        match f.try_lock() {
            Ok(()) => Ok(Some(LockFile(f))),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(e)) => Err(e),
        }
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
