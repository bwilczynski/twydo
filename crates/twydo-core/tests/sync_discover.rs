//! The helper's `discover` action: adopts remote-only lists, never creates remote ones.
#![cfg(unix)]

mod common;

use std::fs;

use common::lists::{flag, offline};
use twydo_core::sync::discover::{DiscoverError, discover};

fn script(root: &std::path::Path, remote_only: &[&str], local_only: &[&str]) {
    let path = root.join("server.json");
    let mut server: serde_json::Value = fs::read(&path)
        .map(|bytes| serde_json::from_slice(&bytes).unwrap())
        .unwrap_or_else(|_| serde_json::json!({"calls":[],"collections":{},"flags":{}}));
    server["discover"] = serde_json::json!({"remote_only": remote_only, "local_only": local_only});
    fs::write(path, server.to_string()).unwrap();
}

fn state(cfg: &twydo_core::config::Config) -> std::path::PathBuf {
    twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap()
}

fn calls(root: &std::path::Path) -> Vec<String> {
    let server: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("server.json")).unwrap()).unwrap();
    serde_json::from_value(server["calls"].clone()).unwrap()
}

#[test]
fn discover_adopts_a_remote_only_list() {
    let (d, cfg) = offline();
    script(d.path(), &["phone-list"], &[]);
    let found = discover(&cfg, &state(&cfg)).unwrap();
    assert_eq!(found.adopted, ["phone-list"]);
    assert!(cfg.vdir.join("phone-list").is_dir());
}

#[test]
fn discover_never_creates_a_remote_list() {
    let (d, cfg) = offline();
    script(d.path(), &[], &["stale"]);
    match discover(&cfg, &state(&cfg)) {
        Err(DiscoverError::Missing { id }) => assert_eq!(id, "stale"),
        other => panic!("{other:?}"),
    }
    assert!(!calls(d.path()).iter().any(|c| c == "MKCALENDAR"));
}

#[test]
fn discover_maps_other_refusals() {
    let (d, cfg) = offline();
    flag(d.path(), "fail_refresh_once");
    match discover(&cfg, &state(&cfg)) {
        Err(DiscoverError::Failed(message)) => assert!(!message.contains("https://"), "{message}"),
        other => panic!("{other:?}"),
    }
}
