//! The batching rule: when an automatic sync is due, given the unsynced edits.

use std::io;

use chrono::{DateTime, Duration, Utc};

use super::status::SyncStatus;
use super::switch;
use crate::config::Config;
use crate::store::existing_vdir_state;
use crate::store::journal::Journal;

/// When the next automatic sync should run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    /// No unsynced edits.
    Nothing,
    /// Run now.
    Now,
    /// Run at this time, unless something makes it due sooner.
    At(DateTime<Utc>),
}

/// How long edits must pause, once `batch_size` are waiting, before the batch syncs: a burst of
/// edits is not interrupted by its own sync (which holds the vdir while it runs).
pub const QUIET: Duration = Duration::seconds(3);

/// Pure rule. Each edit is `(written, edited)`: when its journal entry was written and when the
/// user made the edit (the same, unless it was drained from the write queue). An edit written
/// before `last_attempt` does not count (that attempt covered it). The age and the quiet period
/// use `edited`. The deadline is `max_wait` after the oldest counted edit, pushed out by backoff
/// (`next_attempt`). At `size` edits a sync is due once the newest has been quiet for
/// [`QUIET`], unless backing off, and never later than the deadline.
pub fn decide(
    edits: &[(DateTime<Utc>, DateTime<Utc>)],
    last_attempt: Option<DateTime<Utc>>,
    next_attempt: Option<DateTime<Utc>>,
    size: u32,
    max_wait: std::time::Duration,
    now: DateTime<Utc>,
) -> Due {
    let counted: Vec<DateTime<Utc>> = edits
        .iter()
        .filter(|(written, _)| last_attempt.is_none_or(|l| *written >= l))
        .map(|(_, edited)| *edited)
        .collect();
    let (Some(oldest), Some(newest)) =
        (counted.iter().min().copied(), counted.iter().max().copied())
    else {
        return Due::Nothing;
    };
    let wait = Duration::from_std(max_wait).unwrap_or(Duration::MAX);
    let mut deadline = oldest
        .checked_add_signed(wait)
        .unwrap_or(DateTime::<Utc>::MAX_UTC);
    if let Some(n) = next_attempt {
        deadline = deadline.max(n);
    }
    let backing_off = next_attempt.is_some_and(|n| n > now);
    if counted.len() >= size as usize && !backing_off {
        let quiet = newest
            .checked_add_signed(QUIET)
            .unwrap_or(DateTime::<Utc>::MAX_UTC);
        deadline = deadline.min(quiet);
    }
    if now >= deadline {
        return Due::Now;
    }
    Due::At(deadline)
}

/// [`decide`] over the journal's unsynced entries and the sync status. `Nothing` without
/// `[sync]`, when switched off, or when there is no state yet. Creates nothing.
pub fn due(cfg: &Config, now: DateTime<Utc>) -> io::Result<Due> {
    let Some(sync) = &cfg.sync else {
        return Ok(Due::Nothing);
    };
    if switch::is_off(cfg)? {
        return Ok(Due::Nothing);
    }
    let Some(state) = existing_vdir_state(&cfg.state_dir(), &cfg.vdir)? else {
        return Ok(Due::Nothing);
    };
    if !state.join("journal").is_dir() {
        return Ok(Due::Nothing);
    }
    let edits: Vec<(DateTime<Utc>, DateTime<Utc>)> = Journal::open(&state)?
        .pending()?
        .iter()
        .map(|e| {
            (
                e.created,
                e.from_queue.as_ref().map_or(e.created, |q| q.queued),
            )
        })
        .collect();
    let status = SyncStatus::load(&state)?;
    Ok(decide(
        &edits,
        status.last_attempt,
        status.next_attempt,
        sync.batch_size,
        std::time::Duration::from_secs(sync.max_wait_secs),
        now,
    ))
}

/// What a writer starts after an edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// The batch is full or overdue: start a background sync now.
    SyncNow,
    /// Make sure a batch waiter exists (`tdo sync --background --when-due`).
    Waiter,
}

/// Called after a write that succeeded: what to start, if anything. `None` without `[sync]`,
/// while switched off, or with nothing unsynced. A read error starts the waiter, which checks
/// again itself.
pub fn after_write(cfg: &Config, now: DateTime<Utc>) -> Option<Trigger> {
    match due(cfg, now) {
        Ok(Due::Nothing) => None,
        Ok(Due::Now) => Some(Trigger::SyncNow),
        Ok(Due::At(_)) | Err(_) => Some(Trigger::Waiter),
    }
}

/// After a conflict was resolved: a choice recorded for a held conflict is not an edit and
/// applies only on a sync, so that starts one now (unless switched off); a resolve that
/// wrote the vdir follows [`after_write`].
pub fn after_resolve(cfg: &Config, recorded: bool, now: DateTime<Utc>) -> Option<Trigger> {
    if !recorded {
        return after_write(cfg, now);
    }
    let on = cfg.sync.is_some() && !switch::is_off(cfg).unwrap_or(false);
    on.then_some(Trigger::SyncNow)
}
