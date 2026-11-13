//! The session history: undo and redo of applied, queued and withdrawn steps through
//! `History`, over a `Writer` and its lock. Synthetic fixture content.

mod common;

use std::fs;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};

use twydo_core::config::WriteMode;
use twydo_core::edit::{Change, Op};
use twydo_core::store::fsio::VdirLock;
use twydo_core::store::queue::{Item, Kind, Queue};
use twydo_core::store::{Access, Ctx, Fence, History, StepState, Submitted, WriteError, Writer};

use common::Env;

const A: &str = "0a04f22aeb294b719633c978fe1975e5";
const E: &str = "44123b09407c4ea1adae8f2ecdb64d49";

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 4, 15, 0, 0).unwrap()
}

fn later() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 4, 15, 5, 0).unwrap()
}

fn env() -> Env {
    Env::new(WriteMode::Test, "complete/standalone-lifecycle/edit/list1")
}

fn writer(env: &Env) -> Writer {
    Writer::open(&env.cfg, common::matrix("unverified", &[])).unwrap()
}

fn ctx(env: &Env, now: DateTime<Utc>) -> Ctx<'_> {
    Ctx {
        now,
        request: None,
        config: &env.cfg,
    }
}

fn title(uid: &str, text: &str) -> Op {
    Op::Edit {
        uid: uid.into(),
        changes: vec![Change::Summary(text.into())],
    }
}

fn read(env: &Env, uid: &str) -> String {
    fs::read_to_string(env.file(uid)).unwrap()
}

/// Commits `op` now and records it.
fn apply(env: &Env, w: &Writer, h: &mut History, name: &str, op: &Op) {
    let prepared = w.plan(&env.snap(), op, now()).unwrap();
    let lock = w.lock(Duration::ZERO).unwrap().expect("lock is free");
    let id = lock.commit(prepared, &ctx(env, now())).unwrap();
    h.record(name.into(), &Submitted::Applied(id));
}

/// Queues `op` while `_sync` holds the lock and records it.
fn queue(env: &Env, w: &Writer, h: &mut History, name: &str, op: &Op) -> Item {
    let prepared = w.plan(&env.snap(), op, now()).unwrap();
    let kind = Kind::Op {
        op: op.clone(),
        prepared,
    };
    let item = w
        .enqueue(kind, "edit", "op", Fence::Now, &ctx(env, now()))
        .unwrap();
    h.record(name.into(), &Submitted::Queued(item.clone()));
    item
}

#[test]
fn undo_then_redo_of_an_applied_step_round_trips_the_file() {
    let env = env();
    let w = writer(&env);
    let mut h = History::new();
    let before = read(&env, A);
    apply(&env, &w, &mut h, "rename", &title(A, "Renamed"));
    let edited = read(&env, A);
    assert_ne!(edited, before);
    let lock = w.lock(Duration::ZERO).unwrap().unwrap();
    let c = ctx(&env, later());
    let (got, step) = h.undo(Access::Locked(&lock), "tui", &c).unwrap();
    assert!(matches!(got, Submitted::Applied(_)), "{got:?}");
    assert_eq!(step.title, "rename");
    assert_eq!(read(&env, A), before);
    let (got, step) = h.redo(Access::Locked(&lock), "tui", &c).unwrap();
    assert!(matches!(got, Submitted::Applied(_)), "{got:?}");
    assert_eq!(step.title, "rename");
    assert_eq!(read(&env, A), edited);
}

#[test]
fn undo_of_a_queued_step_cancels_it_and_redo_queues_it_again_under_a_new_id() {
    let env = env();
    let w = writer(&env);
    let mut h = History::new();
    let before = read(&env, A);
    let sync = VdirLock::exclusive(&env.state).unwrap();
    let item = queue(&env, &w, &mut h, "rename", &title(A, "Queued"));
    let c = ctx(&env, later());
    let access = Access::Queue(&w, Fence::Now);
    let (got, step) = h.undo(access, "tui", &c).unwrap();
    assert!(matches!(got, Submitted::Cancelled), "{got:?}");
    assert!(matches!(&step.state, StepState::Queued(i) if i.id == item.id));
    let q = Queue::open(&env.state).unwrap();
    assert!(q.is_empty().unwrap());
    let (got, step) = h.redo(access, "tui", &c).unwrap();
    let Submitted::Queued(again) = got else {
        panic!("{got:?}");
    };
    assert!(matches!(&step.state, StepState::Cancelled(i) if i.id == item.id));
    assert_ne!(again.id, item.id);
    assert_eq!(again.created, item.created);
    assert_eq!(again.label, item.label);
    let queued = q.list().unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].id, again.id);
    drop(sync);
    assert_eq!(read(&env, A), before);
}

#[test]
fn a_refused_queued_step_is_skipped_and_the_step_before_it_is_undone() {
    let env = env();
    let w = writer(&env);
    let mut h = History::new();
    let before = read(&env, A);
    apply(&env, &w, &mut h, "first", &title(A, "first"));
    let sync = VdirLock::exclusive(&env.state).unwrap();
    let item = queue(&env, &w, &mut h, "second", &title(E, "second"));
    drop(sync);
    // A drain refused the queued edit: it was never written.
    let q = Queue::open(&env.state).unwrap();
    assert!(q.claim(&item.id).unwrap());
    q.refuse(&item, "changed elsewhere", false, now()).unwrap();
    let lock = w.lock(Duration::ZERO).unwrap().unwrap();
    let (got, step) = h
        .undo(Access::Locked(&lock), "tui", &ctx(&env, later()))
        .unwrap();
    assert!(matches!(got, Submitted::Applied(_)), "{got:?}");
    assert_eq!(step.title, "first");
    assert_eq!(read(&env, A), before);
    assert!(matches!(
        h.undo(Access::Locked(&lock), "tui", &ctx(&env, later())),
        Err(WriteError::NothingToUndo)
    ));
}

#[test]
fn an_empty_history_says_nothing_to_undo() {
    let env = env();
    let w = writer(&env);
    let mut h = History::new();
    let access = Access::Queue(&w, Fence::Now);
    let c = ctx(&env, now());
    assert!(matches!(
        h.undo(access, "tui", &c),
        Err(WriteError::NothingToUndo)
    ));
    assert!(matches!(
        h.redo(access, "tui", &c),
        Err(WriteError::NothingToRedo)
    ));
}

#[test]
fn a_new_record_clears_the_redo_list() {
    let env = env();
    let w = writer(&env);
    let mut h = History::new();
    apply(&env, &w, &mut h, "first", &title(A, "first"));
    let lock = w.lock(Duration::ZERO).unwrap().unwrap();
    let c = ctx(&env, later());
    h.undo(Access::Locked(&lock), "tui", &c).unwrap();
    drop(lock);
    apply(&env, &w, &mut h, "second", &title(E, "second"));
    let lock = w.lock(Duration::ZERO).unwrap().unwrap();
    assert!(matches!(
        h.redo(Access::Locked(&lock), "tui", &c),
        Err(WriteError::NothingToRedo)
    ));
}
