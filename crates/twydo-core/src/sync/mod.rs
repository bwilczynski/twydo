//! Sync runner support: vdirsyncer output parsing, conflict handling, the runner and its
//! status.

pub mod ended;
pub mod hook;
pub mod manifest;
pub mod output;
pub mod reconcile;
pub mod resolve;
pub mod run;
pub mod running;
pub mod status;

use std::io;

use crate::config::Config;
use crate::conflicts::{Conflict, ConflictStore};
use crate::store::existing_vdir_state;
use crate::store::journal::Journal;
use status::SyncStatus;

/// Why a sync run could not complete (a failed vdirsyncer is not an error: see
/// [`run::RunReport::transient`]).
#[derive(thiserror::Error, Debug)]
pub enum SyncError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("{0}")]
    Spawn(String),
}

/// What `tdo status` and the TUI show.
#[derive(Debug, Clone)]
pub struct SyncSummary {
    pub status: SyncStatus,
    /// Journal entries not yet confirmed by a sync.
    pub pending: usize,
    /// Unresolved conflicts.
    pub conflicts: Vec<Conflict>,
}

/// The sync state of `cfg`'s vdir; `None` without `[sync]`. Creates nothing: a vdir with no
/// state yet has never synced.
pub fn summary(cfg: &Config) -> io::Result<Option<SyncSummary>> {
    if cfg.sync.is_none() {
        return Ok(None);
    }
    let Some(state) = existing_vdir_state(&cfg.state_dir(), &cfg.vdir)? else {
        return Ok(Some(SyncSummary {
            status: SyncStatus::default(),
            pending: 0,
            conflicts: Vec::new(),
        }));
    };
    let pending = if state.join("journal").is_dir() {
        Journal::open(&state)?.pending()?.len()
    } else {
        0
    };
    let mut conflicts = match ConflictStore::existing(&state)? {
        Some(store) => store.unresolved()?,
        None => Vec::new(),
    };
    let archived = crate::retired::list_state(&state)?;
    conflicts.retain(|c| !archived.iter().any(|b| b.conflicts.contains(&c.id)));
    Ok(Some(SyncSummary {
        status: SyncStatus::load(&state)?,
        pending,
        conflicts,
    }))
}
