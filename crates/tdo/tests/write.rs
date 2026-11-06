//! The TUI writer against a temp vdir copied from synthetic phase-0 fixtures.

use std::cell::{Cell, RefCell};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use tdo::write::{Refused, Request, Writer};
use twydo_core::compat::Matrix;
use twydo_core::config::{Config, SyncConfig};
use twydo_core::edit::{Change, Op};
use twydo_core::store::fsio::VdirLock;
use twydo_core::sync::batch::Trigger;
use twydo_core::sync::running::{Running, process_start};
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

/// The triggers a writer fired, in order.
type Fired = Rc<RefCell<Vec<Trigger>>>;

/// A writer with the embedded matrix and a recording sync trigger.
fn writer(cfg: &Config) -> (Writer, Fired) {
    writer_with(cfg, Matrix::embedded().clone())
}

fn writer_with(cfg: &Config, matrix: Matrix) -> (Writer, Fired) {
    let fired: Fired = Rc::default();
    let f = fired.clone();
    let w = Writer::open_with(cfg, matrix, Box::new(move |_, t| f.borrow_mut().push(t)));
    (w, fired)
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
        Err(Refused::Other("nothing to undo".into()))
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
    assert_eq!(
        w.run(&snap, &Request::Apply(op), now()),
        Err(Refused::Other(reason))
    );
    assert_eq!(fs::read(env.task_file()).unwrap(), original);
    assert!(calls.borrow().is_empty());
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
    let err = w.run(&snap, &edit, now()).unwrap_err().message();
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
        let err = w.run(&env.snapshot(), &req, now()).unwrap_err().message();
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
fn first_save_starts_the_waiter() {
    let env = Env::new();
    let (mut w, fired) = writer(&sync_config(&env));
    w.run(&env.snapshot(), &complete_a(), now()).unwrap();
    assert_eq!(*fired.borrow(), [Trigger::Waiter]);
}

#[test]
fn fifth_save_syncs_now() {
    let env = Env::new();
    let mut cfg = sync_config(&env);
    cfg.sync.as_mut().unwrap().batch_size = 5;
    let (mut w, fired) = writer(&cfg);
    for i in 0..5 {
        let edit = Request::Apply(Op::Edit {
            uid: TASK_A.into(),
            changes: vec![Change::Summary(format!("A{i}"))],
        });
        w.run(&env.snapshot(), &edit, now()).unwrap();
    }
    let fired = fired.borrow();
    assert_eq!(fired.len(), 5);
    assert!(
        fired[..4].iter().all(|t| *t == Trigger::Waiter),
        "{fired:?}"
    );
    assert_eq!(fired[4], Trigger::SyncNow);
}

#[test]
fn refused_save_triggers_nothing() {
    let env = Env::new();
    let (mut w, fired) = writer(&sync_config(&env));
    assert!(w.run(&env.snapshot(), &Request::Undo, now()).is_err());
    assert!(fired.borrow().is_empty());
}

#[test]
fn no_sync_section_triggers_nothing() {
    let env = Env::new();
    let (mut w, fired) = writer(&env.config());
    w.run(&env.snapshot(), &complete_a(), now()).unwrap();
    assert!(fired.borrow().is_empty());
}

#[test]
fn switched_off_triggers_nothing() {
    let env = Env::new();
    let cfg = sync_config(&env);
    twydo_core::sync::switch::set(&cfg, true).unwrap();
    let (mut w, fired) = writer(&cfg);
    w.run(&env.snapshot(), &complete_a(), now()).unwrap();
    assert!(fired.borrow().is_empty());
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
    // Title, List, Type, Due, then Start.
    for _ in 0..4 {
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
            .is_err_and(|e| e.message().contains("changed on disk")),
        "{result:?}"
    );
    assert_eq!(fs::read_to_string(env.task_file()).unwrap(), synced);

    app.write_finished(result.map_err(|e| e.message()));
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

/// The state directory the writer uses, with `sync/` created.
fn sync_dir(cfg: &Config) -> PathBuf {
    let state = twydo_core::store::vdir_state_dir(&cfg.state_dir(), &cfg.vdir).unwrap();
    let dir = state.join("sync");
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Records a running sync as the runner would.
fn record_running(cfg: &Config, rec: &Running) {
    fs::write(
        sync_dir(cfg).join("running.json"),
        serde_json::to_vec(rec).unwrap(),
    )
    .unwrap();
}

fn shared_lock(cfg: &Config) -> VdirLock {
    let state = twydo_core::store::vdir_state_dir(&cfg.state_dir(), &cfg.vdir).unwrap();
    VdirLock::shared(&state).unwrap()
}

fn sync_config(env: &Env) -> Config {
    let mut cfg = env.config();
    cfg.sync = Some(SyncConfig::default());
    cfg
}

/// Issue #10: a short sync is waited for, so the save just works.
#[test]
fn write_waits_briefly_for_a_short_sync() {
    let env = Env::new();
    let cfg = env.config();
    let (mut w, _) = writer(&cfg);
    let lock = shared_lock(&cfg);
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        drop(lock);
    });
    let started = Instant::now();
    assert_eq!(
        w.run(&env.snapshot(), &complete_a(), now()),
        Ok("done".into())
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    t.join().unwrap();
}

/// While a long sync holds the vdir lock, a TUI write gives up after its short wait, says how
/// long the sync has run, and writes nothing.
#[test]
fn write_during_a_long_sync_is_busy_after_the_wait() {
    let env = Env::new();
    let cfg = sync_config(&env);
    let (mut w, _) = writer(&cfg);
    w.set_waits(Duration::from_millis(300), Duration::from_secs(1));
    let _sync = shared_lock(&cfg);
    record_running(
        &cfg,
        &Running {
            runner_pid: std::process::id(),
            runner_start: process_start(std::process::id()),
            child_pgid: None,
            child_start: None,
            started: Utc::now() - chrono::Duration::seconds(134),
        },
    );
    let original = fs::read(env.task_file()).unwrap();
    let started = Instant::now();
    let got = w.run(&env.snapshot(), &complete_a(), now());
    let elapsed = started.elapsed();
    match got {
        Err(Refused::Busy {
            running_for: Some(d),
        }) => assert!((134..140).contains(&d.as_secs()), "running for {d:?}"),
        other => panic!("expected busy, got {other:?}"),
    }
    assert!(elapsed >= Duration::from_millis(300), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
    assert_eq!(fs::read(env.task_file()).unwrap(), original);
    assert_eq!(
        w.run(&env.snapshot(), &Request::Undo, now()),
        Err(Refused::Other("nothing to undo".into()))
    );
}

#[test]
fn busy_without_a_recorded_sync_has_no_duration() {
    let env = Env::new();
    let cfg = sync_config(&env);
    let (mut w, _) = writer(&cfg);
    w.set_waits(Duration::from_millis(100), Duration::from_secs(1));
    let _sync = shared_lock(&cfg);
    assert_eq!(
        w.run(&env.snapshot(), &complete_a(), now()),
        Err(Refused::Busy { running_for: None })
    );
    assert_eq!(
        Refused::Busy { running_for: None }.message(),
        "a sync is running; try again in a moment"
    );
}

/// Review focus 1: the sync ended while the question was open; the save is plain.
#[test]
fn stop_and_run_when_the_sync_already_finished_saves_plainly() {
    let env = Env::new();
    let cfg = sync_config(&env);
    let (mut w, _) = writer(&cfg);
    assert_eq!(
        w.stop_and_run(&env.snapshot(), &complete_a(), now()),
        Ok("done".into())
    );
}

/// A vdirsyncer left behind by a runner that is gone holds the lock; stopping kills it.
#[test]
fn stop_and_run_kills_an_orphaned_sync_and_saves() {
    use std::os::unix::process::CommandExt;
    let env = Env::new();
    let cfg = sync_config(&env);
    let (mut w, _) = writer(&cfg);
    w.set_waits(Duration::from_millis(300), Duration::from_secs(3));
    let state = twydo_core::store::vdir_state_dir(&cfg.state_dir(), &cfg.vdir).unwrap();
    sync_dir(&cfg);
    let mut orphan = std::process::Command::new("sleep")
        .arg("30")
        .stdin(VdirLock::shared_for_child(&state).unwrap())
        .process_group(0)
        .spawn()
        .unwrap();
    let mut gone = std::process::Command::new("true").spawn().unwrap();
    gone.wait().unwrap();
    record_running(
        &cfg,
        &Running {
            runner_pid: gone.id(),
            runner_start: None,
            child_pgid: Some(orphan.id()),
            child_start: process_start(orphan.id()),
            started: Utc::now(),
        },
    );
    assert_eq!(
        w.stop_and_run(&env.snapshot(), &complete_a(), now()),
        Ok("done · sync stopped".into())
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while orphan.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "the orphaned sync survived");
        std::thread::sleep(Duration::from_millis(20));
    }
    let text = fs::read_to_string(env.task_file()).unwrap();
    assert!(text.contains("STATUS:COMPLETED"), "{text}");
    assert!(
        !sync_dir(&cfg).join("stop").exists(),
        "the stop request is cleared after the write"
    );
    // PR review 4: nobody else records the killed orphan, so the stop does, with backoff,
    // before the sync after the write could start another stuck run.
    let st = twydo_core::sync::status::SyncStatus::load(&state).unwrap();
    assert_eq!(st.failures, 1);
    assert!(st.next_attempt.is_some());
    assert_eq!(st.last_error.as_deref(), Some("sync stopped"));
}

#[test]
fn stop_and_run_gives_up_when_the_lock_stays_held() {
    let env = Env::new();
    let cfg = sync_config(&env);
    let (mut w, _) = writer(&cfg);
    w.set_waits(Duration::from_millis(100), Duration::from_millis(300));
    let _sync = shared_lock(&cfg);
    record_running(
        &cfg,
        &Running {
            runner_pid: std::process::id(),
            runner_start: process_start(std::process::id()),
            child_pgid: None,
            child_start: None,
            started: Utc::now(),
        },
    );
    assert_eq!(
        w.stop_and_run(&env.snapshot(), &complete_a(), now()),
        Err(Refused::Other("not saved; the sync did not stop".into()))
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
        Err(Refused::Other(
            "\"A2\" changed on disk; reload and try again".into()
        ))
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
                "RELATED-TO:fam-p\r\nSTATUS:COMPLETED\r\nCOMPLETED:20261104T120000Z\r\n",
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

#[test]
fn family_success_message_uses_the_singular_for_one_item() {
    let env = Env::new();
    add_family(&env);
    fs::remove_file(env.vdir().join("list1/fam-2.ics")).unwrap();
    let mut cfg = env.config();
    cfg.writes.mode = twydo_core::config::WriteMode::Test;
    let (mut w, _) = writer(&cfg);
    let done = Request::Apply(Op::Complete {
        uid: "fam-p".into(),
    });
    assert_eq!(
        w.run(&env.snapshot(), &done, now()),
        Ok("done: \"Synthetic parent\" and 1 item".into())
    );
}

/// Issue #10, review focus 5: declining to stop the sync after the task changed meanwhile
/// rebuilds the form from the saved version, as any refused save does.
#[test]
fn declining_after_the_task_changed_rebuilds_the_form() {
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
    app.handle_key(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE));
    let Outcome::Write(req) = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("expected a write");
    };
    app.write_busy(
        req,
        Some(Duration::from_secs(5)),
        app.write_snapshot().clone(),
    );

    let synced = String::from_utf8(fs::read(env.task_file()).unwrap())
        .unwrap()
        .replace("SUMMARY:A2", "SUMMARY:From phone");
    fs::write(env.task_file(), &synced).unwrap();
    app.reload(env.snapshot());

    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    let f = app.form.as_ref().expect("form stays open");
    assert_eq!(f.title, "From phone");
    assert_eq!(
        f.error.as_deref(),
        Some("not saved; the sync is still running; the form now shows the saved version")
    );
}

/// Runs `f` in a thread; true if it finished within `limit`.
fn finishes_within(limit: Duration, f: impl FnOnce() + Send + 'static) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        f();
        let _ = tx.send(());
    });
    rx.recv_timeout(limit).is_ok()
}

/// Issue #10: opening the TUI during a long sync must not wait for it. The runner recovers
/// the journal before vdirsyncer starts, and every write recovers under the lock again.
#[test]
fn tui_opens_while_a_sync_holds_the_vdir() {
    let env = Env::new();
    let cfg = env.config();
    let (mut w, _) = writer(&cfg);
    w.run(&env.snapshot(), &complete_a(), now()).unwrap(); // the journal exists now
    let _sync = shared_lock(&cfg);
    let c = cfg.clone();
    assert!(
        finishes_within(Duration::from_secs(2), move || {
            tdo::cli::recover_at_startup(&c);
        }),
        "startup recovery waited for the sync"
    );
    let c = cfg.clone();
    assert!(
        finishes_within(Duration::from_secs(2), move || {
            let _ = Writer::open_with(&c, Matrix::embedded().clone(), Box::new(|_, _| {}));
        }),
        "opening the writer waited for the sync"
    );
}

/// Review minor 2: the sync after a write must start only once the stop request is
/// withdrawn, or it stops itself and backs off.
#[test]
fn sync_after_a_write_starts_after_the_stop_request_is_cleared() {
    let env = Env::new();
    let cfg = sync_config(&env);
    let stop = sync_dir(&cfg).join("stop");
    let seen = Rc::new(Cell::new(None));
    let s = seen.clone();
    let mut w = Writer::open_with(
        &cfg,
        Matrix::embedded().clone(),
        Box::new(move |_, _| s.set(Some(stop.exists()))),
    );
    record_running(
        &cfg,
        &Running {
            runner_pid: std::process::id(),
            runner_start: process_start(std::process::id()),
            child_pgid: None,
            child_start: None,
            started: Utc::now(),
        },
    );
    assert_eq!(
        w.stop_and_run(&env.snapshot(), &complete_a(), now()),
        Ok("done · sync stopped".into())
    );
    assert_eq!(
        seen.get(),
        Some(false),
        "triggered while the stop was pending"
    );
}

/// PR review 1: `y` on a one-key action must apply against the snapshot the user decided on,
/// not one reloaded while the question was open.
#[test]
fn stop_and_save_refuses_a_one_key_action_on_a_task_changed_meanwhile() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tdo::app::{App, Outcome};
    let env = Env::new();
    let cfg = sync_config(&env);
    let (mut w, _) = writer(&cfg);
    let tz = chrono_tz::America::New_York;
    let mut app = App::new(env.snapshot(), now().with_timezone(&tz));
    let key = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
    app.handle_key(key('2'));
    app.cursor = app
        .visible_rows()
        .iter()
        .position(|r| app.snapshot.tasks[r.task].uid == TASK_A)
        .unwrap();
    app.handle_key(key('d'));
    let Outcome::Write(req) = app.handle_key(key('d')) else {
        panic!("expected delete");
    };
    // As term.rs does on Busy: keep the snapshot the write was made against, then reload.
    let base = app.write_snapshot().clone();
    let old = fs::read_to_string(env.task_file()).unwrap();
    fs::write(
        env.task_file(),
        old.replace("SUMMARY:A2", "SUMMARY:Remote update"),
    )
    .unwrap();
    app.reload(env.snapshot());
    app.write_busy(req, Some(Duration::from_secs(5)), base);
    let Outcome::StopSyncAndWrite(req) = app.handle_key(key('y')) else {
        panic!("expected stop");
    };
    let result = w.stop_and_run(app.write_snapshot(), &req, now());
    assert!(result.is_err(), "{result:?}");
    assert!(env.task_file().exists(), "the remote update was deleted");
}

/// PR re-review: a runner queued behind an orphan (it holds run.lock and waits for the vdir)
/// must not miss the stop and restart vdirsyncer once the save has withdrawn its request.
#[test]
fn queued_runner_cannot_miss_an_orphan_stop() {
    check_queued_orphan_stop(false);
}

/// A stop also applies before recovery: the runner may own run.lock while its status read
/// is delayed. A FIFO holds it there until the stopping save has withdrawn its request.
#[test]
fn queued_runner_delayed_reading_status_cannot_miss_an_orphan_stop() {
    check_queued_orphan_stop(true);
}

fn check_queued_orphan_stop(delay_status: bool) {
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    use std::process::{Command, Stdio};
    let env = Env::new();
    let mut cfg = sync_config(&env);
    cfg.sync.as_mut().unwrap().metasync = false;
    let marker = env.dir.path().join("started-again");
    let fake = env.dir.path().join("fake.sh");
    fs::write(
        &fake,
        format!("#!/bin/sh\necho ran > '{}'\nsleep 0.2\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    cfg.sync.as_mut().unwrap().command = fake.display().to_string();
    let config_root = env.dir.path().join("config");
    fs::create_dir_all(config_root.join("twydo")).unwrap();
    fs::write(
        config_root.join("twydo/config.toml"),
        format!(
            "vdir = '{}'\nstate_dir = '{}'\ntimezone = 'America/New_York'\n[sync]\ncommand = '{}'\nmetasync = false\n",
            cfg.vdir.display(),
            cfg.state_dir().display(),
            fake.display()
        ),
    )
    .unwrap();
    let (mut w, _) = writer(&cfg);
    let state = twydo_core::store::vdir_state_dir(&cfg.state_dir(), &cfg.vdir).unwrap();
    let sync = sync_dir(&cfg);
    let status_path = sync.join("status.json");
    // Held read-write, so the runner's open does not block but its read waits for the data.
    let mut fifo = None;
    if delay_status {
        assert!(
            Command::new("mkfifo")
                .arg(&status_path)
                .status()
                .unwrap()
                .success()
        );
        fifo = Some(
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&status_path)
                .unwrap(),
        );
    }
    let mut orphan = Command::new("sleep")
        .arg("30")
        .stdin(VdirLock::shared_for_child(&state).unwrap())
        .process_group(0)
        .spawn()
        .unwrap();
    record_running(
        &cfg,
        &Running {
            runner_pid: 0,
            runner_start: None,
            child_pgid: Some(orphan.id()),
            child_start: process_start(orphan.id()),
            started: Utc::now(),
        },
    );
    let run_lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(sync.join("run.lock"))
        .unwrap();
    let mut queued = Command::new(env!("CARGO_BIN_EXE_tdo"))
        .args(["sync", "--background"])
        .env("XDG_CONFIG_HOME", &config_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match run_lock.try_lock() {
            Ok(()) => run_lock.unlock().unwrap(),
            Err(std::fs::TryLockError::WouldBlock) => break,
            Err(e) => panic!("{e:?}"),
        }
        assert!(Instant::now() < deadline, "runner did not take run.lock");
        std::thread::sleep(Duration::from_millis(5));
    }
    // Delay it before recovery (the FIFO status read) or during recovery (SIGSTOP),
    // until the save has withdrawn its request.
    let pid = queued.id().to_string();
    if delay_status {
        // Once the runner has the FIFO open, a plain status file takes its name, so the save's
        // own status read (deciding the sync after the write) does not wait on the FIFO too.
        let fds = PathBuf::from(format!("/proc/{pid}/fd"));
        let has_fifo = || {
            fs::read_dir(&fds)
                .into_iter()
                .flatten()
                .any(|e| fs::read_link(e.unwrap().path()).is_ok_and(|t| t == status_path))
        };
        while !has_fifo() {
            assert!(Instant::now() < deadline, "runner did not open status.json");
            std::thread::sleep(Duration::from_millis(5));
        }
        let plain = sync.join("status.json.plain");
        let status = twydo_core::sync::status::SyncStatus::default();
        fs::write(&plain, serde_json::to_vec(&status).unwrap()).unwrap();
        fs::rename(&plain, &status_path).unwrap();
    } else {
        assert!(
            Command::new("kill")
                .args(["-STOP", &pid])
                .status()
                .unwrap()
                .success()
        );
    }
    let result = w.stop_and_run(&env.snapshot(), &complete_a(), Utc::now());
    if let Some(mut fifo) = fifo {
        use std::io::Write;
        let status = twydo_core::sync::status::SyncStatus::default();
        fifo.write_all(&serde_json::to_vec(&status).unwrap())
            .unwrap();
        // Closing the only writer ends the runner's read.
        drop(fifo);
    } else {
        assert!(
            Command::new("kill")
                .args(["-CONT", &pid])
                .status()
                .unwrap()
                .success()
        );
    }
    queued.wait().unwrap();
    orphan.wait().unwrap();
    assert_eq!(result, Ok("done · sync stopped".into()));
    assert!(
        !marker.exists(),
        "queued runner missed the stop and restarted vdirsyncer"
    );
    let st = twydo_core::sync::status::SyncStatus::load(&state).unwrap();
    assert!(st.failures > 0, "the stop was not recorded");
    assert!(st.next_attempt.is_some());
}

/// The `Enter` chain against a real store (test mode): each row writes one item under the
/// parent; once the parent is deleted elsewhere, the next save is refused, the row closes,
/// and the items already saved stay (Review Focus 5).
#[test]
fn enter_chain_writes_items_until_the_parent_is_gone() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tdo::app::{App, Outcome};
    let key = |c: KeyCode| KeyEvent::new(c, KeyModifiers::NONE);
    let env = Env::new();
    add_family(&env);
    let mut cfg = env.config();
    cfg.writes.mode = twydo_core::config::WriteMode::Test;
    let (mut w, _) = writer(&cfg);
    let tz = chrono_tz::America::New_York;
    let mut app = App::new(env.snapshot(), now().with_timezone(&tz));
    app.handle_key(key(KeyCode::Char('2')));
    app.cursor = app
        .visible_rows()
        .iter()
        .position(|r| app.snapshot.tasks[r.task].uid == "fam-p")
        .unwrap();
    app.handle_key(key(KeyCode::Char('a')));
    let mut save = |app: &mut App, title: &str| {
        for c in title.chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        let Outcome::Write(req) = app.handle_key(key(KeyCode::Enter)) else {
            panic!("expected a write");
        };
        let result = w.run(app.write_snapshot(), &req, now());
        if result.is_err() {
            app.write_refused();
        }
        app.reload(env.snapshot());
        app.write_finished(result.map_err(|e| e.message()));
    };
    save(&mut app, "Socks");
    let socks = app
        .snapshot
        .tasks
        .iter()
        .find(|t| t.summary == "Socks")
        .unwrap();
    assert_eq!(socks.parent.as_deref(), Some("fam-p"));
    let socks = socks.uid.clone();
    let row = app.new_row.as_ref().expect("the next row opens");
    assert_eq!(row.anchor.as_deref(), Some(socks.as_str()));

    // Every save uses the same second. Position metadata still keeps entry order,
    // and the chain opens at the end even though the titles would sort differently.
    save(&mut app, "Hat");
    save(&mut app, "Able");
    let p = app.snapshot.find("fam-p").unwrap();
    let items: Vec<_> = app
        .visible_rows()
        .iter()
        .filter(|r| r.depth > 0 && app.snapshot.parent(r.task) == Some(p))
        .map(|r| &app.snapshot.tasks[r.task])
        .collect();
    assert_eq!(
        items.iter().map(|t| t.summary.as_str()).collect::<Vec<_>>(),
        ["Synthetic one", "Synthetic two", "Socks", "Hat", "Able"]
    );
    assert_eq!(
        items[2..]
            .iter()
            .map(|t| t.meta.display_order())
            .collect::<Vec<_>>(),
        [3, 4, 5]
    );
    assert!(
        app.visible_rows()
            .iter()
            .any(|r| { r.depth == 0 && app.snapshot.tasks[r.task].uid == "fam-3" }),
        "the completed item remains visible separately in Done"
    );
    assert_eq!(
        app.new_row.as_ref().unwrap().anchor.as_deref(),
        Some(items.last().unwrap().uid.as_str())
    );

    fs::remove_file(env.vdir().join("list1/fam-p.ics")).unwrap();
    app.reload(env.snapshot());
    save(&mut app, "Gone");
    assert!(app.new_row.is_none(), "the row closes");
    assert_eq!(app.message.as_deref(), Some("no task with UID fam-p"));
    assert!(app.snapshot.find(&socks).is_some(), "the saved item stays");
    assert!(!app.snapshot.tasks.iter().any(|t| t.summary == "Gone"));
}

/// Keyboard/card requests and the real writer together, using the shipped verified matrix.
/// These fixtures are copied to a tempdir; no production vdir, config or sync is used.
mod verified_recurrence_tui {
    use super::{Env, now, writer};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::fs;
    use std::path::Path;
    use tdo::app::{App, Outcome};
    use tdo::write::{Request, Writer};
    use twydo_core::config::WriteMode;
    use twydo_core::model::{Status, TaskKind};

    const FC: &str = "61ed74be3ba34212ab7476b698de1263";
    const U1: &str = "91fb1d1636754afda633eb2bda234417";
    const U2: &str = "be99b147d81b4df6a63dc134e559dab6";
    fn env(step: &str) -> Env {
        env_in("plan8-tui-a", step)
    }
    fn env_in(scenario: &str, step: &str) -> Env {
        let e = Env::new();
        let list = e.vdir().join("list1");
        fs::remove_dir_all(&list).unwrap();
        fs::create_dir(&list).unwrap();
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/phase0/v1")
            .join(scenario)
            .join(step);
        for p in fs::read_dir(src).unwrap().map(|p| p.unwrap().path()) {
            if p.extension().is_some_and(|x| x == "ics") {
                fs::copy(&p, list.join(p.file_name().unwrap())).unwrap();
            }
        }
        assert_eq!(e.config().writes.mode, WriteMode::Verified);
        e
    }
    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn select(a: &mut App, uid: &str, done: bool) {
        a.handle_key(key(if done { '4' } else { '2' }));
        a.cursor = a
            .visible_rows()
            .iter()
            .position(|r| a.snapshot.tasks[r.task].uid == uid)
            .unwrap();
    }
    fn request(o: Outcome) -> Request {
        match o {
            Outcome::Write(r) => r,
            other => panic!("expected TUI write, got {other:?}"),
        }
    }
    fn run(e: &Env, a: &mut App, w: &mut Writer, r: Request) {
        let result = w.run(a.write_snapshot(), &r, now());
        a.reload(e.snapshot());
        a.write_finished(result.clone().map_err(|e| e.message()));
        result.unwrap();
    }
    fn card_repeat(a: &mut App, text: &str) -> Request {
        a.handle_key(key('r'));
        a.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        for c in text.chars() {
            a.handle_key(key(c));
        }
        request(a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)))
    }

    #[test]
    fn completion_anchored_card_and_date_keys_use_verified_permissions_and_keep_refused_text() {
        let e = env_in("plan8-rfra", "202-seeded-after-upload");
        let uid = "twydo-p8-rfra-fields-p";
        let (mut w, _) = writer(&e.config());
        let mut a = App::new(
            e.snapshot(),
            now().with_timezone(&chrono_tz::America::New_York),
        );
        select(&mut a, uid, false);
        let path = e.vdir().join(format!("list1/{uid}.ics"));
        let original = fs::read(&path).unwrap();
        let r = request(a.handle_key(key(']')));
        run(&e, &mut a, &mut w, r);
        assert_ne!(fs::read(&path).unwrap(), original);
        assert!(a.snapshot.tasks[a.snapshot.find(uid).unwrap()].from_completion);
        let r = request(a.handle_key(key('u')));
        run(&e, &mut a, &mut w, r);
        assert_eq!(fs::read(&path).unwrap(), original);
        let r = request(a.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)));
        run(&e, &mut a, &mut w, r);
        select(&mut a, uid, false);
        let r = card_repeat(&mut a, "weekly after done");
        run(&e, &mut a, &mut w, r);
        let task = &a.snapshot.tasks[a.snapshot.find(uid).unwrap()];
        assert_eq!(task.rrule.as_deref(), Some("FREQ=WEEKLY"));
        assert!(task.from_completion);
        select(&mut a, uid, false);
        let r = card_repeat(&mut a, "daily after done");
        a.form.as_mut().unwrap().title.push_str(" retained");
        let typed = a.form.as_ref().unwrap().title.clone();
        // Re-submit the grouped card after adding its ordinary field change.
        let _ = r;
        let r = request(a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        let before = fs::read(&path).unwrap();
        let err = w.run(a.write_snapshot(), &r, now()).unwrap_err();
        a.write_finished(Err(err.message()));
        assert_eq!(a.form.as_ref().unwrap().title, typed);
        assert_eq!(a.form.as_ref().unwrap().repeat, "daily after done");
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn completion_anchored_interval_card_accepts_only_the_captured_pure_change() {
        for mixed in [false, true] {
            let e = env_in("plan8-rfra", "202-seeded-after-upload");
            let uid = "twydo-p8-rfra-interval-p";
            let (mut w, _) = writer(&e.config());
            let mut a = App::new(
                e.snapshot(),
                now().with_timezone(&chrono_tz::America::New_York),
            );
            select(&mut a, uid, false);
            let first = card_repeat(&mut a, "every 2 months after done");
            let r = if mixed {
                a.form.as_mut().unwrap().title.push_str(" retained");
                request(a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)))
            } else {
                first
            };
            let before = fs::read(e.vdir().join(format!("list1/{uid}.ics"))).unwrap();
            let result = w.run(a.write_snapshot(), &r, now());
            if mixed {
                let err = result.unwrap_err();
                a.write_finished(Err(err.message()));
                assert!(a.form.as_ref().unwrap().title.ends_with(" retained"));
                assert_eq!(a.form.as_ref().unwrap().repeat, "every 2 months after done");
                assert_eq!(
                    fs::read(e.vdir().join(format!("list1/{uid}.ics"))).unwrap(),
                    before
                );
            } else {
                result.unwrap();
                let snap = e.snapshot();
                assert_eq!(
                    snap.tasks[snap.find(uid).unwrap()].rrule.as_deref(),
                    Some("FREQ=MONTHLY;INTERVAL=2")
                );
            }
        }
    }

    #[test]
    fn repeat_card_uses_verified_permissions_and_mixed_refusal_keeps_typing() {
        let e = env("010-first-phone-actions");
        let (mut w, _) = writer(&e.config());
        let mut a = App::new(
            e.snapshot(),
            now().with_timezone(&chrono_tz::America::New_York),
        );
        select(&mut a, FC, false);
        let original = fs::read(e.vdir().join(format!("list1/{FC}.ics"))).unwrap();
        let r = card_repeat(&mut a, "weekly");
        run(&e, &mut a, &mut w, r);
        let t = &a.snapshot.tasks[a.snapshot.find(FC).unwrap()];
        assert_eq!(t.rrule.as_deref(), Some("FREQ=WEEKLY"));
        let r = request(a.handle_key(key('u')));
        run(&e, &mut a, &mut w, r);
        assert_eq!(
            fs::read(e.vdir().join(format!("list1/{FC}.ics"))).unwrap(),
            original
        );
        let r = request(a.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)));
        run(&e, &mut a, &mut w, r);
        select(&mut a, FC, false);
        a.handle_key(key('r'));
        a.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        for c in "monthly".chars() {
            a.handle_key(key(c));
        }
        a.form.as_mut().unwrap().title.push_str(" changed");
        let typed = a.form.as_ref().unwrap().title.clone();
        let before = fs::read(e.vdir().join(format!("list1/{FC}.ics"))).unwrap();
        let r = request(a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        let err = w.run(a.write_snapshot(), &r, now()).unwrap_err();
        a.write_finished(Err(err.message()));
        assert_eq!(a.form.as_ref().unwrap().title, typed);
        assert_eq!(a.form.as_ref().unwrap().repeat, "monthly");
        assert_eq!(
            fs::read(e.vdir().join(format!("list1/{FC}.ics"))).unwrap(),
            before
        );
        a.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let r = card_repeat(&mut a, "none");
        run(&e, &mut a, &mut w, r);
        let t = &a.snapshot.tasks[a.snapshot.find(FC).unwrap()];
        assert!(t.rrule.is_none());
        assert_eq!(t.kind, TaskKind::Checklist);
    }

    #[test]
    fn ticks_complete_captured_until_tasks_and_reset_a_monthly_checklist() {
        for (uid, step, confirm, history_count) in [
            (U1, "001-after-upload", false, 1),
            (U2, "001-after-upload", false, 1),
            (FC, "010-first-phone-actions", true, 4),
        ] {
            let e = env(step);
            let (mut w, _) = writer(&e.config());
            let before = e.snapshot().tasks.len();
            let mut a = App::new(
                e.snapshot(),
                now().with_timezone(&chrono_tz::America::New_York),
            );
            select(&mut a, uid, false);
            let first = a.handle_key(key('x'));
            let r = if confirm {
                assert!(matches!(first, Outcome::Continue));
                request(a.handle_key(key('x')))
            } else {
                request(first)
            };
            run(&e, &mut a, &mut w, r);
            assert_eq!(a.snapshot.tasks.len(), before + history_count);
            let i = a.snapshot.find(uid).unwrap();
            assert_eq!(a.snapshot.tasks[i].status, Status::Open);
            if uid == FC {
                for &c in a.snapshot.children(i) {
                    assert_eq!(a.snapshot.tasks[c].status, Status::Open);
                    assert!(a.snapshot.tasks[c].due.is_none());
                }
            }
        }
    }

    #[test]
    fn done_ticks_reopen_an_ended_original_and_a_history_copy_in_verified_mode() {
        for (uid, step, keeps_rule) in [
            (U1, "030-end-series-and-items", true),
            (
                "7435cbd64d9443888f9eb8ebc84518f4",
                "050-remove-repeat-and-refinish",
                false,
            ),
        ] {
            let e = env(step);
            let (mut w, _) = writer(&e.config());
            let count = e.snapshot().tasks.len();
            let mut a = App::new(
                e.snapshot(),
                now().with_timezone(&chrono_tz::America::New_York),
            );
            select(&mut a, uid, true);
            let r = request(a.handle_key(key('x')));
            run(&e, &mut a, &mut w, r);
            let t = &a.snapshot.tasks[a.snapshot.find(uid).unwrap()];
            assert_eq!(t.status, Status::Open);
            assert_eq!(t.rrule.is_some(), keeps_rule);
            assert_eq!(a.snapshot.tasks.len(), count);
        }
    }
    #[test]
    fn verified_checklist_card_creation_and_conversion_are_usable() {
        let e = env_in("plan8-checklist-b", "002-seeded");
        let (mut w, _) = writer(&e.config());
        let mut a = App::new(
            e.snapshot(),
            now().with_timezone(&chrono_tz::America::New_York),
        );
        a.view = twydo_core::views::View::List("list1".into());
        a.handle_key(key('o'));
        a.handle_key(key('c'));
        a.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        let form = a.form.as_mut().unwrap();
        form.kind = TaskKind::Checklist;
        form.due = "2026-12-10".into();
        form.repeat = "monthly".into();
        let r = request(a.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)));
        run(&e, &mut a, &mut w, r);
        let created = a.snapshot.tasks.iter().find(|t| t.summary == "c").unwrap();
        let uid = created.uid.clone();
        assert_eq!(created.kind, TaskKind::Checklist);
        assert_eq!(created.rrule.as_deref(), Some("FREQ=MONTHLY"));
        select(&mut a, "twydo-p8-c29-d", false);
        a.handle_key(key('c'));
        let r = request(a.handle_key(key('c')));
        run(&e, &mut a, &mut w, r);
        assert_eq!(
            a.snapshot.tasks[a.snapshot.find("twydo-p8-c29-d").unwrap()].kind,
            TaskKind::Checklist
        );
        // Add through the Enter chain, then escape only the unsaved next row.
        select(&mut a, &uid, false);
        a.handle_key(key('a'));
        for title in ['x', 'q'] {
            a.handle_key(key(title));
            let r = request(a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
            run(&e, &mut a, &mut w, r);
        }
        a.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let p = a.snapshot.find(&uid).unwrap();
        assert_eq!(a.snapshot.children(p).len(), 2);
        for &i in a.snapshot.children(p) {
            assert_eq!(
                a.snapshot.tasks[i]
                    .due
                    .as_ref()
                    .unwrap()
                    .local_date(a.snapshot.tz()),
                chrono::NaiveDate::from_ymd_opt(2026, 12, 10).unwrap()
            );
        }
    }

    #[test]
    fn verified_checklist_item_delete_keys_undo_and_reset_use_exact_targets() {
        let e = env_in("plan8-checklist-b", "011-phone-items-and-text");
        let (mut w, _) = writer(&e.config());
        let mut a = App::new(
            e.snapshot(),
            now().with_timezone(&chrono_tz::America::New_York),
        );
        let parent = "12d762a5639e406c8ec865517882f130";
        let p = a.snapshot.find(parent).unwrap();
        let q = a
            .snapshot
            .children(p)
            .iter()
            .map(|&i| &a.snapshot.tasks[i])
            .find(|t| t.summary == "q")
            .unwrap()
            .uid
            .clone();
        select(&mut a, parent, false);
        a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        select(&mut a, &q, false);
        a.handle_key(key('d'));
        let r = request(a.handle_key(key('d')));
        run(&e, &mut a, &mut w, r);
        assert!(a.snapshot.find(&q).is_none());
        assert!(a.snapshot.find(parent).is_some());
        let r = request(a.handle_key(key('u')));
        run(&e, &mut a, &mut w, r);
        assert!(a.snapshot.find(&q).is_some());
        let r = request(a.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)));
        run(&e, &mut a, &mut w, r);
        assert!(a.snapshot.find(&q).is_none());
        let p = a.snapshot.find(parent).unwrap();
        let z = a.snapshot.tasks[a.snapshot.children(p)[0]].uid.clone();
        select(&mut a, &z, false);
        let r = request(a.handle_key(key('x')));
        run(&e, &mut a, &mut w, r);
        assert_eq!(
            a.snapshot.tasks[a.snapshot.find(parent).unwrap()].status,
            Status::Open
        );
        select(&mut a, parent, false);
        let r = request(a.handle_key(key('x')));
        run(&e, &mut a, &mut w, r);
        let p = a.snapshot.find(parent).unwrap();
        assert_eq!(
            a.snapshot.tasks[p]
                .due
                .as_ref()
                .unwrap()
                .local_date(a.snapshot.tz()),
            chrono::NaiveDate::from_ymd_opt(2027, 1, 10).unwrap()
        );
        assert_eq!(
            a.snapshot.tasks[a.snapshot.find(&z).unwrap()].status,
            Status::Open
        );
        assert!(a.snapshot.find(&q).is_none());
        assert!(
            !a.snapshot
                .tasks
                .iter()
                .any(|t| t.summary == "q" && t.parent.as_deref() == Some(parent))
        );
        // Text cards use the same shipping writer on reset items and their parent.
        select(&mut a, &z, false);
        a.handle_key(key('i'));
        a.form.as_mut().unwrap().title = "x".into();
        let r = request(a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        run(&e, &mut a, &mut w, r);
        assert_eq!(a.snapshot.tasks[a.snapshot.find(&z).unwrap()].summary, "x");
        assert_eq!(
            a.snapshot.tasks[a.snapshot.find(&z).unwrap()]
                .parent
                .as_deref(),
            Some(parent)
        );
        select(&mut a, parent, false);
        a.handle_key(key('i'));
        a.form.as_mut().unwrap().title = "renamed".into();
        a.form.as_mut().unwrap().notes = "z".into();
        let r = request(a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        run(&e, &mut a, &mut w, r);
        let task = &a.snapshot.tasks[a.snapshot.find(parent).unwrap()];
        assert_eq!(task.summary, "renamed");
        assert_eq!(task.rrule.as_deref(), Some("FREQ=MONTHLY"));
        // An uncaptured date edit retains typing and changes no resource.
        select(&mut a, parent, false);
        a.handle_key(key('i'));
        a.form.as_mut().unwrap().title = "kept unsaved".into();
        a.form.as_mut().unwrap().due = "2027-01-11".into();
        let r = request(a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        let before = fs::read(e.vdir().join(format!("list1/{parent}.ics"))).unwrap();
        let err = w.run(a.write_snapshot(), &r, now()).unwrap_err();
        a.write_finished(Err(err.message()));
        assert_eq!(a.form.as_ref().unwrap().title, "kept unsaved");
        assert_eq!(
            fs::read(e.vdir().join(format!("list1/{parent}.ics"))).unwrap(),
            before
        );
    }
}
