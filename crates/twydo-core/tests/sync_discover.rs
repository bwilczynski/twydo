//! Discovery: adopts remote-only lists, never creates remote ones.
#![cfg(unix)]

mod common;

use std::fs;
use std::path::Path;

use common::lists::offline;
use twydo_core::sync::discover::{DiscoverError, discover};
use twydo_testdav::{add_collection, calls, collections, flag};

/// The server holds these collections and the vdir these (empty) folders.
fn script(
    root: &Path,
    cfg: &twydo_core::config::Config,
    remote_only: &[&str],
    local_only: &[&str],
) {
    for id in remote_only {
        add_collection(root, id, id, "#112233");
    }
    for id in local_only {
        fs::create_dir_all(cfg.vdir.join(id)).unwrap();
    }
}

fn state(cfg: &twydo_core::config::Config) -> std::path::PathBuf {
    twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap()
}

#[test]
fn discover_adopts_a_remote_only_list() {
    let (d, _server, cfg) = offline();
    script(d.path(), &cfg, &["phone-list"], &[]);
    let found = discover(&cfg, &state(&cfg)).unwrap();
    assert_eq!(found.adopted, ["phone-list"]);
    assert!(cfg.vdir.join("phone-list").is_dir());
    assert!(calls(d.path()).iter().any(|c| c == "DISCOVER"));
}

#[test]
fn discover_never_creates_a_remote_list() {
    let (d, _server, cfg) = offline();
    script(d.path(), &cfg, &[], &["stale"]);
    match discover(&cfg, &state(&cfg)) {
        Err(DiscoverError::Missing { id }) => assert_eq!(id, "stale"),
        other => panic!("{other:?}"),
    }
    assert!(!calls(d.path()).iter().any(|c| c == "MKCALENDAR"));
    assert!(collections(d.path()).is_empty());
}

#[test]
fn discover_maps_other_refusals() {
    let (d, _server, cfg) = offline();
    flag(d.path(), "fail_refresh_once");
    match discover(&cfg, &state(&cfg)) {
        Err(DiscoverError::Failed(message)) => assert!(!message.contains("http"), "{message}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_failed_vdirsyncer_discover_removes_adopted_folders() {
    let (d, _server, cfg) = offline();
    script(d.path(), &cfg, &["phone-list", "tablet-list"], &["kept"]);
    add_collection(d.path(), "kept", "kept", "#112233");
    flag(d.path(), "fail_refresh_once");
    match discover(&cfg, &state(&cfg)) {
        Err(DiscoverError::Failed(message)) => assert!(!message.contains("http"), "{message}"),
        other => panic!("{other:?}"),
    }
    assert!(!cfg.vdir.join("phone-list").exists());
    assert!(!cfg.vdir.join("tablet-list").exists());
    assert!(cfg.vdir.join("kept").is_dir());
    assert!(!calls(d.path()).iter().any(|c| c == "MKCALENDAR"));
}

#[test]
fn a_non_plain_server_id_fails_and_creates_nothing() {
    let (d, _server, cfg) = offline();
    script(d.path(), &cfg, &["good-list", "bad%2Fid"], &[]);
    match discover(&cfg, &state(&cfg)) {
        Err(DiscoverError::Failed(message)) => assert!(!message.contains("http"), "{message}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(fs::read_dir(&cfg.vdir).unwrap().count(), 0);
    assert!(!calls(d.path()).iter().any(|c| c == "DISCOVER"));
}

/// vdirsyncer names a folder after the decoded id, so a server id such as `a:b` may already
/// have a local folder that Twydo's stricter rule would refuse to adopt.
#[test]
fn a_non_plain_server_id_that_already_has_a_local_folder_does_not_block_discovery() {
    let (d, _server, cfg) = offline();
    script(d.path(), &cfg, &["good-list", "paired:id"], &["paired:id"]);
    let found = discover(&cfg, &state(&cfg)).unwrap();
    assert_eq!(found.adopted, ["good-list"]);
    assert!(cfg.vdir.join("paired:id").is_dir());
    assert!(cfg.vdir.join("good-list").is_dir());
    assert!(calls(d.path()).iter().any(|c| c == "DISCOVER"));
}

#[test]
fn vdirsyncer_discover_gets_a_non_terminal_stdin_at_end_of_file() {
    let (d, _server, cfg) = offline();
    script(d.path(), &cfg, &["phone-list"], &[]);
    let found = discover(&cfg, &state(&cfg)).unwrap();
    assert_eq!(found.adopted, ["phone-list"]);
    let seen = fs::read_to_string(d.path().join("discover.stdin")).unwrap();
    assert_eq!(seen.trim(), "tty=no eof=yes");
}

#[test]
fn a_discover_that_exits_zero_with_an_error_line_removes_adopted_folders() {
    let (d, _server, cfg) = offline();
    script(d.path(), &cfg, &["phone-list", "tablet-list"], &[]);
    flag(d.path(), "error_line_exit_zero");
    match discover(&cfg, &state(&cfg)) {
        Err(DiscoverError::Failed(message)) => assert!(!message.contains("http"), "{message}"),
        other => panic!("{other:?}"),
    }
    assert!(!cfg.vdir.join("phone-list").exists());
    assert!(!cfg.vdir.join("tablet-list").exists());
    assert!(calls(d.path()).iter().any(|c| c == "DISCOVER"));
}
