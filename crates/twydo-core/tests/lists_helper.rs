use std::fs;
use twydo_core::config::{Config, SyncConfig};
use twydo_core::lists::helper::{HelperRequest, invoke};

fn config(root: &std::path::Path) -> Config {
    let mut cfg = Config::with_vdir(root.to_path_buf());
    cfg.state_dir = Some(root.join("state"));
    cfg.sync = Some(SyncConfig::default());
    cfg
}

#[test]
fn bundled_helper_runs_outside_the_repository_and_validates_protocol() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let mut request = HelperRequest::probe(&cfg).unwrap();
    request.version = 77;
    let message = invoke(&cfg, &request).unwrap_err().to_string();
    assert!(message.contains("protocol"), "{message}");
}

#[cfg(unix)]
fn executable(root: &std::path::Path, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join("python-wrapper");
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path.to_str().unwrap().to_string()
}

#[cfg(unix)]
#[test]
fn invalid_helper_output_does_not_leak_raw_output_or_stderr() {
    let d = tempfile::tempdir().unwrap();
    let mut cfg = config(d.path());
    cfg.sync.as_mut().unwrap().list_python = executable(
        d.path(),
        "cat >/dev/null\necho private-account-secret\necho private-secret >&2",
    );
    let message = invoke(&cfg, &HelperRequest::probe(&cfg).unwrap())
        .unwrap_err()
        .to_string();
    assert!(!message.contains("private"), "{message}");
    assert!(message.contains("helper"), "{message}");
}

#[cfg(unix)]
#[test]
fn hung_helper_is_killed_with_a_bounded_timeout() {
    let d = tempfile::tempdir().unwrap();
    let mut cfg = config(d.path());
    let sync = cfg.sync.as_mut().unwrap();
    sync.list_python = executable(d.path(), "cat >/dev/null\nsleep 60");
    sync.timeout_secs = 1;
    let start = std::time::Instant::now();
    let result = invoke(&cfg, &HelperRequest::probe(&cfg).unwrap())
        .unwrap_err()
        .to_string();
    assert!(result.contains("timed out"), "{result}");
    assert!(start.elapsed().as_secs() < 5);
}

#[cfg(unix)]
#[test]
fn helper_preserves_caller_directory_for_relative_vdirsyncer_settings() {
    let d = tempfile::tempdir().unwrap();
    let mut cfg = config(d.path());
    cfg.sync.as_mut().unwrap().list_python = executable(
        d.path(),
        r#"cat >/dev/null
python3 -c 'import os,json; print(json.dumps({"version":1,"ok":True,"result":{"cwd":os.getcwd()}}))'"#,
    );
    let result = invoke(&cfg, &HelperRequest::probe(&cfg).unwrap()).unwrap();
    assert_eq!(
        result.result["cwd"].as_str(),
        std::env::current_dir().unwrap().to_str()
    );
}
