use std::fs;
use twydo_core::{
    config::{Config, SyncConfig, WriteMode},
    lists::{ListService, backup},
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
    assert!(
        service
            .delete_list(&preview)
            .unwrap_err()
            .to_string()
            .contains("server-enforced")
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
