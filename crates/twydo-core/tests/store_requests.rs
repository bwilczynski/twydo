//! Request records on journal entries and queue items, `set_vdir_busy` and `take_drained`:
//! the hooks the daemon needs from the store. Synthetic fixture content.

mod common;

use std::fs;
use std::time::Instant;

use chrono::{DateTime, TimeZone, Utc};

use twydo_core::compat::Matrix;
use twydo_core::config::WriteMode;
use twydo_core::edit::{Change, Op};
use twydo_core::store::fsio::VdirLock;
use twydo_core::store::journal::{Entry, Journal, RequestRef};
use twydo_core::store::queue::{Queue, Queued};
use twydo_core::store::{DrainOutcome, Submitted};

use common::Env;

const A: &str = "0a04f22aeb294b719633c978fe1975e5";
const E: &str = "44123b09407c4ea1adae8f2ecdb64d49";
const CASES: [&str; 5] = ["one-time", "monthly", "date-only", "timed", "start-date"];

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 4, 15, 0, 0).unwrap()
}

fn later() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 4, 15, 5, 0).unwrap()
}

fn matrix() -> Matrix {
    common::matrix(&CASES, "unverified", &[])
}

fn env() -> Env {
    Env::new(WriteMode::Test, "plan4-list1/02-edit")
}

fn title(uid: &str, text: &str) -> Op {
    Op::Edit {
        uid: uid.into(),
        changes: vec![Change::Summary(text.into())],
    }
}

fn req(id: &str) -> RequestRef {
    RequestRef {
        session: "s1".into(),
        request_id: id.into(),
    }
}

fn journal(env: &Env) -> Journal {
    Journal::open(&env.state).unwrap()
}

#[test]
fn an_applied_entry_records_its_request() {
    let env = env();
    let mut store = env.store(matrix());
    store.set_request(Some(req("r1")));
    let Submitted::Applied(id) = store
        .submit(&env.snap(), &title(A, "x"), "x", "op", now())
        .unwrap()
    else {
        panic!("applied")
    };
    let j = journal(&env);
    assert_eq!(j.get(&id).unwrap().unwrap().request, Some(req("r1")));
    assert_eq!(j.by_request("r1").unwrap().unwrap().id, id);
    assert!(j.by_request("other").unwrap().is_none());
}

#[test]
fn a_queued_item_and_its_drained_entry_record_the_request() {
    let env = env();
    let mut store = env.store(matrix());
    store.set_request(Some(req("r2")));
    let lock = VdirLock::exclusive(&env.state).unwrap();
    let Submitted::Queued(item) = store
        .submit(&env.snap(), &title(A, "x"), "x", "op", now())
        .unwrap()
    else {
        panic!("queued")
    };
    assert_eq!(item.request, Some(req("r2")));
    let queue = Queue::open(&env.state).unwrap();
    assert!(matches!(
        queue.state(&item.id).unwrap(),
        Some(Queued::Pending(_))
    ));
    assert!(matches!(
        queue.by_request("r2").unwrap(),
        Some(Queued::Pending(i)) if i.id == item.id
    ));
    drop(lock);
    // Another request drains it: the entry still carries the item's request.
    store.set_request(Some(req("other")));
    let drained = store.drain(later()).unwrap();
    assert_eq!(drained.len(), 1);
    let DrainOutcome::Applied(id) = &drained[0].outcome else {
        panic!("applied")
    };
    assert_eq!(
        journal(&env).get(id).unwrap().unwrap().request,
        Some(req("r2"))
    );
    assert!(queue.state(&item.id).unwrap().is_none());
}

#[test]
fn a_claimed_item_is_found_by_state() {
    let env = env();
    let mut store = env.store(matrix());
    store.set_request(Some(req("r3")));
    let _lock = VdirLock::exclusive(&env.state).unwrap();
    let Submitted::Queued(item) = store
        .submit(&env.snap(), &title(A, "x"), "x", "op", now())
        .unwrap()
    else {
        panic!("queued")
    };
    let queue = Queue::open(&env.state).unwrap();
    assert!(queue.claim(&item.id).unwrap());
    assert!(matches!(
        queue.state(&item.id).unwrap(),
        Some(Queued::Claimed(_))
    ));
    assert!(matches!(
        queue.by_request("r3").unwrap(),
        Some(Queued::Claimed(_))
    ));
}

#[test]
fn a_refused_queued_item_is_found_by_request() {
    let env = env();
    let mut store = env.store(matrix());
    store.set_request(Some(req("r4")));
    let lock = VdirLock::exclusive(&env.state).unwrap();
    let Submitted::Queued(item) = store
        .submit(&env.snap(), &title(A, "x"), "x", "op", now())
        .unwrap()
    else {
        panic!("queued")
    };
    let path = env.file(A);
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("SUMMARY:", "SUMMARY:remote ")).unwrap();
    drop(lock);
    let drained = store.drain(later()).unwrap();
    assert!(matches!(drained[0].outcome, DrainOutcome::Refused(_)));
    let queue = Queue::open(&env.state).unwrap();
    assert!(matches!(
        queue.state(&item.id).unwrap(),
        Some(Queued::Refused(r)) if r.item.request == Some(req("r4"))
    ));
    assert!(matches!(
        queue.by_request("r4").unwrap(),
        Some(Queued::Refused(_))
    ));
}

#[test]
fn undo_entries_and_queued_reverts_record_the_request() {
    let env = env();
    let mut store = env.store(matrix());
    store.set_request(Some(req("w")));
    store
        .submit(&env.snap(), &title(A, "x"), "x", "op", now())
        .unwrap();
    store.set_request(Some(req("u1")));
    let Submitted::Applied(id) = store.undo_last(now()).unwrap() else {
        panic!("applied")
    };
    assert_eq!(
        journal(&env).get(&id).unwrap().unwrap().request,
        Some(req("u1"))
    );
    // Redo while a sync holds the lock: the queued revert carries the request.
    store.set_request(Some(req("u2")));
    let lock = VdirLock::exclusive(&env.state).unwrap();
    let Submitted::Queued(item) = store.redo_last(now()).unwrap() else {
        panic!("queued")
    };
    assert_eq!(item.request, Some(req("u2")));
    drop(lock);
    store.set_request(None);
    let drained = store.drain(later()).unwrap();
    let DrainOutcome::Applied(id) = &drained[0].outcome else {
        panic!("applied")
    };
    assert_eq!(
        journal(&env).get(id).unwrap().unwrap().request,
        Some(req("u2"))
    );
}

#[test]
fn vdir_busy_queues_without_touching_the_lock() {
    let env = env();
    let mut store = env.store(matrix());
    store.set_vdir_busy(true);
    let before = fs::read(env.file(A)).unwrap();
    let started = Instant::now();
    let got = store
        .submit(&env.snap(), &title(A, "x"), "x", "op", now())
        .unwrap();
    assert!(
        started.elapsed().as_millis() < 100,
        "{:?}",
        started.elapsed()
    );
    assert!(matches!(got, Submitted::Queued(_)), "{got:?}");
    assert_eq!(fs::read(env.file(A)).unwrap(), before);
    // Undo of a still-queued step cancels without the lock.
    assert!(matches!(
        store.undo_last(now()).unwrap(),
        Submitted::Cancelled
    ));
    assert!(Queue::open(&env.state).unwrap().is_empty().unwrap());
}

#[test]
fn vdir_busy_queues_undo_of_an_applied_step() {
    let env = env();
    let mut store = env.store(matrix());
    store
        .submit(&env.snap(), &title(A, "x"), "x", "op", now())
        .unwrap();
    store.set_vdir_busy(true);
    let got = store.undo_last(now()).unwrap();
    assert!(matches!(got, Submitted::Queued(_)), "{got:?}");
}

#[test]
fn take_drained_returns_items_a_submit_drained() {
    let env = env();
    let mut store = env.store(matrix());
    store.set_collect_drained(true);
    let lock = VdirLock::exclusive(&env.state).unwrap();
    store
        .submit(&env.snap(), &title(A, "one"), "one", "op", now())
        .unwrap();
    drop(lock);
    assert!(store.take_drained().is_empty());
    let got = store
        .submit(&env.snap(), &title(E, "two"), "two", "op", later())
        .unwrap();
    assert!(matches!(got, Submitted::Applied(_)), "{got:?}");
    let drained = store.take_drained();
    assert_eq!(drained.len(), 1);
    assert!(matches!(drained[0].outcome, DrainOutcome::Applied(_)));
    assert!(store.take_drained().is_empty());
}

#[test]
fn entries_without_a_request_still_read() {
    let env = env();
    let mut store = env.store(matrix());
    store
        .submit(&env.snap(), &title(A, "x"), "x", "op", now())
        .unwrap();
    let entry = env.journal().remove(0);
    assert_eq!(entry.request, None);
    let mut json = serde_json::to_value(&entry).unwrap();
    assert!(json.get("request").is_none());
    json.as_object_mut().unwrap().remove("request");
    let back: Entry = serde_json::from_value(json).unwrap();
    assert_eq!(back, entry);
    let with = Entry {
        request: Some(req("r")),
        ..entry
    };
    let json = serde_json::to_value(&with).unwrap();
    assert_eq!(json["request"]["request_id"], "r");
    assert_eq!(json["request"]["session"], "s1");
}

#[test]
fn redo_after_a_cancelled_step_carries_the_redo_request() {
    let env = env();
    let mut store = env.store(matrix());
    store.set_vdir_busy(true);
    store.set_request(Some(req("r1")));
    store
        .submit(&env.snap(), &title(A, "x"), "x", "op", now())
        .unwrap();
    store.set_request(Some(req("r2")));
    assert!(matches!(
        store.undo_last(now()).unwrap(),
        Submitted::Cancelled
    ));
    let queue = Queue::open(&env.state).unwrap();
    // The withdrawn item stays on record for its request, and for the undo's.
    assert!(matches!(
        queue.by_request("r1").unwrap(),
        Some(Queued::Cancelled(_))
    ));
    assert!(queue.cancelled_by("r2").unwrap().is_some());
    store.set_request(Some(req("r3")));
    let Submitted::Queued(item) = store.redo_last(now()).unwrap() else {
        panic!("queued")
    };
    assert_eq!(item.request, Some(req("r3")));
    assert!(matches!(
        queue.by_request("r3").unwrap(),
        Some(Queued::Pending(_))
    ));
    assert!(matches!(
        queue.by_request("r1").unwrap(),
        Some(Queued::Cancelled(_))
    ));
    store.set_vdir_busy(false);
    let drained = store.drain(later()).unwrap();
    let DrainOutcome::Applied(id) = &drained[0].outcome else {
        panic!("applied")
    };
    assert_eq!(
        journal(&env).get(id).unwrap().unwrap().request,
        Some(req("r3"))
    );
}

#[test]
fn a_store_collects_nothing_drained_unless_asked() {
    let env = env();
    let mut store = env.store(matrix());
    let lock = VdirLock::exclusive(&env.state).unwrap();
    store
        .submit(&env.snap(), &title(A, "one"), "one", "op", now())
        .unwrap();
    drop(lock);
    let got = store
        .submit(&env.snap(), &title(E, "two"), "two", "op", later())
        .unwrap();
    assert!(matches!(got, Submitted::Applied(_)), "{got:?}");
    assert!(store.take_drained().is_empty());
}
