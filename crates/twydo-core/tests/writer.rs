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

/// A 0.3.1 queue item (`kind: "op"`, with `prepared`) and journal entry, as the code at this
/// task's base wrote them, with the task text shortened.
fn item_json() -> String {
    r#"{"format":1,"id":"20261104T150000000000Z-00000000-0000265988","created":"2026-11-04T15:00:00Z",
"origin":{"tool":"op","pid":265988,"started":"2026-11-13T21:56:18.851287398Z"},
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
