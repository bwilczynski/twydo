//! The TUI loop on the link, driven without a terminal against an in-process daemon. Every
//! test has its own socket; content is synthetic and no real vdirsyncer runs.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::Utc;
use common::TestEnv;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use tdo::app::{App, SYNC_DONE};
use tdo::term::link::{Connector, Link};
use tdo::term::{Flow, Session, first_app, step, unit_notice};
use twydo_daemon::client::Client;
use twydo_daemon::protocol::methods::{HelloParams, Instance};
use twydo_daemon::server::lane::LongKind;

const BOUND: Duration = Duration::from_secs(20);

fn task_ics(uid: &str, summary: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\nUID:{uid}\r\n\
         SUMMARY:{summary}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
    )
}

/// A link that connects to `socket` as `instance`; no daemon is started for it and no
/// configuration is read.
fn link_to(socket: PathBuf, instance: Instance) -> Link {
    let connect: Connector = Box::new(move |session| {
        let hello = HelloParams {
            client: "tui-test".into(),
            version: "0".into(),
            protocol: twydo_daemon::PROTOCOL,
            session,
            instance: instance.clone(),
        };
        Client::connect(&socket, hello)
            .map_err(|e| tdo::client::ConnectError::Unavailable(e.to_string()))
    });
    Link::start_with(connect)
}

/// An app on the first snapshot of the daemon for `env`, and its session.
fn open(env: &TestEnv) -> (App, Session) {
    let link = link_to(env.socket(), env.instance());
    let app = first_app(&link, None, BOUND);
    let session = Session::new(link, &app);
    (app, session)
}

fn now() -> chrono::DateTime<chrono_tz::Tz> {
    Utc::now().with_timezone(&chrono_tz::UTC)
}

fn key(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

/// Steps the loop until `done` holds, within [`BOUND`].
fn pump(app: &mut App, session: &mut Session, what: &str, done: impl Fn(&App) -> bool) {
    let end = Instant::now() + BOUND;
    while !done(app) {
        assert!(
            Instant::now() < end,
            "never: {what}; message {:?}",
            app.message
        );
        step(app, session, None, now());
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn tui_keeps_running_while_the_daemon_is_down() {
    let env = TestEnv::new("");
    fs::write(
        env.vdir().join("inbox/a.ics"),
        task_ics("uid-a", "Synthetic"),
    )
    .unwrap();
    let daemon = env.in_process_daemon(None);
    let (mut app, mut session) = open(&env);
    assert_eq!(app.snapshot.tasks.len(), 1);

    daemon.shutdown();
    pump(&mut app, &mut session, "the down message", |a| {
        a.message
            .as_deref()
            .is_some_and(|m| m.contains("daemon not running"))
    });
    for _ in 0..5 {
        let t = Instant::now();
        assert_eq!(
            step(&mut app, &mut session, Some(key('j')), now()),
            Flow::Continue
        );
        assert!(t.elapsed() < Duration::from_millis(50), "{:?}", t.elapsed());
    }
    assert_eq!(
        app.snapshot.tasks.len(),
        1,
        "the rows stay while it is down"
    );

    let _again = env.in_process_daemon(None);
    pump(&mut app, &mut session, "the connection is back", |a| {
        a.message
            .as_deref()
            .is_none_or(|m| !m.contains("daemon not running"))
    });
    assert_eq!(app.message.as_deref(), Some("daemon restarted"));
    assert_eq!(app.snapshot.tasks.len(), 1);
}

#[test]
fn a_list_preference_change_reaches_the_tui_without_a_restart() {
    let env = TestEnv::new("");
    let _daemon = env.in_process_daemon(None);
    let (mut app, mut session) = open(&env);
    assert!(app.snapshot.config.lists.is_empty());
    let before = app.sidebar_items();

    let mut text = fs::read_to_string(env.config_path()).unwrap();
    text.push_str("[lists.inbox]\nhidden = true\n");
    fs::write(env.config_path(), text).unwrap();

    pump(&mut app, &mut session, "the hidden preference", |a| {
        a.snapshot
            .config
            .lists
            .get("inbox")
            .is_some_and(|p| p.hidden)
    });
    assert_ne!(
        app.sidebar_items(),
        before,
        "the sidebar follows the preference"
    );
}

#[test]
fn a_binding_change_tells_the_user_to_restart_the_daemon() {
    let env = TestEnv::new("");
    let _daemon = env.in_process_daemon(None);
    let (mut app, mut session) = open(&env);

    let text = fs::read_to_string(env.config_path()).unwrap();
    fs::write(
        env.config_path(),
        text.replace("mode = \"test\"", "mode = \"verified\""),
    )
    .unwrap();

    pump(&mut app, &mut session, "the restart note", |a| {
        a.message
            .as_deref()
            .is_some_and(|m| m.contains("restart the daemon"))
    });
    let shown = app.message.clone().unwrap();
    assert!(
        shown.contains("restart the daemon (tdo daemon stop) and reopen tdo"),
        "{shown}"
    );
    assert!(shown.contains("writes"), "{shown}");
}

#[test]
fn refusals_are_shown_once_and_acknowledged() {
    let env = TestEnv::new("");
    let file = env.vdir().join("inbox/a.ics");
    fs::write(&file, task_ics("uid-a", "Synthetic")).unwrap();
    let daemon = env.in_process_daemon(None);
    let (mut app, mut session) = open(&env);
    // The task is in the "All" view; completing it during a long job queues the edit.
    app.view = twydo_core::views::View::All;

    let job = daemon.hold_long(LongKind::Test);
    step(&mut app, &mut session, Some(key('x')), now());
    assert!(
        app.message.as_deref().is_some_and(|m| m.contains("queued")),
        "{:?}",
        app.message
    );
    // Another device changes the task before the queue is written.
    fs::write(&file, task_ics("uid-a", "From phone")).unwrap();
    drop(job);

    let refusal = "Not saved: \"Synthetic\" changed on another device";
    pump(&mut app, &mut session, "the refusal", |a| {
        a.message.as_deref().is_some_and(|m| m.contains(refusal))
    });
    // The loop acknowledged it: the daemon's status no longer lists it.
    let c = env.connect(&env.socket());
    let end = Instant::now() + BOUND;
    loop {
        let st: twydo_daemon::protocol::methods::StatusView =
            c.call("status.get", serde_json::json!({})).unwrap();
        if st.refusals.is_empty() {
            break;
        }
        step(&mut app, &mut session, None, now());
        assert!(Instant::now() < end, "never acknowledged: {st:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
    // Shown once: after the message is replaced, later status updates do not bring it back.
    app.message = Some("something else".into());
    for _ in 0..20 {
        step(&mut app, &mut session, None, now());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(app.message.as_deref(), Some("something else"));
}

#[test]
fn tui_never_opens_the_config_file() {
    let env = TestEnv::new("");
    let file = env.vdir().join("inbox/a.ics");
    fs::write(&file, task_ics("uid-a", "Synthetic")).unwrap();
    let _daemon = env.in_process_daemon(None);
    fs::set_permissions(env.config_path(), fs::Permissions::from_mode(0o000)).unwrap();

    let (mut app, mut session) = open(&env);
    assert_eq!(app.snapshot.tasks.len(), 1);
    app.view = twydo_core::views::View::All;
    step(&mut app, &mut session, Some(key('x')), now());
    let saved = fs::read_to_string(&file).unwrap();
    assert!(saved.contains("STATUS:COMPLETED"), "{saved}");
    pump(&mut app, &mut session, "the completed task", |a| {
        a.snapshot.tasks[0].status == twydo_core::model::Status::Completed
    });
    fs::set_permissions(env.config_path(), fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn a_write_answer_follows_its_snapshot_so_the_new_row_is_followed() {
    let env = TestEnv::new("");
    fs::write(
        env.vdir().join("inbox/a.ics"),
        task_ics("uid-a", "Synthetic"),
    )
    .unwrap();
    let _daemon = env.in_process_daemon(None);
    let (mut app, mut session) = open(&env);
    app.view = twydo_core::views::View::List("inbox".into());
    let revision = app.snapshot_revision;
    // `o` opens the new row; type a title; Enter saves.
    for e in [key('o'), key('N'), key('e'), key('w')] {
        step(&mut app, &mut session, Some(e), now());
    }
    step(
        &mut app,
        &mut session,
        Some(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        ))),
        now(),
    );
    assert!(
        app.snapshot_revision > revision,
        "the snapshot came with the answer"
    );
    assert_eq!(app.snapshot.tasks.len(), 2);
    let selected = app
        .selected()
        .map(|i| app.snapshot.tasks[i].summary.clone());
    assert_eq!(selected.as_deref(), Some("New"));
    assert_ne!(app.message.as_deref(), Some(SYNC_DONE));
}

#[test]
fn warns_once_without_a_unit() {
    let env = TestEnv::new("");
    let _daemon = env.in_process_daemon(None);
    let (mut app, _session) = open(&env);
    unit_notice(&mut app, true);
    assert_eq!(app.message, None, "a unit is installed");
    unit_notice(&mut app, false);
    assert_eq!(
        app.message.as_deref(),
        Some("daemon won't start at login; run tdo daemon install")
    );
}

#[test]
fn the_warning_can_be_switched_off() {
    let env = TestEnv::new("[daemon]\nunit_warning = false\n");
    let _daemon = env.in_process_daemon(None);
    let (mut app, _session) = open(&env);
    assert!(!app.snapshot.config.daemon.unit_warning);
    unit_notice(&mut app, false);
    assert_eq!(app.message, None);
}

#[test]
fn the_warning_switch_reaches_the_tui_live() {
    let env = TestEnv::new("");
    let _daemon = env.in_process_daemon(None);
    let (mut app, mut session) = open(&env);
    assert!(app.snapshot.config.daemon.unit_warning);

    let mut text = fs::read_to_string(env.config_path()).unwrap();
    text.push_str("[daemon]\nunit_warning = false\n");
    fs::write(env.config_path(), text).unwrap();

    pump(&mut app, &mut session, "the switch", |a| {
        !a.snapshot.config.daemon.unit_warning
    });
}
