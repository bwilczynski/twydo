//! The session history: undo and redo of applied, queued and withdrawn steps through
//! `History`, over a `Writer` and its lock. Synthetic fixture content.

mod common;

use std::fs;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};

use twydo_core::config::WriteMode;
use twydo_core::edit::{Change, Op};
use twydo_core::store::fsio::VdirLock;
use twydo_core::store::queue::{Item, Kind, Queue};
use twydo_core::store::{
    Access, Ctx, DrainOutcome, Drained, Fence, History, Step, StepState, Submitted, WriteError,
    Writer,
};

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

fn lazy(w: &Writer, wait: Duration) -> Access<'_> {
    Access::Try {
        writer: w,
        wait,
        fence: Fence::Now,
    }
}

fn undo(h: &mut History, a: Access<'_>, env: &Env) -> Result<(Submitted, Step), WriteError> {
    h.undo(a, "tui", &ctx(env, later()), &mut Vec::new())
}

fn redo(h: &mut History, a: Access<'_>, env: &Env) -> Result<(Submitted, Step), WriteError> {
    h.redo(a, "tui", &ctx(env, later()), &mut Vec::new())
}

/// Commits `op` now and records it; the journal entry id.
fn apply(env: &Env, w: &Writer, h: &mut History, name: &str, op: &Op) -> String {
    let prepared = w.plan(&env.snap(), op, now()).unwrap();
    let lock = w.lock(Duration::ZERO).unwrap().expect("lock is free");
    let id = lock.commit(prepared, &ctx(env, now())).unwrap();
    h.record(name.into(), &Submitted::Applied(id.clone()));
    id
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
    let a = lazy(&w, Duration::ZERO);
    let (got, step) = undo(&mut h, a, &env).unwrap();
    assert!(matches!(got, Submitted::Applied(_)), "{got:?}");
    assert_eq!(step.title, "rename");
    assert_eq!(read(&env, A), before);
    let (got, step) = redo(&mut h, a, &env).unwrap();
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
    let access = Access::Queue(&w, Fence::Now);
    let (got, step) = undo(&mut h, access, &env).unwrap();
    assert!(matches!(got, Submitted::Cancelled), "{got:?}");
    assert!(matches!(&step.state, StepState::Queued(i) if i.id == item.id));
    let q = Queue::open(&env.state).unwrap();
    assert!(q.is_empty().unwrap());
    let (got, step) = redo(&mut h, access, &env).unwrap();
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
    let a = lazy(&w, Duration::ZERO);
    let (got, step) = undo(&mut h, a, &env).unwrap();
    assert!(matches!(got, Submitted::Applied(_)), "{got:?}");
    assert_eq!(step.title, "first");
    assert_eq!(read(&env, A), before);
    assert!(matches!(
        undo(&mut h, a, &env),
        Err(WriteError::NothingToUndo)
    ));
}

#[test]
fn an_empty_history_says_nothing_to_undo() {
    let env = env();
    let w = writer(&env);
    let mut h = History::new();
    let access = Access::Queue(&w, Fence::Now);
    assert!(matches!(
        undo(&mut h, access, &env),
        Err(WriteError::NothingToUndo)
    ));
    assert!(matches!(
        redo(&mut h, access, &env),
        Err(WriteError::NothingToRedo)
    ));
}

#[test]
fn a_new_record_clears_the_redo_list() {
    let env = env();
    let w = writer(&env);
    let mut h = History::new();
    apply(&env, &w, &mut h, "first", &title(A, "first"));
    let a = lazy(&w, Duration::ZERO);
    undo(&mut h, a, &env).unwrap();
    apply(&env, &w, &mut h, "second", &title(E, "second"));
    assert!(matches!(
        redo(&mut h, a, &env),
        Err(WriteError::NothingToRedo)
    ));
}

#[test]
fn undo_of_a_queued_step_does_not_wait_for_a_held_lock() {
    let env = env();
    let w = writer(&env);
    let mut h = History::new();
    let _sync = VdirLock::exclusive(&env.state).unwrap();
    queue(&env, &w, &mut h, "rename", &title(A, "Queued"));
    let started = Instant::now();
    let (got, _) = undo(&mut h, lazy(&w, Duration::from_secs(2)), &env).unwrap();
    assert!(matches!(got, Submitted::Cancelled), "{got:?}");
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn undo_of_a_queued_step_a_drain_already_committed_reverts_its_entry() {
    let env = env();
    let w = writer(&env);
    let mut h = History::new();
    let before = read(&env, A);
    let sync = VdirLock::exclusive(&env.state).unwrap();
    let item = queue(&env, &w, &mut h, "rename", &title(A, "Queued"));
    drop(sync);
    let lock = w.lock(Duration::ZERO).unwrap().unwrap();
    lock.drain(&ctx(&env, now())).unwrap();
    drop(lock);
    assert!(read(&env, A).contains("SUMMARY:Queued"));
    let (got, step) = undo(&mut h, lazy(&w, Duration::ZERO), &env).unwrap();
    assert!(matches!(got, Submitted::Applied(_)), "{got:?}");
    assert!(matches!(&step.state, StepState::Queued(i) if i.id == item.id));
    assert_eq!(read(&env, A), before);
}

#[test]
fn redo_of_a_cancelled_step_with_the_lock_free_applies_it_and_reports_the_drain() {
    let env = env();
    let w = writer(&env);
    let mut h = History::new();
    let sync = VdirLock::exclusive(&env.state).unwrap();
    queue(&env, &w, &mut h, "rename", &title(A, "Queued"));
    undo(&mut h, Access::Queue(&w, Fence::Now), &env).unwrap();
    drop(sync);
    let mut drained: Vec<Drained> = Vec::new();
    let a = lazy(&w, Duration::ZERO);
    let (got, step) = h.redo(a, "tui", &ctx(&env, later()), &mut drained).unwrap();
    assert!(matches!(got, Submitted::Applied(_)), "{got:?}");
    assert!(matches!(step.state, StepState::Cancelled(_)));
    assert!(read(&env, A).contains("SUMMARY:Queued"));
    assert_eq!(drained.len(), 1);
    assert!(matches!(drained[0].outcome, DrainOutcome::Applied(_)));
}

#[test]
fn undo_of_an_applied_step_while_the_lock_is_held_queues_a_revert() {
    let env = env();
    let w = writer(&env);
    let mut h = History::new();
    apply(&env, &w, &mut h, "rename", &title(A, "Renamed"));
    let edited = read(&env, A);
    let _sync = VdirLock::exclusive(&env.state).unwrap();
    let (got, _) = undo(&mut h, lazy(&w, Duration::from_millis(50)), &env).unwrap();
    let Submitted::Queued(item) = got else {
        panic!("{got:?}");
    };
    assert!(matches!(item.kind, Kind::Revert { .. }));
    assert_eq!(read(&env, A), edited);
    assert_eq!(Queue::open(&env.state).unwrap().len().unwrap(), 1);
}

#[test]
fn forget_applied_drops_the_step_naming_that_entry() {
    let env = env();
    let w = writer(&env);
    let mut h = History::new();
    let id = apply(&env, &w, &mut h, "rename", &title(A, "Renamed"));
    h.forget_applied(&id);
    assert!(matches!(
        undo(&mut h, lazy(&w, Duration::ZERO), &env),
        Err(WriteError::NothingToUndo)
    ));
}
