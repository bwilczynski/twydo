//! The batch waiter against a fake clock and a fake sync.

use std::cell::{Cell, RefCell};
use std::fs::{self, File, OpenOptions};
use std::rc::Rc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use twydo_core::config::{Config, SyncConfig};
use twydo_core::store::bind_vdir_state;
use twydo_core::store::journal::{Entry, Journal, Stage, Write};
use twydo_core::sync::SyncError;
use twydo_core::sync::run::{RunReport, Skip};
use twydo_core::sync::status::SyncStatus;
use twydo_core::sync::switch;
use twydo_core::sync::waiter::{Clock, WaiterExit, wait_and_sync, wait_and_sync_with};

fn start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 1, 10, 0, 0).unwrap()
}

struct Fx {
    _dir: tempfile::TempDir,
    cfg: Config,
}

impl Fx {
    fn new() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let vdir = dir.path().join("vdir");
        fs::create_dir_all(vdir.join("c1")).unwrap();
        let mut cfg = Config::with_vdir(vdir);
        cfg.state_dir = Some(dir.path().join("state"));
        cfg.sync = Some(SyncConfig {
            command: "unused".into(),
            ..SyncConfig::default()
        });
        let fx = Fx { _dir: dir, cfg };
        fs::create_dir_all(fx.state().join("sync")).unwrap();
        fx
    }

    fn state(&self) -> std::path::PathBuf {
        bind_vdir_state(&self.cfg.state_dir(), &self.cfg.vdir).unwrap()
    }
}

/// Writes an applied, unsynced journal entry created at `at`.
fn edit(cfg: &Config, id: &str, at: DateTime<Utc>) {
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let e = Entry {
        format: 1,
        id: id.into(),
        created: at,
        kind: "edit".into(),
        uids: vec!["u1".into()],
        stage: Stage::Applied,
        inverse_of: None,
        synced: None,
        holds: Vec::new(),
        family: None,
        writes: vec![Write {
            path: "c1/u1.ics".into(),
            before: None,
            after: Some("x".into()),
        }],
    };
    Journal::open(&state).unwrap().prepare(&e).unwrap();
}

/// Marks every pending entry synced at `at`.
fn confirm_all(cfg: &Config, at: DateTime<Utc>) {
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let j = Journal::open(&state).unwrap();
    for e in j.pending().unwrap() {
        j.mark_synced(&e.id, at).unwrap();
    }
}

/// What a real sync does to the state the rule reads: confirms every pending entry and
/// records the attempt (clearing any backoff).
fn fake_sync(cfg: &Config, at: DateTime<Utc>) {
    confirm_all(cfg, at);
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let mut s = SyncStatus::load(&state).unwrap();
    s.last_attempt = Some(at);
    s.next_attempt = None;
    s.save(&state).unwrap();
}

/// A clock whose `sleep_until` jumps to the target (never backwards) after running a hook.
struct FakeClock {
    now: Rc<Cell<DateTime<Utc>>>,
    sleeps: Vec<DateTime<Utc>>,
    on_sleep: Box<dyn FnMut()>,
}

impl FakeClock {
    fn new(now: Rc<Cell<DateTime<Utc>>>) -> FakeClock {
        FakeClock {
            now,
            sleeps: Vec::new(),
            on_sleep: Box::new(|| {}),
        }
    }
}

impl Clock for FakeClock {
    fn now(&mut self) -> DateTime<Utc> {
        self.now.get()
    }

    fn sleep_until(&mut self, t: DateTime<Utc>) {
        assert!(self.sleeps.len() < 20, "the waiter spins");
        self.sleeps.push(t);
        (self.on_sleep)();
        if t > self.now.get() {
            self.now.set(t);
        }
    }
}

fn clock() -> (Rc<Cell<DateTime<Utc>>>, FakeClock) {
    let now = Rc::new(Cell::new(start()));
    let c = FakeClock::new(now.clone());
    (now, c)
}

#[test]
fn nothing_unsynced_exits_without_sleeping() {
    let fx = Fx::new();
    let (_, mut c) = clock();
    let mut syncs = 0;
    let mut sync = || -> Result<RunReport, SyncError> {
        syncs += 1;
        Ok(RunReport::default())
    };
    let exit = wait_and_sync(&fx.cfg, &mut c, &mut sync).unwrap();
    assert_eq!(exit, WaiterExit::NothingDue);
    assert!(c.sleeps.is_empty());
    assert_eq!(syncs, 0);
}

#[test]
fn sleeps_to_the_deadline_then_syncs_once() {
    let fx = Fx::new();
    let oldest = start() - Duration::seconds(10);
    edit(&fx.cfg, "e1", oldest);
    edit(&fx.cfg, "e2", start());
    let (now, mut c) = clock();
    let mut syncs = 0;
    let cfg = fx.cfg.clone();
    let mut sync = || -> Result<RunReport, SyncError> {
        syncs += 1;
        fake_sync(&cfg, now.get());
        Ok(RunReport::default())
    };
    let exit = wait_and_sync(&fx.cfg, &mut c, &mut sync).unwrap();
    assert_eq!(exit, WaiterExit::NothingDue);
    assert_eq!(c.sleeps, vec![oldest + Duration::seconds(60)]);
    assert_eq!(syncs, 1);
}

#[test]
fn a_second_waiter_exits_at_once() {
    let fx = Fx::new();
    edit(&fx.cfg, "e1", start());
    let held: File = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(fx.state().join("sync/waiter.lock"))
        .unwrap();
    held.try_lock().unwrap();
    let (_, mut c) = clock();
    let mut syncs = 0;
    let mut sync = || -> Result<RunReport, SyncError> {
        syncs += 1;
        Ok(RunReport::default())
    };
    let exit = wait_and_sync(&fx.cfg, &mut c, &mut sync).unwrap();
    assert_eq!(exit, WaiterExit::AlreadyRunning);
    assert!(c.sleeps.is_empty());
    assert_eq!(syncs, 0);
}

#[test]
fn switched_off_while_sleeping_exits() {
    let fx = Fx::new();
    edit(&fx.cfg, "e1", start());
    let (_, mut c) = clock();
    let cfg = fx.cfg.clone();
    c.on_sleep = Box::new(move || switch::set(&cfg, true).unwrap());
    let mut syncs = 0;
    let mut sync = || -> Result<RunReport, SyncError> {
        syncs += 1;
        Ok(RunReport::default())
    };
    let exit = wait_and_sync(&fx.cfg, &mut c, &mut sync).unwrap();
    assert_eq!(exit, WaiterExit::SwitchedOff);
    assert_eq!(c.sleeps.len(), 1);
    assert_eq!(syncs, 0);
}

#[test]
fn timer_synced_meanwhile_exits_without_syncing() {
    let fx = Fx::new();
    edit(&fx.cfg, "e1", start());
    let (now, mut c) = clock();
    let cfg = fx.cfg.clone();
    let n = now.clone();
    c.on_sleep = Box::new(move || fake_sync(&cfg, n.get()));
    let mut syncs = 0;
    let mut sync = || -> Result<RunReport, SyncError> {
        syncs += 1;
        Ok(RunReport::default())
    };
    let exit = wait_and_sync(&fx.cfg, &mut c, &mut sync).unwrap();
    assert_eq!(exit, WaiterExit::NothingDue);
    assert_eq!(c.sleeps.len(), 1);
    assert_eq!(syncs, 0);
}

#[test]
fn edit_during_its_sync_gets_its_own_deadline() {
    let fx = Fx::new();
    edit(&fx.cfg, "e1", start());
    let (now, mut c) = clock();
    let cfg = fx.cfg.clone();
    let mut syncs = 0;
    let mut sync = || -> Result<RunReport, SyncError> {
        syncs += 1;
        let at = now.get();
        fake_sync(&cfg, at);
        if syncs == 1 {
            // Made while that sync ran, after it began: not covered by it.
            edit(&cfg, "e2", at + Duration::seconds(1));
        }
        Ok(RunReport::default())
    };
    let exit = wait_and_sync(&fx.cfg, &mut c, &mut sync).unwrap();
    assert_eq!(exit, WaiterExit::NothingDue);
    let first = start() + Duration::seconds(60);
    assert_eq!(c.sleeps, vec![first, first + Duration::seconds(61)]);
    assert_eq!(syncs, 2);
}

#[test]
fn edit_landing_at_exit_is_picked_up() {
    let fx = Fx::new();
    let (now, mut c) = clock();
    let cfg = fx.cfg.clone();
    let mut syncs = 0;
    let sync_cfg = fx.cfg.clone();
    let n = now.clone();
    let mut sync = || -> Result<RunReport, SyncError> {
        syncs += 1;
        fake_sync(&sync_cfg, n.get());
        Ok(RunReport::default())
    };
    let unlocks = RefCell::new(0);
    let mut after_unlock = || {
        *unlocks.borrow_mut() += 1;
        if *unlocks.borrow() == 1 {
            edit(&cfg, "late", now.get());
        }
    };
    let exit = wait_and_sync_with(&fx.cfg, &mut c, &mut sync, &mut after_unlock).unwrap();
    assert_eq!(exit, WaiterExit::NothingDue);
    assert_eq!(c.sleeps, vec![start() + Duration::seconds(60)]);
    assert_eq!(syncs, 1);
    assert_eq!(*unlocks.borrow(), 2);
}

#[test]
fn backoff_is_honoured() {
    let fx = Fx::new();
    edit(&fx.cfg, "e1", start());
    let mut s = SyncStatus::load(&fx.state()).unwrap();
    s.next_attempt = Some(start() + Duration::minutes(10));
    s.save(&fx.state()).unwrap();
    let (now, mut c) = clock();
    let cfg = fx.cfg.clone();
    let mut syncs = 0;
    let mut sync = || -> Result<RunReport, SyncError> {
        syncs += 1;
        fake_sync(&cfg, now.get());
        Ok(RunReport::default())
    };
    let exit = wait_and_sync(&fx.cfg, &mut c, &mut sync).unwrap();
    assert_eq!(exit, WaiterExit::NothingDue);
    assert_eq!(c.sleeps, vec![start() + Duration::minutes(10)]);
    assert_eq!(syncs, 1);
}

#[test]
fn busy_runner_is_retried_after_a_pause() {
    let fx = Fx::new();
    edit(&fx.cfg, "e1", start());
    let (now, mut c) = clock();
    let cfg = fx.cfg.clone();
    let mut syncs = 0;
    let mut sync = || -> Result<RunReport, SyncError> {
        syncs += 1;
        if syncs == 1 {
            return Ok(RunReport {
                skipped: Some(Skip::Busy),
                ..RunReport::default()
            });
        }
        fake_sync(&cfg, now.get());
        Ok(RunReport::default())
    };
    let exit = wait_and_sync(&fx.cfg, &mut c, &mut sync).unwrap();
    assert_eq!(exit, WaiterExit::NothingDue);
    let deadline = start() + Duration::seconds(60);
    assert_eq!(c.sleeps, vec![deadline, deadline + Duration::seconds(5)]);
    assert_eq!(syncs, 2);
}
