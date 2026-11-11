//! The TUI writer against a temp vdir copied from synthetic phase-0 fixtures.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use common::WriteDaemon;
use tdo::write::Request;
use twydo_core::compat::Matrix;
use twydo_core::config::{Config, SyncConfig};
use twydo_core::edit::{Change, Op};
use twydo_core::views::Snapshot;
use twydo_daemon::server::lane::LongKind;

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

/// A daemon for `cfg` with the embedded matrix, and a client of it.
fn writer(cfg: &Config) -> WriteDaemon {
    writer_with(cfg, Matrix::embedded().clone())
}

fn writer_with(cfg: &Config, matrix: Matrix) -> WriteDaemon {
    WriteDaemon::start(cfg.clone(), matrix)
}

fn complete_a() -> Request {
    Request::Apply(Op::Complete { uid: TASK_A.into() })
}

#[test]
fn complete_then_undo_then_redo() {
    let env = Env::new();
    let original = fs::read(env.task_file()).unwrap();
    let w = writer(&env.config());
    assert_eq!(w.run_fresh(&complete_a()), Ok("done".into()));
    assert!(
        String::from_utf8(fs::read(env.task_file()).unwrap())
            .unwrap()
            .contains("STATUS:COMPLETED")
    );
    assert_eq!(w.run_fresh(&Request::Undo), Ok("undone: A2".into()));
    assert_eq!(fs::read(env.task_file()).unwrap(), original);
    assert_eq!(w.run_fresh(&Request::Redo), Ok("redone: A2".into()));
    assert_ne!(fs::read(env.task_file()).unwrap(), original);
}

/// A daemon starts and serves over a pending collection operation (so the operation can be
/// reconciled through it); once the operation is reconciled, a daemon writes.
#[test]
fn daemon_started_during_a_pending_list_operation_writes_after_reconciliation() {
    let env = Env::new();
    let mut cfg = env.config();
    cfg.sync = Some(SyncConfig::default());
    let mut request = twydo_core::lists::helper::HelperRequest::probe(&cfg).unwrap();
    request.action = "preview-delete".into();
    request.collection = Some("list1".into());
    let record = twydo_core::lists::operation::prepare(&cfg, request, now()).unwrap();
    {
        let dir = common::short_socket_dir();
        let opts = twydo_daemon::server::DaemonOptions::new(dir.path().join("s").join("tdo.sock"));
        let instance = twydo_daemon::protocol::methods::Instance {
            config: dir.path().join("config.toml"),
            vdir: Some(cfg.vdir.clone()),
        };
        let h = twydo_daemon::server::Daemon::start(cfg.clone(), instance, opts).unwrap();
        assert!(
            h.wait_ready(Duration::from_secs(10)),
            "the daemon must start over an unfinished list operation"
        );
        h.shutdown();
    }
    let mut service = twydo_core::lists::ListService::open(&cfg, None).unwrap();
    assert!(
        service
            .reconcile(&record.id)
            .unwrap_err()
            .to_string()
            .contains("read-only")
    );
    assert!(
        twydo_core::lists::operation::active(&cfg)
            .unwrap()
            .is_none()
    );
    let w = writer(&cfg);
    assert_eq!(w.run_fresh(&complete_a()), Ok("done".into()));
    assert_eq!(w.run_fresh(&Request::Undo), Ok("undone: A2".into()));
}

#[test]
fn success_messages_per_op() {
    let env = Env::new();
    let w = writer(&env.config());
    let edit = Request::Apply(Op::Edit {
        uid: TASK_A.into(),
        changes: vec![Change::Summary("A3".into())],
    });
    assert_eq!(w.run_fresh(&edit), Ok("saved".into()));
    let delete = Request::Apply(Op::Delete { uid: TASK_A.into() });
    assert_eq!(w.run_fresh(&delete), Ok("deleted · u to undo".into()));
}

#[test]
fn undo_with_empty_session() {
    let env = Env::new();
    let w = writer(&env.config());
    assert_eq!(w.run_fresh(&Request::Undo), Err("nothing to undo".into()));
}

/// The undo stack lives in the daemon's session: after a restart (a new epoch) there is none.
#[test]
fn undo_after_a_daemon_restart_says_nothing_to_undo() {
    let env = Env::new();
    let first = writer(&env.config());
    assert_eq!(first.run(&complete_a()), Ok("done".into()));
    let epoch = first.epoch.clone();
    first.handle.shutdown();
    drop(first);
    let second = writer(&env.config());
    assert_ne!(second.epoch, epoch);
    assert_eq!(second.run(&Request::Undo), Err("nothing to undo".into()));
    assert!(
        fs::read_to_string(env.task_file())
            .unwrap()
            .contains("STATUS:COMPLETED"),
        "the completed task stays completed"
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
    let w = writer_with(&env.config(), matrix);
    assert_eq!(w.run(&Request::Apply(op)), Err(reason));
    assert_eq!(fs::read(env.task_file()).unwrap(), original);
}

#[test]
fn stale_snapshot_is_refused_and_untouched() {
    let env = Env::new();
    // The view the edit is planned on is the one before the file changes.
    let w = writer(&env.config());
    let mut bytes = fs::read(env.task_file()).unwrap();
    bytes.extend_from_slice(b"X-EXTRA:1\r\n");
    fs::write(env.task_file(), &bytes).unwrap();
    let edit = Request::Apply(Op::Edit {
        uid: TASK_A.into(),
        changes: vec![Change::Summary("A3".into())],
    });
    let err = w.run(&edit).unwrap_err();
    assert!(err.contains("changed on disk"), "{err}");
    assert_eq!(fs::read(env.task_file()).unwrap(), bytes);
}

/// A state directory that belongs to another vdir keeps a daemon from starting at all.
#[test]
fn daemon_refuses_a_state_dir_of_another_vdir() {
    let env = Env::new();
    let cfg = env.config();
    let state = twydo_core::store::vdir_state_dir(&cfg.state_dir(), &env.vdir()).unwrap();
    fs::create_dir_all(&state).unwrap();
    fs::write(
        state.join(twydo_core::store::VDIR_ID_FILE),
        "/some/other/vdir\n",
    )
    .unwrap();
    let dir = common::short_socket_dir();
    let opts = twydo_daemon::server::DaemonOptions::new(dir.path().join("s").join("tdo.sock"));
    let instance = twydo_daemon::protocol::methods::Instance {
        config: dir.path().join("config.toml"),
        vdir: Some(cfg.vdir.clone()),
    };
    let err = match twydo_daemon::server::Daemon::start(cfg, instance, opts) {
        Ok(_) => panic!("the daemon started"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("belongs to vdir /some/other/vdir"), "{err}");
}

#[test]
fn move_message_names_list() {
    let env = Env::new();
    let w = writer(&env.config());
    let mv = Request::Apply(Op::Move {
        uid: TASK_A.into(),
        to: "list2".into(),
    });
    assert_eq!(w.run_fresh(&mv), Ok("moved to Two".into()));
}

/// Review Focus 1 through the form: a sync that lands while the form is open (here, a
/// phone edit of the title) must make the save fail as stale, not overwrite it.
#[test]
fn sync_while_form_open_is_stale_not_overwritten() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tdo::app::{App, Outcome};
    let env = Env::new();
    let w = writer(&env.config());
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

    // The form was planned on this view.
    let planned = w.revision();
    // The phone renames the task; the sync lands and the watcher reloads.
    let synced = String::from_utf8(fs::read(env.task_file()).unwrap())
        .unwrap()
        .replace("SUMMARY:A2", "SUMMARY:From phone");
    fs::write(env.task_file(), &synced).unwrap();
    app.reload(w.snapshot());

    let Outcome::Write(req) = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("expected a write");
    };
    let result = w.run_at(planned, &req);
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
    assert_eq!(w.run(&req), Ok("saved".into()));
    let text = fs::read_to_string(env.task_file()).unwrap();
    assert!(text.contains("SUMMARY:From phone"), "{text}");
}

fn sync_config(env: &Env) -> Config {
    let mut cfg = env.config();
    cfg.sync = Some(SyncConfig::default());
    cfg
}

fn state_of(cfg: &Config) -> PathBuf {
    twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap()
}

fn wait_for(what: &str, ok: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Waits until no write is queued any more (the daemon applied or refused them all).
fn wait_drained(cfg: &Config) {
    let state = state_of(cfg);
    wait_for("the queue to drain", || {
        twydo_core::store::queue::Queue::existing(&state).is_none_or(|q| q.is_empty().unwrap())
    });
}

/// A save while a sync holds the vdir does not wait: it is queued, and says so.
#[test]
fn save_during_a_sync_is_queued_at_once() {
    let env = Env::new();
    let cfg = sync_config(&env);
    let w = writer(&cfg);
    let _sync = w.handle.hold_long(LongKind::Test);
    let original = fs::read(env.task_file()).unwrap();
    let started = Instant::now();
    let got = w.run(&complete_a());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(got, Ok("done · queued (a sync is running)".into()));
    assert_eq!(fs::read(env.task_file()).unwrap(), original);
}

/// A queued save a drain refuses is reported once, by the title the user saw.
#[test]
fn drain_reports_refusals_with_the_title() {
    let env = Env::new();
    let cfg = sync_config(&env);
    let w = writer(&cfg);
    let sync = w.handle.hold_long(LongKind::Test);
    w.run(&complete_a()).unwrap();
    // The phone changes the task before the queue is written.
    let old = fs::read_to_string(env.task_file()).unwrap();
    fs::write(
        env.task_file(),
        old.replace("SUMMARY:A2", "SUMMARY:From phone"),
    )
    .unwrap();
    drop(sync);
    let state = state_of(&cfg);
    let unshown = || {
        twydo_core::store::queue::Queue::existing(&state)
            .map(|q| q.unshown().unwrap())
            .unwrap_or_default()
    };
    wait_for("the refusal", || !unshown().is_empty());
    let refusals = unshown();
    assert_eq!(
        refusals
            .iter()
            .map(twydo_core::store::queue::refusal_message)
            .collect::<Vec<_>>(),
        vec!["Not saved: \"A2\" changed on another device".to_string()]
    );
    // Acknowledged, it is not shown again.
    let ack: twydo_daemon::protocol::methods::AckResult = w
        .client
        .call(
            "refusals.ack",
            twydo_daemon::protocol::methods::AckParams {
                request_id: w.client.next_request_id(),
                ids: refusals.iter().map(|r| r.item.id.clone()).collect(),
            },
        )
        .unwrap();
    assert_eq!(ack.acknowledged, 1);
    assert!(unshown().is_empty(), "shown once");
    let text = fs::read_to_string(env.task_file()).unwrap();
    assert!(!text.contains("STATUS:COMPLETED"), "{text}");
}

/// When the sync ends the daemon writes what was queued.
#[test]
fn a_queued_save_is_written_when_the_sync_ends() {
    let env = Env::new();
    let cfg = sync_config(&env);
    let w = writer(&cfg);
    let sync = w.handle.hold_long(LongKind::Test);
    let first = w.run(&complete_a()).unwrap();
    assert!(first.contains("queued"), "{first}");
    drop(sync);
    wait_drained(&cfg);
    let text = fs::read_to_string(env.task_file()).unwrap();
    assert!(text.contains("STATUS:COMPLETED"), "{text}");
}

#[test]
fn undo_of_a_queued_save_cancels_it() {
    let env = Env::new();
    let cfg = sync_config(&env);
    let w = writer(&cfg);
    let sync = w.handle.hold_long(LongKind::Test);
    let original = fs::read(env.task_file()).unwrap();
    let first = w.run(&complete_a()).unwrap();
    assert!(first.contains("queued"), "{first}");
    assert_eq!(w.run_fresh(&Request::Undo), Ok("undone: A2".into()));
    drop(sync);
    wait_drained(&cfg);
    assert_eq!(fs::read(env.task_file()).unwrap(), original);
}

/// A recovery is a job: during a sync it waits for the lane and runs after it (the backup
/// here does not exist, so the job then fails with its own reason, not "try again").
#[test]
fn recover_during_a_sync_runs_after_it() {
    let env = Env::new();
    let cfg = sync_config(&env);
    let w = writer(&cfg);
    let sync = w.handle.hold_long(LongKind::Test);
    let req = Request::Recover {
        backup: "any-backup".into(),
        to: "list1".into(),
    };
    let (caller, session) = (w.client.caller(), w.client.session().unwrap());
    let based_on = twydo_daemon::protocol::methods::BasedOn {
        epoch: w.epoch.clone(),
        revision: w.revision(),
    };
    let snap = w.fetch();
    let started = Instant::now();
    let job = std::thread::spawn(move || {
        tdo::write::execute(&caller, &session, &based_on, &snap, &req, "recover-1")
    });
    std::thread::sleep(Duration::from_millis(300));
    assert!(!job.is_finished(), "the recovery ran during the sync");
    drop(sync);
    let err = job.join().unwrap().unwrap_err();
    assert!(started.elapsed() >= Duration::from_millis(250));
    assert!(!err.contains("try again"), "{err}");
}

#[test]
fn undo_and_redo_name_the_task() {
    let env = Env::new();
    let w = writer(&env.config());
    w.run_fresh(&complete_a()).unwrap();
    assert_eq!(w.run_fresh(&Request::Undo), Ok("undone: A2".into()));
    assert_eq!(w.run_fresh(&Request::Redo), Ok("redone: A2".into()));
}

#[test]
fn undo_of_a_delete_names_the_deleted_task() {
    let env = Env::new();
    let w = writer(&env.config());
    let delete = Request::Apply(Op::Delete { uid: TASK_A.into() });
    w.run_fresh(&delete).unwrap();
    assert_eq!(w.run_fresh(&Request::Undo), Ok("undone: A2".into()));
}

#[test]
fn stale_message_names_the_task_not_the_file() {
    let env = Env::new();
    // The view the edit is planned on is the one before the file changes.
    let w = writer(&env.config());
    let mut bytes = fs::read(env.task_file()).unwrap();
    bytes.extend_from_slice(b"X-EXTRA:1\r\n");
    fs::write(env.task_file(), &bytes).unwrap();
    let edit = Request::Apply(Op::Edit {
        uid: TASK_A.into(),
        changes: vec![Change::Summary("A3".into())],
    });
    assert_eq!(
        w.run(&edit),
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
    let w = writer(&cfg);
    let run = |w: &WriteDaemon, op: Op| w.run_fresh(&Request::Apply(op));
    let uid = || "fam-p".to_string();
    assert_eq!(
        run(&w, Op::Complete { uid: uid() }),
        Ok("done: \"Synthetic parent\" and 2 items".into())
    );
    assert_eq!(
        run(&w, Op::Reopen { uid: uid() }),
        Ok("reopened: \"Synthetic parent\" and 3 items".into())
    );
    assert_eq!(
        run(
            &w,
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
        run(&w, Op::Delete { uid: new_uid }),
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
    let w = writer(&cfg);
    let done = Request::Apply(Op::Complete {
        uid: "fam-p".into(),
    });
    assert_eq!(
        w.run_fresh(&done),
        Ok("done: \"Synthetic parent\" and 1 item".into())
    );
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
    let w = writer(&cfg);
    let tz = chrono_tz::America::New_York;
    let mut app = App::new(env.snapshot(), now().with_timezone(&tz));
    app.handle_key(key(KeyCode::Char('2')));
    app.cursor = app
        .visible_rows()
        .iter()
        .position(|r| app.snapshot.tasks[r.task].uid == "fam-p")
        .unwrap();
    app.handle_key(key(KeyCode::Char('a')));
    let save = |app: &mut App, title: &str| {
        for c in title.chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        let Outcome::Write(req) = app.handle_key(key(KeyCode::Enter)) else {
            panic!("expected a write");
        };
        let result = w.run(&req);
        if result.is_err() {
            app.write_refused();
        }
        app.reload(w.snapshot());
        app.write_finished(result);
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
    app.reload(w.snapshot());
    save(&mut app, "Gone");
    assert!(app.new_row.is_none(), "the row closes");
    assert_eq!(app.message.as_deref(), Some("no task with UID fam-p"));
    assert!(app.snapshot.find(&socks).is_some(), "the saved item stays");
    assert!(!app.snapshot.tasks.iter().any(|t| t.summary == "Gone"));
}

/// Keyboard/card requests and the real writer together, using the shipped verified matrix.
/// These fixtures are copied to a tempdir; no production vdir, config or sync is used.
mod verified_recurrence_tui {
    use super::{Env, WriteDaemon, now, wait_drained, writer};
    use chrono::{TimeZone, Utc};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::fs;
    use std::path::Path;
    use tdo::app::{App, Outcome};
    use tdo::write::Request;
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
            .unwrap_or_else(|| {
                panic!(
                    "{uid} missing from {:?}; visible {:?}",
                    a.view,
                    a.visible_rows()
                        .iter()
                        .map(|r| &a.snapshot.tasks[r.task].uid)
                        .collect::<Vec<_>>()
                )
            });
    }
    fn request(o: Outcome) -> Request {
        match o {
            Outcome::Write(r) => r,
            other => panic!("expected TUI write, got {other:?}"),
        }
    }
    fn run(_e: &Env, a: &mut App, w: &WriteDaemon, r: Request) {
        let result = w.run(&r);
        a.reload(w.snapshot());
        a.write_finished(result.clone());
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
        let w = writer(&e.config());
        let mut a = App::new(
            e.snapshot(),
            now().with_timezone(&chrono_tz::America::New_York),
        );
        select(&mut a, uid, false);
        let path = e.vdir().join(format!("list1/{uid}.ics"));
        let original = fs::read(&path).unwrap();
        let r = request(a.handle_key(key(']')));
        run(&e, &mut a, &w, r);
        assert_ne!(fs::read(&path).unwrap(), original);
        assert!(a.snapshot.tasks[a.snapshot.find(uid).unwrap()].from_completion);
        let r = request(a.handle_key(key('u')));
        run(&e, &mut a, &w, r);
        assert_eq!(fs::read(&path).unwrap(), original);
        let r = request(a.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)));
        run(&e, &mut a, &w, r);
        select(&mut a, uid, false);
        let r = card_repeat(&mut a, "weekly after done");
        run(&e, &mut a, &w, r);
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
        let err = w.run(&r).unwrap_err();
        a.write_finished(Err(err));
        assert_eq!(a.form.as_ref().unwrap().title, typed);
        assert_eq!(a.form.as_ref().unwrap().repeat, "daily after done");
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn completion_anchored_interval_card_accepts_only_the_captured_pure_change() {
        for mixed in [false, true] {
            let e = env_in("plan8-rfra", "202-seeded-after-upload");
            let uid = "twydo-p8-rfra-interval-p";
            let w = writer(&e.config());
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
            let result = w.run(&r);
            if mixed {
                let err = result.unwrap_err();
                a.write_finished(Err(err));
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
        let w = writer(&e.config());
        let mut a = App::new(
            e.snapshot(),
            now().with_timezone(&chrono_tz::America::New_York),
        );
        select(&mut a, FC, false);
        let original = fs::read(e.vdir().join(format!("list1/{FC}.ics"))).unwrap();
        let r = card_repeat(&mut a, "weekly");
        run(&e, &mut a, &w, r);
        let t = &a.snapshot.tasks[a.snapshot.find(FC).unwrap()];
        assert_eq!(t.rrule.as_deref(), Some("FREQ=WEEKLY"));
        let r = request(a.handle_key(key('u')));
        run(&e, &mut a, &w, r);
        assert_eq!(
            fs::read(e.vdir().join(format!("list1/{FC}.ics"))).unwrap(),
            original
        );
        let r = request(a.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)));
        run(&e, &mut a, &w, r);
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
        let err = w.run(&r).unwrap_err();
        a.write_finished(Err(err));
        assert_eq!(a.form.as_ref().unwrap().title, typed);
        assert_eq!(a.form.as_ref().unwrap().repeat, "monthly");
        assert_eq!(
            fs::read(e.vdir().join(format!("list1/{FC}.ics"))).unwrap(),
            before
        );
        a.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let r = card_repeat(&mut a, "none");
        run(&e, &mut a, &w, r);
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
            let w = writer(&e.config());
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
            run(&e, &mut a, &w, r);
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
            let w = writer(&e.config());
            let count = e.snapshot().tasks.len();
            let mut a = App::new(
                e.snapshot(),
                now().with_timezone(&chrono_tz::America::New_York),
            );
            select(&mut a, uid, true);
            let r = request(a.handle_key(key('x')));
            run(&e, &mut a, &w, r);
            let t = &a.snapshot.tasks[a.snapshot.find(uid).unwrap()];
            assert_eq!(t.status, Status::Open);
            assert_eq!(t.rrule.is_some(), keeps_rule);
            assert_eq!(a.snapshot.tasks.len(), count);
        }
    }
    #[test]
    fn history_family_done_selection_reopens_once_and_undo_restores_bytes() {
        let e = env_in("plan6-list1", "05-complete-parent");
        let uid = "14b5a37a52224e8c959b5002e2275fc9";
        let originals: Vec<_> = fs::read_dir(e.vdir().join("list1"))
            .unwrap()
            .map(|p| {
                let p = p.unwrap().path();
                let b = fs::read(&p).unwrap();
                (p, b)
            })
            .collect();
        let w = writer(&e.config());
        let mut a = App::new(
            e.snapshot(),
            now().with_timezone(&chrono_tz::America::New_York),
        );
        select(&mut a, uid, true);
        let r = request(a.handle_key(key('x')));
        run(&e, &mut a, &w, r);
        let p = a.snapshot.find(uid).unwrap();
        assert_eq!(a.snapshot.tasks[p].status, Status::Open);
        assert!(a.snapshot.tasks[p].rrule.is_none());
        for &c in a.snapshot.children(p) {
            assert_eq!(a.snapshot.tasks[c].status, Status::Open);
        }
        let r = request(a.handle_key(key('u')));
        run(&e, &mut a, &w, r);
        for (p, b) in originals {
            assert_eq!(fs::read(p).unwrap(), b);
        }
    }

    #[test]
    fn verified_checklist_card_creation_and_conversion_are_usable() {
        let e = env_in("plan8-checklist-b", "002-seeded");
        let w = writer(&e.config());
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
        run(&e, &mut a, &w, r);
        let created = a.snapshot.tasks.iter().find(|t| t.summary == "c").unwrap();
        let uid = created.uid.clone();
        assert_eq!(created.kind, TaskKind::Checklist);
        assert_eq!(created.rrule.as_deref(), Some("FREQ=MONTHLY"));
        select(&mut a, "twydo-p8-c29-d", false);
        a.handle_key(key('c'));
        let r = request(a.handle_key(key('c')));
        run(&e, &mut a, &w, r);
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
            run(&e, &mut a, &w, r);
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
    fn verified_parent_delete_keys_remove_one_flat_family_and_undo_once() {
        let parent = "12d762a5639e406c8ec865517882f130";
        let q = "2cb7991c71624ddda29877c098acad43";
        let z = "3199749e2ea64f10bdaebc00197768ca";
        let b = "twydo-p8-c29-b";
        let b_q = "96ee97f3cc8c403391c57ec8b8e5d192";
        let b_z = "f9ae2ffd776e42919f9696f37dd6e849";
        let d = "twydo-p8-c29-d";

        let e = env_in("plan8-checklist-b", "011-phone-items-and-text");
        let original = item_bytes(&e);
        let w = writer(&e.config());
        let mut a = App::new(
            e.snapshot(),
            now().with_timezone(&chrono_tz::America::New_York),
        );
        select(&mut a, parent, false);
        a.handle_key(key('d'));
        let r = request(a.handle_key(key('d')));
        assert!(
            matches!(r, Request::Apply(twydo_core::edit::Op::Delete { ref uid }) if uid == parent),
            "{r:?}"
        );
        run(&e, &mut a, &w, r);
        for uid in [parent, q, z] {
            assert!(a.snapshot.find(uid).is_none(), "{uid}");
        }
        for uid in [b, b_q, b_z, d] {
            assert!(a.snapshot.find(uid).is_some(), "{uid}");
        }
        let after = item_bytes(&e);
        for uid in [b, b_q, b_z, d] {
            assert_eq!(
                after[&format!("{uid}.ics")],
                original[&format!("{uid}.ics")],
                "{uid}"
            );
        }
        // One undo restores the whole family and one redo removes it again.
        let r = request(a.handle_key(key('u')));
        run(&e, &mut a, &w, r);
        assert_eq!(item_bytes(&e), original);
        let r = request(a.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)));
        run(&e, &mut a, &w, r);
        assert_eq!(item_bytes(&e), after);

        // An empty parent has the same one-resource scope.
        let e = env_in("plan8-checklist-b", "011-phone-items-and-text");
        for uid in [q, z] {
            fs::remove_file(e.vdir().join(format!("list1/{uid}.ics"))).unwrap();
        }
        let empty_original = item_bytes(&e);
        let w = writer(&e.config());
        let mut a = App::new(
            e.snapshot(),
            now().with_timezone(&chrono_tz::America::New_York),
        );
        select(&mut a, parent, false);
        a.handle_key(key('d'));
        let r = request(a.handle_key(key('d')));
        run(&e, &mut a, &w, r);
        assert!(a.snapshot.find(parent).is_none());
        for uid in [b, b_q, b_z, d] {
            assert!(a.snapshot.find(uid).is_some(), "{uid}");
        }
        let r = request(a.handle_key(key('u')));
        run(&e, &mut a, &w, r);
        assert_eq!(item_bytes(&e), empty_original);

        // A completed family is selectable in Done and is removed as a family.
        let e = env_in("issue27-items", "020-phone-complete-project");
        let pa = "twydo-i27-p-a";
        let pa_x = "twydo-i27-p-a-x";
        let ca = "twydo-i27-c-a";
        let ca_x = "twydo-i27-c-a-x";
        let completed_original = item_bytes(&e);
        let w = writer(&e.config());
        let mut a = App::new(
            e.snapshot(),
            now().with_timezone(&chrono_tz::America::New_York),
        );
        select(&mut a, pa, true);
        a.handle_key(key('d'));
        let r = request(a.handle_key(key('d')));
        run(&e, &mut a, &w, r);
        for uid in [pa, pa_x] {
            assert!(a.snapshot.find(uid).is_none(), "{uid}");
        }
        for uid in [ca, ca_x] {
            assert!(a.snapshot.find(uid).is_some(), "{uid}");
        }
        let r = request(a.handle_key(key('u')));
        run(&e, &mut a, &w, r);
        assert_eq!(item_bytes(&e), completed_original);
    }

    #[test]
    fn verified_checklist_item_delete_keys_undo_and_reset_use_exact_targets() {
        let e = env_in("plan8-checklist-b", "011-phone-items-and-text");
        let w = writer(&e.config());
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
        run(&e, &mut a, &w, r);
        assert!(a.snapshot.find(&q).is_none());
        assert!(a.snapshot.find(parent).is_some());
        let r = request(a.handle_key(key('u')));
        run(&e, &mut a, &w, r);
        assert!(a.snapshot.find(&q).is_some());
        let r = request(a.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)));
        run(&e, &mut a, &w, r);
        assert!(a.snapshot.find(&q).is_none());
        let p = a.snapshot.find(parent).unwrap();
        let z = a.snapshot.tasks[a.snapshot.children(p)[0]].uid.clone();
        select(&mut a, &z, false);
        let r = request(a.handle_key(key('x')));
        run(&e, &mut a, &w, r);
        assert_eq!(
            a.snapshot.tasks[a.snapshot.find(parent).unwrap()].status,
            Status::Open
        );
        select(&mut a, parent, false);
        let r = request(a.handle_key(key('x')));
        run(&e, &mut a, &w, r);
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
        run(&e, &mut a, &w, r);
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
        run(&e, &mut a, &w, r);
        let task = &a.snapshot.tasks[a.snapshot.find(parent).unwrap()];
        assert_eq!(task.summary, "renamed");
        assert_eq!(task.rrule.as_deref(), Some("FREQ=MONTHLY"));
        // A day-29 date edit stays outside the scheduling contract and retains typing.
        select(&mut a, parent, false);
        a.handle_key(key('i'));
        a.form.as_mut().unwrap().title = "kept unsaved".into();
        a.form.as_mut().unwrap().due = "2027-01-29".into();
        let r = request(a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        let before = fs::read(e.vdir().join(format!("list1/{parent}.ics"))).unwrap();
        let err = w.run(&r).unwrap_err();
        a.write_finished(Err(err));
        assert_eq!(a.form.as_ref().unwrap().title, "kept unsaved");
        assert_eq!(
            fs::read(e.vdir().join(format!("list1/{parent}.ics"))).unwrap(),
            before
        );
    }

    #[test]
    fn checklist_due_capability_date_keys_card_cancel_and_completion() {
        let e = env("010-first-phone-actions");
        let w = writer(&e.config());
        let mut a = App::new(
            e.snapshot(),
            now().with_timezone(&chrono_tz::America::New_York),
        );
        select(&mut a, FC, false);
        let original = item_bytes(&e);
        a.handle_key(key('i'));
        a.form.as_mut().unwrap().due = "2026-12-11".into();
        a.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(item_bytes(&e), original);
        select(&mut a, FC, false);
        let r = request(a.handle_key(key(']')));
        run(&e, &mut a, &w, r);
        assert_eq!(
            a.snapshot.tasks[a.snapshot.find(FC).unwrap()]
                .due
                .as_ref()
                .unwrap()
                .local_date(a.snapshot.tz())
                .to_string(),
            "2026-12-11"
        );
        let r = request(a.handle_key(key('u')));
        run(&e, &mut a, &w, r);
        assert_eq!(item_bytes(&e), original);
        select(&mut a, FC, false);
        a.handle_key(key('i'));
        a.form.as_mut().unwrap().title = "synthetic scheduled checklist".into();
        a.form.as_mut().unwrap().notes = "preserved mixed save".into();
        a.form.as_mut().unwrap().due = "2026-12-11".into();
        let r = request(a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        run(&e, &mut a, &w, r);
        select(&mut a, FC, false);
        a.handle_key(key('x'));
        let r = request(a.handle_key(key('x')));
        run(&e, &mut a, &w, r);
        assert_eq!(
            a.snapshot.tasks[a.snapshot.find(FC).unwrap()]
                .due
                .as_ref()
                .unwrap()
                .local_date(a.snapshot.tz())
                .to_string(),
            "2027-01-11"
        );
    }

    #[test]
    fn checklist_interval_candidate_refusal_keeps_card_text_and_resources() {
        let e = env("010-first-phone-actions");
        let w = writer(&e.config());
        let mut a = App::new(
            e.snapshot(),
            now().with_timezone(&chrono_tz::America::New_York),
        );
        select(&mut a, FC, false);
        let r = card_repeat(&mut a, "2m");
        let before = item_bytes(&e);
        let err = w.run(&r).unwrap_err();
        a.write_finished(Err(err));
        assert_eq!(a.form.as_ref().unwrap().repeat, "2m");
        assert_eq!(item_bytes(&e), before);
    }

    #[test]
    fn text_capability_closed_checklist_card_saves_text_and_keeps_refused_mixed_input() {
        let e = env_in("plan8-checklist-b", "011-phone-items-and-text");
        let uid = "twydo-p8-c29-b";
        let path = e.vdir().join("list1").join(format!("{uid}.ics"));
        let source = fs::read_to_string(&path).unwrap().replace(
            "STATUS:NEEDS-ACTION",
            "STATUS:COMPLETED\nCOMPLETED:20261210T120000Z",
        );
        fs::write(&path, source).unwrap();
        let before = item_bytes(&e);
        let w = writer(&e.config());
        let mut a = App::new(
            e.snapshot(),
            now().with_timezone(&chrono_tz::America::New_York),
        );
        select(&mut a, uid, true);
        a.handle_key(key('i'));
        a.form.as_mut().unwrap().title = "Edited completed checklist".into();
        a.form.as_mut().unwrap().notes = "New notes".into();
        let r = request(a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        run(&e, &mut a, &w, r);
        let task = &a.snapshot.tasks[a.snapshot.find(uid).unwrap()];
        assert_eq!(task.summary, "Edited completed checklist");
        assert_eq!(task.description.as_deref(), Some("New notes"));
        assert_eq!(task.status, Status::Completed);
        let undo = request(a.handle_key(key('u')));
        run(&e, &mut a, &w, undo);
        assert_eq!(item_bytes(&e), before);
        select(&mut a, uid, true);
        a.handle_key(key('i'));
        a.form.as_mut().unwrap().title = "Keep typed title".into();
        a.form.as_mut().unwrap().due = "2026-12-11".into();
        let r = request(a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        let error = w.run(&r).unwrap_err();
        a.write_finished(Err(error));
        assert_eq!(a.form.as_ref().unwrap().title, "Keep typed title");
        assert_eq!(a.form.as_ref().unwrap().due, "2026-12-11");
        assert_eq!(item_bytes(&e), before);
    }

    #[test]
    fn checklist_schedule_stale_form_keeps_typing_and_item_dates_stay_inherited() {
        let e = env("010-first-phone-actions");
        let w = writer(&e.config());
        let mut a = App::new(
            e.snapshot(),
            now().with_timezone(&chrono_tz::America::New_York),
        );
        let p = a.snapshot.find(FC).unwrap();
        let child = a
            .snapshot
            .children(p)
            .iter()
            .copied()
            .find(|&i| a.snapshot.tasks[i].status == Status::Open)
            .unwrap();
        let child_uid = a.snapshot.tasks[child].uid.clone();
        select(&mut a, FC, false);
        a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        select(&mut a, &child_uid, false);
        let before = item_bytes(&e);
        let r = request(a.handle_key(key(']')));
        assert!(w.run(&r).is_err());
        assert_eq!(item_bytes(&e), before);
        select(&mut a, FC, false);
        a.handle_key(key('i'));
        a.form.as_mut().unwrap().title = "kept unsaved".into();
        a.form.as_mut().unwrap().due = "2026-12-11".into();
        let original = fs::read_to_string(e.vdir().join(format!("list1/{child_uid}.ics"))).unwrap();
        fs::write(
            e.vdir().join("list1/new-synthetic-child.ics"),
            original.replace(&child_uid, "new-synthetic-child"),
        )
        .unwrap();
        let before = item_bytes(&e);
        let r = request(a.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        let err = w.run(&r).unwrap_err();
        a.write_finished(Err(err));
        assert_eq!(a.form.as_ref().unwrap().title, "kept unsaved");
        assert_eq!(a.form.as_ref().unwrap().due, "2026-12-11");
        assert_eq!(item_bytes(&e), before);
    }

    fn item_bytes(e: &Env) -> std::collections::BTreeMap<String, Vec<u8>> {
        fs::read_dir(e.vdir().join("list1"))
            .unwrap()
            .map(|p| p.unwrap().path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "ics"))
            .map(|p| {
                (
                    p.file_name().unwrap().to_str().unwrap().to_owned(),
                    fs::read(p).unwrap(),
                )
            })
            .collect()
    }

    fn item_app(e: &Env, parent: &str) -> App {
        let at = Utc.with_ymd_and_hms(2026, 11, 7, 16, 0, 0).unwrap();
        let mut a = App::new(
            e.snapshot(),
            at.with_timezone(&chrono_tz::America::New_York),
        );
        a.expanded.insert(parent.into());
        a
    }

    #[test]
    fn individual_item_deletion_keys_target_open_completed_and_last_items_and_restore_bytes() {
        for (step, prefix, child, done) in [
            ("001-seeded", "c", "q", false),
            ("001-seeded", "p", "q", false),
            ("020-phone-complete-project", "c", "x", false),
            ("020-phone-complete-project", "p", "x", true),
            ("020-phone-complete-project", "m", "q", false),
            ("020-phone-complete-project", "w", "q", false),
        ] {
            let e = env_in("issue27-items", step);
            let parent = format!("twydo-i27-{prefix}-a");
            let uid = format!("{parent}-{child}");
            let before = item_bytes(&e);
            let mut expected = before.clone();
            expected.remove(&(uid.clone() + ".ics"));
            let w = writer(&e.config());
            let mut a = item_app(&e, &parent);
            select(&mut a, &uid, done);
            assert!(!matches!(a.handle_key(key('d')), Outcome::Write(_)));
            let r = request(a.handle_key(key('d')));
            assert!(
                matches!(&r, Request::Apply(twydo_core::edit::Op::Delete { uid: target }) if target == &uid)
            );
            run(&e, &mut a, &w, r);
            assert_eq!(item_bytes(&e), expected, "{uid}");
            let p = a.snapshot.find(&parent).unwrap();
            assert_eq!(
                a.snapshot.tasks[p].kind,
                if matches!(prefix, "c" | "m") {
                    TaskKind::Checklist
                } else {
                    TaskKind::Project
                }
            );
            // Empty parents remain visible in their eligible view.
            a.handle_key(key('2'));
            assert!(
                a.visible_rows()
                    .iter()
                    .any(|r| a.snapshot.tasks[r.task].uid == parent)
            );
            let r = request(a.handle_key(key('u')));
            run(&e, &mut a, &w, r);
            assert_eq!(item_bytes(&e), before);
            let r = request(a.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)));
            run(&e, &mut a, &w, r);
            assert_eq!(item_bytes(&e), expected);
        }
    }

    #[test]
    fn individual_item_deletion_keys_queue_during_sync_and_undo_after_drain() {
        let e = env_in("issue27-items", "020-phone-complete-project");
        let parent = "twydo-i27-p-a";
        let uid = "twydo-i27-p-a-x";
        let before = item_bytes(&e);
        let w = writer(&e.config());
        let mut a = item_app(&e, parent);
        select(&mut a, uid, true);
        a.handle_key(key('d'));
        let r = request(a.handle_key(key('d')));
        let state = twydo_core::store::vdir_state_dir(&e.config().state_dir(), &e.vdir()).unwrap();
        let lock = twydo_core::store::fsio::VdirLock::exclusive(&state).unwrap();
        let message = w.run(&r).unwrap();
        assert!(message.contains("queued"));
        assert_eq!(item_bytes(&e), before);
        drop(lock);
        wait_drained(&e.config());
        a.reload(w.snapshot());
        let mut expected = before.clone();
        expected.remove(&(uid.to_owned() + ".ics"));
        assert_eq!(item_bytes(&e), expected);
        let r = request(a.handle_key(key('u')));
        run(&e, &mut a, &w, r);
        assert_eq!(item_bytes(&e), before);
    }
}

#[test]
fn capability_move_card_save_is_atomic_and_retains_refused_text() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tdo::app::{App, Outcome};
    let env = Env::new();
    let original = fs::read(env.task_file()).unwrap();
    let tz = chrono_tz::America::New_York;
    let mut app = App::new(env.snapshot(), now().with_timezone(&tz));
    app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
    app.cursor = app
        .visible_rows()
        .iter()
        .position(|r| app.snapshot.tasks[r.task].uid == TASK_A)
        .unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    app.form.as_mut().unwrap().title = "Synthetic moved title".into();
    app.form.as_mut().unwrap().list = "list2".into();
    let Outcome::Write(req) = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("expected card save");
    };
    let w = writer(&env.config());
    let result = w.run(&req);
    assert!(result.is_ok(), "{result:?}");
    assert!(!env.task_file().exists());
    assert!(
        env.snapshot()
            .tasks
            .iter()
            .any(|t| t.collection == "list2" && t.summary == "Synthetic moved title")
    );
    w.run_fresh(&Request::Undo).unwrap();
    assert_eq!(fs::read(env.task_file()).unwrap(), original);
}

#[test]
fn capability_move_refused_schedule_keeps_card_text() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tdo::app::{App, Outcome};
    let env = Env::new();
    let original = fs::read(env.task_file()).unwrap();
    let mut app = App::new(
        env.snapshot(),
        now().with_timezone(&chrono_tz::America::New_York),
    );
    app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
    app.cursor = app
        .visible_rows()
        .iter()
        .position(|r| app.snapshot.tasks[r.task].uid == TASK_A)
        .unwrap();
    app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    let f = app.form.as_mut().unwrap();
    f.title = "Synthetic retained title".into();
    f.list = "list2".into();
    f.due = "2026-11-10".into();
    f.repeat = "daily until 2026-12-01".into();
    let Outcome::Write(req) = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("expected proposed save");
    };
    let w = writer(&env.config());
    let result = w.run(&req);
    assert!(result.is_err(), "{result:?}");
    app.write_finished(result);
    let f = app.form.as_ref().unwrap();
    assert_eq!(f.title, "Synthetic retained title");
    assert_eq!(f.list, "list2");
    assert_eq!(f.repeat, "daily until 2026-12-01");
    assert_eq!(fs::read(env.task_file()).unwrap(), original);
    assert!(
        !fs::read_dir(env.vdir().join("list2")).unwrap().any(|p| p
            .unwrap()
            .path()
            .extension()
            .is_some_and(|e| e == "ics"))
    );
}

/// The text of a stale refusal comes from the client's own titles; any other refusal keeps the
/// daemon's message, and so does a stale one whose task the client does not know.
#[test]
fn refusal_text_names_the_task_by_the_clients_title() {
    use twydo_daemon::protocol::rpc::{ErrorKind, RpcError};
    let mut stale = RpcError::refused(ErrorKind::Stale, "the daemon's words");
    stale.data.as_mut().unwrap().uid = Some("u1".into());
    let title = |uid: &str| (uid == "u1").then(|| "Mine".to_string());
    assert_eq!(
        tdo::write::refusal_text(&stale, title),
        "\"Mine\" changed on disk; reload and try again"
    );
    assert_eq!(
        tdo::write::refusal_text(&stale, |_| None),
        "the daemon's words"
    );
    let other = RpcError::refused(ErrorKind::Blocked, "blocked");
    assert_eq!(tdo::write::refusal_text(&other, title), "blocked");
}

/// Issue #10: a client that connects while a long job holds the lane is served at once.
#[test]
fn a_client_opens_while_a_sync_holds_the_lane() {
    use twydo_daemon::client::Client;
    use twydo_daemon::protocol::methods::{HelloParams, Instance};
    let env = Env::new();
    let cfg = sync_config(&env);
    let w = writer(&cfg);
    let _sync = w.handle.hold_long(LongKind::Test);
    let hello = HelloParams {
        client: "test".into(),
        version: "0".into(),
        protocol: twydo_daemon::PROTOCOL,
        session: None,
        instance: Instance {
            config: w
                .handle
                .socket()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("config.toml"),
            vdir: Some(cfg.vdir.clone()),
        },
    };
    let limit = Duration::from_secs(2);
    let started = Instant::now();
    let (client, _) = Client::connect(w.handle.socket(), hello).expect("hello");
    assert!(
        started.elapsed() < limit,
        "hello took {:?}",
        started.elapsed()
    );
    let started = Instant::now();
    let wire: twydo_daemon::protocol::wire::WireSnapshot =
        client.call("snapshot.get", serde_json::json!({})).unwrap();
    assert!(
        started.elapsed() < limit,
        "snapshot.get took {:?}",
        started.elapsed()
    );
    let snap = twydo_daemon::protocol::wire::from_wire(wire);
    assert!(snap.tasks.iter().any(|t| t.uid == TASK_A));
    let started = Instant::now();
    let _: serde_json::Value = client.call("status.get", serde_json::json!({})).unwrap();
    assert!(
        started.elapsed() < limit,
        "status.get took {:?}",
        started.elapsed()
    );
}
