mod common;

#[cfg(unix)]
use common::lists::{flag, offline};
use std::fs;
use twydo_core::config::{Config, SyncConfig};
use twydo_core::lists::{ListRequest, ListService};

/// The embedded matrix with the user's best-effort acceptance removed.
fn without_acceptance() -> twydo_core::compat::Matrix {
    let mut matrix = twydo_core::compat::Matrix::embedded().clone();
    for row in &mut matrix.contract {
        row.accepted_risk = None;
    }
    matrix
}

fn setup() -> (tempfile::TempDir, Config) {
    let d = tempfile::tempdir().unwrap();
    let mut cfg = Config::with_vdir(d.path().join("vdir"));
    cfg.state_dir = Some(d.path().join("state"));
    cfg.sync = Some(SyncConfig::default());
    fs::create_dir_all(cfg.vdir.join("synthetic")).unwrap();
    fs::write(cfg.vdir.join("synthetic/displayname"), "Before").unwrap();
    fs::write(cfg.vdir.join("synthetic/color"), "#3289D2").unwrap();
    (d, cfg)
}

#[test]
fn shipping_gate_admits_verified_create_and_refuses_unverified_edit_before_preparing() {
    let (_d, cfg) = setup();
    let mut service = ListService::open_with(&cfg, None, without_acceptance()).unwrap();
    let expected = service.state("synthetic").unwrap();
    // Creation passed both device directions; it fails later here only because this
    // fixture has no usable helper or vdirsyncer pair.
    let create = service
        .execute(ListRequest::Create {
            id: "new-id".into(),
            name: "New".into(),
            color: (55, 148, 23),
        })
        .unwrap_err()
        .to_string();
    assert!(!create.contains("not verified"), "{create}");
    let edit = service
        .execute(ListRequest::Edit {
            id: "synthetic".into(),
            name: "After".into(),
            color: Some((50, 137, 210)),
            expected,
        })
        .unwrap_err()
        .to_string();
    assert!(
        edit.contains("list-metadata.collection: atomicity"),
        "{edit}"
    );
    assert!(!cfg.vdir.join("new-id").exists());
    assert_eq!(
        fs::read_to_string(cfg.vdir.join("synthetic/displayname")).unwrap(),
        "Before"
    );
}

#[cfg(unix)]
#[test]
fn shipping_metadata_gate_requires_server_protection_even_in_test_mode() {
    use twydo_core::config::WriteMode;
    for mode in [WriteMode::Test, WriteMode::Verified] {
        let (d, mut cfg) = offline();
        ListService::open(&cfg, None)
            .unwrap()
            .execute(create())
            .unwrap();
        cfg.writes.mode = mode;
        let before = fs::read(d.path().join("server.json")).unwrap();
        let mut service = ListService::open_with(&cfg, None, without_acceptance()).unwrap();
        let expected = service.state("new-id").unwrap();
        let error = service
            .execute(ListRequest::Edit {
                id: "new-id".into(),
                name: "Desktop rename".into(),
                color: expected.color,
                expected,
            })
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("list-metadata.collection: atomicity")
                && error.contains("concurrent remote edits"),
            "{error}"
        );
        assert_eq!(fs::read(d.path().join("server.json")).unwrap(), before);
        assert!(
            twydo_core::lists::operation::active(&cfg)
                .unwrap()
                .is_none()
        );
        assert_eq!(service.state("new-id").unwrap().name, "Synthetic list");
    }
}

#[cfg(unix)]
#[test]
fn metadata_interoperability_cannot_bypass_missing_or_failed_atomicity() {
    use twydo_core::compat::Verdict;
    use twydo_core::config::WriteMode;
    for mode in [WriteMode::Test, WriteMode::Verified] {
        for atomicity in [
            None,
            Some(Verdict::Unverified),
            Some(Verdict::Fail),
            Some(Verdict::Unsupported),
        ] {
            let (d, mut cfg) = offline();
            ListService::open(&cfg, None)
                .unwrap()
                .execute(create())
                .unwrap();
            cfg.writes.mode = mode;
            let mut matrix = without_acceptance();
            let row = matrix
                .contract
                .iter_mut()
                .find(|r| r.id == twydo_core::compat::ContractId::ListMetadata)
                .unwrap();
            row.iphone_baseline = Verdict::Pass;
            row.twydo_to_iphone = Verdict::Pass;
            row.iphone_to_twydo = Verdict::Pass;
            row.atomicity = Some(twydo_core::compat::Verdict::Pass);
            row.atomicity = atomicity;
            let before = fs::read(d.path().join("server.json")).unwrap();
            let mut service = ListService::open_with(&cfg, None, matrix).unwrap();
            let expected = service.state("new-id").unwrap();
            let error = service
                .execute(ListRequest::Edit {
                    id: "new-id".into(),
                    name: "Unprotected rename".into(),
                    color: expected.color,
                    expected,
                })
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("list-metadata.collection: atomicity")
                    && error.contains("concurrent remote edits"),
                "{mode:?} {atomicity:?}: {error}"
            );
            assert_eq!(fs::read(d.path().join("server.json")).unwrap(), before);
            assert!(
                twydo_core::lists::operation::active(&cfg)
                    .unwrap()
                    .is_none()
            );
        }
    }
}

#[test]
fn invalid_or_blank_list_input_never_starts_a_remote_operation() {
    let (_d, cfg) = setup();
    let mut service = ListService::open(&cfg, None).unwrap();
    for (id, name) in [("../other", "Name"), ("safe", "  "), ("safe", "bad\nname")] {
        let message = service
            .execute(ListRequest::Create {
                id: id.into(),
                name: name.into(),
                color: (50, 137, 210),
            })
            .unwrap_err()
            .to_string();
        assert!(
            !message.contains("not verified"),
            "input must be checked before capability: {message}"
        );
    }
    assert!(!cfg.state_dir().exists());
}

#[cfg(unix)]
fn prototype_metadata_service(cfg: &Config) -> ListService {
    // Exercise the prototype and reconciliation only against the offline transport.
    // This synthetic matrix is never loaded by the application or a device round.
    let mut matrix = twydo_core::compat::Matrix::embedded().clone();
    let row = matrix
        .contract
        .iter_mut()
        .find(|r| r.id == twydo_core::compat::ContractId::ListMetadata)
        .unwrap();
    row.iphone_baseline = twydo_core::compat::Verdict::Pass;
    row.twydo_to_iphone = twydo_core::compat::Verdict::Pass;
    row.iphone_to_twydo = twydo_core::compat::Verdict::Pass;
    row.atomicity = Some(twydo_core::compat::Verdict::Pass);
    ListService::open_with(cfg, None, matrix).unwrap()
}

#[cfg(unix)]
fn create() -> ListRequest {
    ListRequest::Create {
        id: "new-id".into(),
        name: "Synthetic list".into(),
        color: (50, 137, 210),
    }
}

#[cfg(unix)]
#[test]
fn actual_service_and_bundled_helper_create_edit_and_confirm_local_discovery() {
    let (_d, cfg) = offline();
    let mut service = prototype_metadata_service(&cfg);
    service.execute(create()).unwrap();
    assert_eq!(service.state("new-id").unwrap().name, "Synthetic list");
    fs::write(cfg.vdir.join("new-id/held.ics"), "held task bytes").unwrap();
    let expected = service.state("new-id").unwrap();
    service
        .execute(ListRequest::Edit {
            id: "new-id".into(),
            name: "Renamed".into(),
            color: expected.color,
            expected,
        })
        .unwrap();
    assert_eq!(service.state("new-id").unwrap().name, "Renamed");
    assert_eq!(
        fs::read_to_string(cfg.vdir.join("new-id/held.ics")).unwrap(),
        "held task bytes"
    );
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    assert!(!state.join("lists/active").exists());
}

#[cfg(unix)]
#[test]
fn failed_local_discovery_reconciles_without_recreating_a_remote_list() {
    let (d, cfg) = offline();
    flag(d.path(), "fail_refresh_once");
    let mut service = ListService::open(&cfg, None).unwrap();
    assert!(service.execute(create()).is_err());
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let id = fs::read_to_string(state.join("lists/active")).unwrap();
    service.reconcile(id.trim()).unwrap();
    assert!(!state.join("lists/active").exists());
    let server: serde_json::Value =
        serde_json::from_slice(&fs::read(d.path().join("server.json")).unwrap()).unwrap();
    assert_eq!(
        server["calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| **v == "MKCALENDAR")
            .count(),
        1
    );
}

#[cfg(unix)]
#[test]
fn failed_edit_can_adopt_server_values_without_reissuing_the_edit() {
    let (d, cfg) = offline();
    let mut service = prototype_metadata_service(&cfg);
    service.execute(create()).unwrap();
    flag(d.path(), "ignore_write");
    let expected = service.state("new-id").unwrap();
    assert!(
        service
            .execute(ListRequest::Edit {
                id: "new-id".into(),
                name: "Unapplied rename".into(),
                color: expected.color,
                expected
            })
            .is_err()
    );
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let id = fs::read_to_string(state.join("lists/active")).unwrap();
    assert!(service.reconcile(id.trim()).is_err());
    service.keep_server(id.trim()).unwrap();
    assert_eq!(service.state("new-id").unwrap().name, "Synthetic list");
    let server: serde_json::Value =
        serde_json::from_slice(&fs::read(d.path().join("server.json")).unwrap()).unwrap();
    assert_eq!(
        server["calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| **v == "PROPPATCH")
            .count(),
        1
    );
    assert!(!state.join("lists/active").exists());
}

#[cfg(unix)]
#[test]
fn timeout_after_remote_creation_is_reconciled_without_another_create() {
    let (d, mut cfg) = offline();
    cfg.sync.as_mut().unwrap().timeout_secs = 1;
    flag(d.path(), "hang_after_create_once");
    let mut service = ListService::open(&cfg, None).unwrap();
    assert!(service.execute(create()).is_err());
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let id = fs::read_to_string(state.join("lists/active")).unwrap();
    let record = twydo_core::lists::operation::read(&state, id.trim()).unwrap();
    assert!(matches!(
        record.phase,
        twydo_core::lists::operation::Phase::RemoteStarted
            | twydo_core::lists::operation::Phase::Uncertain
    ));
    service.reconcile(id.trim()).unwrap();
    let server: serde_json::Value =
        serde_json::from_slice(&fs::read(d.path().join("server.json")).unwrap()).unwrap();
    assert_eq!(
        server["calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| **v == "MKCALENDAR")
            .count(),
        1
    );
    assert!(!state.join("lists/active").exists());
}

#[cfg(unix)]
#[test]
fn interrupted_metadata_temps_reconcile_without_recreating_the_collection() {
    use twydo_core::lists::operation::{self, Phase};
    for (key, partial) in [
        ("displayname", ""),
        ("displayname", "Synt"),
        ("color", "#3289D2"),
    ] {
        let (d, cfg) = offline();
        flag(d.path(), "fail_refresh_once");
        let mut service = ListService::open(&cfg, None).unwrap();
        assert!(service.execute(create()).is_err());
        let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
        let mut record = operation::active(&cfg).unwrap().unwrap();
        record.phase = Phase::RemoteConfirmed;
        operation::save(&state, &record).unwrap();
        let dir = cfg.vdir.join("new-id");
        fs::remove_file(dir.join(key)).unwrap();
        if key == "displayname" {
            // displayname is written first; color does not exist at this crash point.
            fs::remove_file(dir.join("color")).unwrap();
        }
        // A similar filename alone is insufficient; invalid scratch bytes must
        // still be refused by the real helper before it reads the remote target.
        let temp = dir.join(format!(".{key}.tdo-tmp"));
        fs::write(&temp, "unrelated bytes").unwrap();
        let server_before = fs::read(d.path().join("server.json")).unwrap();
        assert!(service.reconcile(&record.id).is_err());
        assert_eq!(
            fs::read(d.path().join("server.json")).unwrap(),
            server_before
        );
        assert_eq!(fs::read_to_string(&temp).unwrap(), "unrelated bytes");
        fs::write(dir.join(format!(".{key}.tdo-tmp")), partial).unwrap();
        service.reconcile(&record.id).unwrap();
        assert!(operation::active(&cfg).unwrap().is_none());
        assert_eq!(service.state("new-id").unwrap().name, "Synthetic list");
        assert_eq!(service.state("new-id").unwrap().color, Some((50, 137, 210)));
        assert!(!dir.join(format!(".{key}.tdo-tmp")).exists());
        let server: serde_json::Value =
            serde_json::from_slice(&fs::read(d.path().join("server.json")).unwrap()).unwrap();
        assert_eq!(
            server["calls"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|v| **v == "MKCALENDAR")
                .count(),
            1
        );
    }
}

#[cfg(unix)]
#[test]
fn helper_has_its_own_deadline_when_its_monitoring_parent_exits() {
    use std::process::Command;
    use std::time::{Duration, Instant};
    use twydo_core::lists::{helper::HelperRequest, operation};
    let (d, mut cfg) = offline();
    cfg.sync.as_mut().unwrap().timeout_secs = 1;
    flag(d.path(), "hang_after_create_once");
    let server_path = d.path().join("server.json");
    let mut server: serde_json::Value =
        serde_json::from_slice(&fs::read(&server_path).unwrap()).unwrap();
    server["flags"]["hang_seconds"] = 3.into();
    fs::write(&server_path, server.to_string()).unwrap();
    let mut request = HelperRequest::probe(&cfg).unwrap();
    request.action = "create".into();
    request.collection = Some("new-id".into());
    request.name = Some("Synthetic list".into());
    request.color = Some("#3289D2".into());
    let record = operation::prepare(&cfg, request, chrono::Utc::now()).unwrap();
    let arguments = serde_json::to_string(&vec![
        cfg.sync.as_ref().unwrap().list_python.clone(),
        "-I".into(),
        "-c".into(),
        include_str!("../src/lists/helper.py").into(),
    ])
    .unwrap();
    let payload = serde_json::to_string(&serde_json::to_string(&record.request).unwrap()).unwrap();
    let launcher = format!(
        "import subprocess\np=subprocess.Popen({arguments},stdin=subprocess.PIPE,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,start_new_session=True)\np.stdin.write({payload}.encode())\np.stdin.close()\n"
    );
    assert!(
        Command::new("python3")
            .args(["-I", "-c", &launcher])
            .status()
            .unwrap()
            .success()
    );
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let deadline = Instant::now() + Duration::from_millis(2200);
    let mut stopped_on_time = false;
    while Instant::now() < deadline {
        let current = operation::read(&state, &record.id).unwrap();
        if current.phase == operation::Phase::Uncertain
            && twydo_core::store::fsio::VdirLock::try_exclusive(&state)
                .unwrap()
                .is_some()
        {
            stopped_on_time = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // A broken deadline still gets finite cleanup from the synthetic transport's 3s hang.
    let cleanup = Instant::now() + Duration::from_secs(3);
    while twydo_core::store::fsio::VdirLock::try_exclusive(&state)
        .unwrap()
        .is_none()
        && Instant::now() < cleanup
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        stopped_on_time,
        "orphan helper did not enforce its own configured deadline"
    );
}

#[cfg(unix)]
#[test]
fn reconciliation_refuses_changed_connection_before_reading_the_old_target() {
    let (d, cfg) = offline();
    flag(d.path(), "fail_refresh_once");
    let mut service = ListService::open(&cfg, None).unwrap();
    assert!(service.execute(create()).is_err());
    let before = fs::read(d.path().join("server.json")).unwrap();
    let path = d.path().join("connection.json");
    let mut config: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    config["remote_url"] = "https://example.com/another-account/".into();
    fs::write(path, config.to_string()).unwrap();
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let id = fs::read_to_string(state.join("lists/active")).unwrap();
    assert!(service.reconcile(id.trim()).is_err());
    assert_eq!(fs::read(d.path().join("server.json")).unwrap(), before);
    assert!(state.join("lists/active").exists());
}

#[cfg(unix)]
#[test]
fn changed_saved_write_policy_refuses_a_worker_using_old_test_mode() {
    let (d, cfg) = offline();
    let path = d.path().join("config.toml");
    let sync = cfg.sync.as_ref().unwrap();
    fs::write(&path, format!("vdir = {:?}\nstate_dir = {:?}\n[writes]\nmode = \"verified\"\n[sync]\npair = \"synthetic\"\nlocal_storage = \"local\"\nlist_python = {:?}\nvdirsyncer_config = {:?}\n", cfg.vdir.to_str().unwrap(), cfg.state_dir().to_str().unwrap(),sync.list_python,sync.vdirsyncer_config.as_ref().unwrap().to_str().unwrap())).unwrap();
    let mut service = ListService::open(&cfg, Some(&path)).unwrap();
    assert!(service.execute(create()).is_err());
    assert!(!d.path().join("server.json").exists());
}

#[cfg(unix)]
#[test]
fn changed_saved_state_binding_refuses_create_preview_and_reconciliation() {
    for action in ["create", "preview", "reconcile"] {
        let (d, cfg) = offline();
        ListService::open(&cfg, None)
            .unwrap()
            .execute(create())
            .unwrap();
        let operation = if action == "reconcile" {
            let mut request = twydo_core::lists::helper::HelperRequest::probe(&cfg).unwrap();
            request.action = "edit".into();
            request.collection = Some("new-id".into());
            request.name = Some("After".into());
            request.expected = Some(serde_json::json!({"name":"Synthetic list","color":"#379417"}));
            Some(
                twydo_core::lists::operation::prepare(&cfg, request, chrono::Utc::now())
                    .unwrap()
                    .id,
            )
        } else {
            None
        };
        let path = d.path().join("config.toml");
        let sync = cfg.sync.as_ref().unwrap();
        fs::write(&path,format!("vdir = {:?}\nstate_dir = {:?}\n[writes]\nmode = \"test\"\n[sync]\npair = \"synthetic\"\nlocal_storage = \"local\"\nlist_python = {:?}\nvdirsyncer_config = {:?}\n",cfg.vdir.to_str().unwrap(),d.path().join("other-state").to_str().unwrap(),sync.list_python,sync.vdirsyncer_config.as_ref().unwrap().to_str().unwrap())).unwrap();
        let before = fs::read(d.path().join("server.json")).unwrap();
        let mut service = ListService::open(&cfg, Some(&path)).unwrap();
        let message = match action {
            "create" => service
                .execute(ListRequest::Create {
                    id: "second-id".into(),
                    name: "Second".into(),
                    color: (1, 2, 3),
                })
                .unwrap_err()
                .to_string(),
            "preview" => service.preview_delete("new-id").unwrap_err().to_string(),
            _ => service
                .reconcile(operation.as_deref().unwrap())
                .unwrap_err()
                .to_string(),
        };
        assert!(
            message.contains("configuration or write policy changed"),
            "{action}: {message}"
        );
        assert_eq!(fs::read(d.path().join("server.json")).unwrap(), before);
        assert!(!d.path().join("other-state").exists());
        if action == "reconcile" {
            assert!(
                twydo_core::lists::operation::active(&cfg)
                    .unwrap()
                    .is_some()
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn explicit_collection_save_respects_but_does_not_depend_on_automatic_sync_switch() {
    let (_d, cfg) = offline();
    twydo_core::sync::switch::set(&cfg, true).unwrap();
    ListService::open(&cfg, None)
        .unwrap()
        .execute(create())
        .unwrap();
    assert!(twydo_core::sync::switch::is_off(&cfg).unwrap());
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    assert!(!state.join("sync/requested").exists());
    assert!(
        twydo_core::store::journal::Journal::open(&state)
            .unwrap()
            .pending()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn missing_or_failed_capabilities_refuse_test_mode_too() {
    let (_d, mut cfg) = setup();
    cfg.writes.mode = twydo_core::config::WriteMode::Test;
    let mut missing = twydo_core::compat::Matrix::embedded().clone();
    missing
        .contract
        .retain(|r| r.id != twydo_core::compat::ContractId::ListCreate);
    assert!(
        ListService::open_with(&cfg, None, missing)
            .unwrap()
            .execute(ListRequest::Create {
                id: "new".into(),
                name: "New".into(),
                color: (1, 2, 3)
            })
            .is_err()
    );
    let mut failed = twydo_core::compat::Matrix::embedded().clone();
    failed
        .contract
        .iter_mut()
        .find(|r| r.id == twydo_core::compat::ContractId::ListCreate)
        .unwrap()
        .twydo_to_iphone = twydo_core::compat::Verdict::Fail;
    assert!(
        ListService::open_with(&cfg, None, failed)
            .unwrap()
            .execute(ListRequest::Create {
                id: "new".into(),
                name: "New".into(),
                color: (1, 2, 3)
            })
            .is_err()
    );
    assert!(!cfg.state_dir().exists());
}

#[test]
fn missing_pair_never_prepares_a_remote_change() {
    let (_d, mut cfg) = setup();
    cfg.sync = None;
    let mut service = ListService::open(&cfg, None).unwrap();
    let message = service
        .execute(ListRequest::Create {
            id: "new".into(),
            name: "New".into(),
            color: (1, 2, 3),
        })
        .unwrap_err()
        .to_string();
    assert!(message.contains("pair"));
    assert!(!cfg.state_dir().exists());
}

#[cfg(unix)]
#[test]
fn refusal_names_the_failing_direction() {
    use twydo_core::compat::Verdict;
    use twydo_core::config::WriteMode;
    let (_d, mut cfg) = offline();
    ListService::open(&cfg, None)
        .unwrap()
        .execute(create())
        .unwrap();
    cfg.writes.mode = WriteMode::Verified;
    let mut matrix = without_acceptance();
    let row = matrix
        .contract
        .iter_mut()
        .find(|r| r.id == twydo_core::compat::ContractId::ListMetadata)
        .unwrap();
    row.iphone_baseline = Verdict::Pass;
    row.iphone_to_twydo = Verdict::Pass;
    row.twydo_to_iphone = Verdict::Unverified;
    row.atomicity = Some(Verdict::Pass);
    let mut service = ListService::open_with(&cfg, None, matrix).unwrap();
    let expected = service.state("new-id").unwrap();
    let error = service
        .execute(ListRequest::Edit {
            id: "new-id".into(),
            name: "Rename".into(),
            color: expected.color,
            expected,
        })
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("list-metadata.collection: twydo_to_iphone"),
        "{error}"
    );
    assert!(!error.contains("concurrent"), "{error}");
}
