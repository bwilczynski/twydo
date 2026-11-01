//! The store: gated, journaled apply with undo and redo, against a tempdir copy of the
//! iPhone capture `plan4-list1/02-edit` (synthetic test-account content).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, TimeZone, Utc};

use twydo_core::compat::Matrix;
use twydo_core::config::{Config, WriteMode};
use twydo_core::edit::{Change, NewTask, Op};
use twydo_core::ical::{Component, parse};
use twydo_core::model::meta_edit::MetaJson;
use twydo_core::store::journal::{Entry, Journal, Stage, Write};
use twydo_core::store::{Store, WriteError};
use twydo_core::views::Snapshot;

const A: &str = "0a04f22aeb294b719633c978fe1975e5";
const E: &str = "44123b09407c4ea1adae8f2ecdb64d49";
const ALL_OPS: [&str; 6] = ["create", "edit", "complete", "reopen", "move", "delete"];
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
    let mut s = String::from("matrix_version = 1\n");
    for case in CASES {
        for op in ALL_OPS {
            s.push_str(&format!(
                "[[operation]]\ncase = \"{case}\"\nop = \"{op}\"\niphone_baseline = \"pass\"\n\
                 mac_to_iphone = \"unverified\"\niphone_to_mac = \"unverified\"\n\
                 twydo_to_iphone = \"{twydo}\"\niphone_to_twydo = \"{twydo}\"\n"
            ));
        }
    }
    s.push_str(
        "[[rule]]\nclass = \"months-day-1-28\"\npreserve = \"pass\"\nexecute = \"pass\"\nedit = \"pass\"\n",
    );
    s.push_str(&format!(
        "[[edge_case]]\nid = \"reopen-undo-before-after-sync\"\nverdict = \"{edge}\"\n"
    ));
    Matrix::from_toml_str(&s).unwrap()
}

struct Env {
    _tmp: tempfile::TempDir,
    vdir: PathBuf,
    state: PathBuf,
    cfg: Config,
}

impl Env {
    fn new(mode: WriteMode) -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let vdir = tmp.path().join("vdir");
        let state = tmp.path().join("state");
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/phase0/v1/plan4-list1/02-edit");
        let list1 = vdir.join("list1");
        fs::create_dir_all(&list1).unwrap();
        fs::create_dir_all(vdir.join("list2")).unwrap();
        for e in fs::read_dir(&src).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "ics") {
                fs::copy(&p, list1.join(p.file_name().unwrap())).unwrap();
            }
        }
        let mut cfg = Config::with_vdir(vdir.clone());
        cfg.timezone = Some("America/New_York".to_string());
        cfg.state_dir = Some(state.clone());
        // The store keeps its journal in the vdir's own state dir.
        let state = twydo_core::store::vdir_state_dir(&state, &vdir).unwrap();
        cfg.writes.mode = mode;
        Env {
            _tmp: tmp,
            vdir,
            state,
            cfg,
        }
    }

    fn snap(&self) -> Snapshot {
        let report = twydo_core::vdir::load(&self.vdir).unwrap();
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        Snapshot::new(report, self.cfg.clone())
    }

    fn store(&self, m: Matrix) -> Store {
        let (store, report) = Store::open_with(&self.cfg, m).unwrap();
        assert!(report.stuck.is_empty(), "{report:?}");
        store
    }

    fn file(&self, uid: &str) -> PathBuf {
        self.vdir.join("list1").join(format!("{uid}.ics"))
    }

    /// Every file under the vdir (relative path → bytes).
    fn files(&self) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        for list in ["list1", "list2"] {
            for e in fs::read_dir(self.vdir.join(list)).unwrap() {
                let p = e.unwrap().path();
                let name = format!("{list}/{}", p.file_name().unwrap().to_string_lossy());
                out.insert(name, fs::read(&p).unwrap());
            }
        }
        out
    }

    fn journal(&self) -> Vec<Entry> {
        if !self.state.join("journal").is_dir() {
            return Vec::new();
        }
        Journal::open(&self.state).unwrap().list().unwrap()
    }
}

fn test_env() -> (Env, Store) {
    let env = Env::new(WriteMode::Test);
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
    let op = Op::Create(NewTask {
        collection: "list2".into(),
        summary: "Synthetic".into(),
        description: None,
        due: None,
        start: None,
    });
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
    let env = Env::new(WriteMode::Verified);
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
    let env = Env::new(WriteMode::Verified);
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

    let env = Env::new(WriteMode::Verified);
    let before = env.files();
    let mut store = env.store(matrix("pass", "pass"));
    store.apply(&env.snap(), &complete(E), now()).unwrap();
    store.undo_last(later()).unwrap();
    assert_eq!(env.files(), before);
}

#[test]
fn stuck_journal_blocks_writes() {
    let env = Env::new(WriteMode::Test);
    let journal = Journal::open(&env.state).unwrap();
    let stuck = Entry {
        format: 1,
        id: "20261101T120000Z-deadbeef".into(),
        created: Utc.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap(),
        kind: "edit".into(),
        uids: vec![A.into()],
        stage: Stage::Prepared,
        inverse_of: None,
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
        collection: "list2".into(),
        summary: "Synthetic".into(),
        description: None,
        due: Some(twydo_core::edit::Due::Time(due)),
        start: None,
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
