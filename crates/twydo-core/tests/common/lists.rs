//! Offline list harness shared by the list tests: a fake CalDAV server and the fake
//! `vdirsyncer`.
#![allow(dead_code)]

use std::fs;

use twydo_core::config::{Config, SyncConfig};

/// A config against a fake CalDAV server and the fake vdirsyncer, both under one temporary
/// directory (also the server's state root). The server guard must outlive the use of the
/// config.
#[cfg(unix)]
pub fn offline() -> (tempfile::TempDir, twydo_testdav::FakeDav, Config) {
    let d = tempfile::tempdir().unwrap();
    let server = twydo_testdav::FakeDav::start(d.path());
    let mut cfg = Config::with_vdir(d.path().join("vdir"));
    fs::create_dir(&cfg.vdir).unwrap();
    cfg.state_dir = Some(d.path().join("state"));
    cfg.writes.mode = twydo_core::config::WriteMode::Test;
    cfg.sync = Some(SyncConfig {
        url: server.url(),
        username: "alice".into(),
        password_command: vec!["printf".into(), "secret".into()],
        command: twydo_testdav::fake_vdirsyncer(d.path())
            .to_string_lossy()
            .into_owned(),
        timeout_secs: 30,
        ..Default::default()
    });
    (d, server, cfg)
}

/// How many times the fake server saw `method`.
#[cfg(unix)]
pub fn count(root: &std::path::Path, method: &str) -> usize {
    twydo_testdav::calls(root)
        .iter()
        .filter(|call| *call == method)
        .count()
}

/// What other processes see of the locks a list operation holds: whether the sync run lock is
/// taken, whether an exclusive vdir lock is refused, and whether a shared one is allowed.
#[cfg(unix)]
pub struct Locks {
    pub run_held: bool,
    pub vdir_exclusive_refused: bool,
    pub shared_allowed: bool,
}

#[cfg(unix)]
pub fn locks(cfg: &Config) -> Locks {
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let open = |name: &str| {
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(state.join(name))
            .unwrap()
    };
    let run = open("sync/run.lock");
    let run_held = matches!(run.try_lock(), Err(std::fs::TryLockError::WouldBlock));
    drop(run);
    let vdir_exclusive_refused = twydo_core::store::fsio::VdirLock::try_exclusive(&state)
        .unwrap()
        .is_none();
    let reader = open("vdir.lock");
    let shared_allowed = reader.try_lock_shared().is_ok();
    Locks {
        run_held,
        vdir_exclusive_refused,
        shared_allowed,
    }
}
