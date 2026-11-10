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

    // Removed in Task 11 (the loop moves to the link).
    pub fn from_summary(s: &twydo_core::sync::SyncSummary) -> ClientSync {
        ClientSync {
            sync: Some(SyncView {
                state: SyncState::Idle,
                auto: !s.off,
                last_success: s.status.last_success,
                last_attempt: s.status.last_attempt,
                next_attempt: s.status.next_attempt,
                failures: s.status.failures,
                last_error: s.status.last_error.clone(),
                job_id: None,
                collections: s
                    .status
                    .collections
                    .iter()
                    .map(|(k, v)| (k.clone(), v.into()))
                    .collect(),
                metasync_error: s.status.metasync_error.clone(),
            }),
            unsynced: s.pending,
            queued: s.queued,
            conflicts: s.conflicts.iter().map(ConflictView::of).collect(),
            refusals: Vec::new(),
            config: ConfigStatus::default(),
            drain_error: None,
        }
    }
}
