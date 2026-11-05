//! The store on a checklist or project: completing, reopening, moving or deleting the parent
//! writes the whole family in one journal entry, as 2Do does (`docs/reference/phase0/plan6-baseline.md`
//! H5–H9), against a tempdir copy of the iPhone capture `plan6-list1/04-reopen-child`.

mod common;

use std::collections::BTreeMap;
use std::fs;

use chrono::{DateTime, TimeZone, Utc};

use twydo_core::compat::Matrix;
use twydo_core::config::WriteMode;
use twydo_core::edit::{Change, NewTask, Op};
use twydo_core::ical::{Component, parse};
use twydo_core::model::TaskKind;
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

/// `R` with `from` -> `to` in its text and `meta` set in its JSON metadata, and a matrix
/// where the weekly case and the rule rows `rules` pass too.
fn reshaped_r(from: &str, to: &str, meta: &[(&str, i64)], rules: &[&str]) -> (Env, Store) {
    let env = Env::new(WriteMode::Test, FIXTURE);
    let mut cases = CASES.to_vec();
    cases.push("weekly");
    let mut m = common::matrix(
        &cases,
        "unverified",
        &[
            ("start-due-alarm-offsets", "pass"),
            ("monthly-15th-three-months-late", "pass"),
        ],
    );
    for class in rules {
        let extra = Matrix::from_toml_str(&format!(
            "matrix_version = 1\n[[rule]]\nclass = \"{class}\"\npreserve = \"pass\"\n\
             execute = \"pass\"\nedit = \"pass\"\n"
        ))
        .unwrap();
        m.rule.extend(extra.rule);
    }
    let store = env.store(m);
    let path = env.file(R);
    let text = fs::read_to_string(&path).unwrap().replace(from, to);
    let mut cal = parse(text.as_bytes()).unwrap();
    {
        let t = cal.root.children_named_mut("VTODO").next().unwrap();
        let p = t.prop_mut("X-2DOAPP-METADATA").unwrap();
        let mut json = twydo_core::model::meta_edit::MetaJson::parse(&p.value).unwrap();
        for (k, v) in meta {
            json.set_i64(k, *v);
        }
        p.set_value(&json.encode());
    }
    fs::write(&path, twydo_core::ical::serialize(&cal)).unwrap();
    (env, store)
}

/// Task 14: a recurring project with a new repeat kind advances through the same next date.
#[test]
fn weekly_recurring_project_advances() {
    let (env, mut store) = reshaped_r(
        "RRULE:FREQ=MONTHLY",
        "RRULE:FREQ=WEEKLY",
        &[("RecurrenceType", 257)],
        &["weeks"],
    );
    let before = env.files();
    store
        .apply(&env.snap(), &Op::Complete { uid: R.into() }, now())
        .unwrap();
    let after = env.files();
    assert_eq!(after.len(), before.len() + 4);
    assert_eq!(
        value(&todo_of(&after[&key(R)]), "DUE").as_deref(),
        Some("20261222")
    );
}

/// A recurring project that ends (here its last occurrence, `COUNT=1`) is not completed: no
/// capture shows what 2Do does with a counted, until or from-completion recurring project, so
/// the gate refuses in every mode and nothing is written.
#[test]
fn completing_a_recurring_project_that_ends_is_refused() {
    let (env, mut store) = reshaped_r(
        "RRULE:FREQ=MONTHLY",
        "RRULE:FREQ=MONTHLY;COUNT=1",
        &[
            ("RecurrenceEndType", 2),
            ("RecurrenceEndRepetitions", 1),
            ("RecurrenceEndRepetitionsOrig", 1),
        ],
        &["mod:count"],
    );
    let before = env.files();
    let err = store
        .apply(&env.snap(), &Op::Complete { uid: R.into() }, now())
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("completing a recurring project that ends or repeats from completion"),
        "{err}"
    );
    assert_eq!(env.files(), before);
}

/// Verified mode with the embedded matrix and a new empty project "Synthetic project" in
/// list2, created through the store. Returns its UID and file.
fn verified_with_empty_project() -> (Env, Store, String, std::path::PathBuf) {
    let env = Env::new(WriteMode::Verified, FIXTURE);
    let mut store = env.store(Matrix::embedded().clone());
    let op = Op::Create(NewTask {
        kind: TaskKind::Project,
        ..NewTask::plain("list2", "Synthetic project")
    });
    store.apply(&env.snap(), &op, now()).unwrap();
    let snap = env.snap();
    let t = snap
        .tasks
        .iter()
        .find(|t| t.summary == "Synthetic project")
        .unwrap();
    let (uid, path) = (t.uid.clone(), t.path.clone());
    (env, store, uid, path)
}

/// An item of `parent` in list2, as a sync would deliver it: the file of a task created by a
/// second store, with RELATED-TO the parent.
fn item_of(env: &Env, store: &mut Store, parent: &str) -> std::path::PathBuf {
    let before = env.files();
    let op = Op::Create(NewTask {
        parent: Some(parent.to_string()),
        ..NewTask::plain("list2", "Synthetic item")
    });
    store.apply(&env.snap(), &op, now()).unwrap();
    let after = env.files();
    let (rel, _) = after
        .iter()
        .find(|(k, _)| !before.contains_key(*k))
        .unwrap();
    env.vdir.join(rel)
}

#[test]
fn type_conversion_refused_when_an_item_joined_after_load() {
    for changes in [
        vec![Change::Kind(TaskKind::Task)],
        vec![
            Change::Summary("Renamed".into()),
            Change::Kind(TaskKind::Task),
        ],
    ] {
        for save in [false, true] {
            let (env, mut store, uid, _) = verified_with_empty_project();
            let snap = env.snap();
            let item = item_of(&env, &mut store, &uid);
            let before = env.files();
            let op = if save {
                Op::Save {
                    uid: uid.clone(),
                    changes: changes.clone(),
                    to: None,
                }
            } else {
                Op::Edit {
                    uid: uid.clone(),
                    changes: changes.clone(),
                }
            };
            match store.apply(&snap, &op, later()) {
                Err(WriteError::Stale(p)) => assert_eq!(p, item, "{op:?}"),
                other => panic!("{op:?}: expected Stale, got {other:?}"),
            }
            assert_eq!(env.files(), before, "{op:?}");
        }
    }
}

#[test]
fn creating_an_item_refused_when_its_parent_is_gone() {
    let (env, mut store, uid, path) = verified_with_empty_project();
    let snap = env.snap();
    fs::remove_file(&path).unwrap();
    let before = env.files();
    let op = Op::Create(NewTask {
        parent: Some(uid),
        ..NewTask::plain("list2", "Synthetic item")
    });
    match store.apply(&snap, &op, later()) {
        Err(WriteError::Stale(p)) => assert_eq!(p, path),
        other => panic!("expected Stale, got {other:?}"),
    }
    assert_eq!(env.files(), before);
}

#[test]
fn creating_an_item_refused_when_its_parent_changed() {
    let (env, mut store, uid, path) = verified_with_empty_project();
    let snap = env.snap();
    let text = fs::read_to_string(&path).unwrap() + "\r\n";
    fs::write(&path, text).unwrap();
    let before = env.files();
    let op = Op::Create(NewTask {
        parent: Some(uid),
        ..NewTask::plain("list2", "Synthetic item")
    });
    match store.apply(&snap, &op, later()) {
        Err(WriteError::Stale(p)) => assert_eq!(p, path),
        other => panic!("expected Stale, got {other:?}"),
    }
    assert_eq!(env.files(), before);
}

// Native phone round trips; see the capture log's regression-test map.
mod recurrence_phone_captures {
    use crate::common::recurrence::*;
    use crate::common::{Env, completed, dtstamp, fixture, todo};
    use std::collections::BTreeMap;
    use std::fs;
    use twydo_core::compat::{Matrix, Verdict};
    use twydo_core::config::WriteMode;
    use twydo_core::edit;
    use twydo_core::edit::{Change, Op};
    use twydo_core::ical::{parse, serialize};
    use twydo_core::model::meta_edit::MetaJson;
    use twydo_core::store::Store;
    use twydo_core::store::journal::{Journal, Stage};

    #[test]
    fn recurring_checklist_family_completion_matches_both_phone_occurrences_and_undo() {
        for (from, to) in [(SET, ADVANCE), (END, REOPEN)] {
            let before_phone = step_files(from);
            let after_phone = step_files(to);
            let copies: BTreeMap<_, _> = after_phone
                .iter()
                .filter(|(id, c)| {
                    !before_phone.contains_key(*id)
                        && todo(c).prop("SUMMARY").unwrap().value.starts_with("FCa")
                })
                .map(|(uid, c)| (todo(c).prop("SUMMARY").unwrap().value.clone(), uid.clone()))
                .collect();
            assert_eq!(copies.len(), 4);
            let now = completed(&after_phone[&copies["FCa"]]);
            let env = Env::new(WriteMode::Verified, &format!("{SCENARIO}/{from}"));
            let mut store = env.store(Matrix::embedded().clone());
            let before = env.files();
            store
                .apply(&env.snap(), &Op::Complete { uid: FC.into() }, now)
                .unwrap();
            let after = env.files();
            let journal = env.journal();
            assert_eq!(journal.len(), 1);
            let entry = &journal[0];
            assert_eq!(
                entry.writes.len(),
                8,
                "four history copies and four live updates"
            );
            let ids: BTreeMap<_, _> = entry
                .writes
                .iter()
                .filter(|w| w.before.is_none())
                .map(|w| {
                    let c = parse(w.after.as_ref().unwrap().as_bytes()).unwrap();
                    (
                        todo(&c).prop("UID").unwrap().value.clone(),
                        copies[&todo(&c).prop("SUMMARY").unwrap().value].clone(),
                    )
                })
                .collect();
            for write in &entry.writes {
                let got = parse(&after[&write.path]).unwrap();
                let ours = &todo(&got).prop("UID").unwrap().value;
                let theirs = ids.get(ours).unwrap_or(ours);
                same(
                    &format!("checklist {to} {theirs}"),
                    &got,
                    &after_phone[theirs],
                    &ids,
                );
                stamped(&got, now);
                let title = &todo(&got).prop("SUMMARY").unwrap().value;
                let seeds = step_files(SEED);
                let source_uid = seeds
                    .iter()
                    .find(|(_, c)| &todo(c).prop("SUMMARY").unwrap().value == title)
                    .unwrap()
                    .0;
                preserved_created(&before_phone[source_uid], &got);
            }
            for (path, bytes) in &before {
                if !entry.writes.iter().any(|w| &w.path == path) {
                    assert_eq!(&after[path], bytes);
                }
            }
            store.undo_last(now).unwrap();
            assert_eq!(
                env.files(),
                before,
                "family undo restores exact source bytes"
            );
            store.redo_last(now).unwrap();
            assert_eq!(env.files(), after, "redo reuses history identities");

            // Interrupt after two history creates: recovery must finish all eight resources,
            // not generate a second family or expose half of the reset indefinitely.
            let recovering = Env::new(WriteMode::Verified, &format!("{SCENARIO}/{from}"));
            drop(recovering.store(Matrix::embedded().clone()));
            let mut interrupted = entry.clone();
            interrupted.stage = Stage::Prepared;
            Journal::open(&recovering.state)
                .unwrap()
                .prepare(&interrupted)
                .unwrap();
            for write in entry.writes.iter().take(2) {
                fs::write(
                    recovering.vdir.join(&write.path),
                    write.after.as_ref().unwrap(),
                )
                .unwrap();
            }
            let (_, report) =
                Store::open_with(&recovering.cfg, Matrix::embedded().clone()).unwrap();
            assert!(report.stuck.is_empty());
            assert_eq!(
                recovering.files(),
                after,
                "recovery uses the recorded copy identities"
            );
        }
    }

    #[test]
    fn recurring_checklist_refuses_uncaptured_item_shapes_without_partial_history() {
        for case in [
            "start-only",
            "metadata-start-only",
            "metadata-time-only",
            "cancelled",
            "timed",
        ] {
            let env = Env::new(WriteMode::Verified, &format!("{SCENARIO}/{SET}"));
            let mut store = env.store(Matrix::embedded().clone());
            let snap = env.snap();
            let child = snap.children(snap.find(FC).unwrap())[0];
            let path = &snap.tasks[child].path;
            let mut input = parse(&fs::read(path).unwrap()).unwrap();
            let now = dtstamp(&input);
            if case == "cancelled" {
                input
                    .root
                    .children_named_mut("VTODO")
                    .next()
                    .unwrap()
                    .prop_mut("STATUS")
                    .unwrap()
                    .set_value("CANCELLED");
            } else if case.ends_with("only") {
                let t = input.root.children_named_mut("VTODO").next().unwrap();
                t.remove_props("DUE");
                if case != "start-only" {
                    t.remove_props("DTSTART");
                    let p = t.prop_mut("X-2DOAPP-METADATA").unwrap();
                    let mut m = MetaJson::parse(&p.value).unwrap();
                    let key = if case == "metadata-start-only" {
                        "StartDate"
                    } else {
                        "duetime"
                    };
                    m.set_i64(key, 1_795_262_400);
                    p.set_value(&m.encode());
                }
            } else {
                edit::apply_changes(
                    &mut input,
                    &[Change::Due(Some(edit::Due::Time(now.naive_utc())))],
                    now,
                )
                .unwrap();
            }
            fs::write(path, serialize(&input)).unwrap();
            let before = env.files();
            assert!(
                store
                    .apply(&env.snap(), &Op::Complete { uid: FC.into() }, now)
                    .is_err(),
                "{case}"
            );
            assert_eq!(env.files(), before, "{case}");
            assert!(env.journal().is_empty(), "{case}");
        }
    }

    #[test]
    fn checklist_repeat_removal_matches_capture_and_needs_both_family_repeat_rows() {
        const BEFORE: &str = "144-phone-removal-and-history-followup";
        const AFTER: &str = "151-twydo-checklist-repeat-removed";
        let before = step_files(BEFORE);
        let want = step_files(AFTER);
        let uid = before
            .iter()
            .find(|(_, c)| {
                todo(c).prop("SUMMARY").unwrap().value == "FCb" && todo(c).prop("RRULE").is_some()
            })
            .unwrap()
            .0
            .clone();
        let env = Env::new(WriteMode::Verified, &format!("{SCENARIO}/{BEFORE}"));
        let op = Op::Edit {
            uid: uid.clone(),
            changes: vec![Change::Repeat(None)],
        };
        let now = dtstamp(&want[&uid]);
        let mut matrix = Matrix::embedded().clone();
        matrix
            .operation
            .iter_mut()
            .find(|r| r.case == "recurring-checklist" && r.op == "repeat")
            .unwrap()
            .twydo_to_iphone = Verdict::Pass;
        matrix
            .operation
            .iter_mut()
            .find(|r| r.case == "checklist" && r.op == "repeat")
            .unwrap()
            .twydo_to_iphone = Verdict::Unverified;
        assert!(
            twydo_core::gate::check(
                &env.snap(),
                &matrix,
                WriteMode::Verified,
                &op,
                now.date_naive()
            )
            .is_err(),
            "the destination plain-checklist repeat row must pass too"
        );
        matrix
            .operation
            .iter_mut()
            .find(|r| r.case == "checklist" && r.op == "repeat")
            .unwrap()
            .twydo_to_iphone = Verdict::Pass;
        let mut store = env.store(matrix);
        let original = env.files();
        store.apply(&env.snap(), &op, now).unwrap();
        let changed = env.files();
        let journal = env.journal();
        assert_eq!(journal.len(), 1);
        assert_eq!(journal[0].writes.len(), 1);
        let target = format!("list1/{uid}.ics");
        for (path, bytes) in &original {
            if path != &target {
                assert_eq!(&changed[path], bytes, "only the parent is edited");
            }
        }
        assert_eq!(
            original.keys().collect::<Vec<_>>(),
            changed.keys().collect::<Vec<_>>()
        );
        let got = parse(&changed[&target]).unwrap();
        same(
            "FCb weekly repeat removal",
            &got,
            &want[&uid],
            &BTreeMap::new(),
        );
        stamped(&got, now);
        preserved_created(&before[&uid], &got);
        store.undo_last(now).unwrap();
        assert_eq!(env.files(), original, "repeat removal is one undo step");
        store.redo_last(now).unwrap();
        assert_eq!(env.files(), changed, "redo restores the same parent bytes");
        assert_eq!(
            before.keys().collect::<Vec<_>>(),
            want.keys().collect::<Vec<_>>()
        );
        for id in before.keys() {
            if id != &uid {
                assert_eq!(
                    fixture(SCENARIO, BEFORE, id),
                    fixture(SCENARIO, AFTER, id),
                    "checklist removal leaves {id} unchanged"
                );
            }
        }
    }
}

mod monthly_checklist_writes {
    use super::*;
    use crate::common::{dtstamp, recurrence::*, todo};
    use twydo_core::edit::Due;
    use twydo_core::repeatinput;

    fn env(step: &str) -> Env {
        Env::new(WriteMode::Verified, &format!("{CHECKLIST_B}/{step}"))
    }
    fn cal(step: &str, uid: &str) -> twydo_core::ical::Calendar {
        parse(&fixture(CHECKLIST_B, step, uid)).unwrap()
    }
    fn member(s: &twydo_core::views::Snapshot, parent: &str, title: &str) -> String {
        s.children(s.find(parent).unwrap())
            .iter()
            .map(|&c| &s.tasks[c])
            .find(|t| t.summary == title)
            .unwrap()
            .uid
            .clone()
    }

    #[test]
    fn monthly_checklist_creation_conversion_and_member_creation_match_phone() {
        let evidence = cal("010-phone-create-convert", CHECKLIST_A_UID);
        let e = env("002-seeded");
        let mut store = e.store(Matrix::embedded().clone());
        let op = Op::Create(NewTask {
            kind: TaskKind::Checklist,
            due: Some(Due::Date(
                chrono::NaiveDate::from_ymd_opt(2026, 12, 10).unwrap(),
            )),
            repeat: repeatinput::parse("monthly", now().date_naive(), &[]).unwrap(),
            ..NewTask::plain("list1", "a")
        });
        let at = dtstamp(&evidence);
        store.apply(&e.snap(), &op, at).unwrap();
        let entry = e.journal().pop().unwrap();
        assert_eq!(entry.writes.len(), 1);
        let created = parse(entry.writes[0].after.as_ref().unwrap().as_bytes()).unwrap();
        let uid = todo(&created).prop("UID").unwrap().value.clone();
        same(
            "direct checklist create",
            &created,
            &evidence,
            &BTreeMap::from([(uid, CHECKLIST_A_UID.into())]),
        );
        let evidence = cal("010-phone-create-convert", CHECKLIST_B_UID);
        store
            .apply(
                &e.snap(),
                &Op::Edit {
                    uid: CHECKLIST_B_UID.into(),
                    changes: vec![Change::Kind(TaskKind::Checklist)],
                },
                dtstamp(&evidence),
            )
            .unwrap();
        let converted = parse(&fs::read(e.file(CHECKLIST_B_UID)).unwrap()).unwrap();
        let (got_lines, got_meta) = comparable(&converted, &BTreeMap::new());
        let (want_lines, mut want_meta) = comparable(&evidence, &BTreeMap::new());
        assert_eq!(
            got_meta.get_str("uid"),
            Some(CHECKLIST_B_UID),
            "Twydo uses the public UID as parent identity"
        );
        assert_ne!(
            want_meta.get_str("uid"),
            Some(CHECKLIST_B_UID),
            "phone creates its own internal parent identity"
        );
        want_meta.set_str("uid", CHECKLIST_B_UID);
        assert_eq!(
            (got_lines, got_meta),
            (want_lines, want_meta),
            "conversion differs only in explicitly mapped internal parent identity"
        );

        let e = env("010-phone-create-convert");
        let mut store = e.store(Matrix::embedded().clone());
        let phone = Env::new(
            WriteMode::Verified,
            &format!("{CHECKLIST_B}/011-phone-items-and-text"),
        );
        let phone_uid = member(&phone.snap(), CHECKLIST_B_UID, "q");
        let evidence = cal("011-phone-items-and-text", &phone_uid);
        store
            .apply(
                &e.snap(),
                &Op::Create(NewTask {
                    parent: Some(CHECKLIST_B_UID.into()),
                    ..NewTask::plain("list1", "q")
                }),
                dtstamp(&evidence),
            )
            .unwrap();
        let entry = e.journal().pop().unwrap();
        assert_eq!(entry.writes.len(), 1);
        let created = parse(entry.writes[0].after.as_ref().unwrap().as_bytes()).unwrap();
        let uid = todo(&created).prop("UID").unwrap().value.clone();
        same(
            "new checklist item inherits date",
            &created,
            &evidence,
            &BTreeMap::from([(uid, phone_uid)]),
        );
    }

    #[test]
    fn individual_checklist_deletion_matches_phone_and_undo_redo() {
        let e = env("011-phone-items-and-text");
        let mut store = e.store(Matrix::embedded().clone());
        let before = e.files();
        for parent in [CHECKLIST_A_UID, CHECKLIST_B_UID] {
            let uid = member(&e.snap(), parent, "q");
            store.apply(&e.snap(), &Op::Delete { uid }, now()).unwrap();
        }
        let phone = env("011d-phone-delete-items");
        assert_eq!(e.files(), phone.files(), "only deleted bytes differ");
        let after = e.files();
        assert!(
            e.journal()
                .iter()
                .all(|j| j.writes.len() == 1 && j.writes[0].after.is_none())
        );
        store.undo_last(now()).unwrap();
        store.undo_last(now()).unwrap();
        assert_eq!(e.files(), before);
        store.redo_last(now()).unwrap();
        store.redo_last(now()).unwrap();
        assert_eq!(e.files(), after);
    }

    #[test]
    fn item_delete_refuses_changed_parent_or_sibling_without_writes() {
        for change in ["parent", "sibling", "missing-sibling", "new-sibling"] {
            let e = env("011-phone-items-and-text");
            let mut store = e.store(Matrix::embedded().clone());
            let snap = e.snap();
            let uid = member(&snap, CHECKLIST_A_UID, "q");
            let sibling = member(&snap, CHECKLIST_A_UID, "z");
            match change {
                "parent" => {
                    let p = e.file(CHECKLIST_A_UID);
                    fs::write(&p, [fs::read(&p).unwrap(), b"\r\n".to_vec()].concat()).unwrap();
                }
                "sibling" => {
                    let p = e.file(&sibling);
                    fs::write(&p, [fs::read(&p).unwrap(), b"\r\n".to_vec()].concat()).unwrap();
                }
                "missing-sibling" => fs::remove_file(e.file(&sibling)).unwrap(),
                "new-sibling" => {
                    let text = fs::read_to_string(e.file(&sibling))
                        .unwrap()
                        .replace(&sibling, "new-synthetic-sibling");
                    fs::write(e.file("new-synthetic-sibling"), text).unwrap();
                }
                _ => unreachable!(),
            }
            let before = e.files();
            let result = store.apply(&snap, &Op::Delete { uid }, now());
            assert!(
                matches!(result, Err(WriteError::Stale(_))),
                "{change}: {result:?}"
            );
            assert_eq!(e.files(), before);
            assert!(e.journal().is_empty());
        }
    }
    #[test]
    fn checklist_text_edits_match_phone_and_mixed_schedule_save_is_atomic() {
        let e = env("010-phone-create-convert");
        let mut store = e.store(Matrix::embedded().clone());
        for (uid, title) in [(CHECKLIST_A_UID, "A"), (CHECKLIST_B_UID, "B")] {
            let want = cal("011-phone-items-and-text", uid);
            let before = e.files();
            store
                .apply(
                    &e.snap(),
                    &Op::Edit {
                        uid: uid.into(),
                        changes: vec![
                            Change::Summary(title.into()),
                            Change::Description(Some("x".into())),
                        ],
                    },
                    dtstamp(&want),
                )
                .unwrap();
            let got = parse(&fs::read(e.file(uid)).unwrap()).unwrap();
            same("parent text fields", &got, &want, &BTreeMap::new());
            for (path, bytes) in before {
                if path != key(uid) {
                    assert_eq!(e.files()[&path], bytes);
                }
            }
        }
        let before = e.files();
        let journal_len = e.journal().len();
        let op = Op::Save {
            uid: CHECKLIST_A_UID.into(),
            changes: vec![
                Change::Summary("unsaved".into()),
                Change::Due(Some(Due::Date(
                    chrono::NaiveDate::from_ymd_opt(2026, 12, 11).unwrap(),
                ))),
            ],
            to: None,
        };
        assert!(store.apply(&e.snap(), &op, now()).is_err());
        assert_eq!(e.files(), before);
        assert_eq!(e.journal().len(), journal_len);
    }

    #[test]
    fn checklist_completion_after_deletion_matches_phone_history_and_reset() {
        for (from, to, parents, total) in [
            (
                "011d-phone-delete-items",
                "012-phone-family-complete",
                [CHECKLIST_A_UID, CHECKLIST_B_UID],
                9,
            ),
            (
                "028-phone-writer-note-followup",
                "029-phone-second-family-reset",
                ["55180c631f304b44be543382c0b85b94", "twydo-p8-c29-d"],
                20,
            ),
        ] {
            let e = env(from);
            let mut store = e.store(Matrix::embedded().clone());
            let phone = env(to);
            let after_snap = phone.snap();
            for parent in parents {
                let uid = member(&e.snap(), parent, "z");
                let title = e.snap().tasks[e.snap().find(parent).unwrap()]
                    .summary
                    .clone();
                let history_parent = after_snap
                    .tasks
                    .iter()
                    .find(|t| {
                        t.summary == title && t.status == twydo_core::model::Status::Completed
                    })
                    .unwrap();
                let child = after_snap.children(after_snap.find(&history_parent.uid).unwrap())[0];
                let item_at = crate::common::completed(&cal(to, &after_snap.tasks[child].uid));
                store
                    .apply(&e.snap(), &Op::Complete { uid: uid.clone() }, item_at)
                    .unwrap();
                assert_eq!(
                    e.snap().tasks[e.snap().find(parent).unwrap()].status,
                    twydo_core::model::Status::Open
                );
                let before = e.files();
                let parent_at = crate::common::completed(&cal(to, &history_parent.uid));
                store
                    .apply(&e.snap(), &Op::Complete { uid: parent.into() }, parent_at)
                    .unwrap();
                let entry = e.journal().pop().unwrap();
                assert_eq!(entry.writes.len(), 4);
                let mut ids = BTreeMap::new();
                for w in entry.writes.iter().filter(|w| w.before.is_none()) {
                    let got = parse(w.after.as_ref().unwrap().as_bytes()).unwrap();
                    let t = todo(&got);
                    let ours = t.prop("UID").unwrap().value.clone();
                    let theirs = if t.prop("RELATED-TO").is_some() {
                        after_snap.tasks[child].uid.clone()
                    } else {
                        history_parent.uid.clone()
                    };
                    ids.insert(ours, theirs);
                }
                for w in &entry.writes {
                    let got = parse(w.after.as_ref().unwrap().as_bytes()).unwrap();
                    let ours = &todo(&got).prop("UID").unwrap().value;
                    let theirs = ids.get(ours).unwrap_or(ours);
                    let mut want = cal(to, theirs);
                    if parent == CHECKLIST_B_UID && ours == &uid {
                        // Captured alternatives, not ignored metadata: the phone's A
                        // reset retains explicit 0; its converted B reset omits it.
                        // Keep the existing explicit-zero writer and require its
                        // subsequent phone return before promoting candidate verdicts.
                        assert_eq!(todo(&got).prop("PERCENT-COMPLETE").unwrap().value, "0");
                        assert!(todo(&want).prop("PERCENT-COMPLETE").is_none());
                        assert_eq!(todo(&want).prop("STATUS").unwrap().value, "NEEDS-ACTION");
                        want.root
                            .children_named_mut("VTODO")
                            .next()
                            .unwrap()
                            .insert_prop(
                                twydo_core::ical::Property::new("PERCENT-COMPLETE", vec![], "0"),
                                None,
                            );
                    }
                    same("post-delete family reset", &got, &want, &ids);
                }
                store.undo_last(parent_at).unwrap();
                assert_eq!(e.files(), before);
                store.redo_last(parent_at).unwrap();
            }
            assert_eq!(e.files().len(), total);
            assert!(!e.snap().tasks.iter().any(|t| t.summary == "q"));
        }
    }

    #[test]
    fn checklist_delete_recovery_reuses_record_and_undo_preserves_remote_edit() {
        let e = env("011-phone-items-and-text");
        let mut store = e.store(Matrix::embedded().clone());
        let before = e.files();
        let uid = member(&e.snap(), CHECKLIST_A_UID, "q");
        store
            .apply(&e.snap(), &Op::Delete { uid: uid.clone() }, now())
            .unwrap();
        let after = e.files();
        let mut entry = e.journal().pop().unwrap();
        entry.stage = Stage::Prepared;
        let recovering = env("011-phone-items-and-text");
        drop(recovering.store(Matrix::embedded().clone()));
        Journal::open(&recovering.state)
            .unwrap()
            .prepare(&entry)
            .unwrap();
        let (_, report) = Store::open_with(&recovering.cfg, Matrix::embedded().clone()).unwrap();
        assert!(report.stuck.is_empty());
        assert_eq!(recovering.files(), after);
        // A remotely recreated/edited item must not be overwritten by undo.
        let remote = String::from_utf8(before[&key(&uid)].clone())
            .unwrap()
            .replace("SUMMARY:q", "SUMMARY:remote synthetic");
        fs::write(e.file(&uid), remote.as_bytes()).unwrap();
        let with_remote = e.files();
        assert!(matches!(store.undo_last(now()), Err(WriteError::Stale(_))));
        assert_eq!(e.files(), with_remote);
    }
    #[test]
    fn checklist_delete_dependencies_survive_undo_redo_and_recovery() {
        for action in [
            "undo-parent-removed",
            "redo-sibling-removed",
            "recover-parent-changed",
        ] {
            let e = env("011-phone-items-and-text");
            let mut store = e.store(Matrix::embedded().clone());
            let snap = e.snap();
            let uid = member(&snap, CHECKLIST_A_UID, "q");
            let sibling = member(&snap, CHECKLIST_A_UID, "z");
            let id = store
                .apply(&snap, &Op::Delete { uid: uid.clone() }, now())
                .unwrap();
            match action {
                "undo-parent-removed" => {
                    fs::remove_file(e.file(CHECKLIST_A_UID)).unwrap();
                    let before = e.files();
                    assert!(matches!(store.undo_last(now()), Err(WriteError::Stale(_))));
                    assert_eq!(e.files(), before);
                    assert!(!e.file(&uid).exists(), "undo cannot recreate an orphan");
                }
                "redo-sibling-removed" => {
                    store.undo_last(now()).unwrap();
                    fs::remove_file(e.file(&sibling)).unwrap();
                    let before = e.files();
                    assert!(matches!(store.redo_last(now()), Err(WriteError::Stale(_))));
                    assert_eq!(e.files(), before);
                    assert!(e.file(&uid).exists(), "redo cannot delete the last item");
                }
                "recover-parent-changed" => {
                    let mut entry = Journal::open(&e.state).unwrap().get(&id).unwrap().unwrap();
                    entry.stage = Stage::Prepared;
                    Journal::open(&e.state).unwrap().prepare(&entry).unwrap();
                    // Simulate crash before the removal, followed by an independent edit.
                    fs::write(
                        e.file(&uid),
                        entry
                            .writes
                            .iter()
                            .find(|w| w.path == key(&uid))
                            .unwrap()
                            .before
                            .as_ref()
                            .unwrap(),
                    )
                    .unwrap();
                    let p = e.file(CHECKLIST_A_UID);
                    let bytes = fs::read(&p).unwrap();
                    fs::write(p, [bytes, b"\r\n".to_vec()].concat()).unwrap();
                    let before = e.files();
                    let (_, report) = Store::open_with(&e.cfg, Matrix::embedded().clone()).unwrap();
                    assert_eq!(report.stuck.len(), 1);
                    assert_eq!(e.files(), before);
                    assert!(e.file(&uid).exists());
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn checklist_item_text_edits_hold_the_parent_for_edit_and_save() {
        for grouped in [false, true] {
            for removed in [false, true] {
                let e = env("011-phone-items-and-text");
                let mut store = e.store(Matrix::embedded().clone());
                let snap = e.snap();
                let uid = member(&snap, CHECKLIST_A_UID, "z");
                let p = e.file(CHECKLIST_A_UID);
                if removed {
                    fs::remove_file(p).unwrap();
                } else {
                    let text = fs::read_to_string(&p)
                        .unwrap()
                        .replace("FREQ=MONTHLY", "FREQ=WEEKLY");
                    fs::write(p, text).unwrap();
                }
                let changes = vec![Change::Summary("kept unsaved".into())];
                let op = if grouped {
                    Op::Save {
                        uid,
                        changes,
                        to: None,
                    }
                } else {
                    Op::Edit { uid, changes }
                };
                let before = e.files();
                assert!(matches!(
                    store.apply(&snap, &op, now()),
                    Err(WriteError::Stale(_))
                ));
                assert_eq!(e.files(), before);
                assert!(e.journal().is_empty());
            }
        }
    }
}
