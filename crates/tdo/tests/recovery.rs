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
    let text = render(&a);
    assert!(text.contains("Keep me"), "{text}");
    assert!(text.contains("Keep my notes"), "{text}");
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
    assert!(render(&a).contains(":forget-list to choose"));
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
    assert!(text.contains(":forget-list to choose a list"), "{text}");
}

fn render(a: &App) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 30)).unwrap();
    terminal.draw(|f| tdo::ui::draw(f, a)).unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect()
}

#[test]
fn bare_forget_list_chooses_a_name_then_requires_confirmation() {
    let (_dir, cfg) = env();
    let mut a = app(&cfg);
    assert_eq!(command(&mut a, "forget-list"), Outcome::Continue);
    assert!(a.picker.is_some(), "{:?}", a.message);
    assert!(a.confirmation.is_none());
    let text = render(&a);
    assert!(text.contains("Forget list"), "{text}");
    assert!(text.contains("Old Projects"), "{text}");
    assert!(text.contains("Enter select"), "{text}");
    // Names are sorted: Old Projects, Personal Tasks. Test both navigation directions.
    a.handle_key(key(KeyCode::Down));
    a.handle_key(key(KeyCode::Char('k')));
    assert_eq!(a.handle_key(key(KeyCode::Enter)), Outcome::Continue);
    assert!(a.picker.is_none());
    assert!(
        a.confirmation
            .as_ref()
            .unwrap()
            .question()
            .contains("Old Projects")
    );
    assert_eq!(
        fs::read_to_string(cfg.vdir.join("old/displayname")).unwrap(),
        "Old Projects"
    );
    let Outcome::ForgetList(p) = a.handle_key(key(KeyCode::Char('y'))) else {
        panic!("expected retirement only after confirmation")
    };
    assert_eq!(p.collection, "old");
    assert!(cfg.vdir.join("old/task.ics").exists());
}

#[test]
fn cancelling_forget_chooser_or_confirmation_preserves_data_and_clears_intent() {
    let (_dir, cfg) = env();
    let mut a = app(&cfg);
    command(&mut a, "forget-list");
    assert!(a.picker.is_some());
    a.handle_key(key(KeyCode::Esc));
    assert!(a.picker.is_none());
    command(&mut a, "forget-list");
    a.handle_key(key(KeyCode::Enter));
    a.handle_key(key(KeyCode::Char('n')));
    assert!(a.confirmation.is_none());
    assert!(cfg.vdir.join("old/task.ics").exists());
    assert!(retired::list(&cfg).unwrap().is_empty());
    // A later recovery destination choice must not retire that destination.
    retired::forget(&cfg, &retired::preview(&cfg, "old").unwrap(), Utc::now()).unwrap();
    a.reload(Snapshot::new(
        twydo_core::vdir::load(&cfg.vdir).unwrap(),
        cfg.clone(),
    ));
    command(&mut a, "recover");
    a.handle_key(key(KeyCode::Char('r')));
    a.handle_key(key(KeyCode::Enter));
    assert!(matches!(
        a.handle_key(key(KeyCode::Char('y'))),
        Outcome::Write(Request::Recover { .. })
    ));
}

#[test]
fn forget_chooser_includes_hidden_and_empty_lists_and_disambiguates_names() {
    let (_dir, mut cfg) = env();
    fs::write(cfg.vdir.join("inbox/displayname"), "Old Projects").unwrap();
    cfg.lists.entry("inbox".into()).or_default().hidden = true;
    cfg.lists.entry("old".into()).or_default().archived = true;
    let mut a = app(&cfg);
    command(&mut a, "forget-list");
    assert!(a.picker.is_some());
    let text = render(&a);
    assert!(text.contains("Old Projects (inbox)"), "{text}");
    assert!(text.contains("Old Projects (old)"), "{text}");
    // Duplicate names sort by collection ID; select the empty inbox by ID internally.
    a.handle_key(key(KeyCode::Enter));
    let Outcome::ForgetList(p) = a.handle_key(key(KeyCode::Char('y'))) else {
        panic!("expected selected collection")
    };
    assert_eq!(p.collection, "inbox");
    assert_eq!(p.tasks, 0);
}

#[test]
fn forget_chooser_reports_no_lists() {
    let (_dir, cfg) = env();
    fs::remove_dir_all(cfg.vdir.join("old")).unwrap();
    fs::remove_dir_all(cfg.vdir.join("inbox")).unwrap();
    let mut a = app(&cfg);
    command(&mut a, "forget-list");
    assert!(a.picker.is_none());
    assert!(
        a.message.as_ref().unwrap().contains("no lists"),
        "{:?}",
        a.message
    );
    assert!(a.confirmation.is_none());
}

#[test]
fn long_duplicate_names_keep_collection_ids_visible() {
    let (_dir, cfg) = env();
    let name = "A very long synthetic collection name ".repeat(5);
    fs::write(cfg.vdir.join("old/displayname"), &name).unwrap();
    fs::write(cfg.vdir.join("inbox/displayname"), &name).unwrap();
    let mut a = app(&cfg);
    missing_list_sync(&mut a);
    command(&mut a, "forget-list");
    let text = render(&a);
    assert!(text.contains("(inbox)"), "{text}");
    assert!(text.contains("(old) · Not Found"), "{text}");
}

fn missing_list_sync(a: &mut App) {
    use std::collections::BTreeMap;
    use twydo_core::sync::{
        SyncSummary,
        status::{CollState, SyncStatus},
    };
    a.set_sync(Some(SyncSummary {
        status: SyncStatus {
            collections: BTreeMap::from([
                (
                    "old".into(),
                    CollState::Error {
                        message: "Unknown error occurred: Not Found;".into(),
                    },
                ),
                ("inbox".into(), CollState::Ok),
                (
                    "offline".into(),
                    CollState::Error {
                        message: "connection refused".into(),
                    },
                ),
            ]),
            ..Default::default()
        },
        pending: 0,
        conflicts: vec![],
    }));
}

#[test]
fn forget_chooser_marks_only_lists_reported_not_found() {
    let (_dir, cfg) = env();
    fs::create_dir_all(cfg.vdir.join("offline")).unwrap();
    fs::write(cfg.vdir.join("offline/displayname"), "Offline List").unwrap();
    let mut a = app(&cfg);
    missing_list_sync(&mut a);
    command(&mut a, "forget-list");
    let text = render(&a);
    assert!(text.contains("▌ Old Projects · Not Found"), "{text}");
    assert!(text.contains("▌ Personal Tasks"), "{text}");
    assert!(text.contains("▌ Offline List"), "{text}");
    assert!(!text.contains("Personal Tasks · Not Found"), "{text}");
    assert!(!text.contains("Offline List · Not Found"), "{text}");
}

fn rendered_lines(a: &App, width: u16, height: u16) -> Vec<String> {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| tdo::ui::draw(f, a)).unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(width as usize)
        .map(|row| row.iter().map(|c| c.symbol()).collect())
        .collect()
}

#[test]
fn forget_confirmation_fits_its_text_without_an_empty_preview() {
    let (_dir, cfg) = env();
    let mut a = app(&cfg);
    command(&mut a, "forget-list old");
    let lines = rendered_lines(&a, 120, 40);
    let top = lines
        .iter()
        .position(|line| line.contains("Forget list"))
        .expect("forget confirmation title");
    let left = lines[top].chars().position(|c| c == '╭').unwrap();
    let bottom = (top + 1..lines.len())
        .find(|i| lines[*i].chars().nth(left) == Some('╰'))
        .unwrap();
    assert!(
        bottom - top <= 5,
        "confirmation has excessive blank space: {lines:#?}"
    );
    assert!(lines[top + 1].contains("Did you delete"), "{lines:#?}");
    assert!(
        lines[top + 1..bottom]
            .iter()
            .any(|line| line.contains("cancel")),
        "{lines:#?}"
    );
}

#[test]
fn forget_confirmation_wraps_the_whole_question_on_narrow_terminals() {
    let (_dir, cfg) = env();
    fs::write(
        cfg.vdir.join("old/displayname"),
        "Synthetic list with a longer name for wrapping",
    )
    .unwrap();
    let mut a = app(&cfg);
    command(&mut a, "forget-list old");
    for width in 30..=120 {
        let lines = rendered_lines(&a, width, 40);
        let top = lines
            .iter()
            .position(|line| line.contains("Forget list"))
            .expect("forget confirmation title");
        assert!(
            lines[top + 1].contains("Did you delete"),
            "width {width}: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("cancel")),
            "width {width}: {lines:#?}"
        );
    }
}
