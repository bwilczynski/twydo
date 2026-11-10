//! The vdir watcher: turns file-system events into one debounced callback per burst of
//! changes (a sync batch reloads once). Logic moved from the TUI's watcher (`tdo/src/term.rs`),
//! which keeps its own copy until the TUI becomes a client.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify::event::{MetadataKind, ModifyKind};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

/// Quiet time after the last change before the callback runs.
pub const DEBOUNCE: Duration = Duration::from_millis(300);

/// A running watch; dropping it stops the watch (the debounce thread ends on its own).
pub struct Watcher {
    _inner: Option<RecommendedWatcher>,
    /// [`start_file`]'s watcher, shared (weakly) with its debounce thread, which moves the
    /// watch of a link's target.
    _file: Option<Arc<Mutex<RecommendedWatcher>>>,
}

/// Watches `vdir` recursively and calls `on_change` on a background thread after each quiet
/// [`DEBOUNCE`]. The callback never runs on notify's own thread, which only sends on a channel.
pub fn start(vdir: &Path, on_change: impl Fn() + Send + 'static) -> notify::Result<Watcher> {
    let (tx, rx) = mpsc::channel::<()>();
    let mut inner = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res
            && is_change(&ev.kind)
        {
            let _ = tx.send(());
        }
    })?;
    inner.watch(vdir, RecursiveMode::Recursive)?;
    std::thread::spawn(move || debounced(rx, on_change));
    Ok(Watcher {
        _inner: Some(inner),
        _file: None,
    })
}

/// Watches the file `path` (through its parent directory, non-recursively, so a save by
/// rename over it is seen) and calls `on_change` after each quiet [`DEBOUNCE`]. Only events
/// that name the file count.
///
/// A `path` that is a symbolic link (a `config.toml` kept in a dotfiles directory) is followed:
/// the directory of its target is watched too, for the target's name, so an edit of the
/// target is seen. The link's own directory stays watched, so a link swapped for another is
/// seen; the target is resolved again after each burst, and the second watch follows it.
pub fn start_file(path: &Path, on_change: impl Fn() + Send + 'static) -> notify::Result<Watcher> {
    let link = Watched::of(path);
    // Recorded only once its directory is watched (see [`follow`]), so a failed watch is
    // retried after the next burst.
    let target = Arc::new(Mutex::new(None::<Watched>));
    let (tx, rx) = mpsc::channel::<()>();
    let names = Arc::clone(&target);
    let link_name = link.name.clone();
    let inner = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(ev) = res else { return };
        if !is_change(&ev.kind) {
            return;
        }
        let target = names.lock().unwrap_or_else(|e| e.into_inner());
        let target_name = target.as_ref().and_then(|t| t.name.as_deref());
        if ev.paths.iter().any(|p| {
            let n = p.file_name();
            n == link_name.as_deref() || (target_name.is_some() && n == target_name)
        }) {
            let _ = tx.send(());
        }
    })?;
    let inner = Arc::new(Mutex::new(inner));
    inner
        .lock()
        .unwrap()
        .watch(&link.dir, RecursiveMode::NonRecursive)?;
    follow(&inner, &link.dir, &target, path);
    // A weak reference: dropping the [`Watcher`] drops notify's watcher, and with it the
    // sender, which ends this thread.
    let weak = Arc::downgrade(&inner);
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        debounced(rx, || {
            if let Some(inner) = weak.upgrade() {
                follow(&inner, &link.dir, &target, &path);
            }
            on_change();
        })
    });
    Ok(Watcher {
        _file: Some(inner),
        _inner: None,
    })
}

/// A watched file: its directory and its name.
struct Watched {
    dir: PathBuf,
    name: Option<OsString>,
}

impl Watched {
    fn of(path: &Path) -> Watched {
        Watched {
            dir: path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf(),
            name: path.file_name().map(|n| n.to_os_string()),
        }
    }
}

/// Resolves the link `path` again and moves the target's watch to its new directory.
///
/// No lock on `target` is held while notify's watcher is called: `watch` waits for notify's
/// event-loop thread, which runs the event handler inline, and the handler locks `target`.
/// The new target is recorded only when its directory is watched (or needs no watch of its
/// own); after a failed watch nothing is recorded, so the next burst tries again.
fn follow(
    inner: &Mutex<RecommendedWatcher>,
    link_dir: &Path,
    target: &Mutex<Option<Watched>>,
    path: &Path,
) {
    let now = fs::canonicalize(path).ok().map(|t| Watched::of(&t));
    let old_dir = target
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|t| t.dir.clone());
    let new_dir = now.as_ref().map(|t| t.dir.clone());
    if old_dir == new_dir {
        *target.lock().unwrap_or_else(|e| e.into_inner()) = now;
        return;
    }
    let watched = {
        let mut w = inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(d) = old_dir.filter(|d| d != link_dir) {
            let _ = w.unwatch(&d);
        }
        match new_dir.filter(|d| d != link_dir) {
            Some(d) => match w.watch(&d, RecursiveMode::NonRecursive) {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("tdo daemon: not watching {}: {e}", d.display());
                    false
                }
            },
            None => true,
        }
    };
    *target.lock().unwrap_or_else(|e| e.into_inner()) = if watched { now } else { None };
}

/// Runs `on_change` once per burst of messages on `rx`; ends when the sender is dropped.
fn debounced(rx: mpsc::Receiver<()>, on_change: impl Fn()) {
    // `rx` disconnects when the watcher (and with it the sender) is dropped.
    while rx.recv().is_ok() {
        loop {
            match rx.recv_timeout(DEBOUNCE) {
                Ok(()) => continue,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        on_change();
    }
}

/// Creates, writes, renames and removes count; opens, reads and access-time updates do not.
/// Reads are not changes: every reload reads all files, so counting them would loop.
fn is_change(kind: &EventKind) -> bool {
    !matches!(
        kind,
        EventKind::Access(_) | EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    /// Waits until `count` exceeds `seen`, within two seconds; returns whether it did.
    fn changed(count: &AtomicUsize, seen: usize) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if count.load(Ordering::SeqCst) > seen {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// Waits for a burst to settle, so the next count starts from a quiet watcher.
    fn settle(count: &AtomicUsize) -> usize {
        loop {
            let n = count.load(Ordering::SeqCst);
            std::thread::sleep(DEBOUNCE * 2);
            if count.load(Ordering::SeqCst) == n {
                return n;
            }
        }
    }

    /// Points the link `path` at `target` by rename, as an editor or a dotfiles tool does.
    fn relink(path: &Path, target: &Path) {
        let tmp = path.with_extension("link.tmp");
        symlink(target, &tmp).unwrap();
        fs::rename(&tmp, path).unwrap();
    }

    /// Codex review: a target directory that cannot be watched yet (no read permission) is
    /// not recorded as watched, so the next burst retries the watch.
    #[test]
    fn a_failed_target_watch_is_retried() {
        let tmp = tempfile::tempdir().unwrap();
        let link_dir = tmp.path().join("link");
        let a_dir = tmp.path().join("a");
        let b_dir = tmp.path().join("b");
        for d in [&link_dir, &a_dir, &b_dir] {
            fs::create_dir(d).unwrap();
        }
        let a = a_dir.join("a.toml");
        let b = b_dir.join("b.toml");
        fs::write(&a, "a").unwrap();
        fs::write(&b, "b").unwrap();
        let path = link_dir.join("config.toml");
        symlink(&a, &path).unwrap();

        let count = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&count);
        let _w = start_file(&path, move || {
            c.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();

        // Retarget to a directory notify cannot watch (search but no read permission).
        fs::set_permissions(&b_dir, fs::Permissions::from_mode(0o300)).unwrap();
        let n = settle(&count);
        relink(&path, &b);
        assert!(changed(&count, n), "the retarget was not seen");
        let n = settle(&count);

        // Readable again; the next burst in the link's directory retries the target's watch.
        fs::set_permissions(&b_dir, fs::Permissions::from_mode(0o700)).unwrap();
        relink(&path, &b);
        assert!(changed(&count, n), "the second retarget was not seen");
        let n = settle(&count);

        fs::write(&b, "b2").unwrap();
        assert!(
            changed(&count, n),
            "an edit of the target was not seen after the retry"
        );
    }
}
