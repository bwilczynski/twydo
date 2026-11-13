//! The single vdir writer: commit, lock, queue and drain through `Writer`/`Locked`, and the
//! frozen durable forms. Synthetic fixture content.

mod common;

use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};

use twydo_core::compat::Matrix;
use twydo_core::config::WriteMode;
use twydo_core::edit::{Change, Op};
use twydo_core::store::fsio::VdirLock;
use twydo_core::store::journal::{Entry, Journal, RequestRef, Stage};
use twydo_core::store::queue::{Item, Kind, Queue};
use twydo_core::store::{Ctx, DrainOutcome, Fence, Writer};

use common::Env;

const A: &str = "0a04f22aeb294b719633c978fe1975e5";

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 4, 15, 0, 0).unwrap()
}

fn matrix() -> Matrix {
    common::matrix("unverified", &[])
}

fn env() -> Env {
    Env::new(WriteMode::Test, "complete/standalone-lifecycle/edit/list1")
}

fn title(text: &str) -> Op {
    Op::Edit {
        uid: A.into(),
        changes: vec![Change::Summary(text.into())],
    }
}

fn request() -> RequestRef {
    RequestRef {
        session: "s1".into(),
        request_id: "r1".into(),
    }
}

fn ctx(env: &Env) -> Ctx<'_> {
    Ctx {
        now: now(),
        request: Some(request()),
        config: &env.cfg,
    }
}

#[test]
fn commit_under_the_lock_journals_and_applies() {
    let env = env();
    let writer = Writer::open(&env.cfg, matrix()).unwrap();
    let ctx = ctx(&env);
    let prepared = writer.plan(&env.snap(), &title("Planned"), now()).unwrap();
    let lock = writer.lock(Duration::ZERO).unwrap().expect("lock is free");
    let id = lock.commit(prepared, &ctx).unwrap();
    drop(lock);
    let entry = Journal::open(&env.state)
        .unwrap()
        .get(&id)
        .unwrap()
        .unwrap();
    assert_eq!(entry.stage, Stage::Applied);
    assert_eq!(entry.request, Some(request()));
    let text = std::fs::read_to_string(env.file(A)).unwrap();
    assert!(text.contains("SUMMARY:Planned"), "{text}");
}

#[test]
fn lock_is_none_while_an_outside_holder_has_it() {
    let env = env();
    let writer = Writer::open(&env.cfg, matrix()).unwrap();
    let _outside = VdirLock::exclusive(&env.state).unwrap();
    assert!(writer.lock(Duration::from_millis(50)).unwrap().is_none());
}

#[test]
fn enqueue_then_drain_applies_the_item_once() {
    let env = env();
    let writer = Writer::open(&env.cfg, matrix()).unwrap();
    let ctx = ctx(&env);
    let op = title("Queued");
    let prepared = writer.plan(&env.snap(), &op, now()).unwrap();
    let item = writer
        .enqueue(Kind::Op { op, prepared }, "t", "daemon", Fence::Now, &ctx)
        .unwrap();
    assert_eq!(item.request, Some(request()));
    assert_eq!(Queue::open(&env.state).unwrap().list().unwrap().len(), 1);

    let lock = writer.lock(Duration::ZERO).unwrap().expect("lock is free");
    let drained = lock.drain(&ctx).unwrap();
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].item.id, item.id);
    let DrainOutcome::Applied(id) = &drained[0].outcome else {
        panic!("applied: {:?}", drained[0].outcome)
    };
    let entry = Journal::open(&env.state).unwrap().get(id).unwrap().unwrap();
    assert_eq!(
        entry.from_queue.as_ref().map(|f| f.id.as_str()),
        Some(item.id.as_str())
    );
    assert!(lock.drain(&ctx).unwrap().is_empty());
    drop(lock);
    let text = std::fs::read_to_string(env.file(A)).unwrap();
    assert!(text.contains("SUMMARY:Queued"), "{text}");
}

const DATA: &str = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VTODO\nUID:0a04f22aeb294b719633c978fe1975e5\n\
                    SUMMARY:A2\nEND:VTODO\nEND:VCALENDAR\n";

/// A 0.3.1-shaped queue item (`kind: "op"`, with `prepared`) and journal entry. Generated
/// after the refactor, with serde code unchanged by it, then shortened to a synthetic task.
fn item_json() -> String {
    r#"{"format":1,"id":"20261104T150000000000Z-00000000-0000265988","created":"2026-11-04T15:00:00Z",
"origin":{"tool":"op","pid":1,"started":"2026-01-01T00:00:00Z"},
"label":"Synthetic title","request":{"session":"s1","request_id":"r1"},
"kind":"op","op":{"Edit":{"uid":"0a04f22aeb294b719633c978fe1975e5","changes":[{"Summary":"Synthetic title"}]}},
"prepared":{"entry":{"format":1,"id":"20261104T150000Z-fabb82b5","created":"2026-11-04T15:00:00Z",
"kind":"edit","uids":["0a04f22aeb294b719633c978fe1975e5"],"stage":"prepared","inverse_of":null,
"writes":[{"path":"list1/0a04f22aeb294b719633c978fe1975e5.ics","before":"@DATA@","after":"@DATA@"}],
"family":{"collection":"list1","parent_uid":"0a04f22aeb294b719633c978fe1975e5",
"loaded":["list1/0a04f22aeb294b719633c978fe1975e5.ics"],"parent_aliases":["0a04f22aeb294b719633c978fe1975e5"]},
"request":{"session":"s1","request_id":"r1"}},
"holds":[],
"family":{"collection":"list1","parent_uid":"0a04f22aeb294b719633c978fe1975e5",
"loaded":["list1/0a04f22aeb294b719633c978fe1975e5.ics"],"parent_aliases":["0a04f22aeb294b719633c978fe1975e5"]}}}"#
        .replace("@DATA@", &DATA.replace('\n', "\\n"))
}

fn entry_json() -> String {
    r#"{"format":1,"id":"20261104T150000Z-4c61fbab","created":"2026-11-04T15:00:00Z","kind":"edit",
"uids":["0a04f22aeb294b719633c978fe1975e5"],"stage":"applied","inverse_of":null,
"writes":[{"path":"list1/0a04f22aeb294b719633c978fe1975e5.ics","before":"@DATA@","after":"@DATA@"}],
"family":{"collection":"list1","parent_uid":"0a04f22aeb294b719633c978fe1975e5",
"loaded":["list1/0a04f22aeb294b719633c978fe1975e5.ics"],"parent_aliases":["0a04f22aeb294b719633c978fe1975e5"]},
"from_queue":{"id":"20261104T150000000000Z-00000000-0000265988","queued":"2026-11-04T15:00:00Z"},
"request":{"session":"s1","request_id":"r1"}}"#
        .replace("@DATA@", &DATA.replace('\n', "\\n"))
}

#[test]
fn a_0_3_1_queue_item_and_journal_entry_still_decode() {
    for (json, what) in [(item_json(), "item"), (entry_json(), "entry")] {
        let want: serde_json::Value = serde_json::from_str(&json).unwrap();
        let back = if what == "item" {
            let item: Item = serde_json::from_str(&json).unwrap();
            assert!(matches!(item.kind, Kind::Op { .. }));
            serde_json::to_value(&item).unwrap()
        } else {
            let entry: Entry = serde_json::from_str(&json).unwrap();
            assert_eq!(entry.stage, Stage::Applied);
            serde_json::to_value(&entry).unwrap()
        };
        assert_eq!(back, want, "{what} re-serializes to the same JSON");
    }
}

/// Marks a collection operation as active with a record that cannot be read: `ensure_idle`
/// then refuses, as it does for an unfinished operation.
fn fence(env: &Env) {
    std::fs::create_dir_all(env.state.join("lists")).unwrap();
    std::fs::write(env.state.join("lists/active"), "unfinished-op").unwrap();
}

#[test]
fn commit_refuses_while_a_collection_operation_is_unfinished() {
    let env = env();
    let writer = Writer::open(&env.cfg, matrix()).unwrap();
    let prepared = writer.plan(&env.snap(), &title("Fenced"), now()).unwrap();
    fence(&env);
    let lock = writer.lock(Duration::ZERO).unwrap().expect("lock is free");
    assert!(lock.commit(prepared, &ctx(&env)).is_err());
    drop(lock);
    assert!(env.journal().is_empty());
}

#[test]
fn enqueue_at_drain_skips_the_fence_that_enqueue_now_checks() {
    let env = env();
    let writer = Writer::open(&env.cfg, matrix()).unwrap();
    let ctx = ctx(&env);
    let op = title("Fenced");
    let kind = |op: &Op| Kind::Op {
        op: op.clone(),
        prepared: writer.plan(&env.snap(), op, now()).unwrap(),
    };
    fence(&env);
    assert!(
        writer
            .enqueue(kind(&op), "t", "daemon", Fence::Now, &ctx)
            .is_err()
    );
    assert!(Queue::open(&env.state).unwrap().list().unwrap().is_empty());
    writer
        .enqueue(kind(&op), "t", "daemon", Fence::AtDrain, &ctx)
        .unwrap();
    assert_eq!(Queue::open(&env.state).unwrap().list().unwrap().len(), 1);
}

#[test]
fn opening_a_store_recovers_before_the_journal_exists() {
    let env = env();
    // A state directory with no journal is skipped by recovery, even with a fence that would
    // make `recover_locked` fail.
    fence(&env);
    let (_store, report) = twydo_core::store::Store::open_with(&env.cfg, matrix()).unwrap();
    assert!(report.stuck.is_empty() && report.resumed.is_empty());
}
