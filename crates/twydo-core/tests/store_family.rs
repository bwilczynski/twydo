//! The store on a checklist or project: completing, reopening, moving or deleting the parent
//! writes the whole family in one journal entry, as 2Do does (`docs/phase0/plan6-baseline.md`
//! H5–H9), against a tempdir copy of the iPhone capture `plan6-list1/04-reopen-child`.

mod common;

use std::collections::BTreeMap;
use std::fs;

use chrono::{DateTime, TimeZone, Utc};

use twydo_core::compat::Matrix;
use twydo_core::config::WriteMode;
use twydo_core::edit::Op;
use twydo_core::ical::{Component, parse};
use twydo_core::store::journal::Stage;
use twydo_core::store::{Store, WriteError};

use common::{Env, Line, fixture, is_stamp, lines, sorted, stamp, with_completed_now};

const FIXTURE: &str = "plan6-list1/04-reopen-child";
const L1: &str = "plan6-list1";
const AFTER: &str = "05-complete-parent";

const K: &str = "a8dcf93bcaa14211a07481eb87af8f11";
const K1B: &str = "28ae80e7369d40feaba54fe228eda2c6";
const K2: &str = "c57dfd30e9574d6ba195117e73dcc6d9";
const K3: &str = "58c8af9fe5694a498f0f0d07a20e5027";
const P: &str = "09c49c2cd76f483498fc2ff3a0f66160";
const P1: &str = "e668818024cc48f4a404fb9ced368789";
const P2: &str = "e0483d712d7a487480c8aaa7e6cd4b8d";
const P3: &str = "6a41bc3350d04ca2abffeebf69893007";
const P3_COPY: &str = "905ce14bad6e4f849d697df5da10ad48";
const R: &str = "8f603cfaa41f4c229b9adcd6223e7393";
const R1: &str = "35baad226105494080416dc20825f49c";
const R2: &str = "72e9476833d14a77a76c64cafeedfa97";
const R3: &str = "a8e8de2d00a24a8596cdd171fc971d4c";
// The copies 2Do created in `05-complete-parent`.
const R_COPY: &str = "14b5a37a52224e8c959b5002e2275fc9";
const R1_COPY: &str = "780885dd967342ec8953c6824f1d674b";
const R2_COPY: &str = "dd8f672c8feb47ccae449f8488918975";
const R3_COPY: &str = "7357bbeb391e4c2da619ca0b0b78f919";

const CASES: [&str; 6] = [
    "one-time",
    "monthly",
    "checklist",
    "project",
    "recurring-project",
    "project-recurring-child",
];

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 4, 15, 0, 0).unwrap()
}

fn later() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 4, 15, 5, 0).unwrap()
}

/// Every family case passes in test mode, with the edge cases a repeating completion needs.
fn matrix() -> Matrix {
    common::matrix(
        &CASES,
        "unverified",
        &[
            ("start-due-alarm-offsets", "pass"),
            ("monthly-15th-three-months-late", "pass"),
        ],
    )
}

fn env() -> (Env, Store) {
    let env = Env::new(WriteMode::Test, FIXTURE);
    let store = env.store(matrix());
    (env, store)
}

fn todo_of(bytes: &[u8]) -> Component {
    let cal = parse(bytes).unwrap();
    cal.root.children_named("VTODO").next().unwrap().clone()
}

fn value(c: &Component, name: &str) -> Option<String> {
    c.prop(name).map(|p| p.value.clone())
}

fn key(uid: &str) -> String {
    format!("list1/{uid}.ics")
}

fn status(files: &BTreeMap<String, Vec<u8>>, uid: &str) -> String {
    value(&todo_of(&files[&key(uid)]), "STATUS").unwrap()
}

/// `got`'s lines equal `want`'s, stamps left out.
fn same_lines(ctx: &str, got: Vec<Line>, want: Vec<Line>) {
    let drop =
        |v: Vec<Line>| -> Vec<Line> { sorted(v.into_iter().filter(|l| !is_stamp(&l.1)).collect()) };
    assert_eq!(drop(got), drop(want), "{ctx}: lines");
}

#[test]
fn completing_checklist_writes_parent_and_open_items() {
    let (env, mut store) = env();
    let before = env.files();
    store
        .apply(&env.snap(), &Op::Complete { uid: K.into() }, now())
        .unwrap();
    let after = env.files();
    assert_eq!(after.len(), before.len());
    for uid in [K, K1B, K2, K3] {
        let t = todo_of(&after[&key(uid)]);
        assert_eq!(value(&t, "STATUS").as_deref(), Some("COMPLETED"), "{uid}");
        assert_eq!(value(&t, "COMPLETED"), Some(stamp(now())), "{uid}");
    }
    let changed: Vec<&String> = after.keys().filter(|k| after[*k] != before[*k]).collect();
    assert_eq!(changed.len(), 4);

    let journal = env.journal();
    assert_eq!(journal.len(), 1);
    let e = &journal[0];
    assert_eq!(e.kind, "complete");
    assert_eq!(e.uids[0], K);
    let mut items = e.uids[1..].to_vec();
    items.sort();
    let mut want = [K1B, K2, K3].map(String::from).to_vec();
    want.sort();
    assert_eq!(items, want);
    assert_eq!(e.writes.len(), 4);
    assert_eq!(e.writes[0].path, key(K));
}

#[test]
fn completing_recurring_project_matches_2do() {
    let (env, mut store) = env();
    let before = env.files();
    store
        .apply(&env.snap(), &Op::Complete { uid: R.into() }, now())
        .unwrap();
    let after = env.files();
    assert_eq!(after.len(), before.len() + 4);

    let journal = env.journal();
    assert_eq!(journal.len(), 1);
    let e = &journal[0];
    assert_eq!(e.kind, "complete");
    assert_eq!(e.uids[0], R);
    // Creates (the parent copy and three child copies), then updates (R, r1, r3).
    let creates = e.writes.iter().take_while(|w| w.before.is_none()).count();
    assert_eq!(creates, 4);
    assert_eq!(e.writes.len(), 7);
    assert_eq!(e.writes[4].path, key(R));

    // Map each created copy to 2Do's copy of the same task (by title).
    let by_title = BTreeMap::from([
        ("R", R_COPY),
        ("r1", R1_COPY),
        ("r2", R2_COPY),
        ("r3", R3_COPY),
    ]);
    let mut map = BTreeMap::new();
    for w in &e.writes[..creates] {
        let t = todo_of(w.after.as_ref().unwrap().as_bytes());
        let title = value(&t, "SUMMARY").unwrap();
        map.insert(value(&t, "UID").unwrap(), by_title[title.as_str()]);
    }
    assert_eq!(map.len(), 4);
    for w in &e.writes[..creates] {
        // Our UIDs as 2Do's, in the unfolded values (the JSON line is folded).
        let mut got = lines(&parse(&after[&w.path]).unwrap());
        for l in &mut got {
            for (ours, theirs) in &map {
                l.3 = l.3.replace(ours.as_str(), theirs);
            }
        }
        let uid = got.iter().find(|l| l.1 == "UID").unwrap().3.clone();
        let want = parse(&fixture(L1, AFTER, &uid)).unwrap();
        // The parent and the open child are completed now; r1 and r3 keep their own time.
        let want = if uid == R_COPY || uid == R2_COPY {
            with_completed_now(lines(&want), now())
        } else {
            lines(&want)
        };
        same_lines(&uid, got, want);
    }

    // The live parent advanced, its completed children reopened, the open one untouched.
    for uid in [R, R1, R3] {
        let want = lines(&parse(&fixture(L1, AFTER, uid)).unwrap());
        same_lines(uid, lines(&parse(&after[&key(uid)]).unwrap()), want);
    }
    assert_eq!(
        value(&todo_of(&after[&key(R)]), "DUE").as_deref(),
        Some("20270115")
    );
    assert_eq!(status(&after, R1), "NEEDS-ACTION");
    assert_eq!(status(&after, R3), "NEEDS-ACTION");
    assert_eq!(after[&key(R2)], before[&key(R2)]);
}

#[test]
fn reopening_parent_reopens_completed_children() {
    let (env, mut store) = env();
    store
        .apply(&env.snap(), &Op::Complete { uid: P.into() }, now())
        .unwrap();
    let done = env.files();
    for uid in [P, P1, P2, P3, P3_COPY] {
        assert_eq!(status(&done, uid), "COMPLETED", "{uid}");
    }
    store
        .apply(&env.snap(), &Op::Reopen { uid: P.into() }, later())
        .unwrap();
    let after = env.files();
    for uid in [P, P1, P2, P3, P3_COPY] {
        assert_eq!(status(&after, uid), "NEEDS-ACTION", "{uid}");
    }
    assert!(value(&todo_of(&after[&key(P3)]), "RRULE").is_some());
    let e = env.journal().pop().unwrap();
    assert_eq!(e.kind, "reopen");
    assert_eq!(e.uids[0], P);
    assert_eq!(e.writes.len(), 5);
}

#[test]
fn moving_parent_moves_family() {
    let (env, mut store) = env();
    let family = [K, K1B, K2, K3];
    let op = Op::Move {
        uid: K.into(),
        to: "list2".into(),
    };
    store.apply(&env.snap(), &op, now()).unwrap();
    let after = env.files();
    for uid in family {
        assert!(!after.contains_key(&key(uid)), "{uid} left in list1");
    }
    let moved: Vec<Component> = after
        .iter()
        .filter(|(k, _)| k.starts_with("list2/"))
        .map(|(_, b)| todo_of(b))
        .collect();
    assert_eq!(moved.len(), 4);
    let parent = moved
        .iter()
        .find(|t| value(t, "SUMMARY").as_deref() == Some("K"))
        .unwrap();
    let new_k = value(parent, "UID").unwrap();
    assert!(value(parent, "RELATED-TO").is_none());
    for t in &moved {
        let uid = value(t, "UID").unwrap();
        assert!(!family.contains(&uid.as_str()), "{uid} kept its UID");
        if uid != new_k {
            assert_eq!(value(t, "RELATED-TO"), Some(new_k.clone()), "{uid}");
        }
    }
    let e = env.journal().pop().unwrap();
    assert_eq!(e.kind, "move");
    assert_eq!(e.uids[0], K);
    assert_eq!(e.writes.len(), 8);
    assert!(e.writes[..4].iter().all(|w| w.before.is_none()));
    assert!(e.writes[4..].iter().all(|w| w.after.is_none()));
}

#[test]
fn deleting_parent_deletes_family() {
    let (env, mut store) = env();
    let before = env.files();
    store
        .apply(&env.snap(), &Op::Delete { uid: P.into() }, now())
        .unwrap();
    let after = env.files();
    let family = [P, P1, P2, P3, P3_COPY];
    for uid in family {
        assert!(!after.contains_key(&key(uid)), "{uid} not deleted");
    }
    assert_eq!(after.len(), before.len() - family.len());
    let e = env.journal().pop().unwrap();
    assert_eq!(e.kind, "delete");
    assert_eq!(e.uids[0], P);
    assert_eq!(e.uids.len(), family.len());
}

#[test]
fn family_write_refused_when_child_changed() {
    let (env, mut store) = env();
    let snap = env.snap();
    let changed = fs::read_to_string(env.file(K3))
        .unwrap()
        .replace("SUMMARY:k3", "SUMMARY:k3b");
    fs::write(env.file(K3), &changed).unwrap();
    let before = env.files();

    match store.apply(&snap, &Op::Complete { uid: K.into() }, now()) {
        Err(WriteError::Stale(p)) => assert_eq!(p, env.file(K3)),
        other => panic!("expected Stale, got {other:?}"),
    }
    assert_eq!(env.files(), before);
    assert!(env.journal().iter().all(|e| e.stage != Stage::Applied));
}

#[test]
fn undo_recurring_project_completion_restores_all() {
    let (env, mut store) = env();
    let before = env.files();
    store
        .apply(&env.snap(), &Op::Complete { uid: R.into() }, now())
        .unwrap();
    assert_ne!(env.files(), before);
    store.undo_last(later()).unwrap();
    assert_eq!(env.files(), before);
}
