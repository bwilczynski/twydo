//! The TUI writer against a temp vdir copied from synthetic phase-0 fixtures.

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use chrono::{TimeZone, Utc};
use tdo::write::{Request, Writer};
use twydo_core::compat::Matrix;
use twydo_core::config::Config;
use twydo_core::edit::{Change, Op};
use twydo_core::views::Snapshot;

/// One-time task "A2".
const TASK_A: &str = "0a04f22aeb294b719633c978fe1975e5";

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let vdir = dir.path().join("vdir");
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/phase0/v1/plan4-list1/02-edit");
        fs::create_dir_all(vdir.join("list1")).unwrap();
        fs::create_dir_all(vdir.join("list2")).unwrap();
        for e in fs::read_dir(&src).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "ics") {
                fs::copy(&p, vdir.join("list1").join(p.file_name().unwrap())).unwrap();
            }
        }
        fs::write(vdir.join("list2/displayname"), "Two").unwrap();
        Env { dir }
    }

    fn vdir(&self) -> PathBuf {
        self.dir.path().join("vdir")
    }

    fn config(&self) -> Config {
        let mut cfg = Config::with_vdir(self.vdir());
        cfg.timezone = Some("America/New_York".into());
        cfg.state_dir = Some(self.dir.path().join("state"));
        cfg
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot::new(twydo_core::vdir::load(&self.vdir()).unwrap(), self.config())
    }

    fn task_file(&self) -> PathBuf {
        self.vdir().join(format!("list1/{TASK_A}.ics"))
    }
}

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 4, 15, 0, 0).unwrap()
}

/// A writer with the embedded matrix and a counting sync trigger.
fn writer(cfg: &Config) -> (Writer, Rc<Cell<u32>>) {
    writer_with(cfg, Matrix::embedded().clone())
}

fn writer_with(cfg: &Config, matrix: Matrix) -> (Writer, Rc<Cell<u32>>) {
    let calls = Rc::new(Cell::new(0));
    let c = calls.clone();
    let w = Writer::open_with(cfg, matrix, Box::new(move |_| c.set(c.get() + 1)));
    (w, calls)
}

fn complete_a() -> Request {
    Request::Apply(Op::Complete { uid: TASK_A.into() })
}

#[test]
fn complete_then_undo_then_redo() {
    let env = Env::new();
    let original = fs::read(env.task_file()).unwrap();
    let (mut w, _) = writer(&env.config());
    assert_eq!(
        w.run(&env.snapshot(), &complete_a(), now()),
        Ok("done".into())
    );
    assert!(
        String::from_utf8(fs::read(env.task_file()).unwrap())
            .unwrap()
            .contains("STATUS:COMPLETED")
    );
    assert_eq!(
        w.run(&env.snapshot(), &Request::Undo, now()),
        Ok("undone".into())
    );
    assert_eq!(fs::read(env.task_file()).unwrap(), original);
    assert_eq!(
        w.run(&env.snapshot(), &Request::Redo, now()),
        Ok("redone".into())
    );
    assert_ne!(fs::read(env.task_file()).unwrap(), original);
}

#[test]
fn success_messages_per_op() {
    let env = Env::new();
    let (mut w, _) = writer(&env.config());
    let edit = Request::Apply(Op::Edit {
        uid: TASK_A.into(),
        changes: vec![Change::Summary("A3".into())],
    });
    assert_eq!(w.run(&env.snapshot(), &edit, now()), Ok("saved".into()));
    let delete = Request::Apply(Op::Delete { uid: TASK_A.into() });
    assert_eq!(
        w.run(&env.snapshot(), &delete, now()),
        Ok("deleted · u to undo".into())
    );
}

#[test]
fn undo_with_empty_session() {
    let env = Env::new();
    let (mut w, _) = writer(&env.config());
    assert_eq!(
        w.run(&env.snapshot(), &Request::Undo, now()),
        Err("nothing to undo".into())
    );
}

#[test]
fn blocked_op_returns_gate_reason() {
    let env = Env::new();
    let snap = env.snapshot();
    let matrix = Matrix::from_toml_str("matrix_version = 1\n").unwrap();
    let op = Op::Complete { uid: TASK_A.into() };
    let reason = twydo_core::gate::check(
        &snap,
        &matrix,
        env.config().writes.mode,
        &op,
        now().date_naive(),
    )
    .unwrap_err()
    .to_string();
    let original = fs::read(env.task_file()).unwrap();
    let (mut w, calls) = writer_with(&env.config(), matrix);
    assert_eq!(w.run(&snap, &Request::Apply(op), now()), Err(reason));
    assert_eq!(fs::read(env.task_file()).unwrap(), original);
    assert_eq!(calls.get(), 0);
}

#[test]
fn stale_snapshot_is_refused_and_untouched() {
    let env = Env::new();
    let snap = env.snapshot();
    let mut bytes = fs::read(env.task_file()).unwrap();
    bytes.extend_from_slice(b"X-EXTRA:1\r\n");
    fs::write(env.task_file(), &bytes).unwrap();
    let (mut w, _) = writer(&env.config());
    let edit = Request::Apply(Op::Edit {
        uid: TASK_A.into(),
        changes: vec![Change::Summary("A3".into())],
    });
    let err = w.run(&snap, &edit, now()).unwrap_err();
    assert!(err.contains("changed on disk"), "{err}");
    assert_eq!(fs::read(env.task_file()).unwrap(), bytes);
}

#[test]
fn writer_without_store_reports_open_error() {
    let env = Env::new();
    let cfg = env.config();
    let state = twydo_core::store::vdir_state_dir(&cfg.state_dir(), &env.vdir()).unwrap();
    fs::create_dir_all(&state).unwrap();
    fs::write(
        state.join(twydo_core::store::VDIR_ID_FILE),
        "/some/other/vdir\n",
    )
    .unwrap();
    let (mut w, _) = writer(&cfg);
    for req in [complete_a(), Request::Undo, Request::Redo] {
        let err = w.run(&env.snapshot(), &req, now()).unwrap_err();
        assert!(err.starts_with("cannot write: "), "{err}");
    }
}

#[test]
fn move_message_names_list() {
    let env = Env::new();
    let (mut w, _) = writer(&env.config());
    let mv = Request::Apply(Op::Move {
        uid: TASK_A.into(),
        to: "list2".into(),
    });
    assert_eq!(
        w.run(&env.snapshot(), &mv, now()),
        Ok("moved to Two".into())
    );
}

#[test]
fn after_edit_triggers_sync_only_on_success() {
    let env = Env::new();
    let mut cfg = env.config();
    cfg.sync = Some(twydo_core::config::SyncConfig {
        after_edit: true,
        ..Default::default()
    });
    let (mut w, calls) = writer(&cfg);
    assert!(w.run(&env.snapshot(), &Request::Undo, now()).is_err());
    assert_eq!(calls.get(), 0);
    assert!(w.run(&env.snapshot(), &complete_a(), now()).is_ok());
    assert_eq!(calls.get(), 1);
}

#[test]
fn no_sync_without_after_edit() {
    let env = Env::new();
    let mut cfg = env.config();
    cfg.sync = Some(twydo_core::config::SyncConfig {
        after_edit: false,
        ..Default::default()
    });
    let (mut w, calls) = writer(&cfg);
    assert!(w.run(&env.snapshot(), &complete_a(), now()).is_ok());
    assert_eq!(calls.get(), 0);
}
