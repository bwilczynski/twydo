use chrono::{TimeZone, Utc};
use std::fs;
use twydo_core::config::{Config, WriteMode};
use twydo_core::retired;
use twydo_core::store::journal::{Entry, FORMAT, Journal, Stage, Write};
use twydo_core::store::{Store, bind_vdir_state};
use twydo_core::views::Snapshot;

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 5, 12, 0, 0).unwrap()
}
fn task() -> &'static str {
    "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:original\r\nSUMMARY:Synthetic task\r\nDESCRIPTION:Keep these notes\r\nX-UNKNOWN:preserve\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
}
fn env() -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::with_vdir(dir.path().join("vdir"));
    cfg.state_dir = Some(dir.path().join("state"));
    cfg.writes.mode = WriteMode::Test;
    fs::create_dir_all(cfg.vdir.join("old")).unwrap();
    fs::create_dir_all(cfg.vdir.join("inbox")).unwrap();
    fs::write(cfg.vdir.join("old/displayname"), "Old Projects").unwrap();
    fs::write(cfg.vdir.join("old/task.ics"), task()).unwrap();
    (dir, cfg)
}
fn snap(cfg: &Config) -> Snapshot {
    Snapshot::new(twydo_core::vdir::load(&cfg.vdir).unwrap(), cfg.clone())
}

#[test]
fn forget_preserves_exact_files_then_removes_only_the_confirmed_list() {
    let (_dir, cfg) = env();
    let preview = retired::preview(&cfg, "old").unwrap();
    assert_eq!(preview.name, "Old Projects");
    assert_eq!(preview.tasks, 1);
    let backup = retired::forget(&cfg, &preview, now()).unwrap();
    assert!(!cfg.vdir.join("old").exists());
    assert!(cfg.vdir.join("inbox").is_dir());
    assert_eq!(
        fs::read_to_string(
            retired::data_dir(&cfg, &backup.id)
                .unwrap()
                .join("old/task.ics")
        )
        .unwrap(),
        task()
    );
    assert_eq!(retired::list(&cfg).unwrap().len(), 1);
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    assert!(state.join("sync/rediscover").exists());
}

#[test]
fn changed_confirmation_snapshot_never_removes_the_newer_version() {
    let (_dir, cfg) = env();
    let preview = retired::preview(&cfg, "old").unwrap();
    fs::write(
        cfg.vdir.join("old/task.ics"),
        task().replace("Synthetic", "Changed"),
    )
    .unwrap();
    assert!(retired::forget(&cfg, &preview, now()).is_err());
    assert!(cfg.vdir.join("old/task.ics").exists());
    assert!(retired::list(&cfg).unwrap().is_empty());
}

#[test]
fn retirement_preserves_pending_evidence_and_the_other_half_of_a_move() {
    let (_dir, cfg) = env();
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let journal = Journal::open(&state).unwrap();
    let entry = Entry {
        format: FORMAT,
        id: "pending-edit".into(),
        created: now(),
        kind: "move".into(),
        uids: vec!["original".into()],
        stage: Stage::Applied,
        inverse_of: None,
        from_queue: None,
        request: None,
        synced: None,
        holds: vec![],
        family: None,
        families: Vec::new(),
        writes: vec![
            Write {
                path: "old/task.ics".into(),
                before: Some(task().into()),
                after: None,
            },
            Write {
                path: "inbox/other.ics".into(),
                before: None,
                after: Some(task().into()),
            },
        ],
    };
    journal.prepare(&entry).unwrap();
    let preview = retired::preview(&cfg, "old").unwrap();
    assert_eq!(preview.pending, 1);
    let backup = retired::forget(&cfg, &preview, now()).unwrap();
    let remaining = journal.pending().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].writes.len(), 1);
    assert_eq!(remaining[0].writes[0].path, "inbox/other.ics");
    assert!(remaining[0].synced.is_none());
    assert_eq!(backup.journal[0], entry);
}

#[test]
fn recovery_preserves_content_and_retries_without_duplicates_then_undo_redo() {
    let (_dir, cfg) = env();
    let backup = retired::forget(&cfg, &retired::preview(&cfg, "old").unwrap(), now()).unwrap();
    let (mut store, _) = Store::open(&cfg).unwrap();
    let (op, written) = store
        .recover_list(&snap(&cfg), &backup.id, "inbox", now())
        .unwrap();
    assert!(written);
    let restored = snap(&cfg);
    assert_eq!(restored.tasks.len(), 1);
    let uid = restored.tasks[0].uid.clone();
    assert_ne!(uid, "original");
    let bytes = fs::read_to_string(&restored.tasks[0].path).unwrap();
    assert!(bytes.contains("X-UNKNOWN:preserve"));
    assert!(bytes.contains("DESCRIPTION:Keep these notes"));
    let (again, written) = store
        .recover_list(&restored, &backup.id, "inbox", now())
        .unwrap();
    assert_eq!(again, op);
    assert!(!written);
    assert_eq!(snap(&cfg).tasks.len(), 1);
    store.undo_last(now()).unwrap();
    assert!(snap(&cfg).tasks.is_empty());
    assert!(!cfg.vdir.join("old").exists());
    store.redo_last(now()).unwrap();
    assert_eq!(snap(&cfg).tasks[0].uid, uid);
    assert_eq!(retired::list(&cfg).unwrap().len(), 1);
}

#[test]
fn recovery_refuses_unverified_moves_without_writing() {
    let (_dir, mut cfg) = env();
    let backup = retired::forget(&cfg, &retired::preview(&cfg, "old").unwrap(), now()).unwrap();
    cfg.writes.mode = WriteMode::Verified;
    let matrix = twydo_core::compat::Matrix::from_toml_str("matrix_version = 1\n").unwrap();
    let (mut store, _) = Store::open_with(&cfg, matrix).unwrap();
    assert!(
        store
            .recover_list(&snap(&cfg), &backup.id, "inbox", now())
            .is_err()
    );
    assert!(snap(&cfg).tasks.is_empty());
}

#[cfg(unix)]
#[test]
fn a_symlink_is_never_archived_or_removed() {
    let (dir, cfg) = env();
    fs::write(dir.path().join("private"), "private").unwrap();
    std::os::unix::fs::symlink(dir.path().join("private"), cfg.vdir.join("old/link")).unwrap();
    assert!(retired::preview(&cfg, "old").is_err());
    assert!(cfg.vdir.join("old/task.ics").exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("private")).unwrap(),
        "private"
    );
}

#[test]
fn interrupted_retirement_finishes_before_recovering_the_journal() {
    let (_dir, cfg) = env();
    let b = retired::forget(&cfg, &retired::preview(&cfg, "old").unwrap(), now()).unwrap();
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let record = state.join("retired").join(&b.id).join("backup.json");
    let mut b = b;
    b.retired = false;
    fs::write(record, serde_json::to_vec(&b).unwrap()).unwrap();
    fs::create_dir(cfg.vdir.join("old")).unwrap();
    fs::write(cfg.vdir.join("old/displayname"), "Old Projects").unwrap();
    // task.ics has already been removed: resume must accept an exact subset.
    Store::open(&cfg).unwrap();
    assert!(!cfg.vdir.join("old").exists());
    assert!(retired::get(&cfg, &b.id).unwrap().retired);
}

#[test]
fn archived_conflicts_cannot_recreate_the_retired_collection() {
    use twydo_core::conflicts::{ConflictKind, ConflictStore, NewConflict, Side};
    use twydo_core::sync::resolve::{Keep, resolve};
    let (_dir, cfg) = env();
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let c = ConflictStore::open(&state)
        .unwrap()
        .record(
            NewConflict {
                uid: "original".into(),
                collection: "old".into(),
                path: "old/task.ics".into(),
                kind: ConflictKind::BothChanged,
                held: true,
                vdir_holds: Side::Local,
                local: Some(task().into()),
                remote: Some(task().replace("Synthetic", "Remote")),
            },
            now(),
        )
        .unwrap();
    let b = retired::forget(&cfg, &retired::preview(&cfg, "old").unwrap(), now()).unwrap();
    assert!(resolve(&cfg, &c.id, Keep::Remote, now()).is_err());
    assert!(twydo_core::sync::summary(&cfg).unwrap().is_none()); // sync disabled
    assert_eq!(
        fs::read_to_string(
            state
                .join("retired")
                .join(b.id)
                .join("conflicts")
                .join(c.id)
                .join("remote.ics")
        )
        .unwrap(),
        task().replace("Synthetic", "Remote")
    );
    assert!(!cfg.vdir.join("old").exists());
}

#[test]
fn recovery_remaps_family_and_history_references_as_one_operation() {
    use twydo_core::model::meta_edit::MetaJson;
    let (_dir, cfg) = env();
    let parent=task().replace("UID:original","UID:parent").replace("SUMMARY:Synthetic task",
        "SUMMARY:Parent\r\nX-2DOAPP-METADATA:<2Do Meta>%7B%22TaskType%22%3A2%2C%22uid%22%3A%22parent%22%7D</2Do Meta>");
    let child=task().replace("UID:original","UID:child\r\nRELATED-TO:parent").replace("SUMMARY:Synthetic task",
        "SUMMARY:Child\r\nX-2DOAPP-METADATA:<2Do Meta>%7B%22TaskType%22%3A0%2C%22parentuid%22%3A%22parent%22%7D</2Do Meta>");
    let history=task().replace("UID:original","UID:history\r\nSTATUS:COMPLETED").replace("SUMMARY:Synthetic task",
        "SUMMARY:History\r\nX-2DOAPP-METADATA:<2Do Meta>%7B%22TaskType%22%3A0%2C%22RUID%22%3A%22parent%22%7D</2Do Meta>");
    for (name, text) in [("parent", parent), ("child", child), ("history", history)] {
        fs::write(cfg.vdir.join(format!("old/{name}.ics")), text).unwrap();
    }
    let b = retired::forget(&cfg, &retired::preview(&cfg, "old").unwrap(), now()).unwrap();
    let (mut store, _) = Store::open(&cfg).unwrap();
    store
        .recover_list(&snap(&cfg), &b.id, "inbox", now())
        .unwrap();
    let restored = snap(&cfg);
    assert_eq!(restored.tasks.len(), 4);
    let p = restored
        .tasks
        .iter()
        .find(|t| t.summary == "Parent")
        .unwrap();
    let c = restored
        .tasks
        .iter()
        .find(|t| t.summary == "Child")
        .unwrap();
    let h = restored
        .tasks
        .iter()
        .find(|t| t.summary == "History")
        .unwrap();
    assert_eq!(c.parent.as_deref(), Some(p.uid.as_str()));
    assert_eq!(
        MetaJson::parse(c.meta.raw.as_deref().unwrap())
            .unwrap()
            .get_str("parentuid"),
        Some(p.uid.as_str())
    );
    assert_eq!(
        MetaJson::parse(h.meta.raw.as_deref().unwrap())
            .unwrap()
            .get_str("RUID"),
        Some(p.uid.as_str())
    );
    let journal = Journal::open(&bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap()).unwrap();
    let entries: Vec<_> = journal
        .list()
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == "recover")
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].writes.len(), 4);
}

#[test]
fn unreadable_backup_resources_refuse_the_whole_recovery() {
    let (_dir, cfg) = env();
    fs::write(cfg.vdir.join("old/broken.ics"), "not a calendar").unwrap();
    let b = retired::forget(&cfg, &retired::preview(&cfg, "old").unwrap(), now()).unwrap();
    let (mut store, _) = Store::open(&cfg).unwrap();
    assert!(
        store
            .recover_list(&snap(&cfg), &b.id, "inbox", now())
            .is_err()
    );
    assert!(snap(&cfg).tasks.is_empty());
    assert!(
        retired::data_dir(&cfg, &b.id)
            .unwrap()
            .join("old/broken.ics")
            .exists()
    );
}

#[test]
fn an_edited_recovered_task_is_never_overwritten_on_retry() {
    let (_dir, cfg) = env();
    let b = retired::forget(&cfg, &retired::preview(&cfg, "old").unwrap(), now()).unwrap();
    let (mut store, _) = Store::open(&cfg).unwrap();
    store
        .recover_list(&snap(&cfg), &b.id, "inbox", now())
        .unwrap();
    let file = snap(&cfg).tasks[0].path.clone();
    let changed = fs::read_to_string(&file)
        .unwrap()
        .replace("Synthetic task", "Later edit");
    fs::write(&file, &changed).unwrap();
    assert!(
        store
            .recover_list(&snap(&cfg), &b.id, "inbox", now())
            .is_err()
    );
    assert_eq!(fs::read_to_string(file).unwrap(), changed);
}

#[test]
fn recovery_cannot_use_a_different_destination_on_retry() {
    let (_dir, cfg) = env();
    fs::create_dir(cfg.vdir.join("other")).unwrap();
    let b = retired::forget(&cfg, &retired::preview(&cfg, "old").unwrap(), now()).unwrap();
    let (mut store, _) = Store::open(&cfg).unwrap();
    store
        .recover_list(&snap(&cfg), &b.id, "inbox", now())
        .unwrap();
    assert!(
        store
            .recover_list(&snap(&cfg), &b.id, "other", now())
            .is_err()
    );
    assert_eq!(fs::read_dir(cfg.vdir.join("other")).unwrap().count(), 0);
}

#[test]
fn backup_keeps_a_file_named_like_an_atomic_write_temporary() {
    let (_dir, cfg) = env();
    fs::write(cfg.vdir.join("old/.task.ics.tdo-tmp"), "scratch evidence").unwrap();
    let b = retired::forget(&cfg, &retired::preview(&cfg, "old").unwrap(), now()).unwrap();
    assert_eq!(
        fs::read_to_string(
            retired::data_dir(&cfg, &b.id)
                .unwrap()
                .join("old/.task.ics.tdo-tmp")
        )
        .unwrap(),
        "scratch evidence"
    );
}
#[test]
fn recovered_operation_survives_journal_pruning() {
    let (_dir, cfg) = env();
    let b = retired::forget(&cfg, &retired::preview(&cfg, "old").unwrap(), now()).unwrap();
    let (mut store, _) = Store::open(&cfg).unwrap();
    let (id, _) = store
        .recover_list(&snap(&cfg), &b.id, "inbox", now())
        .unwrap();
    let journal = Journal::open(&bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap()).unwrap();
    journal.mark_synced(&id, now()).unwrap();
    journal.prune(now() + chrono::Duration::days(31)).unwrap();
    assert!(journal.get(&id).unwrap().is_some());
    let path = snap(&cfg).tasks[0].path.clone();
    fs::remove_file(path).unwrap();
    assert!(
        store
            .recover_list(&snap(&cfg), &b.id, "inbox", now())
            .is_err()
    );
}
