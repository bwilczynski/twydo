//! What the status line and the conflict overlay read: the daemon's status and conflict list,
//! as the link last fetched them.

use twydo_daemon::protocol::admin::ConflictView;
use twydo_daemon::protocol::methods::{ConfigStatus, RefusalView, StatusView, SyncState, SyncView};

#[derive(Debug, Clone)]
pub struct ClientSync {
    pub sync: Option<SyncView>,
    /// Journal entries not yet confirmed by a sync.
    pub unsynced: usize,
    /// Edits queued while a sync held the vdir, not yet written.
    pub queued: usize,
    pub conflicts: Vec<ConflictView>,
    pub refusals: Vec<RefusalView>,
    pub config: ConfigStatus,
    pub drain_error: Option<String>,
}

impl ClientSync {
    /// From `status.get` and `conflicts.list`. `sync` is `None` without `[sync]`; the screen
    /// then keeps `App::sync` empty but still reads `refusals` and `drain_error`.
    pub fn from_status(status: &StatusView, conflicts: Vec<ConflictView>) -> ClientSync {
        ClientSync {
            sync: status.sync.clone(),
            unsynced: status.journal.unfinished,
            queued: status.pending_edits,
            conflicts,
            refusals: status.refusals.clone(),
            config: status.config.clone(),
            drain_error: status.drain_error.clone(),
        }
    }

    /// Is a sync running (this daemon's)?
    pub fn running(&self) -> bool {
        self.sync
            .as_ref()
            .is_some_and(|s| s.state == SyncState::Running)
    }

    /// Automatic sync is on (it is when the status does not say otherwise).
    pub fn auto(&self) -> bool {
        self.sync.as_ref().is_none_or(|s| s.auto)
    }
}
