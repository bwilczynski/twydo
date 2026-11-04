//! The store on a checklist or project: completing, reopening, moving or deleting the parent
//! writes the whole family in one journal entry, as 2Do does (`docs/reference/phase0/plan6-baseline.md`
//! H5–H9), against a tempdir copy of the iPhone capture `plan6-list1/04-reopen-child`.

mod common;

use std::collections::BTreeMap;
use std::fs;

use chrono::{DateTime, TimeZone, Utc};

use twydo_core::compat::Matrix;
use twydo_core::config::WriteMode;
use twydo_core::edit::Op;
use twydo_core::ical::{Component, parse};
use twydo_core::store::journal::{Journal, Stage};
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

/// Changes `uid`'s file on disk (after the snapshot was taken).
fn touch(env: &Env, uid: &str) {
    let changed = fs::read_to_string(env.file(uid))
        .unwrap()
        .replace("\nSUMMARY:", "\nSUMMARY:changed ");
    fs::write(env.file(uid), &changed).unwrap();
}

/// `op` is refused Stale naming `uid`'s file, with no file changed and nothing applied.
fn refused_stale(
    env: &Env,
    store: &mut Store,
    snap: &twydo_core::views::Snapshot,
    op: Op,
    uid: &str,
) {
    let before = env.files();
    let applied = env
        .journal()
        .iter()
        .filter(|e| e.stage == Stage::Applied)
        .count();
    match store.apply(snap, &op, now()) {
        Err(WriteError::Stale(p)) => assert_eq!(p, env.file(uid)),
        other => panic!("expected Stale, got {other:?}"),
    }
    assert_eq!(env.files(), before);
    let after = env
        .journal()
        .iter()
        .filter(|e| e.stage == Stage::Applied)
        .count();
    assert_eq!(after, applied);
}

#[test]
fn recurring_project_completion_refused_when_open_child_changed() {
    // r2 is open: the op does not write it, but copies its loaded bytes.
    let (env, mut store) = env();
    let snap = env.snap();
    touch(&env, R2);
    refused_stale(&env, &mut store, &snap, Op::Complete { uid: R.into() }, R2);
}

#[test]
fn checklist_completion_refused_when_completed_item_changed() {
    let (env, mut store) = env();
    store
        .apply(&env.snap(), &Op::Complete { uid: K2.into() }, now())
        .unwrap();
    let snap = env.snap();
    assert_eq!(status(&env.files(), K2), "COMPLETED");
    touch(&env, K2);
    refused_stale(&env, &mut store, &snap, Op::Complete { uid: K.into() }, K2);
}

#[test]
fn project_reopen_refused_when_open_item_changed() {
    let (env, mut store) = env();
    store
        .apply(&env.snap(), &Op::Complete { uid: P.into() }, now())
        .unwrap();
    // p2 reopened elsewhere, so it is open while the parent is completed.
    let p2 = fs::read_to_string(env.file(P2)).unwrap();
    let p2: String = p2
        .split_inclusive('\n')
        .filter(|l| !l.starts_with("COMPLETED:"))
        .map(|l| l.replace("STATUS:COMPLETED", "STATUS:NEEDS-ACTION"))
        .collect();
    fs::write(env.file(P2), p2).unwrap();
    assert_eq!(status(&env.files(), P2), "NEEDS-ACTION");
    let snap = env.snap();
    touch(&env, P2);
    refused_stale(&env, &mut store, &snap, Op::Reopen { uid: P.into() }, P2);
}

#[test]
fn verified_undo_of_recurring_project_completion_needs_its_edge_case() {
    let edges = |undo: &'static str| {
        common::matrix(
            &CASES,
            "pass",
            &[
                ("start-due-alarm-offsets", "pass"),
                ("monthly-15th-three-months-late", "pass"),
                ("reopen-undo-before-after-sync", undo),
            ],
        )
    };
    for (undo, allowed) in [("unverified", false), ("pass", true)] {
        let env = Env::new(WriteMode::Verified, FIXTURE);
        let mut store = env.store(edges(undo));
        let before = env.files();
        store
            .apply(&env.snap(), &Op::Complete { uid: R.into() }, now())
            .unwrap();
        let done = env.files();
        match store.undo_last(later()) {
            Ok(_) if allowed => assert_eq!(env.files(), before),
            Err(WriteError::Blocked(b)) if !allowed => {
                assert_eq!(
                    b.to_string(),
                    "undoing the completion of a repeating task is not verified yet \
                     (reopen-undo-before-after-sync)"
                );
                assert_eq!(env.files(), done);
            }
            other => panic!("{undo}: unexpected {other:?}"),
        }
    }
}

// Plan 6 Task 7 round 01: checklist C dated Dec 10, its items carrying the same date; the
// phone moved C to Dec 11 and removed c1–c3's own dates.
const T1: &str = "plan6-twydo-list1";
const C: &str = "c05c77f7a0bc42d79f32d95ebf519ea3";
const C_ITEMS: [&str; 3] = [
    "f2bcae42b8f84b37b48c18b7f89c5ac0",
    "9e8d9baa64ba4737ad5cdb5566c6a992",
    "a80ca289da5a4ff88e80ee3754703699",
];

fn dated_checklist() -> (Env, Store) {
    let env = Env::new(WriteMode::Test, "plan6-twydo-list1/00-setup");
    let mut cases = CASES.to_vec();
    cases.push("date-only");
    let store = env.store(common::matrix(&cases, "unverified", &[]));
    (env, store)
}

fn new_date() -> Op {
    Op::Edit {
        uid: C.into(),
        changes: vec![twydo_core::edit::Change::Due(Some(
            twydo_core::edit::Due::Date(chrono::NaiveDate::from_ymd_opt(2026, 12, 11).unwrap()),
        ))],
    }
}

/// Non-stamp lines of a resource.
fn content(bytes: &[u8]) -> Vec<Line> {
    sorted(
        lines(&parse(bytes).unwrap())
            .into_iter()
            .filter(|l| !is_stamp(&l.1))
            .collect(),
    )
}

#[test]
fn checklist_date_change_drops_item_dates_like_2do() {
    let (env, mut store) = dated_checklist();
    let id = store.apply(&env.snap(), &new_date(), now()).unwrap();
    for uid in std::iter::once(C).chain(C_ITEMS) {
        let want = fixture(T1, "01-phone-edit", uid);
        assert_eq!(
            content(&fs::read(env.file(uid)).unwrap()),
            content(&want),
            "{uid}"
        );
    }
    let entry = env.journal().into_iter().find(|e| e.id == id).unwrap();
    assert_eq!(entry.writes.len(), 4);
    assert_eq!(entry.uids[0], C);
}

#[test]
fn checklist_date_change_undo_restores_all() {
    let (env, mut store) = dated_checklist();
    let before = env.files();
    store.apply(&env.snap(), &new_date(), now()).unwrap();
    assert_ne!(env.files(), before);
    store.undo_last(later()).unwrap();
    assert_eq!(env.files(), before);
}

#[test]
fn checklist_date_change_refused_when_item_changed() {
    let (env, mut store) = dated_checklist();
    let snap = env.snap();
    let item = env.file(C_ITEMS[1]);
    let changed = fs::read_to_string(&item)
        .unwrap()
        .replace("SUMMARY:c2", "SUMMARY:c2b");
    fs::write(&item, &changed).unwrap();
    let before = env.files();
    match store.apply(&snap, &new_date(), now()) {
        Err(WriteError::Stale(p)) => assert_eq!(p, item),
        other => panic!("expected Stale, got {other:?}"),
    }
    assert_eq!(env.files(), before);
}

/// A sync adds an item to K after the snapshot was loaded: a copy of k3 under a new UID.
fn add_item_behind_snapshot(env: &Env) -> std::path::PathBuf {
    let new = "0000000000000000000000000000beef";
    let text = fs::read_to_string(env.file(K3))
        .unwrap()
        .replace(K3, new)
        .replace("SUMMARY:k3", "SUMMARY:k4");
    let path = env.file(new);
    fs::write(&path, text).unwrap();
    path
}

#[test]
fn family_op_refused_when_an_item_joined_after_load() {
    for op in [
        Op::Delete { uid: K.into() },
        Op::Move {
            uid: K.into(),
            to: "list2".into(),
        },
        Op::Complete { uid: K.into() },
    ] {
        let (env, mut store) = env();
        let snap = env.snap();
        let added = add_item_behind_snapshot(&env);
        let before = env.files();
        match store.apply(&snap, &op, now()) {
            Err(WriteError::Stale(p)) => assert_eq!(p, added, "{op:?}"),
            other => panic!("{op:?}: expected Stale, got {other:?}"),
        }
        assert_eq!(env.files(), before, "{op:?}");
        assert!(env.journal().iter().all(|e| e.stage != Stage::Applied));
    }
}

#[test]
fn family_op_runs_when_unrelated_files_appear() {
    let (env, mut store) = env();
    let snap = env.snap();
    // A new task in the same list that is not K's item does not block the op.
    let text = fs::read_to_string(env.file(K3))
        .unwrap()
        .replace(K3, "0000000000000000000000000000cafe")
        .replace(&format!("RELATED-TO:{K}"), &format!("RELATED-TO:{P}"));
    fs::write(env.file("0000000000000000000000000000cafe"), text).unwrap();
    store
        .apply(&snap, &Op::Delete { uid: K.into() }, now())
        .unwrap();
}

/// K's items removed before load, so K is an empty checklist in the snapshot.
fn empty_checklist() -> (Env, Store) {
    let (env, store) = env();
    for uid in [K1B, K2, K3] {
        fs::remove_file(env.file(uid)).unwrap();
    }
    (env, store)
}

#[test]
fn op_on_empty_parent_refused_when_its_first_item_arrives_after_load() {
    for op in [
        Op::Delete { uid: K.into() },
        Op::Move {
            uid: K.into(),
            to: "list2".into(),
        },
        Op::Complete { uid: K.into() },
    ] {
        let (env, mut store) = empty_checklist();
        let snap = env.snap();
        let k = snap.find(K).unwrap();
        assert!(snap.children(k).is_empty());
        let item = env.file("0000000000000000000000000000f00d");
        let text = fixture(L1, "04-reopen-child", K2)
            .iter()
            .map(|&b| b as char)
            .collect::<String>()
            .replace(K2, "0000000000000000000000000000f00d");
        fs::write(&item, text).unwrap();
        let before = env.files();
        match store.apply(&snap, &op, now()) {
            Err(WriteError::Stale(p)) => assert_eq!(p, item, "{op:?}"),
            other => panic!("{op:?}: expected Stale, got {other:?}"),
        }
        assert_eq!(env.files(), before, "{op:?}");
    }
}

#[test]
fn op_on_plain_task_refused_when_an_item_arrives_after_load() {
    let (env, mut store) = env();
    let snap = env.snap();
    // L (a plain monthly task) gains an item behind the snapshot.
    let l = snap
        .tasks
        .iter()
        .find(|t| t.summary == "L" && t.rrule.is_some())
        .unwrap()
        .uid
        .clone();
    let item = env.file("0000000000000000000000000000d00d");
    let text = String::from_utf8(fixture(L1, "04-reopen-child", K2))
        .unwrap()
        .replace(K2, "0000000000000000000000000000d00d")
        .replace(K, &l);
    fs::write(&item, text).unwrap();
    match store.apply(&snap, &Op::Delete { uid: l }, now()) {
        Err(WriteError::Stale(p)) => assert_eq!(p, item),
        other => panic!("expected Stale, got {other:?}"),
    }
}

/// list2's files, unfolded and keyed by SUMMARY, with the UIDs a move picks at random named by their
/// task's SUMMARY instead.
fn moved_by_summary(env: &Env) -> BTreeMap<String, String> {
    let moved: Vec<String> = env
        .files()
        .into_iter()
        .filter(|(k, _)| k.starts_with("list2/"))
        .map(|(_, b)| {
            // Unfolded, so a UID split across a folded line is still found.
            String::from_utf8(b)
                .unwrap()
                .replace("\r\n ", "")
                .replace("\n ", "")
        })
        .collect();
    let names: Vec<(String, String)> = moved
        .iter()
        .map(|s| {
            let t = todo_of(s.as_bytes());
            (value(&t, "UID").unwrap(), value(&t, "SUMMARY").unwrap())
        })
        .collect();
    names
        .iter()
        .zip(&moved)
        .map(|((_, summary), text)| {
            let mut text = text.clone();
            for (uid, name) in &names {
                text = text.replace(uid.as_str(), &format!("uid-of-{name}"));
            }
            (summary.clone(), text)
        })
        .collect()
}

/// A save that changes a dated checklist's date and moves it: the same files as the edit and
/// then the move, in one journal entry that undo reverts byte for byte.
#[test]
fn save_with_family_date_change_and_move_is_one_step() {
    let changes = match new_date() {
        Op::Edit { changes, .. } => changes,
        _ => unreachable!(),
    };
    let mv = Op::Move {
        uid: C.into(),
        to: "list2".into(),
    };
    let (two, mut store) = dated_checklist();
    store.apply(&two.snap(), &new_date(), now()).unwrap();
    store.apply(&two.snap(), &mv, now()).unwrap();
    assert_eq!(two.journal().len(), 2);

    let (env, mut store) = dated_checklist();
    let before = env.files();
    let save = Op::Save {
        uid: C.into(),
        changes,
        to: Some("list2".into()),
    };
    store.apply(&env.snap(), &save, now()).unwrap();
    let want = moved_by_summary(&two);
    assert_eq!(want.len(), 1 + C_ITEMS.len());
    assert_eq!(moved_by_summary(&env), want);
    for uid in std::iter::once(C).chain(C_ITEMS) {
        assert!(!env.file(uid).exists(), "{uid} left in list1");
    }
    let journal = env.journal();
    assert_eq!(journal.len(), 1);
    assert_eq!(journal[0].kind, "save");
    // Each source edited then removed is one write; each copy one more.
    assert_eq!(journal[0].writes.len(), 2 * (1 + C_ITEMS.len()));

    store.undo_last(later()).unwrap();
    assert_eq!(env.files(), before);
}

/// A grouped save (a family date change and move) interrupted half way: its entry is still
/// Prepared, the copies are written but the sources not removed. Opening the store rolls it
/// forward to the files the whole save writes.
#[test]
fn interrupted_family_save_is_rolled_forward() {
    let changes = match new_date() {
        Op::Edit { changes, .. } => changes,
        _ => unreachable!(),
    };
    let save = Op::Save {
        uid: C.into(),
        changes,
        to: Some("list2".into()),
    };
    let (done, mut store) = dated_checklist();
    store.apply(&done.snap(), &save, now()).unwrap();
    let mut entry = done.journal().pop().unwrap();
    let want = done.files();

    let (env, _) = dated_checklist();
    entry.stage = Stage::Prepared;
    // Every write but the last: the copies exist, one source is not removed yet.
    let (last, first) = entry.writes.split_last().unwrap();
    assert!(last.after.is_none() && last.before.is_some(), "{last:?}");
    for w in first {
        let p = env.vdir.join(&w.path);
        match &w.after {
            Some(text) => fs::write(&p, text).unwrap(),
            None => fs::remove_file(&p).unwrap(),
        }
    }
    Journal::open(&env.state).unwrap().prepare(&entry).unwrap();
    assert_ne!(env.files(), want);

    let (_store, report) = Store::open_with(&env.cfg, matrix()).unwrap();
    assert_eq!(report.resumed, vec![entry.id.clone()]);
    assert!(report.stuck.is_empty(), "{report:?}");
    assert_eq!(env.files(), want);
    let got = env
        .journal()
        .into_iter()
        .find(|e| e.id == entry.id)
        .unwrap();
    assert_eq!(got.stage, Stage::Applied);
}
