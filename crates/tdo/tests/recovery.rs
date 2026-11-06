use chrono::{TimeZone, Utc};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::fs;
use tdo::app::{App, Outcome};
use tdo::write::Request;
use twydo_core::{config::Config, retired, views::Snapshot};

fn key(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
}
fn command(a: &mut App, s: &str) -> Outcome {
    a.handle_key(key(KeyCode::Char(':')));
    for c in s.chars() {
        a.handle_key(key(KeyCode::Char(c)));
    }
    a.handle_key(key(KeyCode::Enter))
}
fn env() -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::with_vdir(dir.path().join("vdir"));
    cfg.state_dir = Some(dir.path().join("state"));
    fs::create_dir_all(cfg.vdir.join("old")).unwrap();
    fs::create_dir_all(cfg.vdir.join("inbox")).unwrap();
    fs::write(cfg.vdir.join("old/displayname"), "Old Projects").unwrap();
    fs::write(cfg.vdir.join("inbox/displayname"), "Personal Tasks").unwrap();
    fs::write(cfg.vdir.join("old/task.ics"),"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:synthetic\r\nSUMMARY:Keep me\r\nDESCRIPTION:Keep my notes\r\nEND:VTODO\r\nEND:VCALENDAR\r\n").unwrap();
    (dir, cfg)
}
fn app(cfg: &Config) -> App {
    App::new(
        Snapshot::new(twydo_core::vdir::load(&cfg.vdir).unwrap(), cfg.clone()),
        Utc.with_ymd_and_hms(2026, 11, 5, 12, 0, 0)
            .unwrap()
            .with_timezone(&chrono_tz::UTC),
    )
}

#[test]
fn forget_requires_confirmation_and_cancel_writes_nothing() {
    let (_dir, cfg) = env();
    let mut a = app(&cfg);
    assert_eq!(
        command(&mut a, "forget-list Old Projects"),
        Outcome::Continue
    );
    assert!(a.confirmation.is_some());
    assert!(cfg.vdir.join("old/task.ics").exists());
    assert_eq!(a.handle_key(key(KeyCode::Char('n'))), Outcome::Continue);
    assert!(a.confirmation.is_none());
    assert!(retired::list(&cfg).unwrap().is_empty());
    command(&mut a, "forget-list old");
    let Outcome::ForgetList(p) = a.handle_key(key(KeyCode::Char('y'))) else {
        panic!("expected confirmed retirement")
    };
    assert_eq!(p.collection, "old");
    assert_eq!(p.name, "Old Projects");
    assert!(
        cfg.vdir.join("old/task.ics").exists(),
        "the event loop performs the write"
    );
}

#[test]
fn recover_destination_argument_keeps_spaces_and_confirms_before_write() {
    let (_dir, cfg) = env();
    let b = retired::forget(&cfg, &retired::preview(&cfg, "old").unwrap(), Utc::now()).unwrap();
    let mut a = app(&cfg);
    command(&mut a, &format!("recover {} Personal Tasks", b.id));
    assert!(a.confirmation.is_some(), "{:?}", a.message);
    assert!(fs::read_dir(cfg.vdir.join("inbox")).unwrap().count() == 1);
    let Outcome::Write(Request::Recover { backup, to }) = a.handle_key(key(KeyCode::Char('y')))
    else {
        panic!("expected recovery request")
    };
    assert_eq!(backup, b.id);
    assert_eq!(to, "inbox");
}

#[test]
fn recover_browser_previews_notes_and_can_choose_destination() {
    let (_dir, cfg) = env();
    retired::forget(&cfg, &retired::preview(&cfg, "old").unwrap(), Utc::now()).unwrap();
    let mut a = app(&cfg);
    command(&mut a, "recover");
    assert!(a.recovery.is_some());
    a.handle_key(key(KeyCode::Enter));
    assert!(a.recovery.as_ref().unwrap().preview.is_some());
    a.handle_key(key(KeyCode::Char('r')));
    assert!(a.picker.is_some());
    a.handle_key(key(KeyCode::Enter));
    assert!(a.confirmation.is_some());
    a.handle_key(key(KeyCode::Esc));
    assert!(a.confirmation.is_none());
    assert_eq!(fs::read_dir(cfg.vdir.join("inbox")).unwrap().count(), 1);
}

#[test]
fn errors_keep_missing_list_names_and_commands_available_after_sync() {
    use std::collections::BTreeMap;
    use twydo_core::sync::{
        SyncSummary,
        status::{CollState, SyncStatus},
    };
    let (_dir, cfg) = env();
    let mut a = app(&cfg);
    a.set_sync(Some(SyncSummary {
        status: SyncStatus {
            last_error: Some("old: Unknown error occurred: Not Found".into()),
            collections: BTreeMap::from([(
                "old".into(),
                CollState::Error {
                    message: "Unknown error occurred: Not Found".into(),
                },
            )]),
            ..Default::default()
        },
        pending: 0,
        conflicts: vec![],
        off: false,
    }));
    command(&mut a, "errors");
    let backend = ratatui::backend::TestBackend::new(120, 30);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| tdo::ui::draw(f, &a)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("Old Projects"), "{text}");
    assert!(text.contains(":forget-list old"), "{text}");
}
