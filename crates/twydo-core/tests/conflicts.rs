use chrono::{DateTime, TimeZone, Utc};
use twydo_core::conflicts::*;

fn t(min: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 1, 10, min, 0).unwrap()
}

fn new(path: &str, local: Option<&str>, remote: Option<&str>) -> NewConflict {
    NewConflict {
        uid: "uid-1".into(),
        collection: "inbox".into(),
        path: path.into(),
        kind: ConflictKind::BothChanged,
        held: true,
        vdir_holds: Side::Local,
        local: local.map(String::from),
        remote: remote.map(String::from),
    }
}

#[test]
fn record_writes_inspectable_files() {
    let d = tempfile::tempdir().unwrap();
    let s = ConflictStore::open(d.path()).unwrap();
    let c = s
        .record(new("inbox/a.ics", Some("L\r\n"), Some("R\r\n")), t(0))
        .unwrap();
    let dir = d.path().join("conflicts").join(&c.id);
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("conflict.json")).unwrap()).unwrap();
    assert_eq!(json["kind"], "both-changed");
    assert_eq!(json["state"], "open");
    assert_eq!(json["vdir_holds"], "local");
    assert_eq!(std::fs::read(dir.join("local.ics")).unwrap(), b"L\r\n");
    assert_eq!(std::fs::read(dir.join("remote.ics")).unwrap(), b"R\r\n");
    let got = s.get(&c.id).unwrap().unwrap();
    assert_eq!(got, c);
    assert_eq!(got.local.as_deref(), Some("L\r\n"));
    assert_eq!(s.list().unwrap(), vec![c]);
}

#[test]
fn deleted_side_has_no_file() {
    let d = tempfile::tempdir().unwrap();
    let s = ConflictStore::open(d.path()).unwrap();
    let c = s.record(new("inbox/a.ics", None, Some("R")), t(0)).unwrap();
    let dir = d.path().join("conflicts").join(&c.id);
    assert!(!dir.join("local.ics").exists());
    assert!(dir.join("remote.ics").exists());
    let got = s.get(&c.id).unwrap().unwrap();
    assert_eq!(got.local, None);
    assert_eq!(got.remote.as_deref(), Some("R"));
}

#[test]
fn record_is_idempotent_for_same_bytes() {
    let d = tempfile::tempdir().unwrap();
    let s = ConflictStore::open(d.path()).unwrap();
    let a = s
        .record(new("inbox/a.ics", Some("L"), Some("R")), t(0))
        .unwrap();
    let b = s
        .record(new("inbox/a.ics", Some("L"), Some("R")), t(1))
        .unwrap();
    assert_eq!(a.id, b.id);
    assert_eq!(s.list().unwrap().len(), 1);
}

#[test]
fn record_supersedes_older_for_path() {
    let d = tempfile::tempdir().unwrap();
    let s = ConflictStore::open(d.path()).unwrap();
    let a = s
        .record(new("inbox/a.ics", Some("L"), Some("R")), t(0))
        .unwrap();
    let other = s
        .record(new("inbox/b.ics", Some("L"), Some("R")), t(0))
        .unwrap();
    let b = s
        .record(new("inbox/a.ics", Some("L2"), Some("R")), t(5))
        .unwrap();
    assert_ne!(a.id, b.id);
    let old = s.get(&a.id).unwrap().unwrap();
    assert_eq!(old.state, ConflictState::Superseded);
    assert_eq!(old.resolved, Some(t(5)));
    assert_eq!(
        s.get(&other.id).unwrap().unwrap().state,
        ConflictState::Open
    );
    let ids: Vec<_> = s.unresolved().unwrap().into_iter().map(|c| c.id).collect();
    assert!(ids.contains(&b.id) && ids.contains(&other.id) && !ids.contains(&a.id));
}

#[test]
fn existing_creates_nothing() {
    let d = tempfile::tempdir().unwrap();
    assert!(ConflictStore::existing(d.path()).unwrap().is_none());
    assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 0);
    ConflictStore::open(d.path()).unwrap();
    assert!(ConflictStore::existing(d.path()).unwrap().is_some());
    let missing = d.path().join("nope");
    assert!(ConflictStore::existing(&missing).unwrap().is_none());
    assert!(!missing.exists());
}

#[test]
fn choose_and_resolve_lifecycle() {
    let d = tempfile::tempdir().unwrap();
    let s = ConflictStore::open(d.path()).unwrap();
    let c = s
        .record(new("inbox/a.ics", Some("L"), Some("R")), t(0))
        .unwrap();
    s.choose(&c.id, Side::Remote).unwrap();
    let got = s.get(&c.id).unwrap().unwrap();
    assert_eq!(
        (got.state, got.choice),
        (ConflictState::Chosen, Some(Side::Remote))
    );
    assert_eq!(s.unresolved().unwrap().len(), 1);
    // Chosen -> Chosen: the user may change their mind before the next sync.
    s.choose(&c.id, Side::Local).unwrap();
    let got = s.get(&c.id).unwrap().unwrap();
    assert_eq!(
        (got.state, got.choice),
        (ConflictState::Chosen, Some(Side::Local))
    );
    s.mark_resolved(&c.id, t(3)).unwrap();
    let got = s.get(&c.id).unwrap().unwrap();
    assert_eq!(
        (got.state, got.resolved),
        (ConflictState::Resolved, Some(t(3)))
    );
    assert!(s.unresolved().unwrap().is_empty());
    assert!(s.choose(&c.id, Side::Remote).is_err());
    assert!(s.get("../x").unwrap().is_none());
}

#[test]
fn prune_keeps_unresolved() {
    let d = tempfile::tempdir().unwrap();
    let s = ConflictStore::open(d.path()).unwrap();
    let open = s
        .record(new("inbox/a.ics", Some("L"), Some("R")), t(0))
        .unwrap();
    let done = s
        .record(new("inbox/b.ics", Some("L"), Some("R")), t(0))
        .unwrap();
    let fresh = s
        .record(new("inbox/c.ics", Some("L"), Some("R")), t(0))
        .unwrap();
    s.mark_resolved(&done.id, t(1)).unwrap();
    s.mark_resolved(&fresh.id, t(30)).unwrap();
    let pruned = s.prune(t(10)).unwrap();
    assert_eq!(pruned, vec![done.id.clone()]);
    assert!(s.get(&done.id).unwrap().is_none());
    assert!(!d.path().join("conflicts").join(&done.id).exists());
    assert!(s.get(&open.id).unwrap().is_some());
    assert!(s.get(&fresh.id).unwrap().is_some());
}

#[test]
fn record_recovers_from_crash_between_write_and_supersede() {
    let d = tempfile::tempdir().unwrap();
    let s = ConflictStore::open(d.path()).unwrap();
    let a = s
        .record(new("inbox/a.ics", Some("L"), Some("R")), t(0))
        .unwrap();
    // Seed a second unresolved conflict for the path, as a crash after the new
    // conflict was written but before the older was superseded would leave.
    let dup = "20260101T000000Z-aaaaaaaa";
    let root = d.path().join("conflicts");
    std::fs::create_dir(root.join(dup)).unwrap();
    for f in ["local.ics", "remote.ics"] {
        std::fs::copy(root.join(&a.id).join(f), root.join(dup).join(f)).unwrap();
    }
    let json = std::fs::read_to_string(root.join(&a.id).join("conflict.json"))
        .unwrap()
        .replace(&a.id, dup);
    std::fs::write(root.join(dup).join("conflict.json"), json).unwrap();
    assert_eq!(s.unresolved().unwrap().len(), 2);
    let c = s
        .record(new("inbox/a.ics", Some("L2"), Some("R")), t(5))
        .unwrap();
    let un = s.unresolved().unwrap();
    assert_eq!(un.len(), 1);
    assert_eq!(un[0].id, c.id);
}

#[test]
fn old_conflict_files_still_load() {
    let d = tempfile::tempdir().unwrap();
    let s = ConflictStore::open(d.path()).unwrap();
    let c = s.record(new("inbox/a.ics", None, Some("R")), t(0)).unwrap();
    let file = d.path().join("conflicts").join(&c.id).join("conflict.json");
    let mut json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert!(json.get("resolved_by").is_none());
    assert!(json.get("earlier").is_none());
    json.as_object_mut().unwrap().remove("resolved_by");
    std::fs::write(&file, json.to_string()).unwrap();
    let got = s.get(&c.id).unwrap().unwrap();
    assert_eq!(got.resolved_by, None);
    assert!(got.earlier.is_empty());
    assert!(s.by_request("anything").unwrap().is_none());
}
