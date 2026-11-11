//! The store on a checklist or project: completing, reopening, moving or deleting the parent
//! writes the whole family in one journal entry, as observed in `complete/family-lifecycle` and
//! `move/family-destination`, against a tempdir copy of the iPhone capture.

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

const FIXTURE: &str = "complete/family-lifecycle/reopen-child/list1";
const L1: &str = "complete/family-lifecycle";
const AFTER: &str = "complete-parent/list1";

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

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 4, 15, 0, 0).unwrap()
}

fn later() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 4, 15, 5, 0).unwrap()
}

/// Every family case passes in test mode, with the edge cases a repeating completion needs.
fn matrix() -> Matrix {
    common::matrix(
        "unverified",
        &[
            (twydo_core::compat::ContractId::CompleteStartOffsets, "pass"),
            (twydo_core::compat::ContractId::CompleteLateMonthly, "pass"),
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
            "pass",
            &[
                (twydo_core::compat::ContractId::CompleteStartOffsets, "pass"),
                (twydo_core::compat::ContractId::CompleteLateMonthly, "pass"),
                (twydo_core::compat::ContractId::CompleteUndo, undo),
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
const T1: &str = "create/family-roundtrip";
const C: &str = "c05c77f7a0bc42d79f32d95ebf519ea3";
const C_ITEMS: [&str; 3] = [
    "f2bcae42b8f84b37b48c18b7f89c5ac0",
    "9e8d9baa64ba4737ad5cdb5566c6a992",
    "a80ca289da5a4ff88e80ee3754703699",
];

fn dated_checklist() -> (Env, Store) {
    let env = Env::new(WriteMode::Test, "create/family-roundtrip/setup/list1");
    let store = env.store(common::matrix("unverified", &[]));
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
        let want = fixture(T1, "initial-phone-edit/list1", uid);
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

// Plan 8 FCa: a recurring monthly checklist whose items carry the inherited Dec 10 date
// (`complete/checklist-and-until/first-phone-actions/list1`). The phone's completion reset in `020` removes the
// live items' inherited dates; a checklist due edit uses the same cleanup writer.
const FC: &str = "61ed74be3ba34212ab7476b698de1263";

fn recurring_checklist_env(step: &str) -> (Env, Store) {
    let env = Env::new(
        WriteMode::Verified,
        &format!("complete/checklist-and-until/{step}"),
    );
    let store = env.store(Matrix::embedded().clone());
    (env, store)
}

fn due(uid: &str, y: i32, m: u32, d: u32) -> Op {
    Op::Edit {
        uid: uid.into(),
        changes: vec![Change::Due(Some(twydo_core::edit::Due::Date(
            chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap(),
        )))],
    }
}

/// A resource's non-stamp lines with the given date properties removed.
fn without_dates(bytes: &[u8], names: &[&str]) -> Vec<Line> {
    sorted(
        lines(&parse(bytes).unwrap())
            .into_iter()
            .filter(|l| !is_stamp(&l.1) && !names.contains(&l.1.as_str()))
            .collect(),
    )
}

#[test]
fn checklist_due_capability_changes_parent_and_cleans_inherited_items() {
    let (env, mut store) = recurring_checklist_env("first-phone-actions/list1");
    let snap = env.snap();
    let p = snap.find(FC).unwrap();
    let children: Vec<String> = snap
        .children(p)
        .iter()
        .map(|&c| snap.tasks[c].uid.clone())
        .collect();
    assert_eq!(children.len(), 3);

    // Unknown properties on the parent and an item must survive the scheduling edit.
    for uid in std::iter::once(FC).chain(children.iter().map(String::as_str)) {
        let path = env.file(uid);
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("END:VTODO", "X-SYNTHETIC-KEEP:present\r\nEND:VTODO");
        fs::write(path, text).unwrap();
    }
    let snap = env.snap();
    let before: BTreeMap<String, Vec<u8>> = env.files();
    let parent_before = fs::read(env.file(FC)).unwrap();
    let child_before: BTreeMap<String, Vec<u8>> = children
        .iter()
        .map(|uid| (uid.clone(), fs::read(env.file(uid)).unwrap()))
        .collect();

    let id = store.apply(&snap, &due(FC, 2026, 12, 11), now()).unwrap();
    let entry = env.journal().into_iter().find(|e| e.id == id).unwrap();
    assert_eq!(entry.writes.len(), 4, "parent and three inherited items");
    assert_eq!(entry.uids[0], FC);
    assert!(entry.family.is_some(), "family guard must be durable");

    // The parent keeps every representation except its two date lines.
    let expected_parent = String::from_utf8(parent_before)
        .unwrap()
        .replace("DTSTART;VALUE=DATE:20261210", "DTSTART;VALUE=DATE:20261211")
        .replace("DUE;VALUE=DATE:20261210", "DUE;VALUE=DATE:20261211");
    let parent_after = fs::read(env.file(FC)).unwrap();
    assert_eq!(
        content(&parent_after),
        content(expected_parent.as_bytes()),
        "parent differs only in its date"
    );
    let parent = todo_of(&parent_after);
    assert_eq!(value(&parent, "RRULE").as_deref(), Some("FREQ=MONTHLY"));
    assert_eq!(value(&parent, "STATUS").as_deref(), Some("NEEDS-ACTION"));
    assert!(value(&parent, "X-SYNTHETIC-KEEP").is_some());

    // Each item loses exactly its inherited DUE/DTSTART, as the phone's `020` reset does.
    for uid in &children {
        let after = fs::read(env.file(uid)).unwrap();
        assert_eq!(
            content(&after),
            without_dates(&child_before[uid], &["DUE", "DTSTART"]),
            "{uid}: only inherited dates change"
        );
        let want = common::recurrence::cal(common::recurrence::ADVANCE, uid);
        let want_lines = sorted(
            lines(&want)
                .into_iter()
                .filter(|l| !is_stamp(&l.1))
                .filter(|l| ["DUE", "DTSTART"].contains(&l.1.as_str()))
                .collect(),
        );
        let got_lines = sorted(
            lines(&parse(&after).unwrap())
                .into_iter()
                .filter(|l| !is_stamp(&l.1))
                .filter(|l| ["DUE", "DTSTART"].contains(&l.1.as_str()))
                .collect(),
        );
        assert_eq!(got_lines, want_lines, "{uid}: matches captured cleanup");
    }

    // Unrelated resources are byte-identical.
    let after: BTreeMap<String, Vec<u8>> = env.files();
    for (path, bytes) in &before {
        if path != &format!("list1/{FC}.ics")
            && !children
                .iter()
                .any(|uid| path == &format!("list1/{uid}.ics"))
        {
            assert_eq!(&after[path], bytes, "{path}");
        }
    }
}

#[test]
fn checklist_due_capability_handles_empty_and_undated_checklists() {
    // Empty: only the parent resource is written.
    let (env, mut store) = recurring_checklist_env("first-phone-actions/list1");
    let snap = env.snap();
    for &c in snap.children(snap.find(FC).unwrap()) {
        fs::remove_file(&snap.tasks[c].path).unwrap();
    }
    let snap = env.snap();
    assert!(snap.children(snap.find(FC).unwrap()).is_empty());
    let before = env.files();
    let id = store.apply(&snap, &due(FC, 2026, 12, 11), now()).unwrap();
    let entry = env.journal().into_iter().find(|e| e.id == id).unwrap();
    assert_eq!(entry.writes.len(), 1);
    let after = env.files();
    for (path, bytes) in &before {
        if path == &format!("list1/{FC}.ics") {
            assert_ne!(&after[path], bytes);
        } else {
            assert_eq!(&after[path], bytes, "{path}");
        }
    }

    // Undated items: the parent moves and item bytes stay exactly as loaded.
    let e = Env::new(
        WriteMode::Verified,
        "complete/checklist-reset/phone-items-and-text/list1",
    );
    let mut store = e.store(Matrix::embedded().clone());
    let initial = e.snap();
    for &i in initial.children(initial.find(common::recurrence::CHECKLIST_A_UID).unwrap()) {
        let path = &initial.tasks[i].path;
        let mut cal = parse(&fs::read(path).unwrap()).unwrap();
        let todo = cal.root.children_named_mut("VTODO").next().unwrap();
        todo.props
            .retain(|p| p.name != "DUE" && p.name != "DTSTART");
        fs::write(path, twydo_core::ical::serialize(&cal)).unwrap();
    }
    let snap = e.snap();
    let parent = snap.find(common::recurrence::CHECKLIST_A_UID).unwrap();
    assert!(
        snap.children(parent)
            .iter()
            .all(|&i| snap.tasks[i].due.is_none())
    );
    let children: Vec<String> = snap
        .children(parent)
        .iter()
        .map(|&c| snap.tasks[c].uid.clone())
        .collect();
    assert!(!children.is_empty());
    let before = e.files();
    let id = store
        .apply(
            &snap,
            &due(common::recurrence::CHECKLIST_A_UID, 2026, 12, 11),
            now(),
        )
        .unwrap();
    let entry = e
        .journal()
        .into_iter()
        .find(|entry| entry.id == id)
        .unwrap();
    assert_eq!(entry.writes.len(), 1);
    for uid in &children {
        assert_eq!(
            e.files()[&format!("list1/{uid}.ics")],
            before[&format!("list1/{uid}.ics")],
            "{uid}"
        );
    }
}

#[test]
fn checklist_due_capability_refuses_neighbors_atomically() {
    let (env, mut store) = recurring_checklist_env("first-phone-actions/list1");
    let before = env.files();
    for (why, op) in [
        ("day 29", due(FC, 2026, 12, 29)),
        (
            "missing due",
            Op::Edit {
                uid: FC.into(),
                changes: vec![Change::Due(None)],
            },
        ),
    ] {
        let snap = env.snap();
        assert!(
            store.apply(&snap, &op, now()).is_err(),
            "{why}: gate refuses"
        );
        assert_eq!(env.files(), before, "{why}: no writes");
        assert!(env.journal().is_empty(), "{why}: no journal entry");
    }

    // A child changed after load: the store refuses without touching the parent.
    let (env, mut store) = recurring_checklist_env("first-phone-actions/list1");
    let snap = env.snap();
    let child = snap.tasks[snap.children(snap.find(FC).unwrap())[0]]
        .uid
        .clone();
    let path = env.file(&child);
    let original = fs::read_to_string(&path).unwrap();
    let changed = original.replace("END:VTODO", "X-SYNTHETIC-REMOTE:changed\r\nEND:VTODO");
    assert_ne!(original, changed);
    fs::write(&path, changed).unwrap();
    let before = env.files();
    match store.apply(&snap, &due(FC, 2026, 12, 11), now()) {
        Err(WriteError::Stale(p)) => assert_eq!(p, path),
        other => panic!("expected Stale, got {other:?}"),
    }
    assert_eq!(env.files(), before);
    assert!(env.journal().is_empty());
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
    let mut cal = parse(&fs::read(env.file(K3)).unwrap()).unwrap();
    let todo = cal.root.children_named_mut("VTODO").next().unwrap();
    todo.prop_mut("UID")
        .unwrap()
        .set_value("0000000000000000000000000000cafe");
    todo.prop_mut("RELATED-TO").unwrap().set_value(P);
    // Rewrite parsed metadata too: folding can split its parent UID across raw lines.
    let prop = todo.prop_mut("X-2DOAPP-METADATA").unwrap();
    let mut meta = twydo_core::model::meta_edit::MetaJson::parse(&prop.value).unwrap();
    meta.set_str("parentuid", P);
    prop.set_value(&meta.encode());
    fs::write(
        env.file("0000000000000000000000000000cafe"),
        twydo_core::ical::serialize(&cal),
    )
    .unwrap();
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
        let text = fixture(L1, "reopen-child/list1", K2)
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
    let text = String::from_utf8(fixture(L1, "reopen-child/list1", K2))
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
    let m = common::matrix(
        "unverified",
        &[
            (twydo_core::compat::ContractId::CompleteStartOffsets, "pass"),
            (twydo_core::compat::ContractId::CompleteLateMonthly, "pass"),
        ],
    );
    let _ = rules;
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

mod item_order {
    use super::*;
    use twydo_core::model::meta_edit::MetaJson;

    fn setup(kind: TaskKind) -> (Env, Store, String) {
        let scenario = match kind {
            TaskKind::Checklist => "insert/checklist-items",
            TaskKind::Project => "insert/project-items",
            _ => unreachable!(),
        };
        let env = Env::new(WriteMode::Verified, &format!("{scenario}/created/list1"));
        let store = env.store(Matrix::embedded().clone());
        let parent = env
            .snap()
            .tasks
            .iter()
            .find(|t| t.parent.is_none())
            .unwrap()
            .uid
            .clone();
        (env, store, parent)
    }

    fn op(parent: &str, title: &str) -> Op {
        Op::Create(NewTask {
            parent: Some(parent.into()),
            ..NewTask::plain("list1", title)
        })
    }

    fn add(env: &Env, store: &mut Store, parent: &str, title: &str) -> String {
        let id = store.apply(&env.snap(), &op(parent, title), now()).unwrap();
        env.journal().iter().find(|e| e.id == id).unwrap().uids[0].clone()
    }

    fn mutate(env: &Env, uid: &str, order: Option<i64>, completed: bool) {
        let path = env.file(uid);
        let mut cal = parse(&fs::read(&path).unwrap()).unwrap();
        let todo = cal.root.children_named_mut("VTODO").next().unwrap();
        let prop = todo.prop_mut("X-2DOAPP-METADATA").unwrap();
        let mut meta = MetaJson::parse(&prop.value).unwrap();
        if let Some(order) = order {
            meta.set_i64("DisplayOrder", order);
        } else {
            meta.set_str("DisplayOrder", "bad");
        }
        prop.set_value(&meta.encode());
        if completed {
            todo.prop_mut("STATUS").unwrap().set_value("COMPLETED");
        }
        fs::write(path, twydo_core::ical::serialize(&cal)).unwrap();
    }

    #[test]
    fn append_matches_phone_positions_and_keeps_same_second_entry_order() {
        for kind in [TaskKind::Checklist, TaskKind::Project] {
            let (env, mut store, parent) = setup(kind);
            let before = env.files();
            let bravo = add(&env, &mut store, &parent, "Bravo");
            let able = add(&env, &mut store, &parent, "Able");
            let snap = env.snap();
            let (b, a) = (snap.find(&bravo).unwrap(), snap.find(&able).unwrap());
            assert_eq!(snap.tasks[b].meta.display_order(), 3);
            assert_eq!(snap.tasks[a].meta.display_order(), 4);
            assert_eq!(snap.tasks[b].created, snap.tasks[a].created);
            assert_eq!(
                snap.tasks[b].created,
                Some(twydo_core::ical::value::IcalDateTime::Utc(now()))
            );
            for (path, bytes) in &before {
                assert_eq!(&env.files()[path], bytes);
            }
            let result = twydo_core::views::build_view(
                &snap,
                &twydo_core::views::View::All,
                now().with_timezone(&chrono_tz::America::New_York),
                &std::collections::HashSet::from([parent.clone()]),
            );
            let titles: Vec<_> = result
                .groups
                .iter()
                .flat_map(|(_, rows)| rows)
                .filter(|r| r.depth == 1)
                .map(|r| snap.tasks[r.task].summary.as_str())
                .collect();
            assert_eq!(titles, ["Zulu", "Alpha", "Mike", "Bravo", "Able"]);

            let after = env.files();
            store.undo_last(later()).unwrap();
            assert!(!env.file(&able).exists());
            store.undo_last(later()).unwrap();
            assert_eq!(env.files(), before);
            store.redo_last(later()).unwrap();
            store.redo_last(later()).unwrap();
            assert_eq!(env.files(), after);
        }
    }

    #[test]
    fn append_handles_empty_sparse_completed_and_malformed_siblings() {
        for kind in [TaskKind::Checklist, TaskKind::Project] {
            for case in ["empty", "sparse-completed", "malformed"] {
                let (env, mut store, parent) = setup(kind);
                let snap = env.snap();
                let kids = snap.children(snap.find(&parent).unwrap());
                match case {
                    "empty" => {
                        for &i in kids {
                            fs::remove_file(&snap.tasks[i].path).unwrap();
                        }
                    }
                    "sparse-completed" => mutate(&env, &snap.tasks[kids[0]].uid, Some(41), true),
                    _ => {
                        for &i in kids {
                            mutate(&env, &snap.tasks[i].uid, None, false);
                        }
                    }
                }
                let before = env.files();
                let uid = add(&env, &mut store, &parent, "New item");
                let snap = env.snap();
                assert_eq!(
                    snap.tasks[snap.find(&uid).unwrap()].meta.display_order(),
                    match case {
                        "empty" => 0,
                        "sparse-completed" => 42,
                        _ => 3,
                    }
                );
                for (path, bytes) in before {
                    assert_eq!(env.files()[&path], bytes);
                }
            }
        }
    }

    #[test]
    fn append_refuses_position_overflow_without_writes() {
        for kind in [TaskKind::Checklist, TaskKind::Project] {
            let (env, mut store, parent) = setup(kind);
            let child = env
                .snap()
                .tasks
                .iter()
                .find(|t| t.parent.is_some())
                .unwrap()
                .uid
                .clone();
            mutate(&env, &child, Some(i64::MAX), false);
            let before = env.files();
            let err = store
                .apply(&env.snap(), &op(&parent, "Overflow"), now())
                .unwrap_err();
            assert!(err.to_string().contains("item order"), "{err}");
            assert_eq!(env.files(), before);
            assert!(env.journal().is_empty());
        }
    }

    #[test]
    fn append_refuses_stale_sibling_changes_removals_and_joins() {
        for kind in [TaskKind::Checklist, TaskKind::Project] {
            for change in ["edit", "remove", "join"] {
                let (env, mut store, parent) = setup(kind);
                let snap = env.snap();
                let child = snap.tasks.iter().find(|t| t.parent.is_some()).unwrap();
                match change {
                    "edit" => mutate(&env, &child.uid, Some(99), false),
                    "remove" => fs::remove_file(&child.path).unwrap(),
                    _ => {
                        add(&env, &mut store, &parent, "Concurrent item");
                    }
                }
                let before = env.files();
                let count = env.journal().len();
                let result = store.apply(&snap, &op(&parent, "Stale item"), now());
                assert!(
                    matches!(result, Err(WriteError::Stale(_))),
                    "{kind:?}, {change}: {result:?}"
                );
                assert_eq!(env.files(), before);
                assert_eq!(env.journal().len(), count);
            }
        }
    }

    #[test]
    fn append_recovery_is_idempotent_and_guards_sibling_dependencies() {
        for kind in [TaskKind::Checklist, TaskKind::Project] {
            for interruption in [
                "after-write",
                "before-write",
                "sibling-edit",
                "sibling-join",
            ] {
                let (env, mut store, parent) = setup(kind);
                let snap = env.snap();
                let uid = add(&env, &mut store, &parent, "Bravo");
                let entry = env.journal().pop().unwrap();
                assert_eq!(entry.holds.len(), 4);
                assert!(entry.family.as_ref().unwrap().loaded.contains(&key(&uid)));
                match interruption {
                    "before-write" => fs::remove_file(env.file(&uid)).unwrap(),
                    "sibling-edit" => mutate(
                        &env,
                        &snap.tasks[snap.children(snap.find(&parent).unwrap())[0]].uid,
                        Some(91),
                        false,
                    ),
                    "sibling-join" => {
                        add(&env, &mut store, &parent, "Concurrent item");
                    }
                    _ => {}
                }
                Journal::open(&env.state)
                    .unwrap()
                    .set_stage(&entry.id, Stage::Prepared)
                    .unwrap();
                let before = env.files();
                let (_, report) = Store::open_with(&env.cfg, Matrix::embedded().clone()).unwrap();
                if interruption.starts_with("sibling-") {
                    assert_eq!(report.stuck.len(), 1, "{report:?}");
                    assert_eq!(env.files(), before);
                } else {
                    assert!(report.stuck.is_empty(), "{report:?}");
                    assert_eq!(env.snap().tasks.iter().filter(|t| t.uid == uid).count(), 1);
                    assert_eq!(
                        fs::read_to_string(env.file(&uid)).unwrap(),
                        entry.writes[0].after.as_ref().unwrap().as_str()
                    );
                }
            }
        }
    }

    #[test]
    fn completion_reopen_and_family_move_preserve_item_positions() {
        for kind in [TaskKind::Checklist, TaskKind::Project] {
            let (env, mut store, parent) = setup(kind);
            let uid = add(&env, &mut store, &parent, "Bravo");
            for op in [
                Op::Complete {
                    uid: parent.clone(),
                },
                Op::Reopen {
                    uid: parent.clone(),
                },
            ] {
                store.apply(&env.snap(), &op, later()).unwrap();
                let snap = env.snap();
                assert_eq!(snap.tasks[snap.find(&uid).unwrap()].meta.display_order(), 3);
            }
            store
                .apply(
                    &env.snap(),
                    &Op::Move {
                        uid: parent,
                        to: "list2".into(),
                    },
                    later(),
                )
                .unwrap();
            let snap = env.snap();
            let t = snap.tasks.iter().find(|t| t.summary == "Bravo").unwrap();
            assert_eq!(t.collection, "list2");
            assert_eq!(t.meta.display_order(), 3);
        }
    }

    #[test]
    fn recurring_parent_completion_preserves_positions_in_live_and_history_items() {
        for (scenario, parent) in [
            (FIXTURE, R),
            ("complete/checklist-reset/phone-delete-items/list1", ""),
        ] {
            let env = Env::new(WriteMode::Test, scenario);
            let mut store = env.store(Matrix::embedded().clone());
            let parent = if parent.is_empty() {
                env.snap()
                    .tasks
                    .iter()
                    .find(|t| t.summary == "B")
                    .unwrap()
                    .uid
                    .clone()
            } else {
                parent.to_string()
            };
            let uid = add(&env, &mut store, &parent, "Order item");
            let before = env.files();
            let position = env
                .snap()
                .tasks
                .iter()
                .find(|t| t.uid == uid)
                .unwrap()
                .meta
                .display_order();
            assert!(position > 0);
            store
                .apply(&env.snap(), &Op::Complete { uid: parent }, later())
                .unwrap();
            let snap = env.snap();
            let copies: Vec<_> = snap
                .tasks
                .iter()
                .filter(|t| t.summary == "Order item")
                .collect();
            assert_eq!(copies.len(), 2);
            assert!(copies.iter().all(|t| t.meta.display_order() == position));
            assert_eq!(
                copies
                    .iter()
                    .filter(|t| t.status == twydo_core::model::Status::Open)
                    .count(),
                1
            );
            store.undo_last(later()).unwrap();
            assert_eq!(env.files(), before);
        }
    }
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
        const BEFORE: &str = "phone-removal-and-history-followup/list1";
        const AFTER: &str = "twydo-checklist-repeat-removed/list1";
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
            .contract
            .iter_mut()
            .find(|r| r.id == twydo_core::compat::ContractId::SetRepeatChecklist)
            .unwrap()
            .twydo_to_iphone = Verdict::Pass;
        matrix
            .contract
            .iter_mut()
            .find(|r| r.id == twydo_core::compat::ContractId::SetRepeatChecklist)
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
            .contract
            .iter_mut()
            .find(|r| r.id == twydo_core::compat::ContractId::SetRepeatChecklist)
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
        let evidence = cal("phone-create-convert/list1", CHECKLIST_A_UID);
        let e = env("seeded/list1");
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
        let evidence = cal("phone-create-convert/list1", CHECKLIST_B_UID);
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

        let e = env("phone-create-convert/list1");
        let mut store = e.store(Matrix::embedded().clone());
        let phone = Env::new(
            WriteMode::Verified,
            &format!("{CHECKLIST_B}/phone-items-and-text/list1"),
        );
        let phone_uid = member(&phone.snap(), CHECKLIST_B_UID, "q");
        let evidence = cal("phone-items-and-text/list1", &phone_uid);
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
        let e = env("phone-items-and-text/list1");
        let mut store = e.store(Matrix::embedded().clone());
        let before = e.files();
        for parent in [CHECKLIST_A_UID, CHECKLIST_B_UID] {
            let uid = member(&e.snap(), parent, "q");
            store.apply(&e.snap(), &Op::Delete { uid }, now()).unwrap();
        }
        let phone = env("phone-delete-items/list1");
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
            let e = env("phone-items-and-text/list1");
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
        let e = env("phone-create-convert/list1");
        let mut store = e.store(Matrix::embedded().clone());
        for (uid, title) in [(CHECKLIST_A_UID, "A"), (CHECKLIST_B_UID, "B")] {
            let want = cal("phone-items-and-text/list1", uid);
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
                    chrono::NaiveDate::from_ymd_opt(2026, 12, 29).unwrap(),
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
                "phone-delete-items/list1",
                "phone-family-complete/list1",
                [CHECKLIST_A_UID, CHECKLIST_B_UID],
                9,
            ),
            (
                "phone-writer-note-followup/list1",
                "phone-second-family-reset/list1",
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
        let e = env("phone-items-and-text/list1");
        let mut store = e.store(Matrix::embedded().clone());
        let before = e.files();
        let uid = member(&e.snap(), CHECKLIST_A_UID, "q");
        store
            .apply(&e.snap(), &Op::Delete { uid: uid.clone() }, now())
            .unwrap();
        let after = e.files();
        let mut entry = e.journal().pop().unwrap();
        entry.stage = Stage::Prepared;
        let recovering = env("phone-items-and-text/list1");
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
            let e = env("phone-items-and-text/list1");
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
                let e = env("phone-items-and-text/list1");
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

    #[test]
    fn deletion_inverses_refuse_new_family_members_without_writes() {
        for redo in [false, true] {
            let e = env("phone-items-and-text/list1");
            let mut store = e.store(Matrix::embedded().clone());
            let snap = e.snap();
            let uid = member(&snap, CHECKLIST_A_UID, "q");
            let sibling = member(&snap, CHECKLIST_A_UID, "z");
            store
                .apply(&snap, &Op::Delete { uid: uid.clone() }, now())
                .unwrap();
            if redo {
                store.undo_last(now()).unwrap();
            }
            let text = fs::read_to_string(e.file(&sibling))
                .unwrap()
                .replace(&sibling, "new-synthetic-child")
                .replace("STATUS:NEEDS-ACTION", "STATUS:COMPLETED");
            fs::write(e.file("new-synthetic-child"), text).unwrap();
            let before = e.files();
            let entries = e.journal().len();
            let result = if redo {
                store.redo_last(now())
            } else {
                store.undo_last(now())
            };
            assert!(
                matches!(result, Err(WriteError::Stale(_))),
                "redo={redo}: {result:?}"
            );
            assert_eq!(e.files(), before);
            assert_eq!(e.journal().len(), entries);
        }
    }

    #[test]
    fn deletion_recovery_refuses_new_family_members_before_or_after_removal() {
        for removed in [false, true] {
            let e = env("phone-items-and-text/list1");
            let mut store = e.store(Matrix::embedded().clone());
            let snap = e.snap();
            let uid = member(&snap, CHECKLIST_A_UID, "q");
            let sibling = member(&snap, CHECKLIST_A_UID, "z");
            let id = store
                .apply(&snap, &Op::Delete { uid: uid.clone() }, now())
                .unwrap();
            let journal = Journal::open(&e.state).unwrap();
            let mut entry = journal.get(&id).unwrap().unwrap();
            entry.stage = Stage::Prepared;
            journal.prepare(&entry).unwrap();
            if !removed {
                fs::write(e.file(&uid), entry.writes[0].before.as_ref().unwrap()).unwrap();
            }
            let text = fs::read_to_string(e.file(&sibling))
                .unwrap()
                .replace(&sibling, "new-synthetic-child");
            fs::write(e.file("new-synthetic-child"), text).unwrap();
            let before = e.files();
            let (_, report) = Store::open_with(&e.cfg, Matrix::embedded().clone()).unwrap();
            assert_eq!(report.stuck.len(), 1, "removed={removed}");
            assert_eq!(e.files(), before);
        }
    }

    #[test]
    fn deletion_inverses_and_recovery_allow_unrelated_new_resources() {
        let e = env("phone-items-and-text/list1");
        let mut store = e.store(Matrix::embedded().clone());
        let snap = e.snap();
        let uid = member(&snap, CHECKLIST_A_UID, "q");
        let id = store
            .apply(&snap, &Op::Delete { uid: uid.clone() }, now())
            .unwrap();
        fs::write(
            e.file("unrelated"),
            b"BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:unrelated\r\nEND:VTODO\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        store.undo_last(now()).unwrap();
        assert!(e.file(&uid).exists());
        store.redo_last(now()).unwrap();
        assert!(!e.file(&uid).exists());
        let journal = Journal::open(&e.state).unwrap();
        let mut entry = journal.get(&id).unwrap().unwrap();
        entry.stage = Stage::Prepared;
        journal.prepare(&entry).unwrap();
        let before = e.files();
        let (_, report) = Store::open_with(&e.cfg, Matrix::embedded().clone()).unwrap();
        assert!(report.stuck.is_empty());
        assert_eq!(report.resumed, vec![id]);
        assert_eq!(e.files(), before);
    }
}

mod individual_item_deletion {
    use super::*;
    use common::item_deletion as evidence;

    #[test]
    fn item_deletions_match_phone_bytes_and_restore_exactly_through_undo_redo() {
        for (before_step, after_step, targets) in [
            (
                evidence::SEED,
                evidence::FIRST,
                vec!["twydo-i27-c-a-q", "twydo-i27-p-a-q"],
            ),
            (
                evidence::CLOSED,
                evidence::EMPTY,
                vec![
                    "twydo-i27-c-a-x",
                    "twydo-i27-p-a-x",
                    "twydo-i27-m-a-q",
                    "twydo-i27-w-a-q",
                ],
            ),
        ] {
            let e = evidence::env(before_step);
            let before = e.files();
            let mut store = e.store(evidence::matrix("pass"));
            for uid in &targets {
                let id = store
                    .apply(&e.snap(), &Op::Delete { uid: (*uid).into() }, now())
                    .unwrap();
                let entry = Journal::open(&e.state).unwrap().get(&id).unwrap().unwrap();
                assert_eq!(entry.writes.len(), 1);
                assert_eq!(entry.writes[0].path, key(uid));
                assert!(entry.writes[0].after.is_none());
                assert!(entry.family.is_some());
                assert!(!entry.holds.is_empty());
            }
            let after = evidence::env(after_step).files();
            assert_eq!(e.files(), after, "{before_step} -> {after_step}");
            for _ in &targets {
                store.undo_last(later()).unwrap();
            }
            assert_eq!(e.files(), before);
            for _ in &targets {
                store.redo_last(later()).unwrap();
            }
            assert_eq!(e.files(), after);
        }
    }
}

#[test]
fn individual_item_deletion_inverses_and_recovery_preserve_remote_family_changes() {
    use common::item_deletion as evidence;
    for (prefix, child) in [("c", "x"), ("p", "x"), ("m", "q"), ("w", "q")] {
        for action in [
            "undo-parent-change",
            "redo-new-sibling",
            "recover-parent-change",
        ] {
            let e = evidence::env(evidence::CLOSED);
            let mut store = e.store(evidence::matrix("pass"));
            let parent = format!("twydo-i27-{prefix}-a");
            let uid = format!("{parent}-{child}");
            let id = store
                .apply(&e.snap(), &Op::Delete { uid: uid.clone() }, now())
                .unwrap();
            let journal = Journal::open(&e.state).unwrap();
            journal.mark_synced(&id, later()).unwrap();
            if action == "redo-new-sibling" {
                store.undo_last(later()).unwrap();
                let content = fs::read_to_string(e.file(&uid))
                    .unwrap()
                    .replace(&uid, "new-synthetic-item");
                fs::write(e.file("new-synthetic-item"), content).unwrap();
            } else {
                let p = e.file(&parent);
                let content = fs::read_to_string(&p)
                    .unwrap()
                    .replace("DESCRIPTION:x", "DESCRIPTION:remote synthetic");
                fs::write(p, content).unwrap();
            }
            if action == "recover-parent-change" {
                let mut entry = journal.get(&id).unwrap().unwrap();
                entry.stage = Stage::Prepared;
                // Crash before the file was removed; recovery must not ignore remote parent changes.
                fs::write(e.file(&uid), entry.writes[0].before.as_ref().unwrap()).unwrap();
                journal.prepare(&entry).unwrap();
                let before = e.files();
                let (_, report) = Store::open_with(&e.cfg, evidence::matrix("pass")).unwrap();
                assert_eq!(report.stuck.len(), 1, "{prefix}: {action}");
                assert_eq!(e.files(), before);
            } else {
                let before = e.files();
                let count = e.journal().len();
                let result = if action == "redo-new-sibling" {
                    store.redo_last(later())
                } else {
                    store.undo_last(later())
                };
                assert!(
                    matches!(result, Err(WriteError::Stale(_))),
                    "{prefix}: {action}: {result:?}"
                );
                assert_eq!(e.files(), before);
                assert_eq!(e.journal().len(), count);
            }
        }
    }
}

#[test]
fn individual_item_deletion_preserves_opaque_content_and_unexecuted_schedules() {
    use common::item_deletion as evidence;
    use twydo_core::edit::Due;
    use twydo_core::ical::{Param, Property, serialize};
    let due = chrono::NaiveDate::from_ymd_opt(2026, 12, 10).unwrap();
    for (prefix, rule) in [
        ("c", "monthly"),
        ("p", "monthly after done"),
        ("m", "yearly 3 times"),
        ("w", "monthly until 2027-02-10"),
    ] {
        let e = evidence::env(evidence::SEED);
        let parent = format!("twydo-i27-{prefix}-a");
        let uid = format!("{parent}-q");
        for member in [&parent, &uid] {
            let path = e.file(member);
            let mut cal = parse(&fs::read(&path).unwrap()).unwrap();
            twydo_core::edit::apply_changes(
                &mut cal,
                &[
                    Change::Due(Some(Due::Time(due.and_hms_opt(9, 30, 0).unwrap()))),
                    Change::Start(Some(due.pred_opt().unwrap())),
                    Change::Repeat(twydo_core::repeatinput::parse(rule, due, &[]).unwrap()),
                ],
                now(),
            )
            .unwrap();
            let todo = cal
                .root
                .children
                .iter_mut()
                .find(|c| c.name == "VTODO")
                .unwrap();
            todo.insert_prop(
                Property::new(
                    "X-SYNTHETIC",
                    vec![Param {
                        name: "LABEL".into(),
                        values: vec!["one,two".into()],
                    }],
                    "opaque value",
                ),
                None,
            );
            let mut extra = Component::new("X-SYNTHETIC-COMPONENT");
            extra.insert_prop(Property::new("X-VALUE", vec![], "untouched"), None);
            todo.push_child(extra);
            let mut alarm = Component::new("VALARM");
            alarm.insert_prop(
                Property::new(
                    "TRIGGER",
                    vec![Param {
                        name: "VALUE".into(),
                        values: vec!["DATE-TIME".into()],
                    }],
                    "20261209T080000Z",
                ),
                None,
            );
            alarm.insert_prop(Property::new("ACTION", vec![], "DISPLAY"), None);
            alarm.insert_prop(
                Property::new("DESCRIPTION", vec![], "synthetic reminder"),
                None,
            );
            todo.push_child(alarm);
            fs::write(path, serialize(&cal)).unwrap();
        }
        let before = e.files();
        let mut expected = before.clone();
        expected.remove(&key(&uid));
        let mut store = e.store(evidence::matrix("pass"));
        store
            .apply(&e.snap(), &Op::Delete { uid: uid.clone() }, now())
            .unwrap();
        assert_eq!(e.files(), expected, "{rule}");
        store.undo_last(later()).unwrap();
        assert_eq!(e.files(), before, "opaque target must be restored exactly");
    }
}

#[test]
fn individual_history_item_deletion_keeps_live_family_and_other_history_untouched() {
    let e = Env::new(
        WriteMode::Verified,
        "complete/family-lifecycle/complete-parent/list1",
    );
    let mut store = e.store(common::item_deletion::matrix("pass"));
    let before = e.files();
    let mut after = before.clone();
    after.remove(&key(R1_COPY));
    store
        .apply(
            &e.snap(),
            &Op::Delete {
                uid: R1_COPY.into(),
            },
            now(),
        )
        .unwrap();
    assert_eq!(e.files(), after);
    assert_eq!(e.files()[&key(R1)], before[&key(R1)]);
    store.undo_last(later()).unwrap();
    assert_eq!(e.files(), before);
}

#[test]
fn individual_item_delete_recovery_is_idempotent_before_and_after_file_removal() {
    use common::item_deletion as evidence;
    for removed in [false, true] {
        let e = evidence::env(evidence::SEED);
        let mut store = e.store(evidence::matrix("pass"));
        let uid = "twydo-i27-p-a-q";
        let id = store
            .apply(&e.snap(), &Op::Delete { uid: uid.into() }, now())
            .unwrap();
        let after = e.files();
        let journal = Journal::open(&e.state).unwrap();
        let mut entry = journal.get(&id).unwrap().unwrap();
        entry.stage = Stage::Prepared;
        journal.prepare(&entry).unwrap();
        if !removed {
            fs::write(e.file(uid), entry.writes[0].before.as_ref().unwrap()).unwrap();
        }
        let (_, report) = Store::open_with(&e.cfg, evidence::matrix("pass")).unwrap();
        assert_eq!(report.resumed, vec![id]);
        assert!(report.stuck.is_empty());
        assert_eq!(e.files(), after);
        let (_, report) = Store::open_with(&e.cfg, evidence::matrix("pass")).unwrap();
        assert!(report.resumed.is_empty() && report.stuck.is_empty());
        assert_eq!(e.journal().len(), 1);
    }
}

#[test]
fn item_delete_refuses_raw_duplicate_identities_and_multiple_parent_links() {
    use common::item_deletion as evidence;
    for extra in ["UID:another-synthetic-identity", "RELATED-TO:twydo-i27-p-a"] {
        let e = evidence::env(evidence::SEED);
        let uid = "twydo-i27-c-a-q";
        let path = e.file(uid);
        let raw = fs::read_to_string(&path)
            .unwrap()
            .replace("END:VTODO", &format!("{extra}\nEND:VTODO"));
        fs::write(path, raw).unwrap();
        let snapshot = e.snap();
        assert!(
            snapshot.find(uid).is_some(),
            "unsupported resource stays readable"
        );
        let before = e.files();
        let mut store = e.store(evidence::matrix("pass"));
        assert!(
            matches!(
                store.apply(&snapshot, &Op::Delete { uid: uid.into() }, now()),
                Err(WriteError::Blocked(_))
            ),
            "{extra}"
        );
        assert_eq!(e.files(), before);
        assert!(e.journal().is_empty());
    }
}

#[test]
fn folded_parent_links_cannot_bypass_item_delete_family_guards() {
    use common::item_deletion as evidence;
    use twydo_core::store::{DrainOutcome, Submitted, fsio::VdirLock};
    for action in [
        "apply",
        "queue",
        "undo",
        "redo",
        "recover-before",
        "recover-after",
    ] {
        let e = evidence::env(evidence::SEED);
        let uid = "twydo-i27-c-a-q";
        let snapshot = e.snap();
        let mut store = e.store(evidence::matrix("pass"));
        let op = Op::Delete { uid: uid.into() };
        let mut queued_lock = None;
        if action == "queue" {
            queued_lock = Some(VdirLock::exclusive(&e.state).unwrap());
            assert!(matches!(
                store.submit(&snapshot, &op, "delete", "op", now()).unwrap(),
                Submitted::Queued(_)
            ));
        } else if action != "apply" {
            let id = store.apply(&snapshot, &op, now()).unwrap();
            if action == "redo" {
                store.undo_last(later()).unwrap();
            }
            if action.starts_with("recover-") {
                let journal = Journal::open(&e.state).unwrap();
                let mut entry = journal.get(&id).unwrap().unwrap();
                entry.stage = Stage::Prepared;
                journal.prepare(&entry).unwrap();
                if action == "recover-before" {
                    fs::write(e.file(uid), entry.writes[0].before.as_ref().unwrap()).unwrap();
                }
            }
        }
        // No metadata or other raw occurrence of the parent UID can hide the folding bug.
        let joined = b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:new-folded-item\r\nSUMMARY:synthetic\r\nRELATED-TO:twydo-i27-\r\n c-a\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let parsed = parse(joined).unwrap();
        assert_eq!(
            todo_of(joined).prop("RELATED-TO").unwrap().value,
            "twydo-i27-c-a"
        );
        assert_eq!(twydo_core::ical::serialize(&parsed), joined);
        fs::write(e.file("new-folded-item"), joined).unwrap();
        let before = e.files();
        match action {
            "apply" => assert!(matches!(
                store.apply(&snapshot, &op, later()),
                Err(WriteError::Stale(_))
            )),
            "queue" => {
                drop(queued_lock);
                let drained = store.drain(later()).unwrap();
                assert_eq!(drained.len(), 1);
                assert!(matches!(drained[0].outcome, DrainOutcome::Refused(_)));
            }
            "undo" => assert!(matches!(
                store.undo_last(later()),
                Err(WriteError::Stale(_))
            )),
            "redo" => assert!(matches!(
                store.redo_last(later()),
                Err(WriteError::Stale(_))
            )),
            _ => {
                let (_, report) = Store::open_with(&e.cfg, evidence::matrix("pass")).unwrap();
                assert_eq!(report.stuck.len(), 1, "{action}");
                assert!(report.resumed.is_empty());
            }
        }
        assert_eq!(e.files(), before, "{action}: no partial writes");
    }
}

fn join_checklist(env: &Env) {
    let snap = env.snap();
    let child = &snap.tasks[snap.children(snap.find(FC).unwrap())[0]];
    let original = fs::read_to_string(&child.path).unwrap();
    let joined = original.replace(&child.uid, "synthetic-new-checklist-child");
    assert_ne!(joined, original);
    fs::write(env.file("synthetic-new-checklist-child"), joined).unwrap();
}

#[test]
fn checklist_schedule_stale_members_refuse_atomically() {
    use twydo_core::store::{DrainOutcome, Submitted, fsio::VdirLock};
    for queued in [false, true] {
        for mutation in ["new", "changed", "removed"] {
            let (e, mut store) = recurring_checklist_env("first-phone-actions/list1");
            let snapshot = e.snap();
            let op = due(FC, 2026, 12, 11);
            let lock = queued.then(|| VdirLock::exclusive(&e.state).unwrap());
            if queued {
                assert!(matches!(
                    store.submit(&snapshot, &op, "due", "op", now()).unwrap(),
                    Submitted::Queued(_)
                ));
            }
            let child = &snapshot.tasks[snapshot.children(snapshot.find(FC).unwrap())[0]];
            match mutation {
                "new" => join_checklist(&e),
                "changed" => {
                    let original = fs::read_to_string(&child.path).unwrap();
                    let changed = original.replace("END:VTODO", "X-REMOTE:changed\r\nEND:VTODO");
                    assert_ne!(original, changed);
                    fs::write(&child.path, changed).unwrap();
                }
                _ => fs::remove_file(&child.path).unwrap(),
            }
            let before = e.files();
            if queued {
                drop(lock);
                let drained = store.drain(later()).unwrap();
                assert_eq!(drained.len(), 1);
                assert!(
                    matches!(drained[0].outcome, DrainOutcome::Refused(_)),
                    "{mutation}: {drained:?}"
                );
            } else {
                assert!(
                    matches!(
                        store.apply(&snapshot, &op, later()),
                        Err(WriteError::Stale(_))
                    ),
                    "{mutation}"
                );
            }
            assert_eq!(e.files(), before);
            assert!(e.journal().is_empty());
        }
    }
}

#[test]
fn checklist_schedule_inverse_and_recovery_keep_dates_consistent() {
    for interruption in [0, 1, 3] {
        let (e, mut store) = recurring_checklist_env("first-phone-actions/list1");
        let before = e.files();
        let id = store
            .apply(&e.snap(), &due(FC, 2026, 12, 11), now())
            .unwrap();
        let after = e.files();
        let journal = Journal::open(&e.state).unwrap();
        let mut entry = journal.get(&id).unwrap().unwrap();
        assert!(entry.family.is_some());
        for w in entry.writes.iter().skip(interruption) {
            fs::write(e.vdir.join(&w.path), w.before.as_ref().unwrap()).unwrap();
        }
        entry.stage = Stage::Prepared;
        journal.prepare(&entry).unwrap();
        let (_recovered, report) = Store::open_with(&e.cfg, Matrix::embedded().clone()).unwrap();
        assert_eq!(report.resumed, vec![id]);
        assert!(report.stuck.is_empty());
        assert_eq!(e.files(), after);
        let (_, report) = Store::open_with(&e.cfg, Matrix::embedded().clone()).unwrap();
        assert!(report.resumed.is_empty());
        store.undo_last(later()).unwrap();
        assert_eq!(
            e.files(),
            before,
            "exact dates, completed item and metadata restored"
        );
        store.redo_last(later()).unwrap();
        assert_eq!(e.files(), after);
    }
    for action in [
        "undo",
        "redo",
        "recover-before",
        "recover-partial",
        "remote-child",
    ] {
        let (e, mut store) = recurring_checklist_env("first-phone-actions/list1");
        let id = store
            .apply(&e.snap(), &due(FC, 2026, 12, 11), now())
            .unwrap();
        if action == "redo" {
            store.undo_last(later()).unwrap();
        }
        if action.starts_with("recover-") {
            let journal = Journal::open(&e.state).unwrap();
            let mut entry = journal.get(&id).unwrap().unwrap();
            entry.stage = Stage::Prepared;
            for w in entry
                .writes
                .iter()
                .skip(usize::from(action == "recover-partial"))
            {
                fs::write(e.vdir.join(&w.path), w.before.as_ref().unwrap()).unwrap();
            }
            journal.prepare(&entry).unwrap();
        }
        if action == "remote-child" {
            let snapshot = e.snap();
            let child = &snapshot.tasks[snapshot.children(snapshot.find(FC).unwrap())[0]];
            let original = fs::read_to_string(&child.path).unwrap();
            fs::write(
                &child.path,
                original.replace("END:VTODO", "X-REMOTE:changed\r\nEND:VTODO"),
            )
            .unwrap();
        } else {
            join_checklist(&e);
        }
        let before = e.files();
        if action.starts_with("recover-") {
            let (_, report) = Store::open_with(&e.cfg, Matrix::embedded().clone()).unwrap();
            assert_eq!(report.stuck.len(), 1, "{action}");
            assert!(report.resumed.is_empty());
        } else {
            let result = if action == "redo" {
                store.redo_last(later())
            } else {
                store.undo_last(later())
            };
            assert!(
                matches!(result, Err(WriteError::Stale(_))),
                "{action}: {result:?}"
            );
        }
        assert_eq!(e.files(), before, "{action}: atomic refusal");
    }
}

#[test]
fn checklist_interval_capability_cli_and_card_are_equivalent() {
    use twydo_core::compat::Verdict;
    let mut outputs = Vec::new();
    for card in [false, true] {
        let (e, _) = recurring_checklist_env("first-phone-actions/list1");
        let mut matrix = Matrix::embedded().clone();
        let row = matrix
            .contract
            .iter_mut()
            .find(|r| r.id == twydo_core::compat::ContractId::SetRepeatChecklistInterval)
            .unwrap();
        row.iphone_baseline = Verdict::Pass;
        row.twydo_to_iphone = Verdict::Pass;
        row.iphone_to_twydo = Verdict::Pass;
        let mut store = e.store(matrix.clone());
        let changes = if card {
            vec![Change::Repeat(
                twydo_core::repeatinput::parse("2m", now().date_naive(), &[]).unwrap(),
            )]
        } else {
            vec![Change::RepeatInterval(2)]
        };
        let before = e.files();
        store
            .apply(
                &e.snap(),
                &Op::Edit {
                    uid: FC.into(),
                    changes,
                },
                now(),
            )
            .unwrap();
        let entry = e.journal().pop().unwrap();
        assert_eq!(entry.writes.len(), 1);
        assert_eq!(entry.holds.len(), 3);
        assert!(entry.family.is_some());
        let after = e.files();
        for (path, bytes) in &before {
            if path != &key(FC) {
                assert_eq!(&after[path], bytes);
            }
        }
        let parent = todo_of(&after[&key(FC)]);
        assert_eq!(
            value(&parent, "RRULE").as_deref(),
            Some("FREQ=MONTHLY;INTERVAL=2")
        );
        let snapshot = e.snap();
        assert!(
            twydo_core::gate::check(
                &snapshot,
                &matrix,
                WriteMode::Verified,
                &Op::Complete { uid: FC.into() },
                now().date_naive()
            )
            .is_err(),
            "editing never grants interval-2 execution"
        );
        outputs.push(content(&after[&key(FC)]));
        join_checklist(&e);
        let before = e.files();
        assert!(matches!(
            store.undo_last(later()),
            Err(WriteError::Stale(_))
        ));
        assert_eq!(e.files(), before);
    }
    assert_eq!(outputs[0], outputs[1]);
}

#[test]
fn checklist_schedule_json_only_members_block_apply_queue_inverse_and_recovery() {
    use twydo_core::model::meta_edit::MetaJson;
    use twydo_core::store::{DrainOutcome, Submitted, fsio::VdirLock};
    for internal_alias in [false, true] {
        for action in [
            "apply",
            "queue",
            "undo",
            "redo",
            "recover-before",
            "recover-partial",
        ] {
            let (e, mut store) = recurring_checklist_env("first-phone-actions/list1");
            if internal_alias {
                let snapshot = e.snap();
                let parent = snapshot.find(FC).unwrap();
                for &i in std::iter::once(&parent).chain(snapshot.children(parent)) {
                    let mut cal = parse(&fs::read(&snapshot.tasks[i].path).unwrap()).unwrap();
                    let prop = cal
                        .root
                        .children_named_mut("VTODO")
                        .next()
                        .unwrap()
                        .prop_mut("X-2DOAPP-METADATA")
                        .unwrap();
                    let mut meta = MetaJson::parse(&prop.value).unwrap();
                    meta.set_str(
                        if i == parent { "uid" } else { "parentuid" },
                        "synthetic-internal-checklist",
                    );
                    prop.set_value(&meta.encode());
                    fs::write(&snapshot.tasks[i].path, twydo_core::ical::serialize(&cal)).unwrap();
                }
            }
            let snapshot = e.snap();
            let op = due(FC, 2026, 12, 11);
            let lock = (action == "queue").then(|| VdirLock::exclusive(&e.state).unwrap());
            if action == "queue" {
                assert!(matches!(
                    store.submit(&snapshot, &op, "due", "op", now()).unwrap(),
                    Submitted::Queued(_)
                ));
            } else if action != "apply" {
                let id = store.apply(&snapshot, &op, now()).unwrap();
                if action == "redo" {
                    store.undo_last(later()).unwrap();
                }
                if action.starts_with("recover-") {
                    let journal = Journal::open(&e.state).unwrap();
                    let mut entry = journal.get(&id).unwrap().unwrap();
                    entry.stage = Stage::Prepared;
                    for w in entry
                        .writes
                        .iter()
                        .skip(usize::from(action == "recover-partial"))
                    {
                        fs::write(e.vdir.join(&w.path), w.before.as_ref().unwrap()).unwrap();
                    }
                    journal.prepare(&entry).unwrap();
                }
            }
            join_checklist(&e);
            let path = e.file("synthetic-new-checklist-child");
            let mut cal = parse(&fs::read(&path).unwrap()).unwrap();
            cal.root
                .children_named_mut("VTODO")
                .next()
                .unwrap()
                .props
                .retain(|p| p.name != "RELATED-TO");
            fs::write(path, twydo_core::ical::serialize(&cal)).unwrap();
            let before = e.files();
            match action {
                "apply" => assert!(matches!(
                    store.apply(&snapshot, &op, later()),
                    Err(WriteError::Stale(_))
                )),
                "queue" => {
                    drop(lock);
                    let drained = store.drain(later()).unwrap();
                    assert_eq!(drained.len(), 1);
                    assert!(matches!(drained[0].outcome, DrainOutcome::Refused(_)));
                }
                "undo" => assert!(matches!(
                    store.undo_last(later()),
                    Err(WriteError::Stale(_))
                )),
                "redo" => assert!(matches!(
                    store.redo_last(later()),
                    Err(WriteError::Stale(_))
                )),
                _ => {
                    let (_, report) = Store::open_with(&e.cfg, Matrix::embedded().clone()).unwrap();
                    assert_eq!(report.stuck.len(), 1, "{action}, alias={internal_alias}");
                    assert!(report.resumed.is_empty());
                }
            }
            assert_eq!(
                e.files(),
                before,
                "{action}, alias={internal_alias}: no writes"
            );
            // A fresh snapshot cannot turn the inconsistent JSON-only member into
            // an admissible family merely because it is absent from the index.
            assert!(
                twydo_core::gate::check(
                    &e.snap(),
                    Matrix::embedded(),
                    WriteMode::Verified,
                    &op,
                    now().date_naive()
                )
                .is_err()
            );
        }
    }
}

#[test]
fn checklist_due_capability_refuses_ambiguous_source_dates_and_alarms() {
    for parent in [true, false] {
        for malformed in [
            "different-start",
            "duplicate-due",
            "duplicate-meta",
            "unreadable-alarm",
        ] {
            let (e, mut store) = recurring_checklist_env("first-phone-actions/list1");
            let snapshot = e.snap();
            let p = snapshot.find(FC).unwrap();
            let i = if parent { p } else { snapshot.children(p)[0] };
            let path = &snapshot.tasks[i].path;
            let original = fs::read(path).unwrap();
            let changed = if malformed == "duplicate-meta" {
                let mut cal = parse(&original).unwrap();
                let todo = cal.root.children_named_mut("VTODO").next().unwrap();
                todo.props
                    .push(todo.prop("X-2DOAPP-METADATA").unwrap().clone());
                twydo_core::ical::serialize(&cal)
            } else {
                let text = String::from_utf8(original.clone()).unwrap();
                match malformed {
                    "different-start" => {
                        text.replace("DTSTART;VALUE=DATE:20261210", "DTSTART;VALUE=DATE:20261209")
                    }
                    "duplicate-due" => {
                        text.replace("END:VTODO", "DUE;VALUE=DATE:20261210\r\nEND:VTODO")
                    }
                    _ => text.replace(
                        "END:VTODO",
                        "BEGIN:VALARM\r\nACTION:AUDIO\r\nEND:VALARM\r\nEND:VTODO",
                    ),
                }
                .into_bytes()
            };
            assert_ne!(changed, original);
            fs::write(path, changed).unwrap();
            let before = e.files();
            assert!(
                store
                    .apply(&e.snap(), &due(FC, 2026, 12, 11), now())
                    .is_err(),
                "parent={parent}, {malformed}"
            );
            assert_eq!(e.files(), before);
            assert!(e.journal().is_empty());
        }
    }
}

mod capability_deletion {
    use super::*;
    use twydo_core::store::{DrainOutcome, Submitted, fsio::VdirLock};

    const E: &str = "44123b09407c4ea1adae8f2ecdb64d49";
    const A: &str = "12d762a5639e406c8ec865517882f130";
    const A_Q: &str = "2cb7991c71624ddda29877c098acad43";
    const A_Z: &str = "3199749e2ea64f10bdaebc00197768ca";
    const B: &str = "twydo-p8-c29-b";
    const B_Q: &str = "96ee97f3cc8c403391c57ec8b8e5d192";
    const B_Z: &str = "f9ae2ffd776e42919f9696f37dd6e849";
    const D: &str = "twydo-p8-c29-d";
    const CA: &str = "twydo-i27-c-a";
    const CA_Q: &str = "twydo-i27-c-a-q";
    const CA_X: &str = "twydo-i27-c-a-x";
    const PA: &str = "twydo-i27-p-a";
    const PA_X: &str = "twydo-i27-p-a-x";

    fn weekly(env: &Env, uid: &str) {
        let path = env.file(uid);
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("RRULE:FREQ=MONTHLY", "RRULE:FREQ=WEEKLY");
        fs::write(path, text).unwrap();
    }

    fn checklist_env() -> Env {
        Env::new(
            WriteMode::Verified,
            "complete/checklist-reset/phone-items-and-text/list1",
        )
    }

    fn history() -> Env {
        Env::new(
            WriteMode::Verified,
            "complete/family-lifecycle/complete-parent/list1",
        )
    }

    #[test]
    fn capability_delete_removes_only_selected_resource_or_flat_family() {
        // Standalone weekly resource.
        let e = Env::new(
            WriteMode::Verified,
            "complete/standalone-lifecycle/edit/list1",
        );
        weekly(&e, E);
        let before = e.files();
        let mut store = e.store(Matrix::embedded().clone());
        store
            .apply(&e.snap(), &Op::Delete { uid: E.into() }, now())
            .unwrap();
        let after = e.files();
        assert!(!after.contains_key(&key(E)));
        for (path, bytes) in &before {
            if path != &key(E) {
                assert_eq!(after[path], *bytes, "{path}");
            }
        }
        store.undo_last(later()).unwrap();
        assert_eq!(e.files(), before);
        store.redo_last(later()).unwrap();
        assert_eq!(e.files(), after);

        // Nonempty recurring checklist: only A, q and z disappear.
        let e = checklist_env();
        let before = e.files();
        let mut store = e.store(Matrix::embedded().clone());
        store
            .apply(&e.snap(), &Op::Delete { uid: A.into() }, now())
            .unwrap();
        let after = e.files();
        for uid in [A, A_Q, A_Z] {
            assert!(!after.contains_key(&key(uid)), "{uid}");
        }
        for uid in [B, B_Q, B_Z, D] {
            assert_eq!(after[&key(uid)], before[&key(uid)], "{uid}");
        }
        store.undo_last(later()).unwrap();
        assert_eq!(e.files(), before);
        store.redo_last(later()).unwrap();
        assert_eq!(e.files(), after);

        // Empty recurring checklist.
        let e = checklist_env();
        for child in [A_Q, A_Z] {
            fs::remove_file(e.file(child)).unwrap();
        }
        let before = e.files();
        let mut store = e.store(Matrix::embedded().clone());
        store
            .apply(&e.snap(), &Op::Delete { uid: A.into() }, now())
            .unwrap();
        let after = e.files();
        assert!(!after.contains_key(&key(A)));
        for (path, bytes) in &before {
            if path != &key(A) {
                assert_eq!(after[path], *bytes, "{path}");
            }
        }

        // Completed flat project and checklist families.
        let e = Env::new(
            WriteMode::Verified,
            "delete/family-items/phone-complete-project/list1",
        );
        let before = e.files();
        let mut store = e.store(Matrix::embedded().clone());
        store
            .apply(&e.snap(), &Op::Delete { uid: PA.into() }, now())
            .unwrap();
        let after = e.files();
        for uid in [PA, PA_X] {
            assert!(!after.contains_key(&key(uid)), "{uid}");
        }
        for (path, bytes) in &before {
            if path != &key(PA) && path != &key(PA_X) {
                assert_eq!(after[path], *bytes, "{path}");
            }
        }
        store
            .apply(&e.snap(), &Op::Delete { uid: CA.into() }, now())
            .unwrap();
        let after = e.files();
        for uid in [CA, CA_Q, CA_X] {
            assert!(!after.contains_key(&key(uid)), "{uid}");
        }
    }

    #[test]
    fn capability_delete_preserves_series_history_and_other_resources() {
        let e = history();
        let before = e.files();
        let snap = e.snap();
        assert!(snap.find(R).is_some(), "live series");
        for uid in [R_COPY, R1_COPY, R2_COPY, R3_COPY] {
            assert!(snap.find(uid).is_some(), "history {uid}");
        }
        let mut store = e.store(Matrix::embedded().clone());
        store
            .apply(&snap, &Op::Delete { uid: R.into() }, now())
            .unwrap();
        let after = e.files();
        for uid in [R, R1, R2, R3] {
            assert!(!after.contains_key(&key(uid)), "{uid}");
        }
        for uid in [R_COPY, R1_COPY, R2_COPY, R3_COPY] {
            assert_eq!(after[&key(uid)], before[&key(uid)], "history {uid}");
        }
        for (path, bytes) in &before {
            if ![R, R1, R2, R3].iter().any(|uid| path == &key(uid)) {
                assert_eq!(after[path], *bytes, "outside family {path}");
            }
        }
        store.undo_last(later()).unwrap();
        assert_eq!(e.files(), before);
    }

    #[test]
    fn capability_delete_refuses_changed_family_before_commit() {
        for change in ["parent", "child", "removed", "new", "unrelated"] {
            let e = checklist_env();
            let snap = e.snap();
            let mut store = e.store(Matrix::embedded().clone());
            match change {
                "parent" => {
                    let p = e.file(A);
                    fs::write(&p, [fs::read(&p).unwrap(), b"\r\n".to_vec()].concat()).unwrap();
                }
                "child" => {
                    let p = e.file(A_Q);
                    fs::write(&p, [fs::read(&p).unwrap(), b"\r\n".to_vec()].concat()).unwrap();
                }
                "removed" => fs::remove_file(e.file(A_Q)).unwrap(),
                "new" => {
                    let text = fs::read_to_string(e.file(A_Q))
                        .unwrap()
                        .replace(A_Q, "new-synthetic-child");
                    fs::write(e.file("new-synthetic-child"), text).unwrap();
                }
                "unrelated" => {
                    let p = e.file(D);
                    fs::write(&p, [fs::read(&p).unwrap(), b"\r\n".to_vec()].concat()).unwrap();
                }
                _ => unreachable!(),
            }
            let before = e.files();
            let result = store.apply(&snap, &Op::Delete { uid: A.into() }, now());
            if change == "unrelated" {
                result.unwrap();
                assert_eq!(
                    e.files()[&key(D)],
                    before[&key(D)],
                    "unrelated edit is preserved"
                );
            } else {
                assert!(
                    matches!(result, Err(WriteError::Stale(_))),
                    "{change}: {result:?}"
                );
                assert_eq!(e.files(), before, "{change}");
                assert!(e.journal().is_empty(), "{change}");
            }
        }
    }

    #[test]
    fn capability_delete_queue_and_inverse_keep_scope() {
        // A queued removal re-gates on the current files and refuses a new member.
        let e = checklist_env();
        let snap = e.snap();
        let mut store = e.store(Matrix::embedded().clone());
        let lock = VdirLock::exclusive(&e.state).unwrap();
        assert!(matches!(
            store
                .submit(&snap, &Op::Delete { uid: A.into() }, "delete", "op", now())
                .unwrap(),
            Submitted::Queued(_)
        ));
        let text = fs::read_to_string(e.file(A_Q))
            .unwrap()
            .replace(A_Q, "new-synthetic-child");
        fs::write(e.file("new-synthetic-child"), text).unwrap();
        let before = e.files();
        drop(lock);
        let drained = store.drain(later()).unwrap();
        assert_eq!(drained.len(), 1);
        assert!(matches!(drained[0].outcome, DrainOutcome::Refused(_)));
        assert_eq!(e.files(), before);

        // A successful queued removal has the same scope as a direct one.
        let e = checklist_env();
        let snap = e.snap();
        let mut store = e.store(Matrix::embedded().clone());
        let lock = VdirLock::exclusive(&e.state).unwrap();
        assert!(matches!(
            store
                .submit(&snap, &Op::Delete { uid: A.into() }, "delete", "op", now())
                .unwrap(),
            Submitted::Queued(_)
        ));
        drop(lock);
        let drained = store.drain(later()).unwrap();
        assert_eq!(drained.len(), 1);
        assert!(
            !matches!(drained[0].outcome, DrainOutcome::Refused(_)),
            "{:?}",
            drained[0].outcome
        );
        for uid in [A, A_Q, A_Z] {
            assert!(!e.file(uid).exists(), "{uid}");
        }
        for uid in [B, B_Q, B_Z, D] {
            assert!(e.file(uid).exists(), "{uid}");
        }

        // A new member makes undo and redo refuse without writes.
        for redo in [false, true] {
            let e = checklist_env();
            let snap = e.snap();
            let original = e.files();
            let mut store = e.store(Matrix::embedded().clone());
            store
                .apply(&snap, &Op::Delete { uid: A.into() }, now())
                .unwrap();
            if redo {
                store.undo_last(later()).unwrap();
            }
            let text = String::from_utf8(original[&key(A_Z)].clone())
                .unwrap()
                .replace(A_Z, "new-synthetic-child");
            fs::write(e.file("new-synthetic-child"), text).unwrap();
            let before = e.files();
            let entries = e.journal().len();
            let result = if redo {
                store.redo_last(later())
            } else {
                store.undo_last(later())
            };
            assert!(
                matches!(result, Err(WriteError::Stale(_))),
                "redo={redo}: {result:?}"
            );
            assert_eq!(e.files(), before);
            assert_eq!(e.journal().len(), entries);
        }

        // A remotely restored member is not overwritten by undo.
        let e = checklist_env();
        let snap = e.snap();
        let before = e.files();
        let mut store = e.store(Matrix::embedded().clone());
        store
            .apply(&snap, &Op::Delete { uid: A.into() }, now())
            .unwrap();
        let remote = String::from_utf8(before[&key(A)].clone())
            .unwrap()
            .replace("SUMMARY:A", "SUMMARY:remote synthetic");
        fs::write(e.file(A), remote.as_bytes()).unwrap();
        let with_remote = e.files();
        assert!(matches!(
            store.undo_last(later()),
            Err(WriteError::Stale(_))
        ));
        assert_eq!(e.files(), with_remote);
    }

    #[test]
    fn capability_delete_recovery_is_idempotent() {
        for partial in [false, true] {
            let e = history();
            let before = e.files();
            let snap = e.snap();
            let mut store = e.store(Matrix::embedded().clone());
            let id = store
                .apply(&snap, &Op::Delete { uid: R.into() }, now())
                .unwrap();
            let after = e.files();
            let journal = Journal::open(&e.state).unwrap();
            let mut entry = journal.get(&id).unwrap().unwrap();
            entry.stage = Stage::Prepared;
            journal.prepare(&entry).unwrap();
            if partial {
                // Put back every removal but the last: recovery finishes the rest.
                for w in entry.writes.iter().take(entry.writes.len() - 1) {
                    fs::write(e.vdir.join(&w.path), w.before.as_ref().unwrap()).unwrap();
                }
            }
            let (mut store, report) = Store::open_with(&e.cfg, Matrix::embedded().clone()).unwrap();
            assert_eq!(report.resumed, vec![id.clone()], "partial={partial}");
            assert!(report.stuck.is_empty(), "partial={partial}");
            assert_eq!(e.files(), after, "partial={partial}");
            for uid in [R_COPY, R1_COPY, R2_COPY, R3_COPY] {
                assert_eq!(e.files()[&key(uid)], before[&key(uid)], "{uid}");
            }
            store.undo(&id, later()).unwrap();
            assert_eq!(e.files(), before, "partial={partial}: undo");
            let (_, again) = Store::open_with(&e.cfg, Matrix::embedded().clone()).unwrap();
            assert!(again.resumed.is_empty() && again.stuck.is_empty());
        }

        // Recovery refuses a member that joined after the whole-family removal.
        let e = checklist_env();
        let snap = e.snap();
        let mut store = e.store(Matrix::embedded().clone());
        let id = store
            .apply(&snap, &Op::Delete { uid: A.into() }, now())
            .unwrap();
        let journal = Journal::open(&e.state).unwrap();
        let mut entry = journal.get(&id).unwrap().unwrap();
        entry.stage = Stage::Prepared;
        journal.prepare(&entry).unwrap();
        let child_before = entry
            .writes
            .iter()
            .find(|w| w.path == key(A_Z))
            .unwrap()
            .before
            .clone()
            .unwrap();
        let text = child_before.replace(A_Z, "new-synthetic-child");
        fs::write(e.file("new-synthetic-child"), text).unwrap();
        let before = e.files();
        let (_, report) = Store::open_with(&e.cfg, Matrix::embedded().clone()).unwrap();
        assert_eq!(report.stuck.len(), 1, "new member");
        assert!(report.resumed.is_empty());
        assert_eq!(e.files(), before);
    }
}

mod json_family_membership {
    use super::*;
    use twydo_core::model::meta_edit::new_task_meta;
    use twydo_core::store::{DrainOutcome, Submitted, fsio::VdirLock};

    const HEAD: &str = "synthetic-family-parent";
    const ALIAS: &str = "synthetic-internal-parent";

    fn resource(uid: &str, parent: Option<&str>, json_parent: Option<&str>) -> Vec<u8> {
        let kind = if uid == HEAD { 2 } else { 0 };
        let meta = new_task_meta(kind, (uid == HEAD).then_some(ALIAS), json_parent);
        let related = parent
            .map(|p| format!("RELATED-TO:{p}\r\n"))
            .unwrap_or_default();
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:{uid}\r\nSUMMARY:Synthetic family\r\nSTATUS:NEEDS-ACTION\r\n{related}X-2DOAPP-METADATA:{}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n", meta.encode()).into_bytes()
    }

    fn env() -> Env {
        let e = Env::new(
            WriteMode::Verified,
            "complete/standalone-lifecycle/edit/list1",
        );
        fs::write(e.file(HEAD), resource(HEAD, None, None)).unwrap();
        fs::write(
            e.file("synthetic-valid-child"),
            resource("synthetic-valid-child", Some(HEAD), Some(ALIAS)),
        )
        .unwrap();
        e
    }

    fn add_ambiguous(e: &Env, standard: Option<&str>) {
        fs::write(
            e.file("synthetic-json-only-child"),
            resource("synthetic-json-only-child", standard, Some(ALIAS)),
        )
        .unwrap();
    }

    #[test]
    fn json_only_membership_is_refused_at_admission() {
        for standard in [None, Some("synthetic-other-parent")] {
            let e = env();
            add_ambiguous(&e, standard);
            let snap = e.snap();
            let before = e.files();
            let op = Op::Delete { uid: HEAD.into() };
            assert!(
                twydo_core::gate::check(
                    &snap,
                    Matrix::embedded(),
                    WriteMode::Verified,
                    &op,
                    now().date_naive()
                )
                .is_err(),
                "JSON reference with standard link {standard:?} was omitted"
            );
            assert!(
                e.store(Matrix::embedded().clone())
                    .apply(&snap, &op, now())
                    .is_err()
            );
            assert_eq!(e.files(), before);
        }
    }

    #[test]
    fn json_only_newcomers_are_guarded_through_commit_inverse_and_recovery() {
        for phase in ["commit", "queue", "undo", "redo", "recovery"] {
            let e = env();
            let snap = e.snap();
            let mut store = e.store(Matrix::embedded().clone());
            let op = Op::Delete { uid: HEAD.into() };
            match phase {
                "commit" => {
                    add_ambiguous(&e, None);
                    let before = e.files();
                    assert!(
                        matches!(store.apply(&snap, &op, now()), Err(WriteError::Stale(_))),
                        "commit omitted JSON-only newcomer"
                    );
                    assert_eq!(e.files(), before);
                }
                "queue" => {
                    let lock = VdirLock::exclusive(&e.state).unwrap();
                    assert!(matches!(
                        store.submit(&snap, &op, "delete", "op", now()).unwrap(),
                        Submitted::Queued(_)
                    ));
                    add_ambiguous(&e, None);
                    let before = e.files();
                    drop(lock);
                    let drained = store.drain(later()).unwrap();
                    assert!(
                        matches!(drained[0].outcome, DrainOutcome::Refused(_)),
                        "queued removal omitted JSON-only newcomer"
                    );
                    assert_eq!(e.files(), before);
                }
                "undo" | "redo" => {
                    store.apply(&snap, &op, now()).unwrap();
                    if phase == "redo" {
                        store.undo_last(later()).unwrap();
                    }
                    add_ambiguous(&e, None);
                    let before = e.files();
                    let result = if phase == "redo" {
                        store.redo_last(later())
                    } else {
                        store.undo_last(later())
                    };
                    assert!(
                        matches!(result, Err(WriteError::Stale(_))),
                        "{phase} omitted JSON-only newcomer"
                    );
                    assert_eq!(e.files(), before);
                }
                "recovery" => {
                    let id = store.apply(&snap, &op, now()).unwrap();
                    let journal = Journal::open(&e.state).unwrap();
                    let mut entry = journal.get(&id).unwrap().unwrap();
                    entry.stage = Stage::Prepared;
                    journal.prepare(&entry).unwrap();
                    add_ambiguous(&e, None);
                    let before = e.files();
                    let (_, report) = Store::open_with(&e.cfg, Matrix::embedded().clone()).unwrap();
                    assert_eq!(report.stuck.len(), 1, "recovery omitted JSON-only newcomer");
                    assert_eq!(e.files(), before);
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn standard_alias_membership_is_refused_at_admission() {
        let e = env();
        fs::write(
            e.file("synthetic-alias-child"),
            resource("synthetic-alias-child", Some(ALIAS), None),
        )
        .unwrap();
        let snap = e.snap();
        let op = Op::Delete { uid: HEAD.into() };
        assert!(
            twydo_core::gate::check(
                &snap,
                Matrix::embedded(),
                WriteMode::Verified,
                &op,
                now().date_naive()
            )
            .is_err(),
            "standard link to internal alias was omitted"
        );
    }

    #[test]
    fn standard_alias_newcomers_are_refused_before_commit() {
        let e = env();
        let snap = e.snap();
        let mut store = e.store(Matrix::embedded().clone());
        fs::write(
            e.file("synthetic-alias-child"),
            resource("synthetic-alias-child", Some(ALIAS), None),
        )
        .unwrap();
        let before = e.files();
        assert!(
            matches!(
                store.apply(&snap, &Op::Delete { uid: HEAD.into() }, now()),
                Err(WriteError::Stale(_))
            ),
            "standard link to internal alias bypassed rescan"
        );
        assert_eq!(e.files(), before);
    }

    #[test]
    fn whole_family_deletion_includes_completed_and_cancelled_members() {
        for kind in [1, 2] {
            for status in ["COMPLETED", "CANCELLED"] {
                let e = env();
                let mut cal = parse(&fs::read(e.file(HEAD)).unwrap()).unwrap();
                let todo = cal.root.children_named_mut("VTODO").next().unwrap();
                todo.prop_mut("STATUS").unwrap().set_value(status);
                let prop = todo.prop_mut("X-2DOAPP-METADATA").unwrap();
                let mut meta = twydo_core::model::meta_edit::MetaJson::parse(&prop.value).unwrap();
                meta.set_i64("TaskType", kind);
                prop.set_value(&meta.encode());
                fs::write(e.file(HEAD), twydo_core::ical::serialize(&cal)).unwrap();
                let child = "synthetic-valid-child";
                let child_bytes = String::from_utf8(fs::read(e.file(child)).unwrap())
                    .unwrap()
                    .replace(
                        "STATUS:NEEDS-ACTION",
                        if status == "COMPLETED" {
                            "STATUS:CANCELLED"
                        } else {
                            "STATUS:COMPLETED"
                        },
                    );
                fs::write(e.file(child), child_bytes).unwrap();
                let before = e.files();
                let mut store = e.store(Matrix::embedded().clone());
                store
                    .apply(&e.snap(), &Op::Delete { uid: HEAD.into() }, now())
                    .unwrap();
                let after = e.files();
                assert!(!e.file(HEAD).exists() && !e.file(child).exists());
                for (path, bytes) in &before {
                    if path != &format!("list1/{HEAD}.ics") && path != &format!("list1/{child}.ics")
                    {
                        assert_eq!(after[path], *bytes);
                    }
                }
                store.undo_last(later()).unwrap();
                assert_eq!(e.files(), before);
            }
        }
    }
}

#[test]
fn history_family_capability_matches_h7() {
    let env = Env::new(
        WriteMode::Verified,
        "complete/family-lifecycle/complete-parent/list1",
    );
    // No ordinary family or scheduling operation rows are required by this write set.
    assert!(
        twydo_core::gate::check(
            &env.snap(),
            &common::matrix("pass", &[]),
            WriteMode::Verified,
            &Op::Reopen { uid: R_COPY.into() },
            now().date_naive()
        )
        .is_ok()
    );
    let mut store = env.store(Matrix::embedded().clone());
    let before = env.files();
    store
        .apply(&env.snap(), &Op::Reopen { uid: R_COPY.into() }, now())
        .unwrap();
    let after = env.files();
    for uid in [R_COPY, R1_COPY, R2_COPY, R3_COPY] {
        assert_eq!(status(&after, uid), "NEEDS-ACTION");
        same_lines(
            uid,
            lines(&parse(&after[&key(uid)]).unwrap()),
            lines(&parse(&fixture(L1, "reopen-parent/list1", uid)).unwrap()),
        );
    }
    for (path, bytes) in &before {
        if ![R_COPY, R1_COPY, R2_COPY, R3_COPY]
            .iter()
            .any(|u| key(u) == *path)
        {
            assert_eq!(&after[path], bytes, "unrelated resource {path}");
        }
    }
    assert_eq!(env.journal().len(), 1);
    store.undo_last(later()).unwrap();
    assert_eq!(env.files(), before);
    store.redo_last(later()).unwrap();
    assert_eq!(env.files(), after);
}

#[test]
fn history_family_capability_refuses_unsafe_relationships() {
    for shape in [
        "nested",
        "cross-list",
        "unknown",
        "plain",
        "duplicate",
        "disagreeing",
        "json-only",
        "active-repeat",
    ] {
        let env = Env::new(
            WriteMode::Verified,
            "complete/family-lifecycle/complete-parent/list1",
        );
        let mut snap = env.snap();
        let p = snap.find(R_COPY).unwrap();
        let c = snap.find(R1_COPY).unwrap();
        match shape {
            "nested" => snap.tasks[c].kind = twydo_core::model::TaskKind::Project,
            "cross-list" => snap.tasks[c].collection = "list2".into(),
            "unknown" => {
                let mut meta = twydo_core::model::meta_edit::MetaJson::parse(
                    snap.tasks[c].meta.raw.as_ref().unwrap(),
                )
                .unwrap();
                meta.set_i64("TaskType", 99);
                snap.tasks[c].meta.raw = Some(meta.encode());
            }
            "plain" => {
                let mut meta = twydo_core::model::meta_edit::MetaJson::parse(
                    snap.tasks[p].meta.raw.as_ref().unwrap(),
                )
                .unwrap();
                meta.set_i64("TaskType", 0);
                snap.tasks[p].meta.raw = Some(meta.encode());
            }
            "duplicate" => snap.tasks.push(snap.tasks[c].clone()),
            "active-repeat" => snap.tasks[c].rrule = Some("FREQ=MONTHLY".into()),
            "json-only" => {
                let mut task = snap.tasks[c].clone();
                task.uid = "synthetic-hidden-member".into();
                task.parent = None;
                snap.tasks.push(task);
            }
            _ => {
                let mut meta = twydo_core::model::meta_edit::MetaJson::parse(
                    snap.tasks[c].meta.raw.as_ref().unwrap(),
                )
                .unwrap();
                meta.set_str("parentuid", "synthetic-wrong-parent");
                snap.tasks[c].meta.raw = Some(meta.encode());
            }
        }
        assert!(
            twydo_core::gate::check(
                &snap,
                Matrix::embedded(),
                WriteMode::Verified,
                &Op::Reopen { uid: R_COPY.into() },
                now().date_naive()
            )
            .is_err(),
            "{shape}"
        );
    }
}

#[test]
fn history_item_capability_keeps_parent_policy() {
    for step in ["complete-parent/list1", "reopen-parent/list1"] {
        let env = Env::new(
            WriteMode::Verified,
            &format!("complete/family-lifecycle/{step}"),
        );
        let mut snap = env.snap();
        let child = snap.find(R1_COPY).unwrap();
        snap.tasks[child].status = twydo_core::model::Status::Completed;
        assert!(
            twydo_core::gate::check(
                &snap,
                Matrix::embedded(),
                WriteMode::Verified,
                &Op::Reopen {
                    uid: R1_COPY.into()
                },
                now().date_naive()
            )
            .is_err()
        );
    }
}

#[test]
fn history_reopen_stale_family_and_inverse_refuse() {
    for action in ["apply", "undo", "redo", "recover"] {
        let env = Env::new(
            WriteMode::Verified,
            "complete/family-lifecycle/complete-parent/list1",
        );
        let mut store = env.store(Matrix::embedded().clone());
        let snap = env.snap();
        let op = Op::Reopen { uid: R_COPY.into() };
        if action != "apply" {
            store.apply(&snap, &op, now()).unwrap();
        }
        if action == "redo" {
            store.undo_last(later()).unwrap();
        }
        if action == "recover" {
            let mut entry = env.journal().remove(0);
            entry.stage = Stage::Prepared;
            fs::write(env.file(R_COPY), entry.writes[0].before.as_ref().unwrap()).unwrap();
            Journal::open(&env.state).unwrap().prepare(&entry).unwrap();
        }
        // A new member is a dependency even when all recorded writes still match.
        let text = String::from_utf8(fixture(L1, "complete-parent/list1", R1_COPY))
            .unwrap()
            .replace(R1_COPY, "synthetic-new-history-child");
        fs::write(env.file("synthetic-new-history-child"), text).unwrap();
        let remote = env.files();
        match action {
            "apply" => assert!(store.apply(&snap, &op, now()).is_err()),
            "undo" => assert!(store.undo_last(later()).is_err()),
            "redo" => assert!(store.redo_last(later()).is_err()),
            _ => {
                drop(store);
                let (_, report) = Store::open_with(&env.cfg, Matrix::embedded().clone()).unwrap();
                assert!(!report.stuck.is_empty());
            }
        }
        assert_eq!(env.files(), remote, "{action} must refuse atomically");
    }
}

#[test]
fn history_reopen_queue_and_recovery_are_idempotent() {
    let env = Env::new(
        WriteMode::Verified,
        "complete/family-lifecycle/complete-parent/list1",
    );
    let mut store = env.store(Matrix::embedded().clone());
    let before = env.files();
    let lock = twydo_core::store::fsio::VdirLock::exclusive(&env.state).unwrap();
    let submitted = store
        .submit(
            &env.snap(),
            &Op::Reopen { uid: R_COPY.into() },
            "reopen history",
            "op",
            now(),
        )
        .unwrap();
    assert!(matches!(submitted, twydo_core::store::Submitted::Queued(_)));
    assert_eq!(env.files(), before);
    let overlay =
        twydo_core::store::queue::view(twydo_core::vdir::load(&env.vdir).unwrap(), env.cfg.clone());
    for uid in [R_COPY, R1_COPY, R2_COPY, R3_COPY] {
        assert_eq!(
            overlay.tasks[overlay.find(uid).unwrap()].status,
            twydo_core::model::Status::Open
        );
    }
    drop(lock);
    let drained = store.drain(now()).unwrap();
    assert_eq!(drained.len(), 1);
    assert!(matches!(
        drained[0].outcome,
        twydo_core::store::DrainOutcome::Applied(_)
    ));
    let after = env.files();
    let mut entry = env.journal().remove(0);
    let recovering = Env::new(
        WriteMode::Verified,
        "complete/family-lifecycle/complete-parent/list1",
    );
    drop(recovering.store(Matrix::embedded().clone()));
    entry.stage = Stage::Prepared;
    entry.from_queue = None;
    Journal::open(&recovering.state)
        .unwrap()
        .prepare(&entry)
        .unwrap();
    let first = &entry.writes[0];
    fs::write(
        recovering.vdir.join(&first.path),
        first.after.as_ref().unwrap(),
    )
    .unwrap();
    let (mut recovered, report) =
        Store::open_with(&recovering.cfg, Matrix::embedded().clone()).unwrap();
    assert!(report.stuck.is_empty());
    assert_eq!(recovering.files(), after);
    drop(recovered);
    let (again, report) = Store::open_with(&recovering.cfg, Matrix::embedded().clone()).unwrap();
    assert!(report.stuck.is_empty());
    assert_eq!(recovering.files(), after);
    recovered = again;
    recovered.undo(&entry.id, later()).unwrap();
    assert_eq!(recovering.files(), before);
}

#[test]
fn history_family_preserves_open_cancelled_members_and_schedule_variants() {
    for kind in [1, 2] {
        let env = Env::new(
            WriteMode::Verified,
            "complete/family-lifecycle/complete-parent/list1",
        );
        let mut parent = parse(&fs::read(env.file(R_COPY)).unwrap()).unwrap();
        let component = parent
            .root
            .children
            .iter_mut()
            .find(|c| c.name == "VTODO")
            .unwrap();
        let prop = component
            .props
            .iter_mut()
            .find(|p| p.name == "X-2DOAPP-METADATA")
            .unwrap();
        let mut meta = twydo_core::model::meta_edit::MetaJson::parse(&prop.value).unwrap();
        meta.set_i64("TaskType", kind);
        prop.value = meta.encode();
        fs::write(env.file(R_COPY), twydo_core::ical::serialize(&parent)).unwrap();
        for (uid, status) in [(R1_COPY, "NEEDS-ACTION"), (R2_COPY, "CANCELLED")] {
            let text = fs::read_to_string(env.file(uid))
                .unwrap()
                .replace("STATUS:COMPLETED", &format!("STATUS:{status}"));
            fs::write(env.file(uid), text).unwrap();
        }
        let mut text = fs::read_to_string(env.file(R3_COPY)).unwrap();
        text = text.replace("END:VTODO", "BEGIN:VALARM\nACTION:DISPLAY\nTRIGGER;VALUE=DATE-TIME:20261208T090000Z\nDESCRIPTION:Synthetic\nEND:VALARM\nEND:VTODO");
        fs::write(env.file(R3_COPY), text).unwrap();
        let before = env.files();
        let mut store = env.store(Matrix::embedded().clone());
        store
            .apply(&env.snap(), &Op::Reopen { uid: R_COPY.into() }, now())
            .unwrap();
        let after = env.files();
        for (path, bytes) in before
            .iter()
            .filter(|(p, _)| **p != key(R_COPY) && **p != key(R3_COPY))
        {
            assert_eq!(&after[path], bytes);
        }
        for uid in [R_COPY, R3_COPY] {
            let held = |bytes: &[u8]| {
                sorted(
                    lines(&parse(bytes).unwrap())
                        .into_iter()
                        .filter(|l| {
                            !is_stamp(&l.1)
                                && !["STATUS", "COMPLETED", "PERCENT-COMPLETE"]
                                    .contains(&l.1.as_str())
                        })
                        .collect(),
                )
            };
            assert_eq!(held(&before[&key(uid)]), held(&after[&key(uid)]));
        }
        // Even an unchanged member is a durable dependency of inverse and recovery.
        let remote = fs::read_to_string(env.file(R1_COPY))
            .unwrap()
            .replace("DESCRIPTION:n", "DESCRIPTION:Synthetic remote");
        fs::write(env.file(R1_COPY), remote).unwrap();
        let remote_files = env.files();
        assert!(store.undo_last(later()).is_err());
        assert_eq!(env.files(), remote_files);
    }
}

#[test]
fn history_reopen_queue_refuses_changed_removed_and_joined_members() {
    for change in ["changed", "removed", "joined"] {
        let env = Env::new(
            WriteMode::Verified,
            "complete/family-lifecycle/complete-parent/list1",
        );
        let mut store = env.store(Matrix::embedded().clone());
        let lock = twydo_core::store::fsio::VdirLock::exclusive(&env.state).unwrap();
        store
            .submit(
                &env.snap(),
                &Op::Reopen { uid: R_COPY.into() },
                "history",
                "op",
                now(),
            )
            .unwrap();
        drop(lock);
        match change {
            "removed" => fs::remove_file(env.file(R1_COPY)).unwrap(),
            "changed" => {
                let text = fs::read_to_string(env.file(R1_COPY))
                    .unwrap()
                    .replace("DESCRIPTION:n", "DESCRIPTION:Synthetic remote");
                fs::write(env.file(R1_COPY), text).unwrap();
            }
            _ => {
                let text = fs::read_to_string(env.file(R1_COPY))
                    .unwrap()
                    .replace(R1_COPY, "synthetic-queued-join");
                fs::write(env.file("synthetic-queued-join"), text).unwrap();
            }
        }
        let remote = env.files();
        let result = store.drain(later()).unwrap();
        assert!(
            matches!(
                result[0].outcome,
                twydo_core::store::DrainOutcome::Refused(_)
            ),
            "{change}: {result:?}"
        );
        assert_eq!(env.files(), remote);
    }
}

#[test]
fn capability_move_flat_family_has_one_identity_map() {
    for uid in [K, P, R] {
        let e = Env::new(WriteMode::Verified, FIXTURE);
        let before = e.files();
        let s = e.snap();
        let i = s.find(uid).unwrap();
        let n = s.children(i).len() + 1;
        let mut store = e.store(Matrix::embedded().clone());
        store
            .apply(
                &s,
                &Op::Move {
                    uid: uid.into(),
                    to: "list2".into(),
                },
                now(),
            )
            .unwrap();
        let entry = e.journal().pop().unwrap();
        assert_eq!(entry.writes.len(), n * 2);
        assert!(entry.writes[..n].iter().all(|w| w.before.is_none()));
        assert!(entry.writes[n..].iter().all(|w| w.after.is_none()));
        let moved = e.snap();
        let parent = moved
            .tasks
            .iter()
            .position(|t| t.collection == "list2" && t.parent.is_none())
            .unwrap();
        assert_eq!(moved.children(parent).len(), n - 1);
        let ids: std::collections::BTreeSet<_> = entry.uids.iter().collect();
        assert_eq!(ids.len(), n * 2);
        // Each preserved field and unknown JSON key survives independently of the
        // original schedule's execution/edit verdicts, including child foreign RUID.
        for (source, dest) in entry.writes[n..].iter().zip(&entry.writes[..n]) {
            let cal = parse(source.before.as_ref().unwrap().as_bytes()).unwrap();
            let original = common::todo(&cal);
            let out = parse(dest.after.as_ref().unwrap().as_bytes()).unwrap();
            let output = common::todo(&out);
            let kept = |l: &Line| {
                !matches!(
                    l.1.as_str(),
                    "UID" | "RELATED-TO" | "X-2DOAPP-METADATA" | "DTSTAMP" | "LAST-MODIFIED"
                )
            };
            assert_eq!(
                lines(&cal).into_iter().filter(kept).collect::<Vec<_>>(),
                lines(&out).into_iter().filter(kept).collect::<Vec<_>>()
            );
            let mut meta = twydo_core::model::meta_edit::MetaJson::parse(
                &original.prop("X-2DOAPP-METADATA").unwrap().value,
            )
            .unwrap();
            let old_uid = original.prop("UID").unwrap().value.as_str();
            let new_uid = output.prop("UID").unwrap().value.as_str();
            if meta.get_str("uid") == Some(old_uid) {
                meta.set_str("uid", new_uid);
            }
            if original.prop("RELATED-TO").is_some() {
                meta.set_str("parentuid", &moved.tasks[parent].uid);
            } else if meta.get_str("RUID") == Some(old_uid) {
                meta.set_str("RUID", new_uid);
            }
            assert_eq!(
                output.prop("X-2DOAPP-METADATA").unwrap().value,
                meta.encode()
            );
        }

        for (path, bytes) in &before {
            if !entry.writes.iter().any(|w| &w.path == path) {
                assert_eq!(e.files().get(path), Some(bytes));
            }
        }
        store.undo_last(later()).unwrap();
        assert_eq!(e.files(), before);
    }
}

fn relocation_meta(e: &Env, uid: &str, key: &str, value: &str) {
    let path = e.file(uid);
    let mut cal = parse(&fs::read(&path).unwrap()).unwrap();
    let todo = cal.root.children_named_mut("VTODO").next().unwrap();
    let prop = todo.prop_mut("X-2DOAPP-METADATA").unwrap();
    let mut meta = twydo_core::model::meta_edit::MetaJson::parse(&prop.value).unwrap();
    meta.set_str(key, value);
    prop.set_value(&meta.encode());
    fs::write(path, twydo_core::ical::serialize(&cal)).unwrap();
}

#[test]
fn capability_move_preserves_distinct_internal_parent_identity() {
    let e = Env::new(WriteMode::Verified, FIXTURE);
    relocation_meta(&e, P, "uid", "synthetic-internal-parent");
    let snap = e.snap();
    for &i in snap.children(snap.find(P).unwrap()) {
        relocation_meta(
            &e,
            &snap.tasks[i].uid,
            "parentuid",
            "synthetic-internal-parent",
        );
    }
    let mut store = e.store(Matrix::embedded().clone());
    store
        .apply(
            &e.snap(),
            &Op::Move {
                uid: P.into(),
                to: "list2".into(),
            },
            now(),
        )
        .unwrap();
    let s = e.snap();
    let p = s
        .tasks
        .iter()
        .position(|t| t.collection == "list2" && t.parent.is_none())
        .unwrap();
    for &i in s.children(p) {
        let meta =
            twydo_core::model::meta_edit::MetaJson::parse(s.tasks[i].meta.raw.as_deref().unwrap())
                .unwrap();
        assert_eq!(meta.get_str("parentuid"), Some("synthetic-internal-parent"));
        assert_eq!(s.tasks[i].parent.as_deref(), Some(s.tasks[p].uid.as_str()));
    }
}

#[test]
fn capability_move_recovery_and_undo_preserve_remote_edits() {
    for phase in [
        "undo-source",
        "undo-destination",
        "undo-json-source",
        "undo-json-destination",
        "recovery-source",
        "recovery-destination",
        "recovery-json-source",
        "recovery-json-destination",
        "undo-changed-destination",
    ] {
        let e = Env::new(WriteMode::Verified, FIXTURE);
        let before = e.files();
        let mut store = e.store(Matrix::embedded().clone());
        store
            .apply(
                &e.snap(),
                &Op::Move {
                    uid: K.into(),
                    to: "list2".into(),
                },
                now(),
            )
            .unwrap();
        let mut entry = e.journal().pop().unwrap();
        let parent_write = &entry.writes[0];
        let new_parent = value(
            &todo_of(parent_write.after.as_ref().unwrap().as_bytes()),
            "UID",
        )
        .unwrap();
        if phase == "undo-changed-destination" {
            let path = e.vdir.join(&parent_write.path);
            let bytes = fs::read_to_string(&path)
                .unwrap()
                .replace("SUMMARY:K", "SUMMARY:Synthetic remote edit");
            fs::write(path, bytes).unwrap();
        } else {
            let destination = phase.ends_with("destination");
            let parent = if destination { &new_parent } else { K };
            let mut cal = parse(&before[&key(K2)]).unwrap();
            let todo = cal.root.children_named_mut("VTODO").next().unwrap();
            todo.prop_mut("UID")
                .unwrap()
                .set_value("synthetic-new-member");
            if phase.contains("json") {
                todo.remove_props("RELATED-TO");
            } else {
                todo.prop_mut("RELATED-TO").unwrap().set_value(parent);
            }
            let prop = todo.prop_mut("X-2DOAPP-METADATA").unwrap();
            let mut meta = twydo_core::model::meta_edit::MetaJson::parse(&prop.value).unwrap();
            meta.set_str("parentuid", parent);
            prop.set_value(&meta.encode());
            fs::write(
                e.vdir.join(if destination {
                    "list2/remote.ics"
                } else {
                    "list1/remote.ics"
                }),
                twydo_core::ical::serialize(&cal),
            )
            .unwrap();
        }
        let remote = e.files();
        if phase.starts_with("recovery") {
            entry.stage = Stage::Prepared;
            Journal::open(&e.state).unwrap().prepare(&entry).unwrap();
            let (_, report) = Store::open_with(&e.cfg, Matrix::embedded().clone()).unwrap();
            assert_eq!(report.stuck.len(), 1, "{phase}");
        } else {
            assert!(store.undo_last(later()).is_err(), "{phase}");
        }
        assert_eq!(e.files(), remote, "{phase}");
    }
}

#[test]
fn capability_move_save_checks_post_edit_shape() {
    let e = Env::new(WriteMode::Verified, FIXTURE);
    let before = e.files();
    let mut store = e.store(Matrix::embedded().clone());
    store
        .apply(
            &e.snap(),
            &Op::Save {
                uid: P.into(),
                changes: vec![Change::Summary("Synthetic moved title".into())],
                to: Some("list2".into()),
            },
            now(),
        )
        .unwrap();
    assert_eq!(e.journal().len(), 1);
    assert!(
        e.snap()
            .tasks
            .iter()
            .any(|t| t.summary == "Synthetic moved title" && t.collection == "list2")
    );
    store.undo_last(later()).unwrap();
    assert_eq!(e.files(), before);
    assert!(
        store
            .apply(
                &e.snap(),
                &Op::Save {
                    uid: P.into(),
                    changes: vec![Change::Kind(TaskKind::Task)],
                    to: Some("list2".into())
                },
                now()
            )
            .is_err()
    );
    assert_eq!(e.files(), before);
}

#[test]
fn capability_move_partial_recovery_keeps_original_identity_map() {
    for cut in [1, 5] {
        let done = Env::new(WriteMode::Verified, FIXTURE);
        let mut store = done.store(Matrix::embedded().clone());
        store
            .apply(
                &done.snap(),
                &Op::Move {
                    uid: R.into(),
                    to: "list2".into(),
                },
                now(),
            )
            .unwrap();
        let expected = done.files();
        let mut entry = done.journal().pop().unwrap();
        let e = Env::new(WriteMode::Verified, FIXTURE);
        let _store = e.store(Matrix::embedded().clone());
        for w in &entry.writes[..cut] {
            match &w.after {
                Some(text) => fs::write(e.vdir.join(&w.path), text).unwrap(),
                None => fs::remove_file(e.vdir.join(&w.path)).unwrap(),
            }
        }
        entry.stage = Stage::Prepared;
        Journal::open(&e.state).unwrap().prepare(&entry).unwrap();
        let (_, report) = Store::open_with(&e.cfg, Matrix::embedded().clone()).unwrap();
        assert!(report.stuck.is_empty(), "{report:?}");
        assert_eq!(report.resumed, [entry.id]);
        assert_eq!(e.files(), expected);
        let (_, report) = Store::open_with(&e.cfg, Matrix::embedded().clone()).unwrap();
        assert!(report.resumed.is_empty());
        assert_eq!(e.files(), expected);
    }
}

mod reparent_candidate {
    use super::*;
    use twydo_core::store::{DrainOutcome, Submitted, fsio::VdirLock};

    fn matrix() -> Matrix {
        common::matrix("pass", &[])
    }
    fn op(parent: Option<&str>) -> Op {
        Op::Reparent {
            uid: K2.into(),
            collection: "list1".into(),
            parent: parent.map(str::to_owned),
        }
    }
    #[test]
    fn reparent_shipping_requires_dedicated_evidence() {
        for mode in [WriteMode::Verified, WriteMode::Test] {
            let e = Env::new(mode, FIXTURE);
            let before = e.files();
            let mut store = e.store(Matrix::embedded().clone());
            for parent in [Some(P), None] {
                let err = store
                    .apply(&e.snap(), &op(parent), now())
                    .unwrap_err()
                    .to_string();
                assert!(err.contains("reparent"), "{err}");
                assert_eq!(e.files(), before);
                assert!(e.journal().is_empty());
            }
        }
    }
    #[test]
    fn reparent_store_updates_exact_membership_once() {
        for parent in [Some(P), None] {
            let e = Env::new(WriteMode::Verified, FIXTURE);
            let before = e.files();
            let mut store = e.store(matrix());
            store.apply(&e.snap(), &op(parent), now()).unwrap();
            let after = e.files();
            let entry = e.journal().pop().unwrap();
            assert_eq!(entry.kind, "reparent");
            assert_eq!(entry.writes.len(), 2);
            assert!(!e.file(K2).exists());
            for (path, bytes) in &before {
                if path != &key(K2) {
                    assert_eq!(&after[path], bytes);
                }
            }
            let snap = e.snap();
            let t = &snap.tasks[snap.find(&entry.uids[1]).unwrap()];
            assert_eq!(t.parent.as_deref(), parent);
            if parent.is_none() {
                assert!(t.due.is_none());
            }
            assert!(entry.families.len() >= 2);
            store.undo_last(later()).unwrap();
            assert_eq!(e.files(), before);
            store.redo_last(later()).unwrap();
            assert_eq!(e.files(), after);
        }
    }
    #[test]
    fn reparent_retained_leaf_alias_survives_queue_recovery_and_inverses() {
        for parent in [Some(P), None] {
            for collection in ["list1", "list2"] {
                for queued in [false, true] {
                    let e = Env::new(WriteMode::Verified, FIXTURE);
                    relocation_meta(&e, K2, "uid", "synthetic-leaf-internal-identity");
                    let before = e.files();
                    let mut store = e.store(matrix());
                    let operation = Op::Reparent {
                        uid: K2.into(),
                        collection: collection.into(),
                        parent: parent.map(str::to_owned),
                    };
                    // A destination parent must live in the selected collection.
                    if parent.is_some() && collection == "list2" {
                        continue;
                    }
                    if queued {
                        let lock = VdirLock::exclusive(&e.state).unwrap();
                        assert!(matches!(
                            store
                                .submit(&e.snap(), &operation, "reparent", "op", now())
                                .unwrap(),
                            Submitted::Queued(_)
                        ));
                        drop(lock);
                        assert!(matches!(
                            store.drain(later()).unwrap()[0].outcome,
                            DrainOutcome::Applied(_)
                        ));
                    } else {
                        store.apply(&e.snap(), &operation, now()).unwrap();
                    }
                    let after = e.files();
                    let mut entry = e.journal().pop().unwrap();
                    let created = &entry.writes[0];
                    let cal = parse(created.after.as_ref().unwrap().as_bytes()).unwrap();
                    let todo = cal.root.children_named("VTODO").next().unwrap();
                    let meta = twydo_core::model::meta_edit::MetaJson::parse(
                        &todo.prop("X-2DOAPP-METADATA").unwrap().value,
                    )
                    .unwrap();
                    assert_eq!(
                        meta.get_str("uid"),
                        Some("synthetic-leaf-internal-identity")
                    );
                    // Simulate interruption after destination creation, before source removal.
                    fs::write(e.file(K2), &before[&key(K2)]).unwrap();
                    entry.stage = Stage::Prepared;
                    Journal::open(&e.state).unwrap().prepare(&entry).unwrap();
                    let (_, report) = Store::open_with(&e.cfg, matrix()).unwrap();
                    assert!(report.stuck.is_empty(), "{report:?}");
                    assert_eq!(report.resumed.len(), 1);
                    assert_eq!(e.files(), after);
                    store.undo_last(later()).unwrap();
                    assert_eq!(e.files(), before);
                    store.redo_last(later()).unwrap();
                    assert_eq!(e.files(), after);
                }
            }
        }
    }

    #[test]
    fn reparent_store_checks_both_families() {
        for queued in [false, true] {
            for uid in [K, K3, P, P3, K2] {
                for removed in [false, true] {
                    let e = Env::new(WriteMode::Verified, FIXTURE);
                    let snap = e.snap();
                    let mut store = e.store(matrix());
                    let lock = queued.then(|| VdirLock::exclusive(&e.state).unwrap());
                    if queued {
                        assert!(matches!(
                            store
                                .submit(&snap, &op(Some(P)), "reparent", "op", now())
                                .unwrap(),
                            Submitted::Queued(_)
                        ));
                    }
                    if removed {
                        fs::remove_file(e.file(uid)).unwrap();
                    } else {
                        let p = e.file(uid);
                        fs::write(&p, [fs::read(&p).unwrap(), b"\r\n".to_vec()].concat()).unwrap();
                    }
                    let before = e.files();
                    if queued {
                        drop(lock);
                        assert!(matches!(
                            store.drain(later()).unwrap()[0].outcome,
                            DrainOutcome::Refused(_)
                        ));
                    } else {
                        assert!(matches!(
                            store.apply(&snap, &op(Some(P)), now()),
                            Err(WriteError::Stale(_))
                        ));
                    }
                    assert_eq!(e.files(), before);
                    assert!(e.journal().is_empty());
                }
            }
        }
    }
    #[test]
    fn reparent_queue_inverse_and_recovery_keep_both_guards() {
        let e = Env::new(WriteMode::Verified, FIXTURE);
        let mut store = e.store(matrix());
        let before = e.files();
        let lock = VdirLock::exclusive(&e.state).unwrap();
        assert!(matches!(
            store
                .submit(&e.snap(), &op(Some(P)), "reparent", "op", now())
                .unwrap(),
            Submitted::Queued(_)
        ));
        drop(lock);
        let drained = store.drain(later()).unwrap();
        assert!(!matches!(drained[0].outcome, DrainOutcome::Refused(_)));
        let after = e.files();
        let mut entry = e.journal().pop().unwrap();
        for w in &entry.writes {
            if let Some(b) = &w.before {
                fs::write(e.vdir.join(&w.path), b).unwrap();
            }
        }
        entry.stage = Stage::Prepared;
        Journal::open(&e.state).unwrap().prepare(&entry).unwrap();
        let (_, report) = Store::open_with(&e.cfg, matrix()).unwrap();
        assert!(report.stuck.is_empty(), "{report:?}");
        assert_eq!(report.resumed.len(), 1);
        assert_eq!(e.files(), after);
        store.undo_last(later()).unwrap();
        assert_eq!(e.files(), before);
        // An unexpected JSON-only member of either parent blocks inverse/recovery.
        store.redo_last(later()).unwrap();
        let mut cal = parse(&before[&key(K3)]).unwrap();
        let t = cal.root.children_named_mut("VTODO").next().unwrap();
        t.prop_mut("UID")
            .unwrap()
            .set_value("synthetic-reparent-newcomer");
        t.remove_props("RELATED-TO");
        fs::write(
            e.file("synthetic-reparent-newcomer"),
            twydo_core::ical::serialize(&cal),
        )
        .unwrap();
        let remote = e.files();
        assert!(store.undo_last(later()).is_err());
        assert_eq!(e.files(), remote);
    }
    #[test]
    fn reparent_guards_reject_cross_list_newcomers() {
        for parent in [K, P] {
            for phase in ["direct", "queue", "undo", "recovery"] {
                let e = Env::new(WriteMode::Verified, FIXTURE);
                let snap = e.snap();
                let mut store = e.store(matrix());
                let lock = (phase == "queue").then(|| VdirLock::exclusive(&e.state).unwrap());
                if phase == "queue" {
                    store
                        .submit(&snap, &op(Some(P)), "reparent", "op", now())
                        .unwrap();
                }
                if matches!(phase, "undo" | "recovery") {
                    store.apply(&snap, &op(Some(P)), now()).unwrap();
                }
                let mut cal = parse(&fs::read(e.file(K3)).unwrap()).unwrap();
                let t = cal.root.children_named_mut("VTODO").next().unwrap();
                t.prop_mut("UID")
                    .unwrap()
                    .set_value("synthetic-cross-list-newcomer");
                t.prop_mut("RELATED-TO").unwrap().set_value(parent);
                let prop = t.prop_mut("X-2DOAPP-METADATA").unwrap();
                let mut m = twydo_core::model::meta_edit::MetaJson::parse(&prop.value).unwrap();
                m.set_str("parentuid", parent);
                prop.set_value(&m.encode());
                fs::write(
                    e.vdir.join("list2/newcomer.ics"),
                    twydo_core::ical::serialize(&cal),
                )
                .unwrap();
                let before = e.files();
                match phase {
                    "direct" => assert!(
                        store.apply(&snap, &op(Some(P)), now()).is_err(),
                        "{parent}, {phase}"
                    ),
                    "queue" => {
                        drop(lock);
                        assert!(matches!(
                            store.drain(later()).unwrap()[0].outcome,
                            DrainOutcome::Refused(_)
                        ));
                    }
                    "undo" => assert!(store.undo_last(later()).is_err()),
                    _ => {
                        let mut entry = e.journal().pop().unwrap();
                        entry.stage = Stage::Prepared;
                        Journal::open(&e.state).unwrap().prepare(&entry).unwrap();
                        let (_, report) = Store::open_with(&e.cfg, matrix()).unwrap();
                        assert_eq!(report.stuck.len(), 1);
                    }
                }
                assert_eq!(e.files(), before);
            }
        }
    }
    #[test]
    fn reparent_candidate_destination_collisions_refuse_without_writes() {
        for same_path in [true, false] {
            let e = Env::new(WriteMode::Verified, FIXTURE);
            let mut store = e.store(matrix());
            let lock = VdirLock::exclusive(&e.state).unwrap();
            let Submitted::Queued(item) = store
                .submit(&e.snap(), &op(Some(P)), "reparent", "op", now())
                .unwrap()
            else {
                panic!("queued")
            };
            let create = &item.writes()[0];
            let path = if same_path {
                e.vdir.join(&create.path)
            } else {
                e.file("synthetic-duplicate-destination")
            };
            fs::write(
                path,
                if same_path {
                    "synthetic conflicting bytes"
                } else {
                    create.after.as_ref().unwrap()
                },
            )
            .unwrap();
            let before = e.files();
            drop(lock);
            let drained = store.drain(later()).unwrap();
            assert!(matches!(drained[0].outcome, DrainOutcome::Refused(_)));
            assert_eq!(e.files(), before);
            assert!(e.journal().is_empty());
        }
    }

    #[test]
    fn reparent_candidate_retains_foreign_series_and_internal_parent_alias() {
        let e = Env::new(WriteMode::Verified, FIXTURE);
        let snap = e.snap();
        let p = snap.find(P).unwrap();
        for i in std::iter::once(p).chain(snap.children(p).iter().copied()) {
            let path = &snap.tasks[i].path;
            let mut cal = parse(&fs::read(path).unwrap()).unwrap();
            let t = cal.root.children_named_mut("VTODO").next().unwrap();
            let prop = t.prop_mut("X-2DOAPP-METADATA").unwrap();
            let mut m = twydo_core::model::meta_edit::MetaJson::parse(&prop.value).unwrap();
            m.set_str(
                if i == p { "uid" } else { "parentuid" },
                "synthetic-internal-parent",
            );
            prop.set_value(&m.encode());
            fs::write(path, twydo_core::ical::serialize(&cal)).unwrap();
        }
        let mut cal = parse(&fs::read(e.file(K2)).unwrap()).unwrap();
        let t = cal.root.children_named_mut("VTODO").next().unwrap();
        t.insert_prop(
            twydo_core::ical::Property::new("X-SYNTHETIC-KEEP", Vec::new(), "opaque"),
            None,
        );
        let prop = t.prop_mut("X-2DOAPP-METADATA").unwrap();
        let mut m = twydo_core::model::meta_edit::MetaJson::parse(&prop.value).unwrap();
        m.set_str("RUID", "synthetic-foreign-series");
        prop.set_value(&m.encode());
        fs::write(e.file(K2), twydo_core::ical::serialize(&cal)).unwrap();
        let before = e.files();
        let snap = e.snap();
        let max = snap
            .children(p)
            .iter()
            .map(|&n| snap.tasks[n].meta.display_order())
            .max()
            .unwrap();
        let mut store = e.store(matrix());
        store.apply(&snap, &op(Some(P)), now()).unwrap();
        let entry = e.journal().pop().unwrap();
        let cal = parse(entry.writes[0].after.as_ref().unwrap().as_bytes()).unwrap();
        let t = cal.root.children_named("VTODO").next().unwrap();
        let meta = twydo_core::model::meta_edit::MetaJson::parse(
            &t.prop("X-2DOAPP-METADATA").unwrap().value,
        )
        .unwrap();
        assert_eq!(t.prop("RELATED-TO").unwrap().value, P);
        assert_eq!(meta.get_str("parentuid"), Some("synthetic-internal-parent"));
        assert_eq!(meta.get_str("RUID"), Some("synthetic-foreign-series"));
        assert_eq!(meta.get_i64("DisplayOrder"), Some(max + 1));
        assert_eq!(t.prop("X-SYNTHETIC-KEEP").unwrap().value, "opaque");
        store.undo_last(later()).unwrap();
        assert_eq!(e.files(), before);
    }
}

#[test]
fn family_guards_reject_new_internal_identity_owners() {
    for phase in ["direct", "undo", "recovery"] {
        let e = Env::new(WriteMode::Verified, FIXTURE);
        let snap = e.snap();
        let source = fs::read(e.file(K)).unwrap();
        let mut store = e.store(Matrix::embedded().clone());
        let op = Op::Delete { uid: K.into() };
        if phase != "direct" {
            store.apply(&snap, &op, now()).unwrap();
        }
        let mut cal = parse(&source).unwrap();
        let t = cal.root.children_named_mut("VTODO").next().unwrap();
        t.prop_mut("UID")
            .unwrap()
            .set_value("synthetic-duplicate-internal-owner");
        fs::write(
            e.vdir.join("list2/duplicate-owner.ics"),
            twydo_core::ical::serialize(&cal),
        )
        .unwrap();
        let before = e.files();
        match phase {
            "direct" => assert!(store.apply(&snap, &op, now()).is_err()),
            "undo" => assert!(store.undo_last(later()).is_err()),
            _ => {
                let mut entry = e.journal().pop().unwrap();
                entry.stage = Stage::Prepared;
                Journal::open(&e.state).unwrap().prepare(&entry).unwrap();
                let (_, report) = Store::open_with(&e.cfg, Matrix::embedded().clone()).unwrap();
                assert_eq!(report.stuck.len(), 1);
            }
        }
        assert_eq!(e.files(), before);
    }
}

#[test]
fn undo_identity_collision_refuses_without_blocking_later_writes() {
    use twydo_core::store::{DrainOutcome, Submitted, fsio::VdirLock};

    for queued in [false, true] {
        let e = Env::new(WriteMode::Verified, FIXTURE);
        let original = e.files();
        let mut store = e.store(Matrix::embedded().clone());
        let id = store
            .apply(&e.snap(), &Op::Delete { uid: P.into() }, now())
            .unwrap();
        let lock = queued.then(|| VdirLock::exclusive(&e.state).unwrap());
        if queued {
            assert!(matches!(
                store.undo(&id, later()).unwrap(),
                Submitted::Queued(_)
            ));
        }
        // A remote writer restores a child as a standalone task under another filename.
        let mut remote = parse(&original[&key(P2)]).unwrap();
        let todo = remote.root.children_named_mut("VTODO").next().unwrap();
        todo.remove_props("RELATED-TO");
        let prop = todo.prop_mut("X-2DOAPP-METADATA").unwrap();
        let mut meta = twydo_core::model::meta_edit::MetaJson::parse(&prop.value).unwrap();
        meta.remove("parentuid");
        prop.set_value(&meta.encode());
        fs::write(
            e.file("synthetic-restored-under-other-name"),
            twydo_core::ical::serialize(&remote),
        )
        .unwrap();
        let before = e.files();
        let entries = e.journal().len();
        if queued {
            drop(lock);
            assert!(matches!(
                store.drain(later()).unwrap()[0].outcome,
                DrainOutcome::Refused(_)
            ));
        } else {
            assert!(matches!(
                store.undo_last(later()),
                Err(WriteError::Stale(_))
            ));
        }
        assert_eq!(e.files(), before);
        assert_eq!(e.journal().len(), entries);
        store
            .apply(
                &e.snap(),
                &Op::Edit {
                    uid: K.into(),
                    changes: vec![Change::Summary("Synthetic unrelated edit".into())],
                },
                later(),
            )
            .unwrap();
    }
}
