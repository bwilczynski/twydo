//! The scheduler (daemon spec §8): the startup sync, the periodic sync, the batched sync after
//! edits, and catching up after the machine slept. One thread decides with [`next`] and posts
//! background syncs to the job runner, which skips them while switched off or backing off.
//!
//! The thread never waits longer than [`MAX_WAIT`] and re-reads the clock on every wake, so a
//! laptop that wakes from sleep syncs within that time. Every wake also asks for a queue drain
//! when items wait and no long job holds the lane: a drain that gave up because another process
//! held the vdir ([`twydo_core::store::WriteError::Busy`]) has no other later trigger.

use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread::JoinHandle;
use std::time::Duration;

use chrono::{DateTime, Utc};
use twydo_core::sync::batch::{self, Due};

use super::Shared;
use crate::protocol::sync::SyncReason;

/// The longest the scheduler waits before it looks at the clock again.
pub const MAX_WAIT: Duration = Duration::from_secs(5);

/// How long to wait after `batch::due` failed before trying again.
const ERROR_WAIT: Duration = Duration::from_secs(5);

/// The scheduler's idea of now; tests replace it.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// The real clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// What the scheduler remembers between wakes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedState {
    /// The startup sync has not been posted yet.
    pub startup_pending: bool,
    /// When the periodic sync is next due.
    pub next_periodic: DateTime<Utc>,
}

/// What the scheduler does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    Sync(SyncReason),
    Wait(DateTime<Utc>),
}

/// The rule: the startup sync first, then a due batch, then the periodic sync; otherwise wait
/// for the earlier of the periodic sync and the batch deadline.
pub fn next(s: &SchedState, batch: Due, now: DateTime<Utc>) -> Next {
    if s.startup_pending {
        return Next::Sync(SyncReason::Startup);
    }
    if batch == Due::Now {
        return Next::Sync(SyncReason::Batch);
    }
    if now >= s.next_periodic {
        return Next::Sync(SyncReason::Periodic);
    }
    match batch {
        Due::At(t) => Next::Wait(t.min(s.next_periodic)),
        _ => Next::Wait(s.next_periodic),
    }
}

/// `now` plus `d`; an overflow means no deadline (the end of time).
fn later(now: DateTime<Utc>, d: chrono::Duration) -> DateTime<Utc> {
    now.checked_add_signed(d)
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}

struct Inner {
    state: SchedState,
    poked: bool,
    stopped: bool,
}

struct Core {
    inner: Mutex<Inner>,
    cv: Condvar,
    clock: Arc<dyn Clock>,
    periodic: chrono::Duration,
}

impl Core {
    /// Posts a background sync for `reason` and moves the periodic sync `periodic` past now.
    fn post(&self, shared: &Shared, reason: SyncReason) {
        shared.runner.background_sync(reason);
        let now = self.clock.now();
        let mut g = self.inner.lock().unwrap();
        g.state.next_periodic = later(now, self.periodic);
    }
}

/// The scheduler thread and its controls.
pub struct Scheduler {
    core: Arc<Core>,
    shared: Weak<Shared>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl Scheduler {
    /// Starts the thread. The startup sync is posted at once.
    pub fn start(shared: Weak<Shared>, clock: Arc<dyn Clock>, periodic: Duration) -> Scheduler {
        let periodic = chrono::Duration::from_std(periodic).unwrap_or(chrono::Duration::MAX);
        let now = clock.now();
        let core = Arc::new(Core {
            inner: Mutex::new(Inner {
                state: SchedState {
                    startup_pending: true,
                    next_periodic: later(now, periodic),
                },
                poked: false,
                stopped: false,
            }),
            cv: Condvar::new(),
            clock,
            periodic,
        });
        let handle = {
            let core = Arc::clone(&core);
            let shared = shared.clone();
            std::thread::spawn(move || run(&core, &shared))
        };
        Scheduler {
            core,
            shared,
            handle: Mutex::new(Some(handle)),
        }
    }

    /// Wakes the thread to decide again (after a write, at the end of a sync).
    pub fn poke(&self) {
        let mut g = self.core.inner.lock().unwrap();
        g.poked = true;
        self.core.cv.notify_all();
    }

    /// Posts a background sync for `reason` now.
    pub fn sync_now(&self, reason: SyncReason) {
        if self.core.inner.lock().unwrap().stopped {
            return;
        }
        if let Some(shared) = self.shared.upgrade() {
            self.core.post(&shared, reason);
        }
        self.poke();
    }

    /// The scheduler's now.
    pub fn now(&self) -> DateTime<Utc> {
        self.core.clock.now()
    }

    /// Stops the thread and waits for it. Idempotent.
    pub fn stop(&self) {
        {
            let mut g = self.core.inner.lock().unwrap();
            g.stopped = true;
            self.core.cv.notify_all();
        }
        if let Some(h) = self.handle.lock().unwrap().take() {
            let _ = h.join();
        }
    }
}

fn run(core: &Core, shared: &Weak<Shared>) {
    loop {
        let wait = {
            let Some(shared) = shared.upgrade() else {
                return;
            };
            if shared.is_stopping() {
                return;
            }
            // A drain that gave up on a busy vdir is retried here.
            if shared.lane.current_long().is_none() && shared.queue_has_items() {
                shared.request_drain();
            }
            let now = core.clock.now();
            match batch::due(&shared.cfg, now) {
                Err(e) => {
                    eprintln!("tdo daemon: cannot tell whether a sync is due: {e}");
                    ERROR_WAIT
                }
                Ok(due) => {
                    let state = core.inner.lock().unwrap().state;
                    match next(&state, due, now) {
                        Next::Sync(reason) => {
                            core.post(&shared, reason);
                            core.inner.lock().unwrap().state.startup_pending = false;
                            // The sync's end pokes; until then there is nothing to decide.
                            MAX_WAIT
                        }
                        Next::Wait(t) => (t - now).to_std().unwrap_or_default().min(MAX_WAIT),
                    }
                }
            }
        };
        let g = core.inner.lock().unwrap();
        let (mut g, _) = core
            .cv
            .wait_timeout_while(g, wait, |i| !i.poked && !i.stopped)
            .unwrap();
        if g.stopped {
            return;
        }
        g.poked = false;
    }
}

impl Shared {
    /// Wakes the scheduler, if it runs.
    pub fn poke_scheduler(&self) {
        if let Some(s) = self.scheduler.lock().unwrap().as_ref() {
            s.poke();
        }
    }

    /// Posts a background sync for `reason` now, if the scheduler runs.
    pub fn sync_now(&self, reason: SyncReason) {
        if let Some(s) = self.scheduler.lock().unwrap().as_ref() {
            s.sync_now(reason);
        }
    }

    /// Starts the scheduler unless the daemon is stopping (checked under the slot's lock,
    /// which teardown takes after stop is set, so a scheduler never starts after teardown
    /// looked).
    pub(super) fn start_scheduler(self: &Arc<Self>, clock: Arc<dyn Clock>, periodic: Duration) {
        let mut slot = self.scheduler.lock().unwrap();
        if self.is_stopping() || slot.is_some() {
            return;
        }
        *slot = Some(Scheduler::start(Arc::downgrade(self), clock, periodic));
    }

    /// Stops the scheduler (teardown, before the runner is closed).
    pub(super) fn stop_scheduler(&self) {
        let s = self.scheduler.lock().unwrap().take();
        if let Some(s) = s {
            s.stop();
        }
    }

    /// What follows switching automatic sync on: a background sync at once when edits are
    /// unsynced (or when that cannot be told).
    pub fn after_switch_on(self: &Arc<Self>) {
        let Some(now) = self.scheduler.lock().unwrap().as_ref().map(Scheduler::now) else {
            return;
        };
        if !matches!(batch::due(&self.cfg, now), Ok(Due::Nothing)) {
            self.sync_now(SyncReason::SwitchOn);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000 + secs, 0).unwrap()
    }

    fn state(startup_pending: bool, next_periodic: i64) -> SchedState {
        SchedState {
            startup_pending,
            next_periodic: t(next_periodic),
        }
    }

    #[test]
    fn startup_comes_first() {
        assert_eq!(
            next(&state(true, 0), Due::Now, t(10)),
            Next::Sync(SyncReason::Startup)
        );
    }

    #[test]
    fn a_due_batch_comes_before_the_periodic_sync() {
        assert_eq!(
            next(&state(false, 0), Due::Now, t(10)),
            Next::Sync(SyncReason::Batch)
        );
    }

    #[test]
    fn periodic_when_due() {
        assert_eq!(
            next(&state(false, 60), Due::Nothing, t(60)),
            Next::Sync(SyncReason::Periodic)
        );
        assert_eq!(
            next(&state(false, 60), Due::At(t(100)), t(61)),
            Next::Sync(SyncReason::Periodic)
        );
    }

    #[test]
    fn waits_for_the_earlier_of_periodic_and_the_batch_deadline() {
        assert_eq!(
            next(&state(false, 60), Due::At(t(30)), t(10)),
            Next::Wait(t(30))
        );
        assert_eq!(
            next(&state(false, 60), Due::At(t(90)), t(10)),
            Next::Wait(t(60))
        );
    }

    #[test]
    fn nothing_due_waits_for_periodic() {
        assert_eq!(
            next(&state(false, 60), Due::Nothing, t(10)),
            Next::Wait(t(60))
        );
    }
}
