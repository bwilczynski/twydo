//! Client write adaptation and representative TUI workflows against synthetic temp vdirs.
//! Exhaustive write semantics belong to twydo-core; queue/lifecycle semantics to the daemon.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use common::WriteDaemon;
use tdo::write::Request;
use twydo_core::compat::Matrix;
use twydo_core::config::{Config, SyncConfig};
use twydo_core::edit::Op;
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
    WriteDaemon::start(cfg.clone(), Matrix::embedded().clone())
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
    use super::{Env, WriteDaemon, now, writer};
    use chrono::{TimeZone, Utc};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::fs;
    use std::path::Path;
    use tdo::app::{App, Outcome};
    use tdo::write::Request;
    use twydo_core::config::WriteMode;
    use twydo_core::model::{Status, TaskKind};

    const FC: &str = "61ed74be3ba34212ab7476b698de1263";
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
    fn completed_item_delete_and_undo_go_through_the_writer() {
        let (step, prefix, child, done) = ("020-phone-complete-project", "p", "x", true);
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
