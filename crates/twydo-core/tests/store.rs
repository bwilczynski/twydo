//! The store: gated, journaled apply with undo and redo, against a tempdir copy of the
//! iPhone capture `plan4-list1/02-edit` (synthetic test-account content).

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, TimeZone, Utc};

use twydo_core::compat::Matrix;
use twydo_core::config::WriteMode;
use twydo_core::edit::{Change, NewTask, Op};
use twydo_core::ical::{Component, parse};
use twydo_core::model::meta_edit::MetaJson;
use twydo_core::store::journal::{Entry, Journal, Stage, Write};
use twydo_core::store::{self, Store, WriteError};

use common::Env;

const A: &str = "0a04f22aeb294b719633c978fe1975e5";
const E: &str = "44123b09407c4ea1adae8f2ecdb64d49";
const FIXTURE: &str = "plan4-list1/02-edit";
const CASES: [&str; 5] = ["one-time", "monthly", "date-only", "timed", "start-date"];

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 4, 15, 0, 0).unwrap()
}

fn later() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 4, 15, 5, 0).unwrap()
}

/// Every op on every case has baseline `pass` and the twydo columns `twydo`; the monthly
/// rule class passes; the undo edge case has `edge`.
fn matrix(twydo: &str, edge: &str) -> Matrix {
    common::matrix(&CASES, twydo, &[("reopen-undo-before-after-sync", edge)])
}

fn test_env() -> (Env, Store) {
    let env = Env::new(WriteMode::Test, FIXTURE);
    let store = env.store(matrix("unverified", "unverified"));
    (env, store)
}

fn complete(uid: &str) -> Op {
    Op::Complete { uid: uid.into() }
}

fn todo_of(bytes: &[u8]) -> Component {
    let cal = parse(bytes).unwrap();
    cal.root.children_named("VTODO").next().unwrap().clone()
}

fn value(c: &Component, name: &str) -> String {
    c.prop(name).unwrap().value.clone()
}

#[test]
fn complete_recurring_writes_copy_and_advances_live() {
    let (env, mut store) = test_env();
    let before = env.files();
    let id = store.apply(&env.snap(), &complete(E), now()).unwrap();

    let after = env.files();
    assert_eq!(after.len(), before.len() + 1);
    let live = todo_of(&after[&format!("list1/{E}.ics")]);
    assert_eq!(value(&live, "DUE"), "20270110");
    assert_eq!(value(&live, "STATUS"), "NEEDS-ACTION");

    let journal = env.journal();
    assert_eq!(journal.len(), 1);
    let e = &journal[0];
    assert_eq!(e.id, id);
    assert_eq!(e.stage, Stage::Applied);
    assert_eq!(e.kind, "complete");
    assert_eq!(e.writes.len(), 2);
    // The copy is written first, then the live task.
    assert!(e.writes[0].before.is_none());
    assert!(e.writes[0].path.starts_with("list1/"));
    let copy = todo_of(e.writes[0].after.as_ref().unwrap().as_bytes());
    assert_eq!(value(&copy, "STATUS"), "COMPLETED");
    assert_eq!(e.writes[1].path, format!("list1/{E}.ics"));
    assert_eq!(
        after[&e.writes[0].path],
        e.writes[0].after.as_ref().unwrap().as_bytes()
    );
}

/// Task 14: completing the last occurrence of a counted series (`COUNT=1`) completes the
/// task in place: one write, no history copy. TO CONFIRM in Task 15 (session C).
#[test]
fn completing_the_last_occurrence_completes_in_place() {
    let env = Env::new(WriteMode::Test, FIXTURE);
    let mut m = matrix("unverified", "unverified");
    let count = Matrix::from_toml_str(
        "matrix_version = 1\n[[rule]]\nclass = \"mod:count\"\npreserve = \"pass\"\n\
         execute = \"pass\"\nedit = \"pass\"\n",
    )
    .unwrap();
    m.rule.extend(count.rule);
    let mut store = env.store(m);
    let path = env.file(E);
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("RRULE:FREQ=MONTHLY", "RRULE:FREQ=MONTHLY;COUNT=1");
    let mut cal = parse(text.as_bytes()).unwrap();
    {
        let t = cal.root.children_named_mut("VTODO").next().unwrap();
        let p = t.prop_mut("X-2DOAPP-METADATA").unwrap();
        let mut meta = MetaJson::parse(&p.value).unwrap();
        meta.set_i64("RecurrenceEndType", 2);
        meta.set_i64("RecurrenceEndRepetitions", 1);
        meta.set_i64("RecurrenceEndRepetitionsOrig", 1);
        p.set_value(&meta.encode());
    }
    fs::write(&path, twydo_core::ical::serialize(&cal)).unwrap();

    let before = env.files();
    store.apply(&env.snap(), &complete(E), now()).unwrap();
    let after = env.files();
    assert_eq!(after.len(), before.len(), "no history copy");
    let live = todo_of(&after[&format!("list1/{E}.ics")]);
    assert_eq!(value(&live, "STATUS"), "COMPLETED");
    assert_eq!(value(&live, "RRULE"), "FREQ=MONTHLY;COUNT=1");
    assert_eq!(value(&live, "DUE"), "20261210");
    let e = &env.journal()[0];
    assert_eq!(e.kind, "complete");
    assert_eq!(e.writes.len(), 1);
}

#[test]
fn unchanged_resources_untouched() {
    let (env, mut store) = test_env();
    let others: Vec<PathBuf> = env
        .files()
        .keys()
        .filter(|k| !k.contains(E))
        .map(|k| env.vdir.join(k))
        .collect();
    let state = |ps: &[PathBuf]| -> Vec<(Vec<u8>, SystemTime)> {
        ps.iter()
            .map(|p| {
                (
                    fs::read(p).unwrap(),
                    fs::metadata(p).unwrap().modified().unwrap(),
                )
            })
            .collect()
    };
    let before = state(&others);
    store.apply(&env.snap(), &complete(E), now()).unwrap();
    assert_eq!(state(&others), before);
}

#[test]
fn apply_refuses_when_file_changed_since_load() {
    let (env, mut store) = test_env();
    let snap = env.snap();
    let changed = fs::read_to_string(env.file(A))
        .unwrap()
        .replace("SUMMARY:A2", "SUMMARY:A3");
    fs::write(env.file(A), &changed).unwrap();

    let op = Op::Edit {
        uid: A.into(),
        changes: vec![Change::Summary("new".into())],
    };
    match store.apply(&snap, &op, now()) {
        Err(WriteError::Stale(p)) => assert_eq!(p, env.file(A)),
        other => panic!("expected Stale, got {other:?}"),
    }
    assert!(env.journal().is_empty());
    assert_eq!(fs::read_to_string(env.file(A)).unwrap(), changed);
}

#[test]
fn undo_restores_bytes_exactly() {
    let (env, mut store) = test_env();
    let before = env.files();
    let id = store.apply(&env.snap(), &complete(E), now()).unwrap();
    let applied = env.files();
    assert_ne!(applied, before);

    let undo_id = store.undo_last(later()).unwrap();
    assert_eq!(env.files(), before);
    let undo = Journal::open(&env.state)
        .unwrap()
        .get(&undo_id)
        .unwrap()
        .unwrap();
    assert_eq!(undo.inverse_of.as_deref(), Some(id.as_str()));
    assert_eq!(undo.stage, Stage::Applied);
    assert!(matches!(
        store.undo_last(later()),
        Err(WriteError::NothingToUndo)
    ));
}

#[test]
fn redo_last_reapplies() {
    let (env, mut store) = test_env();
    assert!(matches!(
        store.redo_last(now()),
        Err(WriteError::NothingToRedo)
    ));
    store.apply(&env.snap(), &complete(E), now()).unwrap();
    let applied = env.files();
    store.undo_last(later()).unwrap();
    store.redo_last(later()).unwrap();
    assert_eq!(env.files(), applied);
    assert!(matches!(
        store.redo_last(later()),
        Err(WriteError::NothingToRedo)
    ));
    // The redo can itself be undone.
    store.undo_last(later()).unwrap();
    assert_eq!(env.files().len(), applied.len() - 1);
}

#[test]
fn new_apply_clears_redo() {
    let (env, mut store) = test_env();
    store.apply(&env.snap(), &complete(E), now()).unwrap();
    store.undo_last(later()).unwrap();
    store.apply(&env.snap(), &complete(A), later()).unwrap();
    assert!(matches!(
        store.redo_last(later()),
        Err(WriteError::NothingToRedo)
    ));
}

#[test]
fn undo_refuses_after_external_change() {
    let (env, mut store) = test_env();
    let op = Op::Edit {
        uid: A.into(),
        changes: vec![Change::Summary("new".into())],
    };
    store.apply(&env.snap(), &op, now()).unwrap();
    let changed = fs::read_to_string(env.file(A))
        .unwrap()
        .replace("SUMMARY:new", "SUMMARY:other");
    fs::write(env.file(A), &changed).unwrap();
    let journal = env.journal();

    match store.undo_last(later()) {
        Err(WriteError::Stale(p)) => assert_eq!(p, env.file(A)),
        other => panic!("expected Stale, got {other:?}"),
    }
    assert_eq!(fs::read_to_string(env.file(A)).unwrap(), changed);
    assert_eq!(env.journal(), journal);
}

/// Logical lines of the VTODO as (name, params, value), the metadata decoded with `RUID`
/// removed, and the lines named in `skip` dropped.
fn comparable(todo: &Component, skip: &[&str]) -> (Vec<String>, Option<MetaJson>) {
    let mut lines = Vec::new();
    let mut meta = None;
    for p in &todo.props {
        if skip.contains(&p.name.as_str()) {
            continue;
        }
        if p.name == "X-2DOAPP-METADATA" {
            let mut m = MetaJson::parse(&p.value).unwrap();
            m.remove("RUID");
            meta = Some(m);
            lines.push(format!("{} {:?}", p.name, p.params));
        } else {
            lines.push(format!("{} {:?} {}", p.name, p.params, p.value));
        }
    }
    (lines, meta)
}

#[test]
fn move_creates_new_uid_like_2do() {
    let (env, mut store) = test_env();
    let source = fs::read(env.file(E)).unwrap();
    let op = Op::Move {
        uid: E.into(),
        to: "list2".into(),
    };
    store.apply(&env.snap(), &op, now()).unwrap();

    assert!(!env.file(E).exists());
    let moved: Vec<PathBuf> = fs::read_dir(env.vdir.join("list2"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(moved.len(), 1);
    let new_uid = moved[0].file_stem().unwrap().to_string_lossy().to_string();
    assert_ne!(new_uid, E);

    let src = todo_of(&source);
    let dst = todo_of(&fs::read(&moved[0]).unwrap());
    assert_eq!(value(&dst, "UID"), new_uid);
    assert_eq!(value(&dst, "DTSTAMP"), "20261104T150000Z");
    assert_eq!(value(&dst, "LAST-MODIFIED"), "20261104T150000Z");
    let ruid = MetaJson::parse(&value(&dst, "X-2DOAPP-METADATA")).unwrap();
    assert_eq!(ruid.get_str("RUID"), Some(new_uid.as_str()));
    let skip = ["UID", "DTSTAMP", "LAST-MODIFIED"];
    assert_eq!(comparable(&dst, &skip), comparable(&src, &skip));

    let e = &env.journal()[0];
    assert_eq!(e.kind, "move");
    assert_eq!(e.writes.len(), 2);
    assert!(e.writes[0].before.is_none() && e.writes[0].path.starts_with("list2/"));
    assert_eq!(e.writes[1].path, format!("list1/{E}.ics"));
    assert!(e.writes[1].after.is_none());
}

#[test]
fn undo_move_restores_source_and_removes_copy() {
    let (env, mut store) = test_env();
    let before = env.files();
    let op = Op::Move {
        uid: E.into(),
        to: "list2".into(),
    };
    store.apply(&env.snap(), &op, now()).unwrap();
    store.undo_last(later()).unwrap();
    assert_eq!(env.files(), before);
}

#[test]
fn delete_then_undo_restores_file() {
    let (env, mut store) = test_env();
    let before = env.files();
    store
        .apply(&env.snap(), &Op::Delete { uid: A.into() }, now())
        .unwrap();
    assert!(!env.file(A).exists());
    store.undo_last(later()).unwrap();
    assert_eq!(env.files(), before);
}

#[test]
fn create_writes_new_uid_file() {
    let (env, mut store) = test_env();
    let before = env.files();
    let op = Op::Create(NewTask::plain("list2", "Synthetic"));
    store.apply(&env.snap(), &op, now()).unwrap();
    let after = env.files();
    assert_eq!(after.len(), before.len() + 1);
    let (path, bytes) = after
        .iter()
        .find(|(k, _)| !before.contains_key(*k))
        .unwrap();
    let todo = todo_of(bytes);
    let uid = value(&todo, "UID");
    assert_eq!(path, &format!("list2/{uid}.ics"));
    assert_eq!(value(&todo, "SUMMARY"), "Synthetic");
    let e = &env.journal()[0];
    assert_eq!(e.kind, "create");
    assert_eq!(e.uids, vec![uid]);
    assert_eq!(e.writes.len(), 1);
    assert!(e.writes[0].before.is_none());
}

#[test]
fn blocked_op_writes_nothing() {
    let env = Env::new(WriteMode::Verified, FIXTURE);
    let mut store = env.store(matrix("unverified", "pass"));
    let before = env.files();
    match store.apply(&env.snap(), &complete(A), now()) {
        Err(WriteError::Blocked(b)) => assert_eq!(
            b.to_string(),
            "complete for one-time tasks is not verified yet (twydo_to_iphone)"
        ),
        other => panic!("expected Blocked, got {other:?}"),
    }
    assert_eq!(env.files(), before);
    assert!(env.journal().is_empty());
}

#[test]
fn undo_of_recurring_completion_needs_edge_case_in_verified_mode() {
    let env = Env::new(WriteMode::Verified, FIXTURE);
    let mut store = env.store(matrix("pass", "unverified"));
    store.apply(&env.snap(), &complete(E), now()).unwrap();
    let applied = env.files();
    match store.undo_last(later()) {
        Err(WriteError::Blocked(b)) => {
            assert!(
                b.to_string().contains("reopen-undo-before-after-sync"),
                "{b}"
            )
        }
        other => panic!("expected Blocked, got {other:?}"),
    }
    assert_eq!(env.files(), applied);
    // One-time completions undo regardless.
    store.apply(&env.snap(), &complete(A), later()).unwrap();
    store.undo_last(later()).unwrap();

    let env = Env::new(WriteMode::Verified, FIXTURE);
    let before = env.files();
    let mut store = env.store(matrix("pass", "pass"));
    store.apply(&env.snap(), &complete(E), now()).unwrap();
    store.undo_last(later()).unwrap();
    assert_eq!(env.files(), before);
}

#[test]
fn stuck_journal_blocks_writes() {
    let env = Env::new(WriteMode::Test, FIXTURE);
    let journal = Journal::open(&env.state).unwrap();
    let stuck = Entry {
        format: 1,
        id: "20261101T120000Z-deadbeef".into(),
        created: Utc.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap(),
        kind: "edit".into(),
        uids: vec![A.into()],
        stage: Stage::Prepared,
        inverse_of: None,
        synced: None,
        writes: vec![Write {
            path: format!("list1/{A}.ics"),
            before: Some("neither".into()),
            after: Some("nor".into()),
        }],
    };
    journal.prepare(&stuck).unwrap();
    let before = env.files();

    let (mut store, report) = Store::open_with(&env.cfg, matrix("unverified", "pass")).unwrap();
    assert_eq!(report.stuck.len(), 1);
    assert_eq!(report.stuck[0].0, stuck.id);
    match store.apply(&env.snap(), &complete(A), now()) {
        Err(WriteError::Stuck(msg)) => assert!(msg.contains(&stuck.id), "{msg}"),
        other => panic!("expected Stuck, got {other:?}"),
    }
    assert_eq!(env.files(), before);

    store.abandon(&stuck.id).unwrap();
    assert_eq!(
        journal.get(&stuck.id).unwrap().unwrap().stage,
        Stage::Abandoned
    );
    store.apply(&env.snap(), &complete(A), now()).unwrap();
    assert_eq!(
        todo_of(&fs::read(env.file(A)).unwrap())
            .prop("STATUS")
            .unwrap()
            .value,
        "COMPLETED"
    );
}

/// Moving a repeat's history copy keeps its `RUID` pointing at the series: only a live
/// task's self-reference follows the new UID (H5).
#[test]
fn move_history_copy_keeps_series_ruid() {
    const COPY: &str = "7d0349b58c3545d2ba6c8d87bcc45e85";
    let (env, mut store) = test_env();
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../fixtures/phase0/v1/plan4-list1/03-complete/{COPY}.ics"
    ));
    fs::copy(&src, env.file(COPY)).unwrap();
    let op = Op::Move {
        uid: COPY.into(),
        to: "list2".into(),
    };
    store.apply(&env.snap(), &op, now()).unwrap();
    assert!(!env.file(COPY).exists());
    let moved: Vec<PathBuf> = fs::read_dir(env.vdir.join("list2"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(moved.len(), 1);
    let dst = todo_of(&fs::read(&moved[0]).unwrap());
    assert_ne!(value(&dst, "UID"), COPY);
    let meta = MetaJson::parse(&value(&dst, "X-2DOAPP-METADATA")).unwrap();
    assert_eq!(meta.get_str("RUID"), Some(E));
    // The metadata blob keeps its bytes.
    let orig = todo_of(&fs::read(&src).unwrap());
    assert_eq!(
        dst.prop("X-2DOAPP-METADATA").unwrap(),
        orig.prop("X-2DOAPP-METADATA").unwrap()
    );
}

/// A Twydo-created timed task carries 2Do's auto alert, and the gate still lets a one-time
/// timed task be completed (alarms only block repeating completions).
#[test]
fn created_timed_task_has_auto_alarm_and_completes() {
    let (env, mut store) = test_env();
    let before = env.files();
    let due = chrono::NaiveDate::from_ymd_opt(2026, 12, 3)
        .unwrap()
        .and_hms_opt(17, 30, 0)
        .unwrap();
    let op = Op::Create(NewTask {
        due: Some(twydo_core::edit::Due::Time(due)),
        ..NewTask::plain("list2", "Synthetic")
    });
    store.apply(&env.snap(), &op, now()).unwrap();
    let after = env.files();
    let (_, bytes) = after
        .iter()
        .find(|(k, _)| !before.contains_key(*k))
        .unwrap();
    let todo = todo_of(bytes);
    let uid = value(&todo, "UID");
    let alarm = todo.children_named("VALARM").next().unwrap();
    assert_eq!(value(alarm, "X-2DOAPP-ISAUTO"), "1");
    let snap = env.snap();
    let t = snap.tasks.iter().find(|t| t.uid == uid).unwrap();
    assert_eq!(t.alarms.len(), 1);
    store.apply(&snap, &complete(&uid), later()).unwrap();
}

#[test]
fn undo_works_on_synced_entry() {
    let (env, mut store) = test_env();
    let before = env.files();
    let id = store.apply(&env.snap(), &complete(E), now()).unwrap();
    Journal::open(&env.state)
        .unwrap()
        .mark_synced(&id, later())
        .unwrap();
    store.undo(&id, later()).unwrap();
    assert_eq!(env.files(), before);
}

#[test]
fn restore_writes_through_journal_and_refuses_stale() {
    // Verified mode and an all-unverified matrix: `restore` has no gate.
    let env = Env::new(WriteMode::Verified, FIXTURE);
    let mut store = env.store(matrix("unverified", "unverified"));
    let rel = format!("list1/{A}.ics");
    let before = fs::read_to_string(env.file(A)).unwrap();
    let id = store
        .restore(
            &rel,
            Some(&before),
            Some("restored\n"),
            vec![A.into()],
            now(),
        )
        .unwrap();
    assert_eq!(fs::read_to_string(env.file(A)).unwrap(), "restored\n");
    let e = env.journal().into_iter().find(|e| e.id == id).unwrap();
    assert_eq!(e.kind, "resolve");
    assert_eq!(e.stage, Stage::Applied);
    assert_eq!(e.writes[0].before.as_deref(), Some(before.as_str()));
    assert_eq!(e.writes[0].after.as_deref(), Some("restored\n"));
    // The file no longer holds `before`: refused, nothing written or journaled.
    let n = env.journal().len();
    match store.restore(
        &rel,
        Some(&before),
        Some("again\n"),
        vec![A.into()],
        later(),
    ) {
        Err(WriteError::Stale(p)) => assert_eq!(p, env.file(A)),
        other => panic!("expected Stale, got {other:?}"),
    }
    assert_eq!(fs::read_to_string(env.file(A)).unwrap(), "restored\n");
    assert_eq!(env.journal().len(), n);
    // Deleting works, and creating requires the file to be absent.
    store
        .restore(&rel, Some("restored\n"), None, vec![A.into()], later())
        .unwrap();
    assert!(!env.file(A).exists());
    store
        .restore(&rel, None, Some("new\n"), vec![A.into()], later())
        .unwrap();
    assert!(matches!(
        store.restore(&rel, None, Some("x\n"), vec![A.into()], later()),
        Err(WriteError::Stale(_))
    ));
}

/// The parent-uid lookup `Op::Create` uses (the gate refuses item creates for now, so the
/// store path is tested through this function).
#[test]
fn parent_json_uid_is_the_metadata_uid_when_it_differs() {
    // `b783e95f…` is a project whose JSON `uid` is not its iCalendar UID.
    let env = Env::new(
        WriteMode::Test,
        "plan7-list1/03b-phone-item-under-converted",
    );
    let snap = env.snap();
    assert_eq!(
        store::parent_json_uid(&snap, "b783e95fdb40450aa81cab922cb114f4").as_deref(),
        Some("53dce1ea31e94a03aab3c6c7182bc306")
    );
    // A plain task (no JSON `uid`), and an unknown parent.
    assert_eq!(
        store::parent_json_uid(&snap, "08aa0eefe10e484d8221dcda095a911b"),
        None
    );
    assert_eq!(store::parent_json_uid(&snap, "nope"), None);
}

#[test]
fn parent_json_uid_is_none_for_tokens_metadata() {
    let env = Env::new(WriteMode::Test, FIXTURE);
    let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//t//EN\r\nBEGIN:VTODO\r\n\
UID:tokparent\r\nSUMMARY:T\r\nSTATUS:NEEDS-ACTION\r\n\
X-2DOAPP-METADATA:@tasktype:project\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
    fs::write(env.vdir.join("list1/tokparent.ics"), ics).unwrap();
    assert_eq!(store::parent_json_uid(&env.snap(), "tokparent"), None);
}

#[test]
fn save_with_one_refused_part_writes_nothing() {
    let (env, mut store) = test_env();
    let before = env.files();
    let op = Op::Save {
        uid: E.into(),
        changes: vec![
            Change::Summary("Synthetic renamed".into()),
            Change::Kind(twydo_core::model::TaskKind::Checklist),
        ],
        to: None,
    };
    let err = store.apply(&env.snap(), &op, now()).unwrap_err();
    assert!(matches!(err, WriteError::Blocked(_)), "{err}");
    assert_eq!(env.files(), before);
    assert!(env.journal().is_empty());
}

#[test]
fn save_with_move_is_one_undo_step() {
    let (env, mut store) = test_env();
    let before = env.files();
    let source = fs::read_to_string(env.file(E)).unwrap();
    let op = Op::Save {
        uid: E.into(),
        changes: vec![Change::Summary("Synthetic renamed".into())],
        to: Some("list2".into()),
    };
    store.apply(&env.snap(), &op, now()).unwrap();

    assert!(!env.file(E).exists());
    let moved: Vec<PathBuf> = fs::read_dir(env.vdir.join("list2"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(moved.len(), 1);
    let dst = todo_of(&fs::read(&moved[0]).unwrap());
    assert_eq!(value(&dst, "SUMMARY"), "Synthetic renamed");

    let journal = env.journal();
    assert_eq!(journal.len(), 1);
    let e = &journal[0];
    assert_eq!(e.kind, "save");
    // The source was edited, then removed: one write, the first before and the last after.
    let src: Vec<&Write> = e
        .writes
        .iter()
        .filter(|w| w.path == format!("list1/{E}.ics"))
        .collect();
    assert_eq!(src.len(), 1);
    assert_eq!(src[0].before.as_deref(), Some(source.as_str()));
    assert!(src[0].after.is_none());
    assert_eq!(e.writes.len(), 2);

    store.undo_last(later()).unwrap();
    assert_eq!(env.files(), before);
}
