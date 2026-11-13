//! Offline list-helper harness shared by the list tests.
#![allow(dead_code)]

use std::fs;

use twydo_core::config::{Config, SyncConfig};

#[cfg(unix)]
pub fn offline() -> (tempfile::TempDir, Config) {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let mut cfg = Config::with_vdir(d.path().join("vdir"));
    fs::create_dir(&cfg.vdir).unwrap();
    cfg.state_dir = Some(d.path().join("state"));
    cfg.writes.mode = twydo_core::config::WriteMode::Test;
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
            "#!/usr/bin/env python3\nimport os, runpy\nos.environ['TWYDO_TEST_LIST_CONNECTION']={:?}\nrunpy.run_path({:?},run_name='__main__')\n",
            connection.to_str().unwrap(),
            backend.to_str().unwrap()
        ),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    cfg.sync = Some(SyncConfig {
        url: "https://dav.example.test/alice/".into(),
        username: "alice".into(),
        password_command: vec!["true".into()],
        list_python: Some(wrapper.to_str().unwrap().into()),
        ..Default::default()
    });
    (d, cfg)
}

#[cfg(unix)]
pub fn flag(root: &std::path::Path, name: &str) {
    let path = root.join("server.json");
    let mut server: serde_json::Value = fs::read(&path)
        .map(|bytes| serde_json::from_slice(&bytes).unwrap())
        .unwrap_or_else(|_| serde_json::json!({"calls":[],"collections":{},"flags":{}}));
    server["flags"][name] = true.into();
    fs::write(path, server.to_string()).unwrap();
}
