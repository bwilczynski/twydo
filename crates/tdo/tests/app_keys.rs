use std::path::PathBuf;

use chrono::{DateTime, TimeZone};
use chrono_tz::Tz;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tdo::app::{App, Mode, Outcome};
use twydo_core::config::Config;
use twydo_core::conflicts::Side;
use twydo_core::views::{Snapshot, View};

const NY: Tz = chrono_tz::America::New_York;

fn now() -> DateTime<Tz> {
    NY.with_ymd_and_hms(2026, 11, 4, 10, 0, 0).unwrap()
}

fn report() -> twydo_core::vdir::LoadReport {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../twydo-core/tests/data/sample-vdir");
    twydo_core::vdir::load(&root).unwrap()
}

fn snapshot_of(report: twydo_core::vdir::LoadReport) -> Snapshot {
    let mut cfg = Config::with_vdir(PathBuf::from("unused"));
    cfg.timezone = Some("America/New_York".into());
    Snapshot::new(report, cfg)
}

fn app() -> App {
    App::new(snapshot_of(report()), now())
}

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn code(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn type_str(a: &mut App, s: &str) {
    for c in s.chars() {
        a.handle_key(key(c));
    }
}

fn titles(a: &App) -> Vec<String> {
    a.visible_rows()
        .iter()
        .map(|r| a.snapshot.tasks[r.task].summary.clone())
        .collect()
}

#[test]
#[allow(non_snake_case)]
fn jk_gg_G_move_and_clamp() {
    let mut a = app();
    assert_eq!(a.visible_rows().len(), 3); // overdue, later today, today
    a.handle_key(key('k'));
    assert_eq!(a.cursor, 0);
    for _ in 0..5 {
        a.handle_key(key('j'));
    }
    assert_eq!(a.cursor, 2);
    a.handle_key(key('k'));
    assert_eq!(a.cursor, 1);
    a.handle_key(key('G'));
    assert_eq!(a.cursor, 2);
    a.handle_key(key('g'));
    assert_eq!(a.cursor, 2, "a single g only arms the pending key");
    a.handle_key(key('g'));
    assert_eq!(a.cursor, 0);
    a.page = 2;
    a.handle_key(ctrl('d'));
    assert_eq!(a.cursor, 2);
    a.handle_key(ctrl('d'));
    assert_eq!(a.cursor, 2);
    a.handle_key(ctrl('u'));
    assert_eq!(a.cursor, 0);
}

#[test]
fn number_keys_switch_views() {
    let mut a = app();
    a.handle_key(key('2'));
    assert_eq!(a.view, View::All);
    a.handle_key(key('5'));
    assert_eq!(a.view, View::Next3Days);
    a.handle_key(key('4'));
    assert_eq!(a.view, View::Done);
    assert_eq!(titles(&a), vec!["Buy stamps"]);
    a.handle_key(key('3'));
    assert_eq!(a.view, View::Scheduled);
    a.handle_key(key('1'));
    assert_eq!(a.view, View::Today);
}

#[test]
fn enter_toggles_parent() {
    let mut a = app();
    a.handle_key(key('2'));
    let before = a.visible_rows().len();
    let pos = titles(&a)
        .iter()
        .position(|t| t == "Prepare tax return")
        .unwrap();
    a.cursor = pos;
    let uid = a.snapshot.tasks[a.selected().unwrap()].uid.clone();
    a.handle_key(code(KeyCode::Enter));
    assert!(a.expanded.contains(&uid));
    assert_eq!(a.visible_rows().len(), before + 2);
    assert!(a.message.is_none());
    a.handle_key(code(KeyCode::Enter));
    assert!(!a.expanded.contains(&uid));
    assert_eq!(a.visible_rows().len(), before);
}

#[test]
fn enter_on_leaf_is_blocked() {
    let mut a = app();
    a.handle_key(code(KeyCode::Enter));
    assert_eq!(a.message.as_deref(), Some("edit task: not available yet"));
}

#[test]
fn write_keys_are_blocked_with_reason() {
    let mut a = app();
    let before = format!("{:?}", a.snapshot.tasks);
    a.handle_key(key('x'));
    assert_eq!(a.message.as_deref(), Some("toggle done: not available yet"));
    for (k, name) in [
        (key(' '), "toggle done"),
        (key('o'), "new task"),
        (key('O'), "new item"),
        (key('i'), "edit task"),
        (key('J'), "move task"),
        (key('K'), "move task"),
        (key('u'), "undo"),
        (ctrl('r'), "redo"),
        (key('t'), "due today"),
        (key('T'), "due tomorrow"),
        (key(']'), "due +1 day"),
        (key('['), "due -1 day"),
        (key('}'), "due +1 week"),
        (key('{'), "due -1 week"),
        (key('r'), "edit repeat"),
        (key('a'), "edit alerts"),
        (key('m'), "move to list"),
    ] {
        a.message = None;
        a.handle_key(k);
        assert_eq!(a.message, Some(format!("{name}: not available yet")));
    }
    a.message = None;
    type_str(&mut a, "dd");
    assert_eq!(a.message.as_deref(), Some("delete: not available yet"));
    a.handle_key(code(KeyCode::Esc));
    assert!(a.message.is_none());
    assert_eq!(format!("{:?}", a.snapshot.tasks), before);
}

fn conflict(id: &str, uid: &str) -> twydo_core::conflicts::Conflict {
    use twydo_core::conflicts::*;
    Conflict {
        id: id.into(),
        uid: uid.into(),
        collection: "c".into(),
        path: "c/x.ics".into(),
        kind: ConflictKind::BothChanged,
        held: true,
        vdir_holds: Side::Local,
        detected: chrono::Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap(),
        state: ConflictState::Open,
        choice: None,
        resolved: None,
        local: None,
        remote: None,
    }
}

fn with_conflicts(a: &mut App, cs: Vec<twydo_core::conflicts::Conflict>) {
    a.set_sync(Some(twydo_core::sync::SyncSummary {
        status: Default::default(),
        pending: 0,
        conflicts: cs,
    }));
}

fn run_cmd(a: &mut App, s: &str) -> Outcome {
    type_str(a, s);
    a.handle_key(code(KeyCode::Enter))
}

#[test]
fn sync_command_returns_sync_outcome() {
    let mut a = app();
    assert_eq!(
        run_cmd(&mut a, ":sync"),
        Outcome::Sync { force_delete: None }
    );
    assert_eq!(a.mode, Mode::Normal);
}

#[test]
fn sync_bang_passes_list() {
    let mut a = app();
    assert_eq!(
        run_cmd(&mut a, ":sync! Groceries"),
        Outcome::Sync {
            force_delete: Some("Groceries".into())
        }
    );
    assert_eq!(run_cmd(&mut a, ":sync!"), Outcome::Continue);
}

#[test]
fn conflicts_overlay_keys_return_resolve() {
    let mut a = app();
    with_conflicts(&mut a, vec![conflict("a", "u1"), conflict("b", "u2")]);
    run_cmd(&mut a, ":conflicts");
    assert!(a.show_conflicts);
    a.handle_key(key('j'));
    assert_eq!(a.conflict_cursor, 1);
    assert_eq!(
        a.handle_key(key('l')),
        Outcome::Resolve {
            id: "b".into(),
            keep: Side::Local
        }
    );
    a.handle_key(key('k'));
    assert_eq!(
        a.handle_key(key('r')),
        Outcome::Resolve {
            id: "a".into(),
            keep: Side::Remote
        }
    );
}

#[test]
fn esc_closes_conflicts() {
    let mut a = app();
    run_cmd(&mut a, ":conflicts");
    assert!(a.show_conflicts);
    a.handle_key(code(KeyCode::Esc));
    assert!(!a.show_conflicts);
}

#[test]
fn search_filters_and_n_cycles() {
    let mut a = app();
    a.handle_key(key('2'));
    let all = a.visible_rows().len();
    a.handle_key(key('/'));
    assert_eq!(a.mode, Mode::Search);
    type_str(&mut a, "PAY");
    for t in ["Pay water bill", "Pay rent", "Pay electricity bill"] {
        assert!(titles(&a).contains(&t.to_string()), "{:?}", titles(&a));
    }
    a.handle_key(code(KeyCode::Enter));
    assert_eq!(a.mode, Mode::Normal);
    assert_eq!(a.search.as_deref(), Some("PAY"));
    let n = a.visible_rows().len();
    assert!(n >= 3 && n < all);
    a.cursor = 0;
    a.handle_key(key('n'));
    assert_eq!(a.cursor, 1);
    for _ in 0..n - 1 {
        a.handle_key(key('n'));
    }
    assert_eq!(a.cursor, 0, "n wraps");
    a.handle_key(key('N'));
    assert_eq!(a.cursor, n - 1, "N wraps backwards");
    a.handle_key(code(KeyCode::Esc));
    assert!(a.search.is_none());
    assert_eq!(a.visible_rows().len(), all);
}

#[test]
fn search_matches_notes_and_esc_clears() {
    let mut a = app();
    a.handle_key(key('2'));
    a.handle_key(key('/'));
    type_str(&mut a, "zzzz-no-such");
    assert!(a.visible_rows().is_empty());
    assert_eq!(a.selected(), None);
    a.handle_key(code(KeyCode::Esc));
    assert_eq!(a.mode, Mode::Normal);
    assert!(a.search.is_none());
    assert!(!a.visible_rows().is_empty());
}

#[test]
fn command_list_and_quit() {
    let mut a = app();
    type_str(&mut a, ":list bills");
    assert_eq!(a.mode, Mode::Command);
    assert_eq!(a.handle_key(code(KeyCode::Enter)), Outcome::Continue);
    assert_eq!(a.view, View::List("bills".into()));
    assert_eq!(a.mode, Mode::Normal);
    type_str(&mut a, ":list nope");
    a.handle_key(code(KeyCode::Enter));
    assert_eq!(a.message.as_deref(), Some("unknown list: nope"));
    assert_eq!(a.view, View::List("bills".into()));
    type_str(&mut a, ":q");
    assert_eq!(a.handle_key(code(KeyCode::Enter)), Outcome::Quit);
}

#[test]
fn sidebar_navigation_opens_views() {
    let mut a = app();
    let items = a.sidebar_items();
    assert_eq!(
        items,
        vec![
            View::Today,
            View::All,
            View::Scheduled,
            View::Done,
            View::List("bills".into()),
            View::List("inbox".into()),
            View::List("personal".into()),
            View::Next3Days,
        ]
    );
    a.handle_key(key('h'));
    assert_eq!(a.focus, tdo::app::Pane::Sidebar);
    a.handle_key(key('j'));
    a.handle_key(code(KeyCode::Enter));
    assert_eq!(a.view, View::All);
    assert_eq!(a.focus, tdo::app::Pane::Tasks);
}

#[test]
fn tick_recomputes_view_on_new_day() {
    let mut a = app();
    assert!(!titles(&a).contains(&"Pay electricity bill".to_string()));
    a.tick(NY.with_ymd_and_hms(2026, 11, 5, 0, 1, 0).unwrap());
    assert!(titles(&a).contains(&"Pay electricity bill".to_string()));
}

#[test]
fn reload_keeps_selection_by_uid() {
    let mut a = app();
    a.cursor = 1;
    let uid = a.snapshot.tasks[a.selected().unwrap()].uid.clone();
    let mut r = report();
    r.tasks.retain(|t| t.summary != "Renew library books");
    a.reload(snapshot_of(r));
    assert_eq!(a.cursor, 0);
    assert_eq!(a.snapshot.tasks[a.selected().unwrap()].uid, uid);
}

#[test]
fn sidebar_j_from_done_lands_on_first_list() {
    let mut a = app();
    a.handle_key(key('h'));
    for _ in 0..3 {
        a.handle_key(key('j'));
    }
    assert_eq!(a.sidebar_items()[a.sidebar_cursor], View::Done);
    a.handle_key(key('j'));
    a.handle_key(code(KeyCode::Enter));
    assert_eq!(a.view, View::List("bills".into()));
}

#[test]
#[allow(non_snake_case)]
fn sidebar_focus_gg_G_and_page_keys_move_the_sidebar() {
    let mut a = app();
    a.handle_key(key('2'));
    a.cursor = 1;
    a.handle_key(key('h'));
    let last = a.sidebar_items().len() - 1;
    a.handle_key(key('G'));
    assert_eq!(a.sidebar_cursor, last);
    assert_eq!(a.cursor, 1, "task cursor untouched");
    type_str(&mut a, "gg");
    assert_eq!(a.sidebar_cursor, 0);
    a.page = 3;
    a.handle_key(ctrl('d'));
    assert_eq!(a.sidebar_cursor, 3);
    a.handle_key(ctrl('d'));
    assert_eq!(a.sidebar_cursor, 6);
    a.handle_key(ctrl('d'));
    assert_eq!(a.sidebar_cursor, last);
    a.handle_key(ctrl('u'));
    assert_eq!(a.sidebar_cursor, last - 3);
    assert_eq!(a.cursor, 1, "task cursor untouched");
    assert_eq!(a.view, View::All);
}

#[test]
fn snapshot_keeps_load_errors_and_errors_command_toggles_overlay() {
    let mut r = report();
    r.errors.push((
        PathBuf::from("bills/broken.ics"),
        "synthetic parse error".into(),
    ));
    let mut a = App::new(snapshot_of(r), now());
    assert_eq!(a.snapshot.errors.len(), 1);
    assert!(!a.show_errors);
    type_str(&mut a, ":errors");
    a.handle_key(code(KeyCode::Enter));
    assert!(a.show_errors);
    type_str(&mut a, ":errors");
    a.handle_key(code(KeyCode::Enter));
    assert!(!a.show_errors);
    type_str(&mut a, ":errors");
    a.handle_key(code(KeyCode::Enter));
    a.handle_key(code(KeyCode::Esc));
    assert!(!a.show_errors, "Esc closes the overlay");
}

#[test]
fn successful_reload_clears_reload_failed_message() {
    let mut a = app();
    a.reload_failed("synthetic io error");
    assert_eq!(
        a.message.as_deref(),
        Some("reload failed: synthetic io error")
    );
    a.reload(snapshot_of(report()));
    assert_eq!(a.message, None);
    // Other messages survive a reload.
    a.handle_key(key('x'));
    a.reload(snapshot_of(report()));
    assert!(a.message.as_deref().unwrap().starts_with("toggle done:"));
}

#[test]
fn reload_keeps_sidebar_cursor() {
    let mut a = app();
    a.handle_key(key('h'));
    a.handle_key(key('j'));
    a.handle_key(key('j'));
    let before = a.sidebar_cursor;
    assert_eq!(a.sidebar_items()[before], View::Scheduled);
    a.reload(snapshot_of(report()));
    assert_eq!(a.sidebar_cursor, before);
    assert_eq!(a.focus, tdo::app::Pane::Sidebar);
}
