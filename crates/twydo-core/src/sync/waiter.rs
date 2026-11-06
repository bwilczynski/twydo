//! The batch waiter (`tdo sync --background --when-due`): sleeps until the oldest unsynced
//! edit's deadline (see [`batch::due`]), then syncs, and loops while edits keep arriving. One
//! per vdir, held by `<vdir-state>/sync/waiter.lock`.

use std::io;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};

use super::SyncError;
use super::batch::{self, Due};
use super::lock::LockFile;
use super::run::{RunReport, Skip};
use super::switch;
use crate::config::Config;
use crate::store::existing_vdir_state;

/// The longest single sleep of [`SystemClock`], so that a changed wall clock (a laptop
/// waking from sleep) is noticed within this time.
const SLICE: StdDuration = StdDuration::from_secs(5);
/// Recheck the journal while waiting: later edits can bring the size deadline forward.
const RECHECK: Duration = Duration::seconds(1);
/// How long the waiter pauses after its sync found another runner busy. That runner was
/// asked to run once more and will most likely cover the edits; the pause keeps the waiter
/// from spinning until it does.
const BUSY_RETRY: Duration = Duration::seconds(5);

/// Wall time and sleeping, faked in tests.
pub trait Clock {
    fn now(&mut self) -> DateTime<Utc>;
    /// Returns at `t` or later (at once when `t` has passed).
    fn sleep_until(&mut self, t: DateTime<Utc>);
}

/// The wall clock. Sleeps in slices of at most 5 s, re-reading the clock between them.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&mut self) -> DateTime<Utc> {
        Utc::now()
    }

    fn sleep_until(&mut self, t: DateTime<Utc>) {
        loop {
            let Ok(left) = (t - Utc::now()).to_std() else {
                return;
            };
            if left.is_zero() {
                return;
            }
            std::thread::sleep(left.min(SLICE));
        }
    }
}

/// Why the waiter stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaiterExit {
    /// Another waiter holds the lock and covers the edits.
    AlreadyRunning,
    /// No unsynced edits left.
    NothingDue,
    /// Automatic sync is switched off.
    SwitchedOff,
}

/// [`wait_and_sync_with`] with no hook.
pub fn wait_and_sync(
    cfg: &Config,
    clock: &mut dyn Clock,
    sync: &mut dyn FnMut() -> Result<RunReport, SyncError>,
) -> io::Result<WaiterExit> {
    wait_and_sync_with(cfg, clock, sync, &mut || {})
}

/// The waiter's loop:
///
/// 1. Takes `<vdir-state>/sync/waiter.lock` without blocking; if another waiter holds it,
///    returns [`WaiterExit::AlreadyRunning`].
/// 2. While sync is on and something is due: rechecks at least once a second until the
///    deadline, or runs `sync`
///    when it has come, then checks again (edits made during that sync get their own
///    deadline). A sync that found another runner busy is retried after a short pause.
/// 3. Before exiting: releases the lock, calls `after_unlock` (a test hook), and checks once
///    more. If an edit landed in between, takes the lock again and loops; if another waiter
///    got it first, that one covers the edit.
///
/// An error from `sync` ends the waiter; the timer catches up.
pub fn wait_and_sync_with(
    cfg: &Config,
    clock: &mut dyn Clock,
    sync: &mut dyn FnMut() -> Result<RunReport, SyncError>,
    after_unlock: &mut dyn FnMut(),
) -> io::Result<WaiterExit> {
    if cfg.sync.is_none() {
        return Ok(WaiterExit::NothingDue);
    }
    let Some(state) = existing_vdir_state(&cfg.state_dir(), &cfg.vdir)? else {
        return Ok(WaiterExit::NothingDue);
    };
    let sync_dir = state.join("sync");
    std::fs::create_dir_all(&sync_dir)?;
    let lock_path = sync_dir.join("waiter.lock");
    let Some(mut lock) = LockFile::take(&lock_path, true)? else {
        return Ok(WaiterExit::AlreadyRunning);
    };
    loop {
        if switch::is_off(cfg)? {
            return Ok(WaiterExit::SwitchedOff);
        }
        let now = clock.now();
        match batch::due(cfg, now)? {
            Due::At(t) => {
                clock.sleep_until(t.min(now + RECHECK));
                continue;
            }
            Due::Now => {
                let report = sync().map_err(|e| match e {
                    SyncError::Io(e) => e,
                    SyncError::Spawn(m) => io::Error::other(m),
                })?;
                if report.skipped == Some(Skip::Busy) {
                    let t = clock.now() + BUSY_RETRY;
                    clock.sleep_until(t);
                }
                continue;
            }
            Due::Nothing => {}
        }
        // Step 4: exit without losing an edit.
        drop(lock);
        after_unlock();
        if switch::is_off(cfg)? {
            return Ok(WaiterExit::SwitchedOff);
        }
        if batch::due(cfg, clock.now())? == Due::Nothing {
            return Ok(WaiterExit::NothingDue);
        }
        lock = match LockFile::take(&lock_path, true)? {
            Some(l) => l,
            None => return Ok(WaiterExit::AlreadyRunning),
        };
    }
}
