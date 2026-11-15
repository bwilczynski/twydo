//! The in-process list engine against the fake server. Names and addresses are synthetic.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tempfile::TempDir;
use twydo_core::config::{Config, SyncConfig};
use twydo_core::lists::call::RemoteRequest;
use twydo_core::lists::engine::run;
use twydo_core::lists::operation::{self, ListOperation, Phase, snapshot};
use twydo_core::store::bind_vdir_state;
use twydo_testdav::{
    FakeDav, add_collection, calls, collections, fake_vdirsyncer, flag, flag_value,
};

const ID: &str = "op-1";

struct Env {
    server_root: TempDir,
    server: FakeDav,
    _work: TempDir,
    cfg: Config,
    state: PathBuf,
    vdir: PathBuf,
}

fn env_with(password_command: &[&str]) -> Env {
    let server_root = tempfile::tempdir().unwrap();
    let server = FakeDav::start(server_root.path());
    let work = tempfile::tempdir().unwrap();
    let vdir = fs::canonicalize(work.path()).unwrap().join("vdir");
    fs::create_dir_all(&vdir).unwrap();
    let mut cfg = Config::with_vdir(vdir.clone());
    cfg.state_dir = Some(work.path().join("state"));
    cfg.sync = Some(SyncConfig {
        url: server.url(),
        username: "alice".into(),
        password_command: password_command.iter().map(|s| s.to_string()).collect(),
        command: fake_vdirsyncer(server_root.path())
            .to_string_lossy()
            .into_owned(),
        timeout_secs: 30,
        ..SyncConfig::default()
    });
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    Env {
        server_root,
        server,
        _work: work,
        cfg,
        state,
        vdir,
    }
}

fn env() -> Env {
    env_with(&["printf", "secret"])
}

fn root(e: &Env) -> &Path {
    e.server_root.path()
}

fn count(e: &Env, method: &str) -> usize {
    calls(root(e)).iter().filter(|c| *c == method).count()
}

/// A local list folder holding a name and a colour.
fn local_list(e: &Env, id: &str, name: &str, color: &str) {
    let dir = e.vdir.join(id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("displayname"), name).unwrap();
    fs::write(dir.join("color"), color).unwrap();
}

fn request(e: &Env, action: &str, collection: &str) -> RemoteRequest {
    let mut request = RemoteRequest::new(&e.cfg, action).unwrap();
    request.collection = Some(collection.into());
    request.operation = Some(e.state.join("lists/operations").join(format!("{ID}.json")));
    request.state = Some(e.state.clone());
    request.vdir = e.vdir.clone();
    match action {
        "create" => {
            request.name = Some("Fresh".into());
            request.color = Some("#aabbcc".into());
        }
        "edit" => {
            request.name = Some("Renamed".into());
            request.expected = Some(json!({"name": "Work", "color": "#112233"}));
        }
        "delete" => request.backup = Some(ID.into()),
        _ => {}
    }
    request
}

/// Writes the record and the files `operation::prepare` leaves, in the shape the current
/// version writes them (a helper-shaped request with the fields the engine ignores).
fn prepare(e: &Env, request: &RemoteRequest, phase: &str, extra: Value) {
    fs::create_dir_all(e.state.join("lists/operations")).unwrap();
    fs::create_dir_all(e.state.join("sync")).unwrap();
    let mut stored = serde_json::to_value(request).unwrap();
    stored["version"] = json!(1);
    stored["pair"] = json!("twydo");
    stored["local_storage"] = json!("twydo_local");
    stored["config"] = json!(e.state.join("vdirsyncer/config"));
    let holds = snapshot(&e.vdir.join(request.collection.as_deref().unwrap())).unwrap();
    let mut record = json!({
        "format": 1,
        "id": ID,
        "created": "2026-11-15T00:00:00Z",
        "vdir": e.vdir,
        "phase": phase,
        "request": stored,
        "holds": holds,
    });
    for (key, value) in extra.as_object().unwrap() {
        record[key] = value.clone();
    }
    fs::write(
        e.state.join("lists/operations").join(format!("{ID}.json")),
        serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();
    fs::write(e.state.join("lists/active"), ID).unwrap();
}

fn record(e: &Env) -> ListOperation {
    operation::read(&e.state, ID).unwrap()
}

#[test]
fn create_walks_prepared_to_remote_confirmed() {
    let e = env();
    let request = request(&e, "create", "fresh");
    prepare(&e, &request, "prepared", json!({}));
    let result = run(&e.cfg, &request).unwrap();
    assert_eq!(result["collection"], "fresh");
    assert_eq!(result["remote_collection"], "fresh");
    assert_eq!(result["name"], "Fresh");
    assert_eq!(result["color"], "#aabbcc");
    let saved = record(&e);
    assert_eq!(saved.phase, Phase::RemoteConfirmed);
    assert_eq!(saved.receipt.as_ref(), Some(&result));
    assert!(saved.remote_target.as_deref().unwrap().ends_with("/fresh/"));
    assert_eq!(saved.connection.as_deref().map(str::len), Some(64));
    assert_eq!(count(&e, "MKCALENDAR"), 1);
    assert_eq!(collections(root(&e))["fresh"].0.as_deref(), Some("Fresh"));
}

#[test]
fn refresh_confirms_locally_after_discover() {
    let e = env();
    add_collection(root(&e), "work", "Work", "#112233");
    local_list(&e, "work", "Renamed", "#112233");
    let after = snapshot(&e.vdir.join("work")).unwrap();
    let mut request = request(&e, "edit", "work");
    request.mode = "refresh".into();
    prepare(
        &e,
        &request,
        "remote-confirmed",
        json!({"local_after": after, "local_base": after}),
    );
    let result = run(&e.cfg, &request).unwrap();
    assert_eq!(result, json!({"refreshed": true}));
    assert_eq!(record(&e).phase, Phase::LocalConfirmed);
    assert_eq!(count(&e, "DISCOVER"), 1);
}

#[test]
fn a_stale_edit_records_no_remote_write() {
    let e = env();
    add_collection(root(&e), "work", "Work", "#112233");
    local_list(&e, "work", "Work", "#112233");
    let mut request = request(&e, "edit", "work");
    request.expected = Some(json!({"name": "Not the name", "color": "#112233"}));
    prepare(&e, &request, "prepared", json!({}));
    let refused = run(&e.cfg, &request).unwrap_err();
    assert_eq!(refused.code, "stale");
    assert!(!refused.sent);
    let saved = record(&e);
    assert_eq!(saved.phase, Phase::Uncertain);
    assert_eq!(
        saved.receipt,
        Some(json!({"no_remote_write": true, "reason": "stale"}))
    );
    assert_eq!(count(&e, "PROPPATCH"), 0);
}

#[test]
fn a_wrong_phase_is_refused_before_any_request() {
    let e = env();
    add_collection(root(&e), "work", "Work", "#112233");
    local_list(&e, "work", "Work", "#112233");
    let request = request(&e, "edit", "work");
    prepare(&e, &request, "remote-confirmed", json!({}));
    let refused = run(&e.cfg, &request).unwrap_err();
    assert_eq!(refused.code, "operation");
    assert!(calls(root(&e)).is_empty());
    assert_eq!(record(&e).phase, Phase::RemoteConfirmed);
}

#[test]
fn a_changed_local_collection_is_refused_before_any_request() {
    let e = env();
    add_collection(root(&e), "work", "Work", "#112233");
    local_list(&e, "work", "Work", "#112233");
    let request = request(&e, "edit", "work");
    prepare(&e, &request, "prepared", json!({}));
    fs::write(e.vdir.join("work/displayname"), "Edited meanwhile").unwrap();
    let refused = run(&e.cfg, &request).unwrap_err();
    assert_eq!(refused.code, "operation");
    assert!(calls(root(&e)).is_empty());
    assert_eq!(record(&e).phase, Phase::Prepared);
}

#[test]
fn a_failing_password_command_records_no_remote_write() {
    let e = env_with(&["false"]);
    let request = request(&e, "create", "fresh");
    prepare(&e, &request, "prepared", json!({}));
    let refused = run(&e.cfg, &request).unwrap_err();
    assert_eq!(refused.code, "credentials");
    assert!(!refused.sent);
    let saved = record(&e);
    assert_eq!(saved.phase, Phase::Uncertain);
    assert_eq!(
        saved.receipt,
        Some(json!({"no_remote_write": true, "reason": "credentials"}))
    );
    assert!(calls(root(&e)).is_empty());
}

#[test]
fn a_hung_server_times_out_as_uncertain() {
    let e = env();
    flag(root(&e), "hang_after_create_once");
    flag_value(root(&e), "hang_seconds", "4");
    let mut request = request(&e, "create", "fresh");
    request.timeout_secs = 1;
    prepare(&e, &request, "prepared", json!({}));
    let refused = run(&e.cfg, &request).unwrap_err();
    assert_eq!(refused.code, "timeout");
    let saved = record(&e);
    assert_eq!(saved.phase, Phase::Uncertain);
    assert!(saved.remote_target.is_some());
    assert_eq!(saved.receipt, None);
}

#[test]
fn reconcile_reports_differing_server_values_as_choice() {
    let e = env();
    add_collection(root(&e), "work", "Server name", "#112233");
    local_list(&e, "work", "Work", "#112233");
    let mut request = request(&e, "edit", "work");
    request.mode = "reconcile".into();
    prepare(
        &e,
        &request,
        "uncertain",
        json!({"remote_target": format!("{}work/", e.server.url())}),
    );
    let refused = run(&e.cfg, &request).unwrap_err();
    assert_eq!(refused.code, "choice");
    let saved = record(&e);
    assert_eq!(saved.phase, Phase::Uncertain);
    assert_eq!(
        operation::observed(&saved),
        Some((Some("Server name".into()), Some("#112233".into())))
    );
    // Keeping the server's values confirms what is there instead.
    request.keep_server = true;
    let result = run(&e.cfg, &request).unwrap();
    assert_eq!(result["name"], "Server name");
    assert_eq!(record(&e).phase, Phase::RemoteConfirmed);
    assert_eq!(count(&e, "PROPPATCH"), 0);
}

#[test]
fn a_changed_connection_is_a_mapping_refusal() {
    let e = env();
    add_collection(root(&e), "work", "Work", "#112233");
    local_list(&e, "work", "Work", "#112233");
    let mut request = request(&e, "edit", "work");
    request.mode = "reconcile".into();
    prepare(
        &e,
        &request,
        "uncertain",
        json!({
            "remote_target": format!("{}work/", e.server.url()),
            "connection": "0".repeat(64),
        }),
    );
    let refused = run(&e.cfg, &request).unwrap_err();
    assert_eq!(refused.code, "mapping");
    assert!(calls(root(&e)).is_empty());
}

#[test]
fn a_record_from_the_current_version_still_reconciles() {
    let e = env();
    add_collection(root(&e), "work", "Renamed", "#112233");
    local_list(&e, "work", "Work", "#112233");
    let operation = e.state.join("lists/operations").join(format!("{ID}.json"));
    fs::create_dir_all(operation.parent().unwrap()).unwrap();
    fs::create_dir_all(e.state.join("sync")).unwrap();
    let stored = json!({
        "version": 1,
        "action": "edit",
        "pair": "twydo",
        "local_storage": "twydo_local",
        "vdir": e.vdir,
        "config": e.state.join("vdirsyncer/config"),
        "collection": "work",
        "operation": operation,
        "state": e.state,
        "name": "Renamed",
        "color": null,
        "expected": {"name": "Work", "color": "#112233"},
        "backup": null,
        "mode": "execute",
        "keep_server": false,
        "timeout_secs": 300
    });
    let fixture = format!(
        r#"{{
  "format": 1, "id": "{ID}", "created": "2026-11-12T10:00:00Z", "vdir": {vdir},
  "phase": "remote-started", "request": {stored},
  "holds": {{"displayname": [87, 111, 114, 107], "color": [35, 49, 49, 50, 50, 51, 51]}},
  "receipt": null, "remote_target": "{target}work/", "connection_digest": "abc",
  "local_base": null, "local_after": null, "retired": null
}}"#,
        vdir = json!(e.vdir),
        target = e.server.url()
    );
    fs::write(&operation, fixture).unwrap();
    fs::write(e.state.join("lists/active"), ID).unwrap();
    let mut request: RemoteRequest = serde_json::from_value(stored).unwrap();
    request.mode = "reconcile".into();
    let result = run(&e.cfg, &request).unwrap();
    assert_eq!(result["name"], "Renamed");
    assert_eq!(record(&e).phase, Phase::RemoteConfirmed);
}
