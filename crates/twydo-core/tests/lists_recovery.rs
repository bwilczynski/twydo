use std::fs;
use twydo_core::{
    config::{Config, SyncConfig, WriteMode},
    lists::{ListResult, ListService, backup},
};

fn offline() -> (tempfile::TempDir, Config) {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let mut cfg = Config::with_vdir(d.path().join("vdir"));
    cfg.state_dir = Some(d.path().join("state"));
    cfg.writes.mode = WriteMode::Test;
    fs::create_dir_all(cfg.vdir.join("synthetic")).unwrap();
    fs::write(cfg.vdir.join("synthetic/displayname"), "Before").unwrap();
    fs::write(
        cfg.vdir.join("synthetic/local.ics"),
        b"unparseable local synthetic task",
    )
    .unwrap();
    fs::write(
        cfg.vdir.join("synthetic/unknown.dat"),
        b"opaque local bytes\0",
    )
    .unwrap();
    let connection = d.path().join("connection.json");
    fs::write(
        &connection,
        serde_json::json!({"vdir":cfg.vdir}).to_string(),
    )
    .unwrap();
    let wrapper = d.path().join("python");
    let backend =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/common/list_backend.py");
    fs::write(
        &wrapper,
        format!(
            "#!/usr/bin/env python3\nimport runpy\nrunpy.run_path({:?},run_name='__main__')\n",
            backend.to_str().unwrap()
        ),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    cfg.sync = Some(SyncConfig {
        pair: "synthetic".into(),
        local_storage: "local".into(),
        list_python: wrapper.to_str().unwrap().into(),
        vdirsyncer_config: Some(connection),
        ..Default::default()
    });
    fs::write(d.path().join("server.json"),serde_json::json!({"calls":[],"flags":{},"collections":{"synthetic":{"name":"Before","color":"#3289D2","unknown":"preserved"}}}).to_string()).unwrap();
    (d, cfg)
}

#[test]
fn preview_preserves_remote_only_malformed_and_local_raw_content_without_deleting() {
    let (d, cfg) = offline();
    let before = twydo_core::lists::operation::snapshot(&cfg.vdir.join("synthetic")).unwrap();
    let mut service = ListService::open(&cfg, None).unwrap();
    let preview = service.preview_delete("synthetic").unwrap();
    assert_eq!(preview.resources, 2);
    let saved = backup::get(&cfg, &preview.backup).unwrap();
    assert_eq!(saved.local, before.unwrap());
    assert_eq!(
        saved.remote.resources[0].data().unwrap(),
        b"malformed synthetic VTODO\0"
    );
    assert_eq!(
        saved.remote.resources[1].data().unwrap(),
        b"opaque synthetic resource"
    );
    assert!(
        twydo_core::lists::operation::active(&cfg)
            .unwrap()
            .is_none()
    );
    let calls: serde_json::Value =
        serde_json::from_slice(&fs::read(d.path().join("server.json")).unwrap()).unwrap();
    assert!(
        !calls["calls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "DELETE")
    );
    let export = d.path().join("export");
    backup::export(&cfg, &preview.backup, &export).unwrap();
    assert_eq!(
        fs::read(export.join("remote/0000.bin")).unwrap(),
        b"malformed synthetic VTODO\0"
    );
    assert!(backup::export(&cfg, &preview.backup, &export).is_err());
    assert!(backup::export(&cfg, &preview.backup, &cfg.vdir.join("export")).is_err());
}

#[test]
fn incomplete_inventory_never_publishes_a_backup_or_leaves_a_write_fence() {
    let (d, cfg) = offline();
    let file = d.path().join("server.json");
    let mut server: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    server["flags"]["inventory_fault"] = "weak".into();
    fs::write(file, server.to_string()).unwrap();
    assert!(
        ListService::open(&cfg, None)
            .unwrap()
            .preview_delete("synthetic")
            .is_err()
    );
    assert!(backup::list(&cfg).unwrap().is_empty());
    assert!(
        twydo_core::lists::operation::active(&cfg)
            .unwrap()
            .is_none()
    );
    assert!(cfg.vdir.join("synthetic/local.ics").exists());
}

#[test]
fn failed_backup_publication_never_deletes_and_releases_the_read_only_fence() {
    let (d, cfg) = offline();
    let bound = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    fs::create_dir_all(bound.join("lists")).unwrap();
    fs::write(
        bound.join("lists/backups"),
        b"synthetic unusable destination",
    )
    .unwrap();
    assert!(
        ListService::open(&cfg, None)
            .unwrap()
            .preview_delete("synthetic")
            .is_err()
    );
    assert!(
        twydo_core::lists::operation::active(&cfg)
            .unwrap()
            .is_none()
    );
    let server: serde_json::Value =
        serde_json::from_slice(&fs::read(d.path().join("server.json")).unwrap()).unwrap();
    assert!(
        !server["calls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "DELETE")
    );
}

#[test]
fn interrupted_read_only_preview_can_be_cleared_without_repeating_network_requests() {
    let (d, cfg) = offline();
    let mut request = twydo_core::lists::helper::HelperRequest::probe(&cfg).unwrap();
    request.action = "preview-delete".into();
    request.collection = Some("synthetic".into());
    let mut record =
        twydo_core::lists::operation::prepare(&cfg, request, chrono::Utc::now()).unwrap();
    twydo_core::lists::operation::set_phase(
        &cfg,
        &mut record,
        twydo_core::lists::operation::Phase::RemoteStarted,
    )
    .unwrap();
    let message = ListService::open(&cfg, None)
        .unwrap()
        .reconcile(&record.id)
        .unwrap_err()
        .to_string();
    assert!(message.contains("read-only"));
    assert!(
        twydo_core::lists::operation::active(&cfg)
            .unwrap()
            .is_none()
    );
    let server: serde_json::Value =
        serde_json::from_slice(&fs::read(d.path().join("server.json")).unwrap()).unwrap();
    assert!(server["calls"].as_array().unwrap().is_empty());
}

#[test]
fn backup_integrity_and_private_directory_permissions_are_checked_before_export() {
    use std::os::unix::fs::PermissionsExt;
    let (d, cfg) = offline();
    let preview = ListService::open(&cfg, None)
        .unwrap()
        .preview_delete("synthetic")
        .unwrap();
    let bound = twydo_core::store::existing_vdir_state(&cfg.state_dir(), &cfg.vdir)
        .unwrap()
        .unwrap();
    let directory = bound.join("lists/backups").join(&preview.backup);
    assert_eq!(
        fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let manifest = directory.join("backup.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["remote"]["resources"][0]["sha256"] = "invalid".into();
    fs::write(&manifest, value.to_string()).unwrap();
    assert!(backup::export(&cfg, &preview.backup, &d.path().join("export")).is_err());
    assert!(!d.path().join("export").exists());
}

fn server(d: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(d.join("server.json")).unwrap()).unwrap()
}
fn deletes(d: &std::path::Path) -> usize {
    server(d)["calls"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| **v == "DELETE")
        .count()
}
fn set_flag(d: &std::path::Path, name: &str) {
    let mut value = server(d);
    value["flags"][name] = true.into();
    fs::write(d.join("server.json"), value.to_string()).unwrap();
}
fn no_active(cfg: &Config) {
    assert!(twydo_core::lists::operation::active(cfg).unwrap().is_none());
}

#[test]
fn delete_sends_one_delete_retires_local_copy_and_keeps_backup() {
    let (d, cfg) = offline();
    let mut service = ListService::open(&cfg, None).unwrap();
    let preview = service.preview_delete("synthetic").unwrap();
    let ListResult::Deleted {
        collection,
        name,
        backup: saved,
        retired,
        ..
    } = service.delete_list(&preview).unwrap()
    else {
        panic!("expected a deletion")
    };
    assert_eq!(
        (collection.as_str(), name.as_str()),
        ("synthetic", "Before")
    );
    assert_eq!(saved, preview.backup);
    assert_eq!(deletes(d.path()), 1);
    assert!(server(d.path())["collections"].get("synthetic").is_none());
    assert!(!cfg.vdir.join("synthetic").exists());
    assert!(backup::get(&cfg, &saved).is_ok());
    backup::export(&cfg, &saved, &d.path().join("export")).unwrap();
    let kept = twydo_core::retired::get(&cfg, &retired).unwrap();
    assert!(kept.retired);
    assert_eq!(kept.collection, "synthetic");
    no_active(&cfg);
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    assert!(state.join("sync/rediscover").exists());
}

#[test]
fn delete_succeeds_when_the_collection_was_forgotten_before() {
    let (_d, cfg) = offline();
    // A copy retired with a later timestamp must not be mistaken for this deletion's.
    let earlier = twydo_core::retired::preview(&cfg, "Before").unwrap();
    let old = twydo_core::retired::forget(
        &cfg,
        &earlier,
        chrono::Utc::now() + chrono::Duration::hours(1),
    )
    .unwrap();
    // The list is discovered again with the same collection identity.
    fs::create_dir_all(cfg.vdir.join("synthetic")).unwrap();
    fs::write(cfg.vdir.join("synthetic/displayname"), "Before").unwrap();
    fs::write(
        cfg.vdir.join("synthetic/local.ics"),
        b"unparseable local synthetic task",
    )
    .unwrap();
    let mut service = ListService::open(&cfg, None).unwrap();
    let preview = service.preview_delete("synthetic").unwrap();
    let ListResult::Deleted { retired, .. } = service.delete_list(&preview).unwrap() else {
        panic!("expected a deletion")
    };
    assert_ne!(retired, old.id);
    assert!(twydo_core::retired::get(&cfg, &retired).unwrap().retired);
    no_active(&cfg);
}

#[test]
fn delete_refuses_changed_local_bytes_without_sending_delete() {
    let (d, cfg) = offline();
    let mut service = ListService::open(&cfg, None).unwrap();
    let preview = service.preview_delete("synthetic").unwrap();
    fs::write(cfg.vdir.join("synthetic/local.ics"), b"edited after backup").unwrap();
    let message = service.delete_list(&preview).unwrap_err().to_string();
    assert!(message.contains("review deletion again"), "{message}");
    assert_eq!(deletes(d.path()), 0);
    no_active(&cfg);
    assert!(cfg.vdir.join("synthetic").exists());
}

#[test]
fn delete_refuses_changed_remote_inventory_without_sending_delete() {
    let (d, cfg) = offline();
    let mut service = ListService::open(&cfg, None).unwrap();
    let preview = service.preview_delete("synthetic").unwrap();
    set_flag(d.path(), "mutate_before_delete");
    let message = service.delete_list(&preview).unwrap_err().to_string();
    assert!(message.contains("review deletion again"), "{message}");
    assert_eq!(deletes(d.path()), 0);
    no_active(&cfg);
    assert!(cfg.vdir.join("synthetic").exists());
}

#[test]
fn delete_refuses_a_preview_whose_backup_belongs_to_another_list() {
    let (d, cfg) = offline();
    let mut service = ListService::open(&cfg, None).unwrap();
    let mut preview = service.preview_delete("synthetic").unwrap();
    fs::create_dir(cfg.vdir.join("other")).unwrap();
    preview.collection = "other".into();
    assert!(service.delete_list(&preview).is_err());
    assert_eq!(deletes(d.path()), 0);
    no_active(&cfg);
}

#[test]
fn delete_refuses_pending_queue_for_the_list() {
    use twydo_core::edit::{NewTask, Op};
    use twydo_core::store::{Store, Submitted};
    let (d, cfg) = offline();
    let mut service = ListService::open(&cfg, None).unwrap();
    let preview = service.preview_delete("synthetic").unwrap();
    let view =
        twydo_core::views::Snapshot::new(twydo_core::vdir::load(&cfg.vdir).unwrap(), cfg.clone());
    let (mut store, _) = Store::open(&cfg).unwrap();
    // While a sync (here, the lock) holds the vdir the edit waits in the queue.
    let lock = twydo_core::store::fsio::VdirLock::exclusive(store.state_dir()).unwrap();
    let submitted = store
        .submit(
            &view,
            &Op::Create(NewTask::plain("synthetic", "Synthetic task")),
            "create",
            "test",
            chrono::Utc::now(),
        )
        .unwrap();
    assert!(matches!(submitted, Submitted::Queued(_)));
    drop(lock);
    drop(store);
    let message = service.delete_list(&preview).unwrap_err().to_string();
    assert!(message.contains("queued task edits"), "{message}");
    assert_eq!(deletes(d.path()), 0);
    no_active(&cfg);
}

#[cfg(unix)]
#[test]
fn timeout_after_delete_reconciles_absence_and_retires_without_a_second_delete() {
    let (d, mut cfg) = offline();
    cfg.sync.as_mut().unwrap().timeout_secs = 1;
    let mut service = ListService::open(&cfg, None).unwrap();
    let preview = service.preview_delete("synthetic").unwrap();
    set_flag(d.path(), "hang_after_delete_once");
    let message = service.delete_list(&preview).unwrap_err().to_string();
    assert!(message.contains("needs reconciliation"), "{message}");
    assert!(
        cfg.vdir.join("synthetic").exists(),
        "no retirement before confirmation"
    );
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let id = fs::read_to_string(state.join("lists/active")).unwrap();
    let result = service.reconcile(id.trim()).unwrap();
    assert!(matches!(result, ListResult::Deleted { .. }), "{result:?}");
    assert_eq!(deletes(d.path()), 1);
    assert!(!cfg.vdir.join("synthetic").exists());
    no_active(&cfg);
    // The finished operation reports the same result when asked again.
    assert!(matches!(
        service.reconcile(id.trim()).unwrap(),
        ListResult::Deleted { .. }
    ));
}

#[cfg(unix)]
#[test]
fn reconcile_with_list_still_present_releases_the_fence() {
    let (d, cfg) = offline();
    let mut service = ListService::open(&cfg, None).unwrap();
    let preview = service.preview_delete("synthetic").unwrap();
    set_flag(d.path(), "fail_delete_once");
    let message = service.delete_list(&preview).unwrap_err().to_string();
    assert!(message.contains("needs reconciliation"), "{message}");
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let id = fs::read_to_string(state.join("lists/active")).unwrap();
    let message = service.reconcile(id.trim()).unwrap_err().to_string();
    assert!(message.contains("still exists"), "{message}");
    no_active(&cfg);
    assert!(cfg.vdir.join("synthetic/local.ics").exists());
    assert!(server(d.path())["collections"].get("synthetic").is_some());
    // The list can be deleted again from a fresh preview.
    let preview = service.preview_delete("synthetic").unwrap();
    assert!(matches!(
        service.delete_list(&preview).unwrap(),
        ListResult::Deleted { .. }
    ));
}

#[cfg(unix)]
#[test]
fn deleted_list_is_not_recreated_by_the_next_sync() {
    use twydo_core::sync::discover::discover;
    let (d, cfg) = offline();
    let mut service = ListService::open(&cfg, None).unwrap();
    let preview = service.preview_delete("synthetic").unwrap();
    service.delete_list(&preview).unwrap();
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let found = discover(&cfg, &state).unwrap();
    assert!(found.adopted.is_empty());
    assert!(!cfg.vdir.join("synthetic").exists());
    assert!(
        !server(d.path())["calls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "MKCALENDAR")
    );
}

/// A delete whose DELETE reached the server and was confirmed, with retirement not begun: the
/// operation is unfinished and the next reconcile retires the list.
#[cfg(unix)]
fn confirmed_but_unretired() -> (
    tempfile::TempDir,
    Config,
    ListService,
    String,
    std::path::PathBuf,
) {
    let (d, mut cfg) = offline();
    cfg.sync.as_mut().unwrap().timeout_secs = 1;
    let mut service = ListService::open(&cfg, None).unwrap();
    let preview = service.preview_delete("synthetic").unwrap();
    set_flag(d.path(), "hang_after_delete_once");
    assert!(service.delete_list(&preview).is_err());
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let id = fs::read_to_string(state.join("lists/active")).unwrap();
    let id = id.trim().to_string();
    // The helper confirmed the deletion (a crash before retirement began).
    let file = state.join("lists/operations").join(format!("{id}.json"));
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    record["phase"] = "remote-confirmed".into();
    record["receipt"] = serde_json::json!({"deleted": true, "collection": "synthetic"});
    fs::write(file, record.to_string()).unwrap();
    (d, cfg, service, id, state)
}

#[cfg(unix)]
#[test]
fn local_edit_after_confirmed_delete_is_preserved_and_retired() {
    let (d, cfg, mut service, id, _state) = confirmed_but_unretired();
    fs::write(
        cfg.vdir.join("synthetic/local.ics"),
        b"edited after the delete",
    )
    .unwrap();
    let ListResult::Deleted { retired, .. } = service.reconcile(&id).unwrap() else {
        panic!("expected a deletion")
    };
    assert_eq!(deletes(d.path()), 1);
    assert!(!cfg.vdir.join("synthetic").exists());
    no_active(&cfg);
    let kept = twydo_core::retired::data_dir(&cfg, &retired).unwrap();
    assert_eq!(
        fs::read(kept.join("synthetic/local.ics")).unwrap(),
        b"edited after the delete"
    );
}

#[cfg(unix)]
#[test]
fn interrupted_retirement_with_partial_directory_resumes() {
    let (d, cfg, mut service, id, state) = confirmed_but_unretired();
    // A crash inside resume_locked: the manifest is in place, removal got part way.
    let files = twydo_core::lists::operation::snapshot(&cfg.vdir.join("synthetic"))
        .unwrap()
        .unwrap();
    let retired_id = "20261112T000000Z-partial";
    let dir = state.join("retired").join(retired_id);
    for (name, bytes) in &files {
        let path = dir.join("data/synthetic").join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    let manifest = twydo_core::retired::Backup {
        format: 1,
        id: retired_id.into(),
        collection: "synthetic".into(),
        name: "Before".into(),
        created: chrono::Utc::now(),
        tasks: 1,
        pending: 0,
        files: files.keys().cloned().collect(),
        journal: vec![],
        conflicts: vec![],
        retired: false,
        recovery: None,
        destination: None,
    };
    fs::write(
        dir.join("backup.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    fs::remove_file(cfg.vdir.join("synthetic/local.ics")).unwrap();
    let ListResult::Deleted { retired, .. } = service.reconcile(&id).unwrap() else {
        panic!("expected a deletion")
    };
    assert_eq!(retired, retired_id);
    assert_eq!(deletes(d.path()), 1);
    assert!(!cfg.vdir.join("synthetic").exists());
    no_active(&cfg);
}

#[cfg(unix)]
#[test]
fn uncertain_delete_with_local_edit_reconciles_and_preserves_edit() {
    // The server applied the DELETE; the helper timed out and the list was edited afterwards.
    let (d, mut cfg) = offline();
    cfg.sync.as_mut().unwrap().timeout_secs = 1;
    let mut service = ListService::open(&cfg, None).unwrap();
    let preview = service.preview_delete("synthetic").unwrap();
    set_flag(d.path(), "hang_after_delete_once");
    assert!(service.delete_list(&preview).is_err());
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let id = fs::read_to_string(state.join("lists/active")).unwrap();
    fs::write(
        cfg.vdir.join("synthetic/local.ics"),
        b"edited while uncertain",
    )
    .unwrap();
    let ListResult::Deleted { retired, .. } = service.reconcile(id.trim()).unwrap() else {
        panic!("expected a deletion")
    };
    assert_eq!(deletes(d.path()), 1);
    assert!(!cfg.vdir.join("synthetic").exists());
    no_active(&cfg);
    let kept = twydo_core::retired::data_dir(&cfg, &retired).unwrap();
    assert_eq!(
        fs::read(kept.join("synthetic/local.ics")).unwrap(),
        b"edited while uncertain"
    );
}

#[cfg(unix)]
#[test]
fn uncertain_delete_with_local_edit_and_list_still_present_releases_the_fence() {
    let (d, cfg) = offline();
    let mut service = ListService::open(&cfg, None).unwrap();
    let preview = service.preview_delete("synthetic").unwrap();
    set_flag(d.path(), "fail_delete_once");
    assert!(service.delete_list(&preview).is_err());
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let id = fs::read_to_string(state.join("lists/active")).unwrap();
    fs::write(
        cfg.vdir.join("synthetic/local.ics"),
        b"edited while uncertain",
    )
    .unwrap();
    let message = service.reconcile(id.trim()).unwrap_err().to_string();
    assert!(message.contains("still exists"), "{message}");
    no_active(&cfg);
    assert_eq!(
        fs::read(cfg.vdir.join("synthetic/local.ics")).unwrap(),
        b"edited while uncertain"
    );
}
