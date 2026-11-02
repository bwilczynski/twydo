use std::path::PathBuf;

use chrono::{DateTime, TimeZone};
use chrono_tz::Tz;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tdo::app::{App, Mode, Outcome, Pane};
use tdo::form::Field;
use tdo::write::Request;
use twydo_core::config::Config;
use twydo_core::dateinput::{DueShift, shift_due};
use twydo_core::edit::{Change, Op};
use twydo_core::sync::resolve::Keep;
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
    assert!(
        a.form.is_none(),
        "Enter on a parent toggles, it does not edit"
    );
    assert_eq!(a.visible_rows().len(), before + 2);
    assert!(a.message.is_none());
    a.handle_key(code(KeyCode::Enter));
    assert!(!a.expanded.contains(&uid));
    assert_eq!(a.visible_rows().len(), before);
}

#[test]
fn enter_on_leaf_opens_form() {
    let mut a = app();
    let uid = selected_uid(&a);
    a.handle_key(code(KeyCode::Enter));
    assert_eq!(a.form.as_ref().map(|f| f.uid.clone()), Some(uid));
    assert_eq!(a.mode, Mode::Insert);
}

#[test]
fn keys_without_support_are_blocked_with_reason() {
    let mut a = app();
    for (k, name) in [
        (key('o'), "new task"),
        (key('O'), "new item"),
        (key('J'), "move task"),
        (key('K'), "move task"),
        (key('a'), "edit alerts"),
    ] {
        a.message = None;
        assert_eq!(a.handle_key(k), Outcome::Continue);
        assert_eq!(a.message, Some(format!("{name}: not available yet")));
    }
    a.handle_key(code(KeyCode::Esc));
    assert!(a.message.is_none());
}

fn write_op(o: Outcome) -> Op {
    match o {
        Outcome::Write(Request::Apply(op)) => op,
        other => panic!("expected a write, got {other:?}"),
    }
}

fn selected_uid(a: &App) -> String {
    a.snapshot.tasks[a.selected().unwrap()].uid.clone()
}

#[test]
fn x_requests_complete_or_reopen() {
    let mut a = app();
    let uid = selected_uid(&a);
    assert_eq!(write_op(a.handle_key(key('x'))), Op::Complete { uid });
    a.handle_key(key('4'));
    let uid = selected_uid(&a);
    assert_eq!(
        a.snapshot.tasks[a.selected().unwrap()].status,
        twydo_core::model::Status::Completed
    );
    assert_eq!(write_op(a.handle_key(key(' '))), Op::Reopen { uid });
}

#[test]
fn date_keys_request_due_edits() {
    let today = now().date_naive();
    for (c, shift) in [
        ('t', DueShift::Today),
        ('T', DueShift::Tomorrow),
        (']', DueShift::Days(1)),
        ('[', DueShift::Days(-1)),
        ('}', DueShift::Days(7)),
        ('{', DueShift::Days(-7)),
    ] {
        let mut a = app();
        let task = a.snapshot.tasks[a.selected().unwrap()].clone();
        let want = shift_due(task.due.as_ref(), NY, today, shift);
        assert_eq!(
            write_op(a.handle_key(key(c))),
            Op::Edit {
                uid: task.uid.clone(),
                changes: vec![Change::Due(Some(want))],
            },
            "key {c}"
        );
    }
}

#[test]
fn dd_requests_delete_and_single_d_does_not() {
    let mut a = app();
    let uid = selected_uid(&a);
    assert_eq!(a.handle_key(key('d')), Outcome::Continue);
    assert_eq!(write_op(a.handle_key(key('d'))), Op::Delete { uid });
}

#[test]
fn u_and_ctrl_r_request_undo_redo() {
    let mut a = app();
    assert_eq!(a.handle_key(key('u')), Outcome::Write(Request::Undo));
    assert_eq!(a.handle_key(ctrl('r')), Outcome::Write(Request::Redo));
    a.focus = Pane::Sidebar;
    assert_eq!(a.handle_key(key('u')), Outcome::Write(Request::Undo));
    let mut empty = App::new(snapshot_of(empty_report()), now());
    assert_eq!(empty.handle_key(ctrl('r')), Outcome::Write(Request::Redo));
}

fn empty_report() -> twydo_core::vdir::LoadReport {
    let mut r = report();
    r.tasks.clear();
    r
}

#[test]
fn write_keys_on_empty_view_do_nothing() {
    let mut a = App::new(snapshot_of(empty_report()), now());
    for k in ["x", "t", "]", "dd"] {
        assert!(
            type_keys(&mut a, k).iter().all(|o| *o == Outcome::Continue),
            "{k}"
        );
        assert!(a.message.is_none(), "{k}");
    }
    let mut a = app();
    a.focus = Pane::Sidebar;
    for k in ["x", "t", "]", "dd"] {
        assert!(
            type_keys(&mut a, k).iter().all(|o| *o == Outcome::Continue),
            "{k}"
        );
        assert!(a.message.is_none(), "{k}");
    }
}

fn type_keys(a: &mut App, s: &str) -> Vec<Outcome> {
    s.chars().map(|c| a.handle_key(key(c))).collect()
}

#[test]
fn cursor_stays_in_place_after_row_leaves() {
    let mut a = app();
    let before = titles(&a);
    assert_eq!(before.len(), 3);
    a.cursor = 1;
    let mut r = report();
    r.tasks.retain(|t| t.summary != before[1]);
    a.reload(snapshot_of(r));
    assert_eq!(a.cursor, 1);
    assert_eq!(titles(&a)[1], before[2]);

    let mut a = app();
    a.cursor = 2;
    let mut r = report();
    r.tasks.retain(|t| t.summary != before[2]);
    a.reload(snapshot_of(r));
    assert_eq!(a.cursor, 1);
}

#[test]
fn write_finished_shows_message() {
    let mut a = app();
    a.write_finished(Err("projects and checklists cannot be edited yet".into()));
    assert_eq!(
        a.message.as_deref(),
        Some("projects and checklists cannot be edited yet")
    );
    a.write_finished(Ok("done".into()));
    assert_eq!(a.message.as_deref(), Some("done"));
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
            keep: Keep::Local
        }
    );
    a.handle_key(key('k'));
    assert_eq!(
        a.handle_key(key('r')),
        Outcome::Resolve {
            id: "a".into(),
            keep: Keep::Remote
        }
    );
}

#[test]
fn conflicts_overlay_c_keeps_current() {
    let mut a = app();
    with_conflicts(&mut a, vec![conflict("a", "u1")]);
    run_cmd(&mut a, ":conflicts");
    assert_eq!(
        a.handle_key(key('c')),
        Outcome::Resolve {
            id: "a".into(),
            keep: Keep::Current
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
    a.handle_key(key('a'));
    a.reload(snapshot_of(report()));
    assert!(a.message.as_deref().unwrap().starts_with("edit alerts:"));
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

#[test]
fn narrow_navigation_skips_hidden_details_and_resize_restores_visible_focus() {
    let mut a = app();
    a.cursor = 1;
    let selected = a.snapshot.tasks[a.selected().unwrap()].uid.clone();
    a.resize(109, 30);
    a.handle_key(key('l'));
    assert_eq!(a.focus, Pane::Tasks);
    a.handle_key(code(KeyCode::Tab));
    assert_eq!(a.focus, Pane::Sidebar);
    a.handle_key(code(KeyCode::Tab));
    assert_eq!(a.focus, Pane::Tasks);
    a.resize(110, 30);
    a.handle_key(key('l'));
    assert_eq!(a.focus, Pane::Details);
    a.resize(80, 20);
    assert_eq!(a.focus, Pane::Tasks);
    assert_eq!(a.page, 8);
    assert_eq!(a.snapshot.tasks[a.selected().unwrap()].uid, selected);
    a.resize(160, 30);
    a.handle_key(code(KeyCode::Tab));
    assert_eq!(a.focus, Pane::Details);
    a.handle_key(key('h'));
    assert_eq!(a.focus, Pane::Tasks);
    a.handle_key(key('h'));
    assert_eq!(a.focus, Pane::Sidebar);
}

fn list_ids_in_sidebar(a: &App) -> Vec<String> {
    a.sidebar_items()
        .into_iter()
        .filter_map(|v| match v {
            View::List(id) => Some(id),
            _ => None,
        })
        .collect()
}

#[test]
fn m_opens_picker_without_current_list() {
    let mut a = app();
    let task = a.snapshot.tasks[a.selected().unwrap()].clone();
    assert_eq!(a.handle_key(key('m')), Outcome::Continue);
    let p = a.picker.as_ref().expect("picker open");
    assert_eq!(p.uid, task.uid);
    let want: Vec<String> = list_ids_in_sidebar(&a)
        .into_iter()
        .filter(|id| *id != task.collection)
        .collect();
    assert_eq!(p.lists, want);
    assert_eq!(p.cursor, 0);
}

#[test]
fn picker_enter_requests_move() {
    let mut a = app();
    let uid = selected_uid(&a);
    a.handle_key(key('m'));
    a.handle_key(key('j'));
    let to = a.picker.as_ref().unwrap().lists[1].clone();
    assert_eq!(
        write_op(a.handle_key(code(KeyCode::Enter))),
        Op::Move { uid, to }
    );
    assert!(a.picker.is_some(), "stays open until the write finishes");
    a.write_finished(Err("blocked".into()));
    assert!(a.picker.is_none());
    assert_eq!(a.message.as_deref(), Some("blocked"));
}

#[test]
fn picker_cursor_clamps() {
    let mut a = app();
    a.handle_key(key('m'));
    let n = a.picker.as_ref().unwrap().lists.len();
    for _ in 0..5 {
        a.handle_key(key('j'));
    }
    assert_eq!(a.picker.as_ref().unwrap().cursor, n - 1);
    for _ in 0..5 {
        a.handle_key(key('k'));
    }
    assert_eq!(a.picker.as_ref().unwrap().cursor, 0);
}

#[test]
fn picker_esc_closes() {
    let mut a = app();
    a.handle_key(key('m'));
    assert_eq!(a.handle_key(code(KeyCode::Esc)), Outcome::Continue);
    assert!(a.picker.is_none());
    a.write_finished(Ok("done".into()));
    assert!(a.picker.is_none());
}

#[test]
fn m_with_single_list_shows_message() {
    let mut r = report();
    let keep = r.tasks[0].collection.clone();
    r.tasks.retain(|t| t.collection == keep);
    r.collections.retain(|c| c.id == keep);
    let mut a = App::new(snapshot_of(r), now());
    a.handle_key(key('2'));
    a.handle_key(key('m'));
    assert!(a.picker.is_none());
    assert_eq!(a.message.as_deref(), Some("no other list to move to"));
}

fn select(a: &mut App, title: &str) {
    a.cursor = titles(a).iter().position(|t| t == title).unwrap();
}

#[test]
fn i_opens_form_in_insert_mode() {
    let mut a = app();
    let uid = selected_uid(&a);
    assert_eq!(a.handle_key(key('i')), Outcome::Continue);
    let f = a.form.as_ref().expect("form open");
    assert_eq!(f.uid, uid);
    assert_eq!(f.focus, Field::Title);
    assert_eq!(a.mode, Mode::Insert);
}

#[test]
fn r_focuses_repeat_or_explains() {
    let mut a = app();
    select(&mut a, "Water plants");
    a.handle_key(key('r'));
    assert_eq!(a.form.as_ref().unwrap().focus, Field::Repeat);
    let mut a = app();
    select(&mut a, "Call the dentist");
    a.handle_key(key('r'));
    assert!(a.form.is_none());
    assert_eq!(
        a.message.as_deref(),
        Some("edit repeat: this task does not repeat")
    );
}

#[test]
fn save_requests_edit_with_changes() {
    let mut a = app();
    let task = a.snapshot.tasks[a.selected().unwrap()].clone();
    a.handle_key(key('i'));
    a.handle_key(key('!'));
    assert_eq!(
        write_op(a.handle_key(code(KeyCode::Enter))),
        Op::Edit {
            uid: task.uid.clone(),
            changes: vec![Change::Summary(format!("{}!", task.summary))],
        }
    );
    assert!(a.form.is_some(), "open until the write finishes");
    a.write_finished(Ok("saved".into()));
    assert!(a.form.is_none());
    assert_eq!(a.mode, Mode::Normal);
    assert_eq!(a.message.as_deref(), Some("saved"));
}

#[test]
fn save_without_changes_closes_with_message() {
    let mut a = app();
    a.handle_key(key('i'));
    assert_eq!(a.handle_key(code(KeyCode::Enter)), Outcome::Continue);
    assert!(a.form.is_none());
    assert_eq!(a.mode, Mode::Normal);
    assert_eq!(a.message.as_deref(), Some("no changes"));
}

#[test]
fn invalid_form_shows_error_without_write() {
    let mut a = app();
    a.handle_key(key('i'));
    a.handle_key(ctrl('u'));
    assert_eq!(a.handle_key(code(KeyCode::Enter)), Outcome::Continue);
    assert_eq!(
        a.form.as_ref().unwrap().error.as_deref(),
        Some("title must not be empty")
    );
}

#[test]
fn failed_write_keeps_form_open_with_error() {
    let mut a = app();
    a.handle_key(key('i'));
    a.handle_key(key('!'));
    a.handle_key(code(KeyCode::Enter));
    a.write_finished(Err("projects and checklists cannot be edited yet".into()));
    let f = a.form.as_ref().expect("still open");
    assert_eq!(
        f.error.as_deref(),
        Some("projects and checklists cannot be edited yet")
    );
    assert!(f.title.ends_with('!'), "typing kept");
    assert_eq!(a.mode, Mode::Insert);
}

#[test]
fn esc_cancels_without_write() {
    let mut a = app();
    a.handle_key(key('i'));
    a.handle_key(key('!'));
    assert_eq!(a.handle_key(code(KeyCode::Esc)), Outcome::Continue);
    assert!(a.form.is_none());
    assert_eq!(a.mode, Mode::Normal);
    assert!(a.message.is_none());
}

#[test]
fn enter_on_notes_requests_editor() {
    let mut a = app();
    a.handle_key(key('i'));
    for _ in 0..3 {
        a.handle_key(code(KeyCode::Tab));
    }
    assert_eq!(a.form.as_ref().unwrap().focus, Field::Notes);
    a.form.as_mut().unwrap().set_notes("old".into());
    assert_eq!(
        a.handle_key(code(KeyCode::Enter)),
        Outcome::EditNotes("old".into())
    );
    a.notes_edited(Ok("new\nlines".into()));
    assert_eq!(a.form.as_ref().unwrap().notes, "new\nlines");
    assert_eq!(a.mode, Mode::Insert);
}

#[test]
fn failing_editor_keeps_notes() {
    let mut a = app();
    a.handle_key(key('i'));
    a.form.as_mut().unwrap().set_notes("keep".into());
    a.notes_edited(Err("editor exited with status 1".into()));
    let f = a.form.as_ref().unwrap();
    assert_eq!(f.notes, "keep");
    assert_eq!(
        f.error.as_deref(),
        Some("notes: editor exited with status 1")
    );
}

#[test]
fn overlays_swallow_write_keys() {
    for open in ["help", "errors", "conflicts"] {
        let mut a = app();
        match open {
            "help" => a.show_help = true,
            "errors" => a.show_errors = true,
            _ => a.show_conflicts = true,
        }
        for k in ["x", " ", "t", "]", "dd", "u", "i", "m", "r"] {
            let outs = type_keys(&mut a, k);
            assert!(outs.iter().all(|o| *o == Outcome::Continue), "{open}: {k}");
            assert!(a.form.is_none() && a.picker.is_none(), "{open}: {k}");
        }
        assert_eq!(a.handle_key(ctrl('r')), Outcome::Continue, "{open}");
    }
}

#[test]
fn paste_goes_to_form_and_never_runs_keys() {
    let mut a = app();
    let before = a.snapshot.tasks.clone();
    assert_eq!(a.paste("x\ndd"), Outcome::Continue);
    assert!(a.form.is_none() && a.message.is_none());
    a.handle_key(key('i'));
    a.handle_key(ctrl('u'));
    assert_eq!(a.paste("fix door\nand window"), Outcome::Continue);
    let f = a.form.as_ref().expect("form stays open");
    assert_eq!(f.title, "fix door and window");
    assert_eq!(format!("{:?}", a.snapshot.tasks), format!("{before:?}"));
}

#[test]
fn success_messages_clear_after_a_few_seconds() {
    let t0 = now();
    let secs = |n| t0 + chrono::Duration::seconds(n);
    let mut a = app();
    a.sync_finished("sync done".into());
    a.expire_message(t0); // the loop's first look starts the clock
    a.expire_message(secs(4));
    assert_eq!(a.message.as_deref(), Some("sync done"));
    a.expire_message(secs(5));
    assert_eq!(a.message, None);

    a.write_finished(Ok("done".into()));
    a.expire_message(secs(100));
    assert_eq!(
        a.message.as_deref(),
        Some("done"),
        "clock starts when first seen"
    );
    a.expire_message(secs(105));
    assert_eq!(a.message, None);

    // Errors stay until Esc or the next message.
    let mut a = app();
    a.sync_finished("vdirsyncer: connection refused".into());
    a.expire_message(secs(600));
    assert!(a.message.is_some());
    a.write_finished(Err("nothing to undo".into()));
    a.expire_message(secs(600));
    assert_eq!(a.message.as_deref(), Some("nothing to undo"));
}

#[test]
fn newer_message_is_not_cleared_by_old_timer() {
    let t0 = now();
    let mut a = app();
    a.sync_finished("sync done".into());
    a.expire_message(t0);
    a.handle_key(key('a'));
    a.expire_message(t0 + chrono::Duration::seconds(60));
    assert_eq!(a.message.as_deref(), Some("edit alerts: not available yet"));
}

#[test]
fn reload_drops_a_pending_d() {
    let mut a = app();
    a.handle_key(key('d'));
    a.reload(snapshot_of(report()));
    assert_eq!(
        a.handle_key(key('d')),
        Outcome::Continue,
        "a d before a reload must not pair with one after it"
    );
}

#[test]
fn key_at_uses_the_current_date() {
    // The minute tick is late: the app still thinks it is before midnight.
    let late = NY.with_ymd_and_hms(2026, 11, 4, 23, 59, 50).unwrap();
    let next_day = NY.with_ymd_and_hms(2026, 11, 5, 0, 0, 5).unwrap();
    let mut a = App::new(snapshot_of(report()), late);
    let task = a.snapshot.tasks[a.selected().unwrap()].clone();
    let want = shift_due(
        task.due.as_ref(),
        a.snapshot.tz(),
        next_day.date_naive(),
        DueShift::Today,
    );
    let op = write_op(a.key_at(key('t'), next_day));
    let uid = a.snapshot.tasks[a.selected().unwrap()].uid.clone();
    assert_eq!(
        op,
        Op::Edit {
            uid,
            changes: vec![Change::Due(Some(want))],
        }
    );
}

#[test]
fn failed_reload_after_a_write_stays_visible() {
    let mut a = app();
    a.reload_failed("synthetic io error");
    a.write_finished(Ok("done".into()));
    assert_eq!(
        a.message.as_deref(),
        Some("done · reload failed: synthetic io error")
    );
    // Not a success message: it stays past the timer.
    a.expire_message(now());
    a.expire_message(now() + chrono::Duration::seconds(10));
    assert!(a.message.is_some());
}

/// Next 3 Days just before midnight: "Water plants" (due Nov 4) is selected and leaves the
/// view at midnight, when it becomes overdue.
fn next3_before_midnight() -> (App, DateTime<Tz>) {
    let before = NY.with_ymd_and_hms(2026, 11, 4, 23, 59, 59).unwrap();
    let after = NY.with_ymd_and_hms(2026, 11, 5, 0, 0, 1).unwrap();
    let mut a = App::new(snapshot_of(report()), before);
    a.handle_key(key('5'));
    assert_eq!(titles(&a)[a.cursor], "Water plants");
    (a, after)
}

#[test]
fn key_is_dropped_when_the_clock_moves_the_selection() {
    let (mut a, after) = next3_before_midnight();
    assert_eq!(a.key_at(key('x'), after), Outcome::Continue);
    assert!(a.message.is_some(), "the user is told why nothing happened");
}

#[test]
fn pending_d_does_not_carry_over_to_another_task() {
    let (mut a, after) = next3_before_midnight();
    let before = a.now;
    assert_eq!(a.key_at(key('d'), before), Outcome::Continue);
    assert_eq!(a.key_at(key('d'), after), Outcome::Continue);
    assert_eq!(
        a.key_at(key('d'), after),
        Outcome::Continue,
        "the dropped second d does not arm a new pending d"
    );
}

#[test]
fn tick_that_moves_the_selection_clears_a_pending_d() {
    let (mut a, after) = next3_before_midnight();
    a.handle_key(key('d'));
    a.tick(after);
    assert_eq!(a.handle_key(key('d')), Outcome::Continue);
}

#[test]
fn successful_reload_clears_a_reload_error_shown_after_a_write() {
    let mut a = app();
    a.reload_failed("synthetic io error");
    a.write_finished(Ok("done".into()));
    a.reload(snapshot_of(report()));
    assert_eq!(a.message, None);
}

#[test]
fn form_input_survives_the_clock_moving_the_selection() {
    let (mut a, after) = next3_before_midnight();
    let before = a.now;
    let uid = selected_uid(&a);
    a.key_at(key('i'), before);
    a.key_at(ctrl('u'), before);
    a.key_at(key('1'), before);
    a.key_at(key('0'), after);
    let op = write_op(a.key_at(code(KeyCode::Enter), after));
    assert_eq!(
        op,
        Op::Edit {
            uid,
            changes: vec![Change::Summary("10".into())],
        }
    );
}

#[test]
fn command_and_search_input_survive_the_clock_moving_the_selection() {
    let (mut a, after) = next3_before_midnight();
    a.key_at(key(':'), a.now);
    a.key_at(key('q'), after);
    assert_eq!(a.key_at(code(KeyCode::Enter), after), Outcome::Quit);

    let (mut a, after) = next3_before_midnight();
    a.key_at(key('/'), a.now);
    a.key_at(key('w'), after);
    assert_eq!(a.search.as_deref(), Some("w"));
}

#[test]
fn picker_survives_the_clock_moving_the_selection() {
    let (mut a, after) = next3_before_midnight();
    let uid = selected_uid(&a);
    a.key_at(key('m'), a.now);
    match write_op(a.key_at(code(KeyCode::Enter), after)) {
        Op::Move { uid: moved, .. } => assert_eq!(moved, uid),
        other => panic!("expected a move, got {other:?}"),
    }
}

fn select_tax_return(a: &mut App) {
    a.handle_key(key('2'));
    a.cursor = titles(a)
        .iter()
        .position(|t| t == "Prepare tax return")
        .unwrap();
}

#[test]
fn x_on_parent_with_open_items_needs_second_x() {
    let mut a = app();
    select_tax_return(&mut a);
    let uid = selected_uid(&a);
    assert!(matches!(a.handle_key(key('x')), Outcome::Continue));
    assert_eq!(
        a.message.as_deref(),
        Some("complete \"Prepare tax return\" and its 2 open items? x to confirm")
    );
    assert_eq!(
        write_op(a.handle_key(key('x'))),
        Op::Complete { uid: uid.clone() }
    );
    // Space confirms too.
    a.handle_key(key('x'));
    assert_eq!(write_op(a.handle_key(key(' '))), Op::Complete { uid });
}

#[test]
fn other_key_cancels_parent_confirmation() {
    let mut a = app();
    select_tax_return(&mut a);
    a.handle_key(key('x'));
    assert!(a.message.is_some());
    a.handle_key(key('j'));
    assert!(a.message.is_none());
    select_tax_return(&mut a);
    // The next x asks again instead of completing.
    assert!(matches!(a.handle_key(key('x')), Outcome::Continue));
    assert!(a.message.is_some());
}

#[test]
fn x_on_parent_with_all_items_done_completes_at_once() {
    let mut a = app();
    select_tax_return(&mut a);
    let uid = selected_uid(&a);
    for t in &mut a.snapshot.tasks {
        if t.parent.as_deref() == Some(uid.as_str()) {
            t.status = twydo_core::model::Status::Completed;
        }
    }
    assert_eq!(write_op(a.handle_key(key('x'))), Op::Complete { uid });
    assert!(a.message.is_none());
}
