//! `resolve`: choosing a side of a recorded conflict, against a tempdir vdir.

use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Utc};

use twydo_core::config::Config;
use twydo_core::conflicts::{ConflictKind, ConflictState, ConflictStore, NewConflict, Side};
use twydo_core::store::bind_vdir_state;
use twydo_core::sync::resolve::{Resolution, ResolveError, resolve};

fn now() -> DateTime<Utc> {
    "2026-11-01T10:00:00Z".parse().unwrap()
}

struct Fx {
    _dir: tempfile::TempDir,
    cfg: Config,
    store: ConflictStore,
}

impl Fx {
    fn new() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let vdir = dir.path().join("vdir");
        fs::create_dir_all(vdir.join("c1")).unwrap();
        let mut cfg = Config::with_vdir(vdir);
        cfg.state_dir = Some(dir.path().join("state"));
        let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
        let store = ConflictStore::open(&state).unwrap();
        Fx {
            _dir: dir,
            cfg,
            store,
        }
    }

    fn file(&self) -> PathBuf {
        self.cfg.vdir.join("c1/x.ics")
    }

    fn record(&self, held: bool, holds: Side, local: Option<&str>, remote: Option<&str>) -> String {
        self.store
            .record(
                NewConflict {
                    uid: "x".into(),
                    collection: "c1".into(),
                    path: "c1/x.ics".into(),
                    kind: ConflictKind::BothChanged,
                    held,
                    vdir_holds: holds,
                    local: local.map(String::from),
                    remote: remote.map(String::from),
                },
                now(),
            )
            .unwrap()
            .id
    }
}

#[test]
fn crlf_only_difference_still_counts_as_holding() {
    let fx = Fx::new();
    let remote = "A\r\nB\r\n";
    // The hook wrote the remote version back with LF line endings.
    fs::write(fx.file(), "A\nB\n").unwrap();
    let id = fx.record(false, Side::Remote, Some("local\r\n"), Some(remote));
    let r = resolve(&fx.cfg, &id, Side::Local, now()).unwrap();
    assert!(matches!(r, Resolution::Written(_)), "{r:?}");
    assert_eq!(fs::read_to_string(fx.file()).unwrap(), "local\r\n");
    assert_eq!(
        fx.store.get(&id).unwrap().unwrap().state,
        ConflictState::Resolved
    );
}

#[test]
fn keeping_what_the_vdir_holds_is_in_place_even_with_lf() {
    let fx = Fx::new();
    fs::write(fx.file(), "A\nB\n").unwrap();
    let id = fx.record(false, Side::Remote, Some("l\r\n"), Some("A\r\nB\r\n"));
    assert_eq!(
        resolve(&fx.cfg, &id, Side::Remote, now()).unwrap(),
        Resolution::AlreadyInPlace
    );
    assert_eq!(fs::read_to_string(fx.file()).unwrap(), "A\nB\n");
}

#[test]
fn a_real_difference_is_stale() {
    let fx = Fx::new();
    fs::write(fx.file(), "changed since\n").unwrap();
    let id = fx.record(false, Side::Remote, Some("l\r\n"), Some("r\r\n"));
    assert!(matches!(
        resolve(&fx.cfg, &id, Side::Local, now()),
        Err(ResolveError::Write(twydo_core::store::WriteError::Stale(_)))
    ));
    assert_eq!(
        fx.store.get(&id).unwrap().unwrap().state,
        ConflictState::Open
    );
}

#[test]
fn retry_after_write_without_resolution_is_idempotent() {
    let fx = Fx::new();
    // The file already holds the kept version (a previous run wrote it, then crashed).
    fs::write(fx.file(), "l\r\n").unwrap();
    let id = fx.record(false, Side::Remote, Some("l\r\n"), Some("r\r\n"));
    assert_eq!(
        resolve(&fx.cfg, &id, Side::Local, now()).unwrap(),
        Resolution::AlreadyInPlace
    );
    assert_eq!(
        fx.store.get(&id).unwrap().unwrap().state,
        ConflictState::Resolved
    );
}

#[test]
fn held_conflict_records_the_choice_and_can_change() {
    let fx = Fx::new();
    fs::write(fx.file(), "l\r\n").unwrap();
    let id = fx.record(true, Side::Local, Some("l\r\n"), Some("r\r\n"));
    assert_eq!(
        resolve(&fx.cfg, &id, Side::Remote, now()).unwrap(),
        Resolution::Recorded
    );
    assert_eq!(
        resolve(&fx.cfg, &id, Side::Local, now()).unwrap(),
        Resolution::Recorded
    );
    let c = fx.store.get(&id).unwrap().unwrap();
    assert_eq!(
        (c.state, c.choice),
        (ConflictState::Chosen, Some(Side::Local))
    );
    // The vdir is untouched: the hook applies the choice.
    assert_eq!(fs::read_to_string(fx.file()).unwrap(), "l\r\n");
}

#[test]
fn unknown_and_finished_conflicts_are_refused() {
    let fx = Fx::new();
    assert!(matches!(
        resolve(&fx.cfg, "nope", Side::Local, now()),
        Err(ResolveError::NotFound(_))
    ));
    let id = fx.record(false, Side::Local, Some("l\r\n"), Some("r\r\n"));
    fs::write(fx.file(), "l\r\n").unwrap();
    resolve(&fx.cfg, &id, Side::Local, now()).unwrap();
    assert!(matches!(
        resolve(&fx.cfg, &id, Side::Remote, now()),
        Err(ResolveError::NotUnresolved(_))
    ));
}
