use std::fs;
use std::path::Path;

use chrono::{TimeZone, Utc};
use twydo_core::store::fsio::{VdirLock, remove_durable, write_atomic};
use twydo_core::store::journal::{
    ApplyError, Entry, Journal, Stage, Write, apply_entry, new_op_id, recover, recover_locked,
};

fn entry(id: &str, writes: Vec<Write>) -> Entry {
    Entry {
        format: 1,
        id: id.to_string(),
        created: Utc.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap(),
        kind: "edit".to_string(),
        uids: vec!["uid-a".to_string()],
        stage: Stage::Prepared,
        inverse_of: None,
        writes,
    }
}

fn w(path: &str, before: Option<&str>, after: Option<&str>) -> Write {
    Write {
        path: path.to_string(),
        before: before.map(str::to_string),
        after: after.map(str::to_string),
    }
}

fn read(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok()
}

#[test]
fn write_atomic_replaces_and_leaves_no_temp() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.ics");
    write_atomic(&p, b"one").unwrap();
    write_atomic(&p, b"two").unwrap();
    assert_eq!(fs::read(&p).unwrap(), b"two");
    let names: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, vec!["a.ics".to_string()]);
}

#[test]
fn remove_durable_missing_is_ok() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("gone.ics");
    remove_durable(&p).unwrap();
    fs::write(&p, "x").unwrap();
    remove_durable(&p).unwrap();
    assert!(!p.exists());
}

#[test]
fn apply_entry_writes_creates_and_removes() {
    let vdir = tempfile::tempdir().unwrap();
    fs::create_dir(vdir.path().join("list")).unwrap();
    fs::write(vdir.path().join("list/edit.ics"), "old").unwrap();
    fs::write(vdir.path().join("list/del.ics"), "doomed").unwrap();
    let e = entry(
        "20261101T120000Z-0000000a",
        vec![
            w("list/new.ics", None, Some("created")),
            w("list/edit.ics", Some("old"), Some("new")),
            w("list/del.ics", Some("doomed"), None),
        ],
    );
    apply_entry(vdir.path(), &e).unwrap();
    assert_eq!(
        read(&vdir.path().join("list/new.ics")).as_deref(),
        Some("created")
    );
    assert_eq!(
        read(&vdir.path().join("list/edit.ics")).as_deref(),
        Some("new")
    );
    assert!(!vdir.path().join("list/del.ics").exists());
    // Applying again is a no-op (every write already equals `after`).
    apply_entry(vdir.path(), &e).unwrap();
}

#[test]
fn apply_entry_rejects_escaping_paths() {
    let vdir = tempfile::tempdir().unwrap();
    for bad in ["../x.ics", "/tmp/x.ics", "list/../../x.ics"] {
        let e = entry("20261101T120000Z-0000000b", vec![w(bad, None, Some("x"))]);
        let err = apply_entry(vdir.path(), &e).unwrap_err();
        assert!(matches!(err, ApplyError::InvalidPath(_)), "{bad}: {err}");
    }
}

#[test]
fn recover_rolls_forward_half_applied() {
    let vdir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    fs::create_dir(vdir.path().join("list")).unwrap();
    // First write already applied (history copy created), second not yet.
    fs::write(vdir.path().join("list/hist.ics"), "history").unwrap();
    fs::write(vdir.path().join("list/live.ics"), "v1").unwrap();
    let id = "20261101T120000Z-0000000c";
    let e = entry(
        id,
        vec![
            w("list/hist.ics", None, Some("history")),
            w("list/live.ics", Some("v1"), Some("v2")),
        ],
    );
    Journal::open(state.path()).unwrap().prepare(&e).unwrap();

    let report = recover(vdir.path(), state.path()).unwrap();
    assert_eq!(report.resumed, vec![id.to_string()]);
    assert!(report.stuck.is_empty());
    assert_eq!(
        read(&vdir.path().join("list/hist.ics")).as_deref(),
        Some("history")
    );
    assert_eq!(
        read(&vdir.path().join("list/live.ics")).as_deref(),
        Some("v2")
    );
    let got = Journal::open(state.path())
        .unwrap()
        .get(id)
        .unwrap()
        .unwrap();
    assert_eq!(got.stage, Stage::Applied);
}

#[test]
fn recover_is_idempotent() {
    let vdir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let id = "20261101T120000Z-0000000d";
    Journal::open(state.path())
        .unwrap()
        .prepare(&entry(id, vec![w("list/a.ics", None, Some("A"))]))
        .unwrap();
    fs::create_dir(vdir.path().join("list")).unwrap();
    let first = recover(vdir.path(), state.path()).unwrap();
    assert_eq!(first.resumed, vec![id.to_string()]);
    let mtime = fs::metadata(vdir.path().join("list/a.ics"))
        .unwrap()
        .modified()
        .unwrap();
    let second = recover(vdir.path(), state.path()).unwrap();
    assert!(second.resumed.is_empty());
    assert!(second.stuck.is_empty());
    assert_eq!(
        fs::metadata(vdir.path().join("list/a.ics"))
            .unwrap()
            .modified()
            .unwrap(),
        mtime
    );
}

#[test]
fn recover_reports_conflict_as_stuck() {
    let vdir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    fs::create_dir(vdir.path().join("list")).unwrap();
    fs::write(vdir.path().join("list/t.ics"), "third").unwrap();
    let id = "20261101T120000Z-0000000e";
    Journal::open(state.path())
        .unwrap()
        .prepare(&entry(
            id,
            vec![w("list/t.ics", Some("before"), Some("after"))],
        ))
        .unwrap();
    let report = recover(vdir.path(), state.path()).unwrap();
    assert!(report.resumed.is_empty());
    assert_eq!(report.stuck.len(), 1);
    assert_eq!(report.stuck[0].0, id);
    assert!(
        report.stuck[0].1.contains("list/t.ics"),
        "{}",
        report.stuck[0].1
    );
    assert_eq!(
        read(&vdir.path().join("list/t.ics")).as_deref(),
        Some("third")
    );
    let got = Journal::open(state.path())
        .unwrap()
        .get(id)
        .unwrap()
        .unwrap();
    assert_eq!(got.stage, Stage::Prepared);
}

#[test]
fn recover_reports_unparseable_entry_as_stuck() {
    let vdir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    Journal::open(state.path()).unwrap();
    let bad = state.path().join("journal/20261101T120000Z-0000000f.json");
    fs::write(&bad, "{not json").unwrap();
    let report = recover(vdir.path(), state.path()).unwrap();
    assert_eq!(report.stuck.len(), 1);
    assert_eq!(report.stuck[0].0, "20261101T120000Z-0000000f");
    assert!(bad.exists());
}

#[test]
fn recover_without_journal_dir_is_noop() {
    let vdir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let state_dir = state.path().join("tdo");
    let report = recover(vdir.path(), &state_dir).unwrap();
    assert!(report.resumed.is_empty() && report.stuck.is_empty());
    assert!(!state_dir.exists());
    assert!(!state_dir.join("journal").exists());
}

#[test]
fn recover_locked_works_under_held_lock() {
    let vdir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let journal = Journal::open(state.path()).unwrap();
    let id = "20261101T120000Z-00000010";
    journal
        .prepare(&entry(id, vec![w("a.ics", None, Some("A"))]))
        .unwrap();
    let _lock = VdirLock::exclusive(state.path()).unwrap();
    let report = recover_locked(vdir.path(), &journal).unwrap();
    assert_eq!(report.resumed, vec![id.to_string()]);
}

#[test]
fn journal_list_is_sorted_and_set_stage_persists() {
    let state = tempfile::tempdir().unwrap();
    let j = Journal::open(state.path()).unwrap();
    j.prepare(&entry("20261101T120002Z-00000002", vec![]))
        .unwrap();
    j.prepare(&entry("20261101T120001Z-00000001", vec![]))
        .unwrap();
    j.set_stage("20261101T120002Z-00000002", Stage::Abandoned)
        .unwrap();
    let ids: Vec<_> = j.list().unwrap().into_iter().map(|e| e.id).collect();
    assert_eq!(
        ids,
        ["20261101T120001Z-00000001", "20261101T120002Z-00000002"]
    );
    let raw =
        fs::read_to_string(state.path().join("journal/20261101T120002Z-00000002.json")).unwrap();
    assert!(raw.contains("\"abandoned\""), "{raw}");
    assert!(j.get("20261101T999999Z-00000000").unwrap().is_none());
}

#[test]
fn new_op_id_has_expected_shape() {
    let now = Utc.with_ymd_and_hms(2026, 11, 1, 9, 5, 7).unwrap();
    let id = new_op_id(now);
    let (ts, hex) = id.split_once('-').unwrap();
    assert_eq!(ts, "20261101T090507Z");
    assert_eq!(hex.len(), 8);
    assert!(
        hex.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );
    assert_ne!(new_op_id(now), new_op_id(now));
}

#[test]
fn exclusive_lock_excludes_shared() {
    let state = tempfile::tempdir().unwrap();
    let lock = VdirLock::exclusive(state.path()).unwrap();
    let path = state.path().join("vdir.lock");
    let res = std::thread::spawn(move || {
        let f = fs::File::open(&path).unwrap();
        f.try_lock_shared()
    })
    .join()
    .unwrap();
    assert!(
        matches!(res, Err(std::fs::TryLockError::WouldBlock)),
        "expected WouldBlock, got {res:?}"
    );
    drop(lock);
    let f = fs::File::open(state.path().join("vdir.lock")).unwrap();
    f.try_lock_shared().unwrap();
    let _shared = VdirLock::shared(state.path()).unwrap();
}

#[test]
fn shared_lock_creates_state_dir() {
    let state = tempfile::tempdir().unwrap();
    let dir = state.path().join("nested/tdo");
    let _l = VdirLock::shared(&dir).unwrap();
    assert!(dir.join("vdir.lock").exists());
}

#[test]
fn reader_lock_only_when_state_dir_exists() {
    let state = tempfile::tempdir().unwrap();
    let missing = state.path().join("nested/tdo");
    assert!(VdirLock::reader(&missing).unwrap().is_none());
    assert!(!state.path().join("nested").exists(), "created a state dir");

    let reader = VdirLock::reader(state.path()).unwrap();
    assert!(reader.is_some());
    // It is shared: another shared lock is granted, an exclusive one is not.
    let f = fs::File::open(state.path().join("vdir.lock")).unwrap();
    f.try_lock_shared().unwrap();
    f.unlock().unwrap();
    let g = fs::File::open(state.path().join("vdir.lock")).unwrap();
    assert!(matches!(
        g.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
}
