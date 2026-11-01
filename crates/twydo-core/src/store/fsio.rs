//! Durable file primitives and the vdir lock.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::Path;

/// Name of the lock file inside the state directory.
pub const LOCK_FILE: &str = "vdir.lock";

fn parent_of(path: &Path) -> io::Result<&Path> {
    match path.parent() {
        Some(p) if p.as_os_str().is_empty() => Ok(Path::new(".")),
        Some(p) => Ok(p),
        None => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path has no parent directory: {}", path.display()),
        )),
    }
}

/// Fsyncs a directory so a rename, create or unlink inside it is durable.
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Replaces `path` with `bytes` atomically: the bytes go to `.<name>.tdo-tmp`
/// in the same directory, the temp file is fsynced, renamed over `path`, and
/// the directory is fsynced. A crash leaves either the old or the new content.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = parent_of(path)?;
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path has no file name: {}", path.display()),
        )
    })?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(".tdo-tmp");
    let tmp = dir.join(tmp_name);
    let result = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)?;
        sync_dir(dir)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Removes `path` durably (the directory is fsynced). A missing file is `Ok`.
pub fn remove_durable(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => sync_dir(parent_of(path)?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// An advisory lock on `<state>/vdir.lock`, released on drop.
///
/// Writers (the store's apply, undo and redo, and journal recovery) take it
/// exclusively for a whole operation (prepare, apply, mark applied). Readers
/// that load the vdir (`tdo tui` and its reloads, `export-json`, `check`) take
/// it shared with [`VdirLock::reader`] while they load, so they never observe a
/// half-applied operation. Locks on two separate file descriptors block each
/// other even inside one process: a caller that already holds the lock must not
/// take it again (see `journal::recover_locked`).
#[derive(Debug)]
pub struct VdirLock(File);

impl VdirLock {
    fn open(state_dir: &Path) -> io::Result<File> {
        fs::create_dir_all(state_dir)?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(state_dir.join(LOCK_FILE))
    }

    /// Blocks until the exclusive lock is held.
    pub fn exclusive(state_dir: &Path) -> io::Result<VdirLock> {
        let f = Self::open(state_dir)?;
        f.lock()?;
        Ok(VdirLock(f))
    }

    /// Blocks until a shared lock is held. Creates the state directory.
    pub fn shared(state_dir: &Path) -> io::Result<VdirLock> {
        let f = Self::open(state_dir)?;
        f.lock_shared()?;
        Ok(VdirLock(f))
    }

    /// A shared lock on an open file description of its own, for a child process to
    /// inherit (as its stdin, say: the lock file is empty). It is never unlocked
    /// explicitly, as a [`VdirLock`] is on drop: the lock lasts until every copy of the
    /// descriptor is closed, so a child that outlives this process keeps writers waiting
    /// until it exits. (A dup of a held [`VdirLock`]'s file would not do: its unlock on drop
    /// releases the lock for every copy.)
    pub fn shared_for_child(state_dir: &Path) -> io::Result<File> {
        let f = Self::open(state_dir)?;
        f.lock_shared()?;
        Ok(f)
    }

    /// A reader's lock: [`VdirLock::shared`] when `state_dir` exists, else `None`
    /// (nothing has ever written through Twydo there, so there is nothing to wait
    /// for, and a read-only command must not create the directory).
    pub fn reader(state_dir: &Path) -> io::Result<Option<VdirLock>> {
        if !state_dir.is_dir() {
            return Ok(None);
        }
        Self::shared(state_dir).map(Some)
    }
}

impl Drop for VdirLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
