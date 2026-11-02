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
        Ok("undone: A2".into())
    );
    assert_eq!(fs::read(env.task_file()).unwrap(), original);
    assert_eq!(
        w.run(&env.snapshot(), &Request::Redo, now()),
        Ok("redone: A2".into())
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

/// Review Focus 1 through the form: a sync that lands while the form is open (here, a
/// phone edit of the title) must make the save fail as stale, not overwrite it.
#[test]
fn sync_while_form_open_is_stale_not_overwritten() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tdo::app::{App, Outcome};
    let env = Env::new();
    let tz = chrono_tz::America::New_York;
    let mut app = App::new(env.snapshot(), now().with_timezone(&tz));
    app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
    let row = app
        .visible_rows()
        .iter()
        .position(|r| app.snapshot.tasks[r.task].uid == TASK_A)
        .unwrap();
    app.cursor = row;
    app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    for _ in 0..2 {
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    }
    // Start = "3d".
    app.handle_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));

    // The phone renames the task; the sync lands and the watcher reloads.
    let synced = String::from_utf8(fs::read(env.task_file()).unwrap())
        .unwrap()
        .replace("SUMMARY:A2", "SUMMARY:From phone");
    fs::write(env.task_file(), &synced).unwrap();
    app.reload(env.snapshot());

    let Outcome::Write(req) = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("expected a write");
    };
    let (mut w, _) = writer(&env.config());
    let result = w.run(app.write_snapshot(), &req, now());
    assert!(
        result
            .as_ref()
            .is_err_and(|e| e.contains("changed on disk")),
        "{result:?}"
    );
    assert_eq!(fs::read_to_string(env.task_file()).unwrap(), synced);

    app.write_finished(result);
    let f = app.form.as_ref().expect("form stays open");
    assert_eq!(
        f.title, "From phone",
        "the form now shows the synced version"
    );
    assert!(f.error.as_deref().unwrap().contains("changed on disk"));

    // Saving again now applies against the version the user can see.
    for c in "3d".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    let Outcome::Write(req) = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("expected a write");
    };
    assert_eq!(w.run(app.write_snapshot(), &req, now()), Ok("saved".into()));
    let text = fs::read_to_string(env.task_file()).unwrap();
    assert!(text.contains("SUMMARY:From phone"), "{text}");
}

/// Review item 5: while a sync holds the vdir lock (shared, for its whole run), a TUI write
/// must not block the UI; it is refused at once and nothing is written.
#[test]
fn write_during_sync_is_refused_without_waiting() {
    let env = Env::new();
    let cfg = env.config();
    let (mut w, _) = writer(&cfg);
    let state = twydo_core::store::vdir_state_dir(&cfg.state_dir(), &env.vdir()).unwrap();
    let sync = twydo_core::store::fsio::VdirLock::shared(&state).unwrap();
    let original = fs::read(env.task_file()).unwrap();
    let started = std::time::Instant::now();
    assert_eq!(
        w.run(&env.snapshot(), &complete_a(), now()),
        Err("a sync is running; try again in a moment".into())
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    assert_eq!(fs::read(env.task_file()).unwrap(), original);
    assert_eq!(
        w.run(&env.snapshot(), &Request::Undo, now()),
        Err("nothing to undo".into())
    );
    drop(sync);
    assert_eq!(
        w.run(&env.snapshot(), &complete_a(), now()),
        Ok("done".into())
    );
}

#[test]
fn undo_and_redo_name_the_task() {
    let env = Env::new();
    let (mut w, _) = writer(&env.config());
    w.run(&env.snapshot(), &complete_a(), now()).unwrap();
    assert_eq!(
        w.run(&env.snapshot(), &Request::Undo, now()),
        Ok("undone: A2".into())
    );
    assert_eq!(
        w.run(&env.snapshot(), &Request::Redo, now()),
        Ok("redone: A2".into())
    );
}

#[test]
fn undo_of_a_delete_names_the_deleted_task() {
    let env = Env::new();
    let (mut w, _) = writer(&env.config());
    let delete = Request::Apply(Op::Delete { uid: TASK_A.into() });
    w.run(&env.snapshot(), &delete, now()).unwrap();
    assert_eq!(
        w.run(&env.snapshot(), &Request::Undo, now()),
        Ok("undone: A2".into())
    );
}

#[test]
fn stale_message_names_the_task_not_the_file() {
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
    assert_eq!(
        w.run(&snap, &edit, now()),
        Err("\"A2\" changed on disk; reload and try again".into())
    );
}

/// Writes a synthetic checklist with two open items and one completed item into list1.
fn add_family(env: &Env) {
    let task = |uid: &str, title: &str, extra: &str| {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\n\
             UID:{uid}\r\nSUMMARY:{title}\r\n{extra}END:VTODO\r\nEND:VCALENDAR\r\n"
        )
    };
    let dir = env.vdir().join("list1");
    let files = [
        (
            "fam-p",
            task(
                "fam-p",
                "Synthetic parent",
                "STATUS:NEEDS-ACTION\r\nX-2DOAPP-METADATA:@tasktype:checklist\r\n",
            ),
        ),
        (
            "fam-1",
            task(
                "fam-1",
                "Synthetic one",
                "RELATED-TO:fam-p\r\nSTATUS:NEEDS-ACTION\r\n",
            ),
        ),
        (
            "fam-2",
            task(
                "fam-2",
                "Synthetic two",
                "RELATED-TO:fam-p\r\nSTATUS:NEEDS-ACTION\r\n",
            ),
        ),
        (
            "fam-3",
            task(
                "fam-3",
                "Synthetic three",
                "RELATED-TO:fam-p\r\nSTATUS:COMPLETED\r\n",
            ),
        ),
    ];
    for (name, body) in files {
        fs::write(dir.join(format!("{name}.ics")), body).unwrap();
    }
}

#[test]
fn family_success_messages() {
    let env = Env::new();
    add_family(&env);
    let mut cfg = env.config();
    cfg.writes.mode = twydo_core::config::WriteMode::Test;
    let (mut w, _) = writer(&cfg);
    let run = |w: &mut Writer, op: Op| w.run(&env.snapshot(), &Request::Apply(op), now());
    let uid = || "fam-p".to_string();
    assert_eq!(
        run(&mut w, Op::Complete { uid: uid() }),
        Ok("done: \"Synthetic parent\" and 2 items".into())
    );
    assert_eq!(
        run(&mut w, Op::Reopen { uid: uid() }),
        Ok("reopened: \"Synthetic parent\" and 3 items".into())
    );
    assert_eq!(
        run(
            &mut w,
            Op::Move {
                uid: uid(),
                to: "list2".into()
            }
        ),
        Ok("moved \"Synthetic parent\" and 3 items to Two".into())
    );
    let moved = env.snapshot();
    let new_uid = moved
        .tasks
        .iter()
        .find(|t| t.summary == "Synthetic parent")
        .unwrap()
        .uid
        .clone();
    assert_eq!(
        run(&mut w, Op::Delete { uid: new_uid }),
        Ok("deleted \"Synthetic parent\" and 3 items · u to undo".into())
    );
}
