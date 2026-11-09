//! The vdir watcher: turns file-system events into one debounced callback per burst of
//! changes (a sync batch reloads once). Logic moved from the TUI's watcher (`tdo/src/term.rs`),
//! which keeps its own copy until the TUI becomes a client.

use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use notify::event::{MetadataKind, ModifyKind};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

/// Quiet time after the last change before the callback runs.
pub const DEBOUNCE: Duration = Duration::from_millis(300);

/// A running watch; dropping it stops the watch (the debounce thread ends on its own).
pub struct Watcher {
    _inner: RecommendedWatcher,
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
    std::thread::spawn(move || {
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
    });
    Ok(Watcher { _inner: inner })
}

/// Creates, writes, renames and removes count; opens, reads and access-time updates do not.
/// Reads are not changes: every reload reads all files, so counting them would loop.
fn is_change(kind: &EventKind) -> bool {
    !matches!(
        kind,
        EventKind::Access(_) | EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime))
    )
}
