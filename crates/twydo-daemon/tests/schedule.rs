//! The scheduler: the startup sync, the periodic sync, batched syncs after edits, the switch,
//! backoff, and the retry of a queue drain that found the vdir busy. The clock is a
//! [`FakeClock`] the tests move, poking the scheduler after each move; the vdirsyncer is a
//! fake script. Synthetic fixture content.

mod common;

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use common::{FakeClock, Fixture, TestDaemon, edit_title, empty, submit, wait_event};
use twydo_core::config::{Config, SyncConfig};
use twydo_core::store::bind_vdir_state;
use twydo_core::store::fsio::VdirLock;
use twydo_daemon::client::{Client, Notification};
use twydo_daemon::protocol::events::Event;
use twydo_daemon::protocol::jobs::JobStarted;
use twydo_daemon::protocol::methods::{OpState, StatusView, WriteOutcome};
use twydo_daemon::protocol::sync::{
    SyncReason, SyncRunParams, SyncSetAutoParams, SyncSetAutoResult,
};
use twydo_daemon::server::DaemonOptions;
use twydo_daemon::server::lane::LongKind;

const BOUND: Duration = Duration::from_secs(10);
/// No sync event for this long after a step: nothing more is coming.
const QUIET: Duration = Duration::from_millis(300);

const OK: &str = "echo 'Syncing twydo/c1' >&2\nexit 0";

fn fx() -> Fixture {
    Fixture::new(&[("c1", "u1", "One", None)])
}

fn root(fx: &Fixture) -> PathBuf {
    fx.vdir().parent().unwrap().to_path_buf()
}

/// A daemon with the scheduler on, a subscribed client, and the clock.
struct Sched {
    td: TestDaemon,
    c: Client,
    clock: Arc<FakeClock>,
    calls: PathBuf,
}

/// A config whose fake vdirsyncer appends a line to `calls` per `sync`, waits for `gate` (so
/// the startup sync cannot end before the test has subscribed), then runs `body`.
fn sync_config(fx: &Fixture, body: &str, tune: impl FnOnce(&mut SyncConfig)) -> Config {
    let calls = root(fx).join("calls");
    let gate = root(fx).join("gate");
    let script = format!(
        "[ \"$1\" = sync ] || exit 0\necho sync >> '{}'\n\
         while [ ! -e '{}' ]; do sleep 0.01; done\n{body}",
        calls.display(),
        gate.display()
    );
    let mut cfg = fx.with_sync(&script);
    tune(cfg.sync.as_mut().unwrap());
    cfg
}

fn start(fx: &Fixture, body: &str, tune: impl FnOnce(&mut SyncConfig)) -> Sched {
    start_with(fx, body, tune, |_| {})
}

/// Starts, subscribes, opens the gate and waits for the startup sync to finish.
fn start_with(
    fx: &Fixture,
    body: &str,
    tune: impl FnOnce(&mut SyncConfig),
    f: impl FnOnce(&mut DaemonOptions),
) -> Sched {
    let cfg = sync_config(fx, body, tune);
    let clock = FakeClock::new();
    let td = TestDaemon::start_scheduled_with(fx, cfg, Arc::clone(&clock), f);
    let c = td.client();
    let _: serde_json::Value = c.call("events.subscribe", empty()).unwrap();
    fs::write(root(fx).join("gate"), "").unwrap();
    wait_event(&c, BOUND, |e| {
        matches!(
            e,
            Event::SyncFinished {
                reason: SyncReason::Startup,
                ..
            }
        )
    });
    Sched {
        td,
        c,
        clock,
        calls: root(fx).join("calls"),
    }
}

impl Sched {
    fn calls(&self) -> usize {
        fs::read_to_string(&self.calls)
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }

    /// The reasons of the syncs that start until none has run for [`QUIET`].
    fn syncs(&self) -> Vec<SyncReason> {
        let mut started = Vec::new();
        let mut running = 0usize;
        let mut quiet_from = Instant::now();
        let end = Instant::now() + BOUND;
        loop {
            let now = Instant::now();
            assert!(now < end, "syncs did not settle: {started:?}");
            if running == 0 && now >= quiet_from + QUIET {
                return started;
            }
            let left = if running == 0 {
                (quiet_from + QUIET).saturating_duration_since(now)
            } else {
                end.saturating_duration_since(now)
            };
            match self.c.notifications().recv_timeout(left) {
                Ok(Notification::Event(ev)) => match ev.event {
                    Event::SyncStarted { reason, .. } => {
                        started.push(reason);
                        running += 1;
                    }
                    Event::SyncFinished { .. } => {
                        running = running.saturating_sub(1);
                        quiet_from = Instant::now();
                    }
                    _ => {}
                },
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(e) => panic!("events ended: {e}"),
            }
        }
    }

    /// Moves the clock by `d`, pokes the scheduler and returns the syncs that followed.
    fn step(&self, d: Duration) -> Vec<SyncReason> {
        self.clock.advance(d);
        self.td.handle().poke_scheduler();
        self.syncs()
    }

    fn set_auto(&self, on: bool) {
        let r: SyncSetAutoResult = self
            .c
            .call(
                "sync.set_auto",
                SyncSetAutoParams {
                    request_id: self.c.next_request_id(),
                    on,
                },
            )
            .unwrap();
        assert_eq!(r.auto, on);
    }

    fn edit(&self, title: &str) {
        let r = submit(&self.c, edit_title("u1", title)).unwrap();
        assert_eq!(r.outcome, WriteOutcome::Applied);
    }
}

#[test]
fn startup_sync_runs_once() {
    let fx = fx();
    let s = start(&fx, OK, |_| {});
    // The startup sync's `sync.finished` arrived in `start`; nothing follows it.
    assert_eq!(s.calls(), 1);
    s.td.handle().poke_scheduler();
    assert_eq!(s.syncs(), Vec::<SyncReason>::new());
    assert_eq!(s.calls(), 1);
}

#[test]
fn periodic_sync_every_interval() {
    let fx = fx();
    let s = start(&fx, OK, |_| {});
    assert_eq!(s.step(Duration::from_secs(61)), vec![SyncReason::Periodic]);
    assert_eq!(s.step(Duration::from_secs(30)), Vec::<SyncReason>::new());
    assert_eq!(s.calls(), 2);
}

#[test]
fn clock_jump_runs_one_periodic_sync() {
    let fx = fx();
    let s = start(&fx, OK, |_| {});
    assert_eq!(
        s.step(Duration::from_secs(2 * 3600)),
        vec![SyncReason::Periodic]
    );
    assert_eq!(s.calls(), 2);
}

#[test]
fn batch_size_reached_syncs_after_the_quiet_pause() {
    let fx = fx();
    let s = start(&fx, OK, |c| c.batch_size = 2);
    s.edit("First");
    s.edit("Second");
    assert_eq!(s.syncs(), Vec::<SyncReason>::new(), "not before the pause");
    assert_eq!(s.step(Duration::from_secs(4)), vec![SyncReason::Batch]);
}

#[test]
fn max_wait_syncs_a_single_edit() {
    let fx = fx();
    let s = start(&fx, OK, |c| c.max_wait_secs = 90);
    s.edit("Only");
    assert_eq!(s.step(Duration::from_secs(30)), Vec::<SyncReason>::new());
    // Past the periodic sync too: the batch goes first and covers it.
    assert_eq!(s.step(Duration::from_secs(61)), vec![SyncReason::Batch]);
}

#[test]
fn switched_off_runs_no_background_sync() {
    let fx = fx();
    let s = start(&fx, OK, |_| {});
    s.set_auto(false);
    s.edit("Offline");
    assert_eq!(s.step(Duration::from_secs(120)), Vec::<SyncReason>::new());
    assert_eq!(s.calls(), 1);
    // A manual sync still runs.
    let started: JobStarted =
        s.c.call(
            "sync.run",
            SyncRunParams {
                request_id: s.c.next_request_id(),
                force_delete: Vec::new(),
            },
        )
        .unwrap();
    let id = started.job_id;
    wait_event(
        &s.c,
        BOUND,
        |e| matches!(e, Event::SyncFinished { job_id: Some(j), reason: SyncReason::Manual, .. } if *j == id),
    );
    assert_eq!(s.calls(), 2);
}

#[test]
fn switch_on_with_unsynced_edits_syncs_at_once() {
    let fx = fx();
    let s = start(&fx, OK, |_| {});
    s.set_auto(false);
    s.edit("Offline");
    s.set_auto(true);
    assert_eq!(s.syncs(), vec![SyncReason::SwitchOn]);
}

#[test]
fn switch_on_with_nothing_unsynced_does_not_sync() {
    let fx = fx();
    let s = start(&fx, OK, |_| {});
    s.set_auto(false);
    s.set_auto(true);
    assert_eq!(s.syncs(), Vec::<SyncReason>::new());
}

#[test]
fn failing_batch_sync_backs_off_instead_of_looping() {
    let fx = fx();
    // The startup sync succeeds; every later one fails.
    let calls = root(&fx).join("calls");
    let body = format!(
        "[ \"$(wc -l < '{}')\" -le 1 ] && {{ echo 'Syncing twydo/c1' >&2; exit 0; }}\n\
         echo 'Syncing twydo/c1' >&2\necho 'error: synthetic failure' >&2\nexit 1",
        calls.display()
    );
    let s = start(&fx, &body, |c| c.batch_size = 1);
    s.edit("Fails");
    assert_eq!(s.step(Duration::from_secs(4)), vec![SyncReason::Batch]);
    let st: StatusView = s.c.call("status.get", empty()).unwrap();
    let next = st.sync.and_then(|v| v.next_attempt);
    assert!(
        next.is_some_and(|t| t > chrono::Utc::now()),
        "next_attempt {next:?}"
    );
    assert_eq!(s.step(Duration::from_secs(10)), Vec::<SyncReason>::new());
    assert_eq!(s.calls(), 2);
}

#[test]
fn busy_drain_is_retried_by_the_scheduler() {
    let fx = fx();
    let (tx, rx) = mpsc::channel::<()>();
    let tx = std::sync::Mutex::new(tx);
    let s = start_with(
        &fx,
        OK,
        |_| {},
        move |o| {
            o.after_drain_passes = Some(Arc::new(move || {
                let _ = tx.lock().unwrap().send(());
            }));
        },
    );
    while rx.try_recv().is_ok() {}
    let job = s.td.handle().hold_long(LongKind::Test);
    let r = submit(&s.c, edit_title("u1", "Queued")).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Queued);
    let cfg = fx.config();
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let lock = VdirLock::exclusive(&state).unwrap();
    // The job's drain hook and the drain after the lane's release both find the vdir busy and
    // give up; nothing else would drain the item.
    drop(job);
    for _ in 0..2 {
        rx.recv_timeout(BOUND).expect("a drain gave up");
    }
    drop(lock);
    s.clock.advance(Duration::from_secs(5));
    s.td.handle().poke_scheduler();
    let op = r.op_id.clone();
    let ev = wait_event(
        &s.c,
        BOUND,
        |e| matches!(e, Event::OpFinished { op_id, .. } if *op_id == op),
    );
    let Event::OpFinished { state, .. } = ev else {
        unreachable!()
    };
    assert!(matches!(state, OpState::Applied { .. }), "{state:?}");
    assert!(fx.read("c1/u1.ics").contains("SUMMARY:Queued"));
}
