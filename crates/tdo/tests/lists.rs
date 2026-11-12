//! Real list-card keys and list jobs run by an in-process daemon; all collections are synthetic.
mod common;

use chrono::{TimeZone, Utc};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};
use std::fs;
use std::time::Duration;
use tdo::app::{App, ListAction, ListCompletion, Outcome, Pane};
use tdo::listform::ListField;
use tdo::term::link::{Command, Connector, Link, Update};
use twydo_core::{
    config::Config,
    views::{Snapshot, View},
};
use twydo_daemon::client::Client;
use twydo_daemon::protocol::methods::{HelloParams, Instance};
use twydo_daemon::server::{Daemon, DaemonHandle, DaemonOptions};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn command(app: &mut App, text: &str) -> Outcome {
    app.handle_key(key(KeyCode::Char(':')));
    for c in text.chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter))
}
/// An in-process daemon for the fixture's configuration, and a link to it.
struct Served {
    _daemon: DaemonHandle,
    _sockets: tempfile::TempDir,
    link: Link,
}

fn serve(cfg: &Config, path: &std::path::Path) -> Served {
    let sockets = common::short_socket_dir();
    let socket = sockets.path().join("s").join("tdo.sock");
    let mut opts = DaemonOptions::new(socket.clone());
    opts.schedule = false;
    let instance = Instance {
        config: path.to_path_buf(),
        vdir: Some(cfg.vdir.clone()),
    };
    let daemon = Daemon::start(cfg.clone(), instance.clone(), opts).expect("start");
    assert!(daemon.wait_ready(Duration::from_secs(10)), "not ready");
    let connect: Connector = Box::new(move |session| {
        let hello = HelloParams {
            client: "lists-test".into(),
            version: "0".into(),
            protocol: twydo_daemon::PROTOCOL,
            session,
            instance: instance.clone(),
        };
        Client::connect(&socket, hello)
            .map_err(|e| tdo::client::ConnectError::Unavailable(e.to_string()))
    });
    let link = Link::start_with(connect);
    let end = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let left = end.saturating_duration_since(std::time::Instant::now());
        match link.next_update(left) {
            Some(Update::Status(_)) => break,
            Some(_) => {}
            None => panic!("the link never connected"),
        }
    }
    Served {
        _daemon: daemon,
        _sockets: sockets,
        link,
    }
}

impl Served {
    /// Runs a list job through the link and waits for its completion.
    fn run(&self, action: ListAction) -> ListCompletion {
        self.link.send(Command::List(action));
        let end = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            let left = end.saturating_duration_since(std::time::Instant::now());
            match self.link.next_update(left) {
                Some(Update::ListDone(c)) => return c,
                Some(_) => {}
                None => panic!("the list job never finished"),
            }
        }
    }
}

fn setup() -> (tempfile::TempDir, Config, std::path::PathBuf, App) {
    let d = tempfile::tempdir().unwrap();
    let vdir = d.path().join("vdir");
    for (id, name) in [("a", "Same"), ("b", "Same"), ("c", "Hidden")] {
        fs::create_dir_all(vdir.join(id)).unwrap();
        fs::write(vdir.join(id).join("displayname"), name).unwrap();
    }
    fs::write(vdir.join("a/open.ics"), "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:synthetic-open\r\nSUMMARY:Synthetic open task\r\nDUE;VALUE=DATE:20261108\r\nSTATUS:NEEDS-ACTION\r\nEND:VTODO\r\nEND:VCALENDAR\r\n").unwrap();
    let path = d.path().join("config.toml");
    fs::write(&path,format!("vdir = {:?}\nstate_dir = {:?}\ntimezone = \"UTC\"\n[sync]\npair = \"synthetic\"\nlocal_storage = \"local\"\n[lists.b]\narchived = true\n[lists.c]\nhidden = true\n",vdir.to_str().unwrap(),d.path().join("state").to_str().unwrap())).unwrap();
    let cfg = Config::load(&path).unwrap();
    let app = App::new(
        Snapshot::new(twydo_core::vdir::load(&vdir).unwrap(), cfg.clone()),
        Utc.with_ymd_and_hms(2026, 11, 8, 12, 0, 0)
            .unwrap()
            .with_timezone(&chrono_tz::UTC),
    );
    (d, cfg, path, app)
}
fn select(app: &mut App, id: &str) {
    app.focus = Pane::Sidebar;
    app.sidebar_cursor = app
        .sidebar_items()
        .iter()
        .position(|v| *v == View::List(id.into()))
        .unwrap();
}
fn render(app: &App) -> String {
    let mut term = Terminal::new(TestBackend::new(120, 32)).unwrap();
    term.draw(|frame| tdo::ui::draw(frame, app)).unwrap();
    term.backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>()
}

#[test]
fn sidebar_card_create_edit_and_cancel_do_not_touch_task_data() {
    let (_d, cfg, path, mut app) = setup();
    let config = fs::read(&path).unwrap();
    let task = fs::read(cfg.vdir.join("a/open.ics")).unwrap();
    select(&mut app, "a");
    app.handle_key(key(KeyCode::Char('o')));
    assert!(app.list_form.as_ref().unwrap().original.is_none());
    app.handle_key(key(KeyCode::Esc));
    app.handle_key(key(KeyCode::Char('i')));
    assert_eq!(
        app.list_form
            .as_ref()
            .unwrap()
            .original
            .as_ref()
            .unwrap()
            .id,
        "a"
    );
    assert!(app.form.is_none());
    let text = render(&app);
    assert!(
        text.contains("Save to CalDAV") && text.contains("this desktop"),
        "{text}"
    );
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(fs::read(&path).unwrap(), config);
    assert_eq!(fs::read(cfg.vdir.join("a/open.ics")).unwrap(), task);
}

#[test]
fn card_explains_best_effort_overwrite_risk() {
    let (_d, _cfg, _path, mut app) = setup();
    select(&mut app, "a");
    app.handle_key(key(KeyCode::Char('i')));
    let text = render(&app).replace(['│', ' '], "");
    assert!(
        text.contains("canoverwriteasimultaneouschangemadeonanotherdevice"),
        "{text}"
    );
}

#[test]
fn focus_views_cannot_be_edited_as_lists() {
    let (_d, _cfg, _path, mut app) = setup();
    app.focus = Pane::Sidebar;
    app.sidebar_cursor = 0;
    app.handle_key(key(KeyCode::Char('i')));
    assert!(app.list_form.is_none() && app.form.is_none());
}

#[test]
fn manager_includes_archived_hidden_and_disambiguated_lists() {
    let (_d, _cfg, _path, mut app) = setup();
    command(&mut app, "lists");
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.targets.len(), 3);
    let text = render(&app);
    assert!(
        text.contains("archived")
            && text.contains("hidden")
            && text.contains("(a)")
            && text.contains("(b)"),
        "{text}"
    );
    app.picker.as_mut().unwrap().cursor = app
        .picker
        .as_ref()
        .unwrap()
        .targets
        .iter()
        .position(|t| t.collection() == "b")
        .unwrap();
    app.handle_key(key(KeyCode::Enter));
    let form = app.list_form.as_mut().unwrap();
    assert!(form.original.as_ref().unwrap().archived);
    form.focus = ListField::Archive;
    assert!(matches!(
        app.handle_key(key(KeyCode::Enter)),
        Outcome::ManageList(ListAction::Archive {
            archived: false,
            ..
        })
    ));
}

#[test]
fn archive_warning_cancels_cleanly_then_persists_locally_without_saving_rename() {
    let (_d, cfg, path, mut app) = setup();
    select(&mut app, "a");
    app.view = View::List("a".into());
    app.handle_key(key(KeyCode::Char('i')));
    let form = app.list_form.as_mut().unwrap();
    form.name = "Unsaved rename".into();
    form.focus = ListField::Archive;
    app.handle_key(key(KeyCode::Enter));
    assert!(
        app.confirmation
            .as_ref()
            .unwrap()
            .question()
            .contains("unfinished")
    );
    app.handle_key(key(KeyCode::Char('n')));
    assert!(!Config::load(&path).unwrap().list_prefs("a").archived);
    app.handle_key(key(KeyCode::Enter));
    let Outcome::ManageList(action) = app.handle_key(key(KeyCode::Char('y'))) else {
        panic!("expected archive worker")
    };
    assert!(app.list_form.as_ref().unwrap().busy);
    let result = serve(&cfg, &path).run(action);
    assert!(result.result.is_ok(), "{:?}", result.result);
    app.list_finished(result);
    assert!(Config::load(&path).unwrap().list_prefs("a").archived);
    assert_eq!(app.view, View::Today);
    assert_eq!(app.list_form.as_ref().unwrap().name, "Unsaved rename");
    assert!(!app.list_form.as_ref().unwrap().busy);
    assert_eq!(
        fs::read_to_string(cfg.vdir.join("a/displayname")).unwrap(),
        "Same"
    );
    assert!(cfg.vdir.join("a/open.ics").exists());
}

#[test]
fn custom_color_is_validated_and_a_refused_save_retains_input() {
    let (_d, cfg, path, mut app) = setup();
    select(&mut app, "a");
    app.handle_key(key(KeyCode::Char('o')));
    let form = app.list_form.as_mut().unwrap();
    form.name = "Synthetic new".into();
    form.color = "#Ab12Cd".into();
    let Outcome::ManageList(action) =
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
    else {
        panic!("expected save")
    };
    assert!(matches!(
        &action,
        ListAction::Save(twydo_core::lists::ListRequest::Create {
            color: (171, 18, 205),
            ..
        })
    ));
    let result = serve(&cfg, &path).run(action);
    assert!(result.result.is_err());
    app.list_finished(result);
    let form = app.list_form.as_ref().unwrap();
    assert_eq!(form.name, "Synthetic new");
    assert_eq!(form.color, "#Ab12Cd");
    // List creation is verified now; this save is refused later, by the collection
    // helper, because the fixture has no usable helper or vdirsyncer pair. The reason
    // depends on the machine's Python, so only the refusal itself is asserted.
    assert!(form.error.as_ref().is_some_and(|e| !e.is_empty()));
}

#[test]
fn blank_name_and_invalid_color_do_not_emit_a_worker_request() {
    let (_d, _cfg, _path, mut app) = setup();
    select(&mut app, "a");
    app.handle_key(key(KeyCode::Char('o')));
    assert_eq!(app.handle_key(key(KeyCode::Enter)), Outcome::Continue);
    assert!(app.list_form.as_ref().unwrap().error.is_some());
    let form = app.list_form.as_mut().unwrap();
    form.name = "New".into();
    form.color = "red".into();
    assert_eq!(app.handle_key(key(KeyCode::Enter)), Outcome::Continue);
    assert!(!app.list_form.as_ref().unwrap().busy);
}

#[test]
fn external_archive_reload_preserves_card_input_and_moves_out_of_archived_view() {
    let (_d, cfg, path, mut app) = setup();
    select(&mut app, "a");
    app.view = View::List("a".into());
    app.handle_key(key(KeyCode::Char('i')));
    app.list_form.as_mut().unwrap().name = "Held input".into();
    twydo_core::lists::prefs::set_archived(&path, &cfg.vdir, "a", true, &fs::read(&path).unwrap())
        .unwrap();
    // The daemon applies the saved preferences and sends the snapshot (cause `config`).
    let next = twydo_core::config::Config::load(&path).unwrap();
    app.reload(Snapshot::new(
        twydo_core::vdir::load(&cfg.vdir).unwrap(),
        next,
    ));
    assert_eq!(app.view, View::Today);
    assert_eq!(app.list_form.as_ref().unwrap().name, "Held input");
}

#[test]
fn manager_recovers_pending_create_even_before_a_local_list_exists() {
    let (_d, cfg, _path, mut app) = setup();
    let mut request = twydo_core::lists::helper::HelperRequest::probe(&cfg).unwrap();
    request.action = "create".into();
    request.collection = Some("pending-id".into());
    request.name = Some("Pending synthetic".into());
    request.color = Some("#3289D2".into());
    let operation = twydo_core::lists::operation::prepare(&cfg, request, Utc::now()).unwrap();
    command(&mut app, "lists");
    let form = app.list_form.as_ref().unwrap();
    assert_eq!(form.name, "Pending synthetic");
    assert_eq!(form.operation.as_deref(), Some(operation.id.as_str()));
    assert_eq!(form.focus, ListField::Reconcile);
    assert!(app.picker.is_none());
}

#[test]
fn busy_card_cannot_be_replaced_or_changed_by_keys_and_paste() {
    let (_d, _cfg, _path, mut app) = setup();
    select(&mut app, "a");
    app.handle_key(key(KeyCode::Char('o')));
    let form = app.list_form.as_mut().unwrap();
    form.name = "Synthetic new".into();
    assert!(matches!(
        app.handle_key(key(KeyCode::Enter)),
        Outcome::ManageList(_)
    ));
    let held = app.list_form.clone();
    for code in [
        KeyCode::Esc,
        KeyCode::Tab,
        KeyCode::Char('o'),
        KeyCode::Enter,
    ] {
        assert_eq!(app.handle_key(key(code)), Outcome::Continue);
    }
    app.paste("injected\ntext");
    assert_eq!(app.list_form, held);
}

#[test]
fn sidebar_delete_previews_before_confirmation_and_cancel_sends_no_delete() {
    use twydo_core::lists::{ListResult, backup::DeletePreview};
    let (_d, cfg, _path, mut app) = setup();
    select(&mut app, "a");
    app.handle_key(key(KeyCode::Char('d')));
    let Outcome::ManageList(action) = app.handle_key(key(KeyCode::Char('d'))) else {
        panic!("preview worker")
    };
    assert!(matches!(action,ListAction::PreviewDelete{ref id} if id=="a"));
    let preview = DeletePreview {
        backup: "synthetic-backup".into(),
        collection: "a".into(),
        name: "Same".into(),
        resources: 2,
        local_files: 2,
    };
    app.list_finished(tdo::app::ListCompletion {
        action,
        result: Ok(ListResult::DeletePreview(preview.clone())),
        pending: None,
        server_values: None,
    });
    assert!(render(&app).contains("ALL DEVICES"));
    assert!(render(&app).contains("synthetic-backup"));
    assert_eq!(app.handle_key(key(KeyCode::Esc)), Outcome::Continue);
    assert!(app.confirmation.is_none());
    app.confirmation = Some(tdo::app::Confirmation::DeleteList(preview.clone()));
    let Outcome::ManageList(action) = app.handle_key(key(KeyCode::Char('y'))) else {
        panic!("delete worker")
    };
    // Only the confirmed preview is sent; the service runs it (see the daemon's delete tests).
    assert_eq!(action, ListAction::Delete(preview));
    assert!(cfg.vdir.join("a/open.ics").exists());
}

fn open_delete_confirmation(app: &mut App, id: &str) -> twydo_core::lists::backup::DeletePreview {
    use twydo_core::lists::{ListResult, backup::DeletePreview};
    select(app, id);
    app.handle_key(key(KeyCode::Char('d')));
    let Outcome::ManageList(action) = app.handle_key(key(KeyCode::Char('d'))) else {
        panic!("preview worker")
    };
    let preview = DeletePreview {
        backup: "synthetic-backup".into(),
        collection: id.into(),
        name: "Same".into(),
        resources: 2,
        local_files: 2,
    };
    app.list_finished(ListCompletion {
        action,
        result: Ok(ListResult::DeletePreview(preview.clone())),
        pending: None,
        server_values: None,
    });
    preview
}

#[test]
fn delete_confirmation_defaults_to_cancel_and_states_best_effort() {
    for code in [KeyCode::Enter, KeyCode::Esc, KeyCode::Char('n')] {
        let (_d, _cfg, _path, mut app) = setup();
        open_delete_confirmation(&mut app, "a");
        let text = render(&app);
        assert!(text.contains("Best-effort"), "{text}");
        assert!(text.contains("exported but not yet restored"), "{text}");
        assert_eq!(app.handle_key(key(code)), Outcome::Continue, "{code:?}");
        assert!(app.confirmation.is_none(), "{code:?}");
    }
}

#[test]
fn deleting_the_open_list_returns_to_today_and_drops_it_from_the_sidebar() {
    use twydo_core::lists::ListResult;
    let (_d, _cfg, _path, mut app) = setup();
    app.view = View::List("a".into());
    let preview = open_delete_confirmation(&mut app, "a");
    app.confirmation = Some(tdo::app::Confirmation::DeleteList(preview.clone()));
    let Outcome::ManageList(action) = app.handle_key(key(KeyCode::Char('y'))) else {
        panic!("delete worker")
    };
    app.list_finished(ListCompletion {
        action,
        result: Ok(ListResult::Deleted {
            operation: "op".into(),
            collection: "a".into(),
            name: "Same".into(),
            backup: "synthetic-backup".into(),
            retired: String::new(),
        }),
        pending: None,
        server_values: None,
    });
    assert_eq!(app.view, View::Today);
    assert!(!app.sidebar_items().contains(&View::List("a".into())));
    assert!(app.snapshot.collections.iter().all(|c| c.id != "a"));
    assert!(app.list_form.is_none() && app.confirmation.is_none());
    assert_eq!(
        app.message.as_deref(),
        Some("Deleted Same from CalDAV; backup synthetic-backup kept")
    );
}

#[test]
fn changed_since_backup_refusal_is_shown_and_dd_previews_again() {
    let (_d, _cfg, _path, mut app) = setup();
    let preview = open_delete_confirmation(&mut app, "a");
    app.confirmation = Some(tdo::app::Confirmation::DeleteList(preview));
    let Outcome::ManageList(action) = app.handle_key(key(KeyCode::Char('y'))) else {
        panic!("delete worker")
    };
    app.list_finished(ListCompletion {
        action,
        result: Err("list changed since backup; review deletion again".into()),
        pending: None,
        server_values: None,
    });
    assert!(render(&app).contains("changed since backup"));
    // The card stays open on the Delete field: Enter previews again, as does closing the
    // card and pressing dd.
    let Outcome::ManageList(again) = app.handle_key(key(KeyCode::Enter)) else {
        panic!("a fresh preview")
    };
    assert!(matches!(again, ListAction::PreviewDelete { ref id } if id == "a"));
    app.list_finished(ListCompletion {
        action: again,
        result: Err("list changed since backup; review deletion again".into()),
        pending: None,
        server_values: None,
    });
    app.handle_key(key(KeyCode::Esc));
    assert!(app.list_form.is_none());
    app.handle_key(key(KeyCode::Char('d')));
    let Outcome::ManageList(again) = app.handle_key(key(KeyCode::Char('d'))) else {
        panic!("a fresh preview")
    };
    assert!(matches!(again, ListAction::PreviewDelete { ref id } if id == "a"));
}

#[test]
fn collection_backup_browser_is_separate_and_export_is_explicit() {
    let (_d, _cfg, _path, mut app) = setup();
    command(&mut app, "list-backups");
    assert!(render(&app).contains("Collection backups"));
    app.handle_key(key(KeyCode::Esc));
    let Outcome::ManageList(ListAction::ExportBackup {
        backup,
        destination,
    }) = command(
        &mut app,
        "export-list-backup synthetic-id /tmp/synthetic export",
    )
    else {
        panic!("export worker")
    };
    assert_eq!(backup, "synthetic-id");
    assert_eq!(
        destination,
        std::path::PathBuf::from("/tmp/synthetic export")
    );
}

#[test]
fn unrelated_export_or_list_completion_cannot_unlock_a_busy_card() {
    let (_d, _cfg, _path, mut app) = setup();
    select(&mut app, "a");
    app.handle_key(key(KeyCode::Char('o')));
    app.list_form.as_mut().unwrap().name = "New synthetic".into();
    assert!(matches!(
        app.handle_key(key(KeyCode::Enter)),
        Outcome::ManageList(_)
    ));
    let held = app.list_form.clone();
    for action in [
        ListAction::ExportBackup {
            backup: "old-backup".into(),
            destination: "/tmp/synthetic-export".into(),
        },
        ListAction::PreviewDelete {
            id: "different-list".into(),
        },
    ] {
        app.list_finished(tdo::app::ListCompletion {
            action,
            result: Err("Unrelated request failed".into()),
            pending: None,
            server_values: None,
        });
        assert_eq!(app.list_form, held);
        assert!(
            app.message
                .as_ref()
                .unwrap()
                .contains("Unrelated request failed")
        );
    }
}

#[test]
fn a_list_save_through_the_daemon_reports_its_pending_operation() {
    let (_d, cfg, path, mut app) = setup();
    let mut request = twydo_core::lists::helper::HelperRequest::probe(&cfg).unwrap();
    request.action = "create".into();
    request.collection = Some("pending-id".into());
    request.name = Some("Pending synthetic".into());
    request.color = Some("#3289D2".into());
    let operation = twydo_core::lists::operation::prepare(&cfg, request, Utc::now()).unwrap();
    // The daemon starts over the unfinished save, so the card can reconcile it.
    let served = serve(&cfg, &path);
    app.lookup = Box::new(tdo::app::DaemonLookup(served.link.caller_source()));
    command(&mut app, "lists");
    let form = app.list_form.as_ref().expect("the pending card opens");
    assert_eq!(form.name, "Pending synthetic");
    assert_eq!(form.id, "pending-id");
    assert_eq!(form.operation.as_deref(), Some(operation.id.as_str()));
    assert_eq!(form.focus, ListField::Reconcile);
    assert!(app.picker.is_none());
    // The card's reconcile runs through the link as a job and reports back.
    let done = served.run(ListAction::Reconcile {
        operation: operation.id.clone(),
        keep_server: true,
    });
    assert!(
        matches!(&done.action, ListAction::Reconcile { operation: o, .. } if *o == operation.id)
    );
}
