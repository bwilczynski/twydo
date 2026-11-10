//! `resolve`: choosing a side of a recorded conflict, against a tempdir vdir.

use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Utc};

use twydo_core::config::Config;
use twydo_core::conflicts::{ConflictKind, ConflictState, ConflictStore, NewConflict, Side};
use twydo_core::store::bind_vdir_state;
use twydo_core::store::journal::RequestRef;
use twydo_core::sync::resolve::{Keep, Resolution, ResolveError, resolve, resolve_with};

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
    let r = resolve(&fx.cfg, &id, Keep::Local, now()).unwrap();
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
        resolve(&fx.cfg, &id, Keep::Remote, now()).unwrap(),
        Resolution::AlreadyInPlace
    );
    assert_eq!(fs::read_to_string(fx.file()).unwrap(), "A\nB\n");
}

#[test]
fn a_real_difference_is_refused() {
    let fx = Fx::new();
    fs::write(fx.file(), "changed since\n").unwrap();
    let id = fx.record(false, Side::Remote, Some("l\r\n"), Some("r\r\n"));
    assert!(matches!(
        resolve(&fx.cfg, &id, Keep::Local, now()),
        Err(ResolveError::Changed { .. })
    ));
    assert_eq!(fs::read_to_string(fx.file()).unwrap(), "changed since\n");
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
        resolve(&fx.cfg, &id, Keep::Local, now()).unwrap(),
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
        resolve(&fx.cfg, &id, Keep::Remote, now()).unwrap(),
        Resolution::Recorded
    );
    assert_eq!(
        resolve(&fx.cfg, &id, Keep::Local, now()).unwrap(),
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
        resolve(&fx.cfg, "nope", Keep::Local, now()),
        Err(ResolveError::NotFound(_))
    ));
    let id = fx.record(false, Side::Local, Some("l\r\n"), Some("r\r\n"));
    fs::write(fx.file(), "l\r\n").unwrap();
    resolve(&fx.cfg, &id, Keep::Local, now()).unwrap();
    assert!(matches!(
        resolve(&fx.cfg, &id, Keep::Remote, now()),
        Err(ResolveError::NotUnresolved(_))
    ));
}

#[test]
fn keeping_what_the_vdir_held_after_it_changed_is_refused() {
    let fx = Fx::new();
    // The vdir held the remote version when the conflict was recorded; since then the task
    // was deleted (or changed again) on the server and that synced.
    let id = fx.record(false, Side::Remote, Some("l\r\n"), Some("r\r\n"));
    assert!(matches!(
        resolve(&fx.cfg, &id, Keep::Remote, now()),
        Err(ResolveError::Changed { .. })
    ));
    assert!(!fx.file().exists(), "nothing written");
    fs::write(fx.file(), "third\r\n").unwrap();
    assert!(matches!(
        resolve(&fx.cfg, &id, Keep::Remote, now()),
        Err(ResolveError::Changed { .. })
    ));
    assert_eq!(fs::read_to_string(fx.file()).unwrap(), "third\r\n");
    assert_eq!(
        fx.store.get(&id).unwrap().unwrap().state,
        ConflictState::Open
    );
}

#[test]
fn keep_current_closes_a_conflict_whose_file_changed() {
    let fx = Fx::new();
    fs::write(fx.file(), "third\r\n").unwrap();
    let id = fx.record(false, Side::Remote, Some("l\r\n"), Some("r\r\n"));
    assert!(resolve(&fx.cfg, &id, Keep::Local, now()).is_err());
    assert!(resolve(&fx.cfg, &id, Keep::Remote, now()).is_err());
    assert_eq!(
        resolve(&fx.cfg, &id, Keep::Current, now()).unwrap(),
        Resolution::KeptCurrent
    );
    assert_eq!(fs::read_to_string(fx.file()).unwrap(), "third\r\n");
    let c = fx.store.get(&id).unwrap().unwrap();
    assert_eq!(c.state, ConflictState::Resolved);
    // Both versions stay in the store.
    assert_eq!(
        (c.local.as_deref(), c.remote.as_deref()),
        (Some("l\r\n"), Some("r\r\n"))
    );
}

#[test]
fn keep_current_is_refused_for_a_held_conflict() {
    let fx = Fx::new();
    fs::write(fx.file(), "l\r\n").unwrap();
    let id = fx.record(true, Side::Local, Some("l\r\n"), Some("r\r\n"));
    assert!(matches!(
        resolve(&fx.cfg, &id, Keep::Current, now()),
        Err(ResolveError::Held(_))
    ));
    assert_eq!(
        fx.store.get(&id).unwrap().unwrap().state,
        ConflictState::Open
    );
}

fn req(id: &str) -> RequestRef {
    RequestRef {
        session: "s1".into(),
        request_id: id.into(),
    }
}

#[test]
fn resolution_records_its_request() {
    // (held, the file holds, keep, expected)
    let cases = [
        (true, Keep::Remote, "recorded"),
        (false, Keep::Remote, "already_in_place"),
        (false, Keep::Local, "written"),
        (false, Keep::Current, "kept_current"),
    ];
    for (n, (held, keep, word)) in cases.into_iter().enumerate() {
        let fx = Fx::new();
        fs::write(fx.file(), "R\n").unwrap();
        let id = fx.record(held, Side::Remote, Some("L\n"), Some("R\n"));
        let rid = format!("r{n}");
        let got = resolve_with(&fx.cfg, &id, keep, now(), Some(req(&rid))).unwrap();
        let (c, by) = fx
            .store
            .by_request(&rid)
            .unwrap()
            .expect("found by request");
        assert_eq!(c.id, id);
        assert_eq!(c.resolved_by.as_ref(), Some(&by));
        assert_eq!(by.request, req(&rid));
        assert_eq!(by.resolution, got);
        assert!(
            serde_json::to_value(&got)
                .unwrap()
                .to_string()
                .contains(word)
        );
        match &got {
            Resolution::Written(op) => assert_eq!(by.op_id.as_deref(), Some(op.as_str())),
            _ => assert_eq!(by.op_id, None),
        }
        assert!(fx.store.by_request("other").unwrap().is_none());
    }
}

/// Codex review: every request that chose a held conflict keeps its record. A chooses local,
/// B remote; A's record still answers A, and the choice stays B's.
#[test]
fn every_request_that_chose_keeps_its_record() {
    let fx = Fx::new();
    fs::write(fx.file(), "L\n").unwrap();
    let id = fx.record(true, Side::Local, Some("L\n"), Some("R\n"));
    resolve_with(&fx.cfg, &id, Keep::Local, now(), Some(req("a"))).unwrap();
    resolve_with(&fx.cfg, &id, Keep::Remote, now(), Some(req("b"))).unwrap();
    // A store opened afresh, as after a restart.
    let state = bind_vdir_state(&fx.cfg.state_dir(), &fx.cfg.vdir).unwrap();
    let store = ConflictStore::open(&state).unwrap();
    let (c, a) = store.by_request("a").unwrap().expect("A's record");
    assert_eq!(c.id, id);
    assert_eq!(a.request, req("a"));
    assert_eq!(a.resolution, Resolution::Recorded);
    let (_, b) = store.by_request("b").unwrap().expect("B's record");
    assert_eq!(b.request, req("b"));
    let c = store.get(&id).unwrap().unwrap();
    assert_eq!(
        (c.state, c.choice),
        (ConflictState::Chosen, Some(Side::Remote))
    );
}

#[test]
fn written_resolution_names_its_request_in_the_journal() {
    let fx = Fx::new();
    fs::write(fx.file(), "R\n").unwrap();
    let id = fx.record(false, Side::Remote, Some("L\n"), Some("R\n"));
    let Resolution::Written(op) =
        resolve_with(&fx.cfg, &id, Keep::Local, now(), Some(req("r9"))).unwrap()
    else {
        panic!("written")
    };
    let state = bind_vdir_state(&fx.cfg.state_dir(), &fx.cfg.vdir).unwrap();
    let e = twydo_core::store::journal::Journal::open(&state)
        .unwrap()
        .get(&op)
        .unwrap()
        .unwrap();
    assert_eq!(e.request, Some(req("r9")));
}
