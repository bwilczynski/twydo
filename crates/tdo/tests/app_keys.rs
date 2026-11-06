use std::path::PathBuf;

use chrono::{DateTime, TimeZone};
use chrono_tz::Tz;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tdo::app::{App, Mode, Outcome, Pane, default_list};
use tdo::form::{Field, NewTarget};
use tdo::write::Request;
use twydo_core::config::Config;
use twydo_core::dateinput::{DueShift, shift_due};
use twydo_core::edit::{Change, Due, NewTask, Op};
use twydo_core::model::TaskKind;
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
    assert_eq!(a.form.as_ref().and_then(|f| f.uid.clone()), Some(uid));
    assert_eq!(a.mode, Mode::Insert);
}

#[test]
fn keys_without_support_are_blocked_with_reason() {
    let mut a = app();
    for (k, name) in [(key('J'), "move task"), (key('K'), "move task")] {
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
        off: false,
        queued: 0,
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
fn colon_sync_off_and_on() {
    let mut a = app();
    assert_eq!(run_cmd(&mut a, ":sync off"), Outcome::SyncSwitch(false));
    assert_eq!(run_cmd(&mut a, ":sync on"), Outcome::SyncSwitch(true));
    assert_eq!(run_cmd(&mut a, ":sync bogus"), Outcome::Continue);
    assert_eq!(a.message.as_deref(), Some("usage: :sync [on|off]"));
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
    a.handle_key(key('J'));
    a.reload(snapshot_of(report()));
    assert!(a.message.as_deref().unwrap().starts_with("move task:"));
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
    assert_eq!(p.action, tdo::app::ListPickerAction::Move(task.uid.clone()));
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
    assert_eq!(f.uid, Some(uid));
    assert_eq!(f.focus, Field::Title);
    assert_eq!(a.mode, Mode::Insert);
}

#[test]
fn r_opens_repeat_on_a_plain_task() {
    let mut a = app();
    select(&mut a, "Water plants");
    a.handle_key(key('r'));
    let f = a.form.as_ref().unwrap();
    assert_eq!((f.focus, f.repeat.as_str()), (Field::Repeat, "wed"));
    let mut a = app();
    select(&mut a, "Call the dentist");
    a.handle_key(key('r'));
    let f = a
        .form
        .as_ref()
        .expect("the card opens on a task that does not repeat");
    assert_eq!((f.focus, f.repeat.as_str()), (Field::Repeat, ""));
    assert_eq!(a.mode, Mode::Insert);

    // A checklist item has no repeat: say so instead.
    let mut a = app();
    a.handle_key(key('2'));
    select(&mut a, "Packing list");
    a.handle_key(code(KeyCode::Enter));
    select(&mut a, "Passport");
    a.handle_key(key('r'));
    assert!(a.form.is_none());
    assert_eq!(
        a.message.as_deref(),
        Some("repeat: checklist items have no dates or repeats")
    );
}

/// Open the card on the selected task and type `s` into the Repeat field.
fn card_with_repeat(a: &mut App, s: &str) {
    a.handle_key(key('r'));
    a.handle_key(ctrl('u'));
    type_str(a, s);
}

#[test]
fn esc_in_card_writes_nothing() {
    let mut a = app();
    select(&mut a, "Call the dentist");
    a.handle_key(key('i'));
    type_str(&mut a, " today");
    a.handle_key(code(KeyCode::Tab));
    a.handle_key(code(KeyCode::Enter)); // List: the picker
    assert!(a.picker.is_some());
    a.handle_key(code(KeyCode::Enter)); // choose the first other list
    assert!(a.picker.is_none());
    a.handle_key(code(KeyCode::Tab));
    a.handle_key(key(' ')); // Type: project
    a.handle_key(code(KeyCode::Tab));
    a.handle_key(ctrl('u')); // Due cleared
    assert_eq!(a.handle_key(code(KeyCode::Esc)), Outcome::Continue);
    assert!(a.form.is_none() && a.picker.is_none());
    assert_eq!(a.mode, Mode::Normal);
    assert!(a.message.is_none());
}

#[test]
fn card_list_field_picks_without_writing() {
    let mut a = app();
    let task = a.snapshot.tasks[a.selected().unwrap()].clone();
    a.handle_key(key('i'));
    a.handle_key(code(KeyCode::Tab));
    assert_eq!(a.form.as_ref().unwrap().focus, Field::List);
    assert_eq!(a.handle_key(code(KeyCode::Enter)), Outcome::Continue);
    let p = a.picker.as_ref().expect("picker open");
    assert_eq!(
        p.action,
        tdo::app::ListPickerAction::SetList,
        "picking for the card"
    );
    assert!(!p.lists.contains(&task.collection));
    a.handle_key(key('j'));
    let to = a.picker.as_ref().unwrap().lists[1].clone();
    assert_eq!(a.handle_key(code(KeyCode::Enter)), Outcome::Continue);
    assert!(a.picker.is_none());
    assert_eq!(a.form.as_ref().unwrap().list, to);
    // Esc in the picker leaves the card as it was.
    a.handle_key(code(KeyCode::Enter));
    a.handle_key(code(KeyCode::Esc));
    assert!(a.picker.is_none() && a.form.is_some());
    assert_eq!(
        write_op(a.handle_key(ctrl('s'))),
        Op::Save {
            uid: task.uid,
            changes: vec![],
            to: Some(to),
        }
    );
}

/// Review Focus 3 through the card: the gate refuses an unverified repeat; the card stays
/// open with the text and the reason.
#[test]
fn refused_save_keeps_card_and_text() {
    let mut a = app();
    select(&mut a, "Call the dentist");
    card_with_repeat(&mut a, "2nd tue");
    let op = write_op(a.handle_key(code(KeyCode::Enter)));
    assert!(matches!(op, Op::Save { .. }), "{op:?}");
    a.write_refused();
    a.write_finished(Err("months-nth-weekday repeats are not verified yet".into()));
    let f = a.form.as_ref().expect("card stays open");
    assert_eq!(f.repeat, "2nd tue");
    assert_eq!(f.focus, Field::Repeat);
    assert_eq!(
        f.error.as_deref(),
        Some("months-nth-weekday repeats are not verified yet")
    );
    assert_eq!(a.mode, Mode::Insert);
}

/// Review Focus 2: a sync reload while the card is open keeps the typing, and the save goes
/// against the snapshot the card opened on.
#[test]
fn sync_reload_keeps_card_text() {
    let mut a = app();
    on_rent(&mut a);
    let uid = selected_uid(&a);
    a.handle_key(key('i'));
    type_str(&mut a, " now");
    let base_due = a.snapshot.tasks[a.snapshot.find(&uid).unwrap()].due.clone();
    a.reload(snapshot_of(rent_advanced()));
    let f = a.form.as_ref().expect("card stays open");
    assert_eq!(f.title, "Pay rent now");
    assert_eq!(f.uid.as_deref(), Some(uid.as_str()));
    let ws = a.write_snapshot();
    assert_eq!(ws.tasks[ws.find(&uid).unwrap()].due, base_due, "the base");
    assert_ne!(
        a.snapshot.tasks[a.snapshot.find(&uid).unwrap()].due,
        base_due
    );
    assert_eq!(
        write_op(a.handle_key(code(KeyCode::Enter))),
        Op::Save {
            uid,
            changes: vec![Change::Summary("Pay rent now".into())],
            to: None,
        }
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
        Op::Save {
            uid: task.uid.clone(),
            changes: vec![Change::Summary(format!("{}!", task.summary))],
            to: None,
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
    a.handle_key(code(KeyCode::BackTab));
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
    a.handle_key(key('J'));
    a.expire_message(t0 + chrono::Duration::seconds(60));
    assert_eq!(a.message.as_deref(), Some("move task: not available yet"));
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
        Op::Save {
            uid,
            changes: vec![Change::Summary("10".into())],
            to: None,
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
fn reload_drops_parent_confirmation_and_its_message() {
    let mut a = app();
    select_tax_return(&mut a);
    let uid = selected_uid(&a);
    a.handle_key(key('x'));
    assert!(a.message.is_some());
    a.reload(a.snapshot.clone());
    assert!(a.message.is_none(), "the confirmation no longer applies");
    // The next x asks again instead of completing.
    assert_eq!(selected_uid(&a), uid);
    assert!(matches!(a.handle_key(key('x')), Outcome::Continue));
    assert!(a.message.is_some());

    // Another message survives the reload.
    let mut a = app();
    select_tax_return(&mut a);
    a.message = Some("something else".into());
    a.reload(a.snapshot.clone());
    assert_eq!(a.message.as_deref(), Some("something else"));
}

#[test]
fn tick_moving_the_selection_drops_parent_confirmation_and_its_message() {
    let mut r = report();
    let parent = r
        .tasks
        .iter_mut()
        .find(|t| t.summary == "Prepare tax return")
        .unwrap();
    parent.due = Some(twydo_core::ical::value::IcalDateTime::Date(
        now().date_naive(),
    ));
    let uid = parent.uid.clone();
    let mut a = App::new(snapshot_of(r), now());
    a.view = View::Next3Days;
    a.cursor = titles(&a)
        .iter()
        .position(|t| t == "Prepare tax return")
        .unwrap();
    a.handle_key(key('x'));
    assert!(a.message.is_some());
    // Tomorrow the parent is overdue, leaving Next 3 Days and moving the selection.
    a.tick(now() + chrono::Duration::days(1));
    assert_ne!(selected_uid(&a), uid);
    assert!(a.message.is_none(), "the confirmation no longer applies");
    // A tick that keeps the selection keeps the confirmation.
    let mut a = app();
    select_tax_return(&mut a);
    a.handle_key(key('x'));
    a.tick(a.now);
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

#[test]
fn x_on_parent_with_one_open_item_uses_the_singular() {
    let mut a = app();
    select_tax_return(&mut a);
    let uid = selected_uid(&a);
    let child = a
        .snapshot
        .tasks
        .iter()
        .position(|t| t.parent.as_deref() == Some(uid.as_str()))
        .unwrap();
    a.snapshot.tasks[child].status = twydo_core::model::Status::Completed;
    a.handle_key(key('x'));
    assert_eq!(
        a.message.as_deref(),
        Some("complete \"Prepare tax return\" and its 1 open item? x to confirm")
    );
}

fn write_req(o: Outcome) -> Request {
    match o {
        Outcome::Write(req) => req,
        other => panic!("expected a write, got {other:?}"),
    }
}

/// The report after "Pay rent" (repeating) was completed: 2Do's live task keeps its UID and
/// moves to its next due date.
fn rent_advanced() -> twydo_core::vdir::LoadReport {
    let mut r = report();
    let rent = r
        .tasks
        .iter_mut()
        .find(|t| t.summary == "Pay rent")
        .unwrap();
    rent.due = Some(twydo_core::ical::value::IcalDateTime::Date(
        chrono::NaiveDate::from_ymd_opt(2026, 12, 7).unwrap(),
    ));
    r
}

/// Open the bills list with the cursor on "Pay rent"; returns its row.
fn on_rent(a: &mut App) -> usize {
    type_str(a, ":list bills");
    a.handle_key(code(KeyCode::Enter));
    let row = titles(a).iter().position(|t| t == "Pay rent").unwrap();
    a.cursor = row;
    row
}

#[test]
fn completing_a_repeat_keeps_the_cursor_on_its_row() {
    let mut a = app();
    let row = on_rent(&mut a);
    let rent = selected_uid(&a);
    assert_eq!(
        write_op(a.handle_key(key('x'))),
        Op::Complete { uid: rent.clone() }
    );
    a.reload(snapshot_of(rent_advanced()));
    assert_ne!(
        titles(&a).iter().position(|t| t == "Pay rent"),
        Some(row),
        "the next occurrence sorts elsewhere"
    );
    assert_eq!(a.cursor, row, "the cursor stays where it was");
    assert_ne!(selected_uid(&a), rent);
}

#[test]
fn reload_after_a_repeat_completion_follows_the_selection_again() {
    let mut a = app();
    on_rent(&mut a);
    a.handle_key(key('x'));
    a.reload(snapshot_of(rent_advanced()));
    let uid = selected_uid(&a);
    // A later reload (the file watcher, a sync) keeps the cursor on the selected task.
    let mut r = rent_advanced();
    r.tasks.retain(|t| t.summary != "Pay water bill");
    a.reload(snapshot_of(r));
    assert_eq!(selected_uid(&a), uid);
}

#[test]
fn a_refused_repeat_completion_keeps_the_selection() {
    let mut a = app();
    on_rent(&mut a);
    let rent = selected_uid(&a);
    write_req(a.handle_key(key('x')));
    // The write waited on a sync that reordered the list, then was refused; the loop
    // reloads after a refusal too.
    a.write_refused();
    let mut r = report();
    r.tasks.retain(|t| t.summary != "Pay water bill");
    a.reload(snapshot_of(r));
    assert_eq!(
        selected_uid(&a),
        rent,
        "rent is still open and still selected"
    );
}

// ---- The new row: `o`, `O`, `a` and the Enter chain ----

fn create_op(o: Outcome) -> NewTask {
    match write_op(o) {
        Op::Create(t) => t,
        other => panic!("expected a create, got {other:?}"),
    }
}

fn target(collection: &str, parent: Option<(&str, TaskKind)>, due_today: bool) -> NewTarget {
    NewTarget {
        collection: collection.into(),
        parent: parent.map(|(p, _)| p.to_string()),
        parent_kind: parent.map(|(_, k)| k),
        due_today,
    }
}

/// `r` plus an undated task `title` (UID `uid`) in `collection`, an item of `parent`: what the
/// vdir holds after the row's save, as the reload before `write_finished` sees it.
fn with_task(
    mut r: twydo_core::vdir::LoadReport,
    uid: &str,
    title: &str,
    collection: &str,
    parent: Option<&str>,
) -> twydo_core::vdir::LoadReport {
    let mut t = r
        .tasks
        .iter()
        .find(|t| t.summary == "Renew library books")
        .unwrap()
        .clone();
    t.uid = uid.into();
    t.summary = title.into();
    t.collection = collection.into();
    t.path = PathBuf::from(format!("{collection}/{uid}.ics"));
    t.due = None;
    t.parent = parent.map(str::to_string);
    r.tasks.push(t);
    r
}

/// The All view with the cursor on `title`.
fn in_all(a: &mut App, title: &str) {
    a.handle_key(key('2'));
    select(a, title);
}

const PACKING: &str = "sv-personal-2";

#[test]
fn o_in_list_targets_that_list() {
    let mut a = app();
    on_rent(&mut a);
    let rent = selected_uid(&a);
    assert_eq!(a.handle_key(key('o')), Outcome::Continue);
    assert_eq!(a.mode, Mode::Insert);
    let r = a.new_row.as_ref().expect("row open");
    assert_eq!(r.target, target("bills", None, false));
    assert_eq!((r.below, r.anchor.as_deref()), (true, Some(rent.as_str())));
    type_str(&mut a, "Pay gas");
    assert_eq!(a.new_row.as_ref().unwrap().title, "Pay gas");
    assert_eq!(
        create_op(a.handle_key(code(KeyCode::Enter))),
        NewTask::plain("bills", "Pay gas")
    );
    // The row stays until the write reports back; then the cursor follows the new task.
    assert!(a.new_row.is_some());
    a.reload(snapshot_of(with_task(
        report(),
        "new-1",
        "Pay gas",
        "bills",
        None,
    )));
    a.write_finished(Ok("added".into()));
    assert!(a.new_row.is_none());
    assert_eq!(a.mode, Mode::Normal);
    assert_eq!(selected_uid(&a), "new-1");
    assert_eq!(a.message.as_deref(), Some("added"));
}

#[test]
fn o_in_today_is_due_today_in_default_list() {
    let mut a = app();
    assert_eq!(a.view, View::Today);
    a.handle_key(key('o'));
    assert_eq!(
        a.new_row.as_ref().unwrap().target,
        target("inbox", None, true),
        "no default_list in the config: the list named Inbox"
    );
    type_str(&mut a, "Stretch");
    let today = now().date_naive();
    assert_eq!(
        create_op(a.handle_key(code(KeyCode::Enter))),
        NewTask {
            due: Some(Due::Date(today)),
            ..NewTask::plain("inbox", "Stretch")
        }
    );

    // The config's default_list wins; an unknown one falls back to Inbox, then the first list.
    let mut s = snapshot_of(report());
    s.config.default_list = Some("personal".into());
    assert_eq!(default_list(&s), "personal");
    let mut a = App::new(s.clone(), now());
    a.handle_key(key('o'));
    assert_eq!(
        a.new_row.as_ref().unwrap().target,
        target("personal", None, true)
    );
    s.config.default_list = Some("gone".into());
    assert_eq!(default_list(&s), "inbox");
    let mut r = report();
    r.collections.retain(|c| c.id != "inbox");
    r.tasks.retain(|t| t.collection != "inbox");
    assert_eq!(default_list(&snapshot_of(r)), "bills");
}

/// Review Focus 1: a new undated task added in Scheduled goes to the default list, the
/// status line says where, and the cursor stays.
#[test]
fn o_in_scheduled_adds_to_inbox_and_says_so() {
    let mut a = app();
    a.handle_key(key('3'));
    a.handle_key(key('j'));
    let (cursor, uid) = (a.cursor, selected_uid(&a));
    a.handle_key(key('o'));
    assert_eq!(
        a.new_row.as_ref().unwrap().target,
        target("inbox", None, false)
    );
    type_str(&mut a, "x");
    assert_eq!(
        create_op(a.handle_key(code(KeyCode::Enter))),
        NewTask::plain("inbox", "x")
    );
    a.reload(snapshot_of(with_task(
        report(),
        "new-1",
        "x",
        "inbox",
        None,
    )));
    a.write_finished(Ok("added".into()));
    assert!(a.new_row.is_none());
    assert_eq!(a.message.as_deref(), Some("Added \"x\" to Inbox"));
    assert_eq!((a.cursor, selected_uid(&a)), (cursor, uid));
}

#[test]
fn o_on_item_targets_parent() {
    let mut a = app();
    in_all(&mut a, "Packing list");
    a.handle_key(code(KeyCode::Enter));
    select(&mut a, "Charger");
    let charger = selected_uid(&a);
    a.handle_key(key('o'));
    let r = a.new_row.as_ref().unwrap();
    assert_eq!(
        r.target,
        target("personal", Some((PACKING, TaskKind::Checklist)), false)
    );
    assert_eq!(r.anchor.as_deref(), Some(charger.as_str()));
    type_str(&mut a, "Socks");
    assert_eq!(
        create_op(a.handle_key(code(KeyCode::Enter))),
        NewTask {
            parent: Some(PACKING.into()),
            ..NewTask::plain("personal", "Socks")
        }
    );
}

#[test]
#[allow(non_snake_case)]
fn O_opens_above() {
    let mut a = app();
    on_rent(&mut a);
    let rent = selected_uid(&a);
    a.handle_key(key('O'));
    let r = a.new_row.as_ref().unwrap();
    assert_eq!((r.below, r.anchor.as_deref()), (false, Some(rent.as_str())));
    assert_eq!(r.target, target("bills", None, false));
}

#[test]
fn enter_in_parent_chains_next_item() {
    let mut a = app();
    in_all(&mut a, "Packing list");
    a.handle_key(key('a'));
    type_str(&mut a, "Socks");
    let item = NewTask {
        parent: Some(PACKING.into()),
        ..NewTask::plain("personal", "Socks")
    };
    assert_eq!(create_op(a.handle_key(code(KeyCode::Enter))), item);
    a.reload(snapshot_of(with_task(
        report(),
        "new-1",
        "Socks",
        "personal",
        Some(PACKING),
    )));
    a.write_finished(Ok("added".into()));
    let parent = a.snapshot.find(PACKING).unwrap();
    let last = a
        .visible_rows()
        .iter()
        .rfind(|r| a.snapshot.parent(r.task) == Some(parent))
        .map(|r| a.snapshot.tasks[r.task].uid.clone())
        .unwrap();
    assert_ne!(
        last, "new-1",
        "the timestamp-free reload sorts the saved item before existing items"
    );
    let r = a.new_row.as_ref().expect("the next item's row opens");
    assert_eq!(r.title, "");
    assert_eq!((r.below, r.anchor.as_deref()), (true, Some(last.as_str())));
    assert_eq!(
        r.target,
        target("personal", Some((PACKING, TaskKind::Checklist)), false)
    );
    assert_eq!(a.mode, Mode::Insert);
    assert_eq!(
        selected_uid(&a),
        "new-1",
        "the cursor follows the saved item"
    );
    type_str(&mut a, "Hat");
    assert_eq!(
        create_op(a.handle_key(code(KeyCode::Enter))),
        NewTask {
            summary: "Hat".into(),
            ..item
        }
    );
}

#[test]
fn enter_on_empty_row_closes() {
    let mut a = app();
    for typed in ["", "   "] {
        a.handle_key(key('o'));
        type_str(&mut a, typed);
        assert_eq!(a.handle_key(code(KeyCode::Enter)), Outcome::Continue);
        assert!(a.new_row.is_none());
        assert_eq!(a.mode, Mode::Normal);
    }
}

#[test]
fn esc_discards_row_keeps_saved_items() {
    let mut a = app();
    in_all(&mut a, "Packing list");
    a.handle_key(key('a'));
    type_str(&mut a, "Socks");
    a.handle_key(code(KeyCode::Enter));
    a.reload(snapshot_of(with_task(
        report(),
        "new-1",
        "Socks",
        "personal",
        Some(PACKING),
    )));
    a.write_finished(Ok("added".into()));
    type_str(&mut a, "Hat");
    assert_eq!(a.handle_key(code(KeyCode::Esc)), Outcome::Continue);
    assert!(a.new_row.is_none() && a.form.is_none());
    assert_eq!(a.mode, Mode::Normal);
    assert!(a.snapshot.find("new-1").is_some(), "the saved item stays");
    assert_eq!(selected_uid(&a), "new-1");
    // Typed text never counts: Esc writes nothing.
    a.handle_key(key('o'));
    type_str(&mut a, "Something");
    assert_eq!(a.handle_key(code(KeyCode::Esc)), Outcome::Continue);
    assert!(a.new_row.is_none());
}

#[test]
fn a_on_parent_opens_row_at_end_and_expands() {
    let mut a = app();
    in_all(&mut a, "Prepare tax return");
    let project = selected_uid(&a);
    assert!(!a.expanded.contains(&project));
    a.handle_key(key('a'));
    assert!(a.expanded.contains(&project), "the parent expands");
    let last_item = |a: &App| {
        let p = a.snapshot.find(&project).unwrap();
        a.visible_rows()
            .iter()
            .filter(|r| a.snapshot.parent(r.task) == Some(p))
            .last()
            .map(|r| a.snapshot.tasks[r.task].uid.clone())
    };
    let r = a.new_row.as_ref().unwrap();
    assert_eq!(
        r.target,
        target("bills", Some(("sv-bills-4", TaskKind::Project)), false)
    );
    assert_eq!((r.below, r.anchor.clone()), (true, last_item(&a)));
    a.handle_key(code(KeyCode::Esc));

    // On an item: a sibling at the end of the same parent.
    let first = a
        .visible_rows()
        .iter()
        .position(|r| a.snapshot.parent(r.task).is_some())
        .unwrap();
    a.cursor = first;
    a.handle_key(key('a'));
    let r = a.new_row.as_ref().unwrap();
    assert_eq!(r.target.parent.as_deref(), Some("sv-bills-4"));
    assert_eq!(r.anchor, last_item(&a));

    // A parent without items: the row goes under the parent itself.
    let mut rep = report();
    rep.tasks.retain(|t| t.parent.as_deref() != Some(PACKING));
    let mut a = App::new(snapshot_of(rep), now());
    in_all(&mut a, "Packing list");
    a.handle_key(key('a'));
    let r = a.new_row.as_ref().unwrap();
    assert_eq!((r.below, r.anchor.as_deref()), (true, Some(PACKING)));
}

#[test]
fn a_on_plain_task_asks_p_or_c() {
    const ASK: &str = "Add items: make it a (p)roject or (c)hecklist?";
    let mut a = app();
    select(&mut a, "Call the dentist");
    let uid = selected_uid(&a);
    assert_eq!(a.handle_key(key('a')), Outcome::Continue);
    assert_eq!(a.message.as_deref(), Some(ASK));
    assert!(a.new_row.is_none());
    assert_eq!(
        write_op(a.handle_key(key('c'))),
        Op::Save {
            uid: uid.clone(),
            changes: vec![Change::Kind(TaskKind::Checklist)],
            to: None,
        }
    );
    assert!(a.new_row.is_none(), "the row waits for the conversion");
    let mut r = report();
    r.tasks.iter_mut().find(|t| t.uid == uid).unwrap().kind = TaskKind::Checklist;
    a.reload(snapshot_of(r));
    a.write_finished(Ok("saved".into()));
    let row = a.new_row.as_ref().expect("the item row opens");
    assert_eq!(
        row.target,
        target("inbox", Some((uid.as_str(), TaskKind::Checklist)), true)
    );
    assert_eq!(
        (row.below, row.anchor.as_deref()),
        (true, Some(uid.as_str()))
    );
    assert_eq!(a.mode, Mode::Insert);

    // `p` converts to a project; a refused conversion says why and opens no row.
    let mut a = app();
    select(&mut a, "Call the dentist");
    a.handle_key(key('a'));
    assert_eq!(
        write_op(a.handle_key(key('p'))),
        Op::Save {
            uid: uid.clone(),
            changes: vec![Change::Kind(TaskKind::Project)],
            to: None,
        }
    );
    a.write_refused();
    a.reload(snapshot_of(report()));
    a.write_finished(Err("task-to-project is not verified yet".into()));
    assert!(a.new_row.is_none());
    assert_eq!(a.mode, Mode::Normal);
    assert_eq!(
        a.message.as_deref(),
        Some("task-to-project is not verified yet")
    );

    // Any other key cancels, and does nothing else.
    let mut a = app();
    select(&mut a, "Call the dentist");
    let cursor = a.cursor;
    a.handle_key(key('a'));
    assert_eq!(a.handle_key(key('j')), Outcome::Continue);
    assert!(a.message.is_none() && a.new_row.is_none());
    assert_eq!(a.cursor, cursor);
    assert_eq!(
        a.handle_key(key('c')),
        Outcome::Continue,
        "the question is gone"
    );
}

#[test]
fn tab_moves_row_into_card_with_title() {
    let mut a = app();
    on_rent(&mut a);
    a.handle_key(key('o'));
    type_str(&mut a, "Pay gas");
    assert_eq!(a.handle_key(code(KeyCode::Tab)), Outcome::Continue);
    assert!(a.new_row.is_none());
    assert_eq!(a.mode, Mode::Insert);
    let f = a.form.as_ref().expect("the card opens");
    assert_eq!((f.uid.as_deref(), f.title.as_str()), (None, "Pay gas"));
    assert_eq!(f.list, "bills");
    assert_eq!(
        create_op(a.handle_key(ctrl('s'))),
        NewTask::plain("bills", "Pay gas")
    );
    a.reload(snapshot_of(with_task(
        report(),
        "new-1",
        "Pay gas",
        "bills",
        None,
    )));
    a.write_finished(Ok("added".into()));
    assert!(a.form.is_none());
    assert_eq!(selected_uid(&a), "new-1", "the cursor follows the new task");

    // A checklist item's card has only Title and Notes, and writes no dates even in Today.
    let mut r = report();
    let packing = r.tasks.iter_mut().find(|t| t.uid == PACKING).unwrap();
    packing.due = Some(twydo_core::ical::value::IcalDateTime::Date(
        now().date_naive(),
    ));
    let mut a = App::new(snapshot_of(r), now());
    select(&mut a, "Packing list");
    a.handle_key(key('a'));
    assert!(a.new_row.as_ref().unwrap().target.due_today);
    type_str(&mut a, "Socks");
    a.handle_key(code(KeyCode::Tab));
    let f = a.form.as_ref().unwrap();
    assert_eq!(f.fields(), vec![Field::Title, Field::Notes]);
    assert_eq!(
        create_op(a.handle_key(ctrl('s'))),
        NewTask {
            parent: Some(PACKING.into()),
            ..NewTask::plain("personal", "Socks")
        }
    );
}

/// Review Focus 5: the parent is deleted (on the phone) mid-chain. The row keeps its text
/// through the reload; the next save is refused and the row closes. Earlier items stay.
#[test]
fn chain_after_parent_deleted_closes_row() {
    let mut a = app();
    in_all(&mut a, "Packing list");
    a.handle_key(key('a'));
    type_str(&mut a, "Socks");
    a.handle_key(code(KeyCode::Enter));
    a.reload(snapshot_of(with_task(
        report(),
        "new-1",
        "Socks",
        "personal",
        Some(PACKING),
    )));
    a.write_finished(Ok("added".into()));
    type_str(&mut a, "Hat");
    let mut gone = with_task(report(), "new-1", "Socks", "personal", Some(PACKING));
    gone.tasks.retain(|t| t.uid != PACKING);
    a.reload(snapshot_of(gone));
    assert_eq!(
        a.new_row.as_ref().unwrap().title,
        "Hat",
        "the text survives"
    );
    let t = create_op(a.handle_key(code(KeyCode::Enter)));
    assert_eq!(t.parent.as_deref(), Some(PACKING));
    assert!(
        a.write_snapshot().find(PACKING).is_none(),
        "a create is checked against the tasks as they are now"
    );
    a.write_refused();
    a.write_finished(Err(format!("no task with UID {PACKING}")));
    assert!(a.new_row.is_none());
    assert_eq!(a.mode, Mode::Normal);
    assert_eq!(
        a.message.as_deref(),
        Some(format!("no task with UID {PACKING}").as_str())
    );
    assert!(a.snapshot.find("new-1").is_some(), "the earlier item stays");
}

/// A refused row save (an unverified item, a sync that would not stop) keeps the row and its
/// text, with the reason.
#[test]
fn refused_row_save_keeps_text() {
    let mut a = app();
    in_all(&mut a, "Packing list");
    a.handle_key(key('a'));
    type_str(&mut a, "Socks");
    a.handle_key(code(KeyCode::Enter));
    a.write_refused();
    a.reload(snapshot_of(report()));
    a.write_finished(Err("checklist items are not verified yet".into()));
    let r = a.new_row.as_ref().expect("the row stays");
    assert_eq!(r.title, "Socks");
    assert_eq!(
        r.error.as_deref(),
        Some("checklist items are not verified yet")
    );
    assert_eq!(a.mode, Mode::Insert);
    // Typing clears the reason.
    type_str(&mut a, "!");
    assert_eq!(a.new_row.as_ref().unwrap().error, None);
}

#[test]
fn status_line_names_target() {
    use ratatui::{Terminal, backend::TestBackend};
    fn status(a: &mut App) -> String {
        a.resize(120, 30);
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        term.draw(|f| tdo::ui::draw(f, a)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..120).map(|x| buf[(x, 29)].symbol()).collect()
    }
    let mut a = app();
    a.handle_key(key('o'));
    let s = status(&mut a);
    assert!(s.contains(" INSERT "), "{s}");
    assert!(
        s.contains("New task in Inbox · Enter save  Tab more  Esc cancel"),
        "{s}"
    );
    a.handle_key(code(KeyCode::Esc));
    on_rent(&mut a);
    a.handle_key(key('o'));
    assert!(status(&mut a).contains("New task in Bills ·"));
}

#[test]
fn row_editing_keys_and_paste() {
    let mut a = app();
    a.handle_key(key('o'));
    type_str(&mut a, "Call the plumber");
    a.handle_key(ctrl('w'));
    assert_eq!(a.new_row.as_ref().unwrap().title, "Call the");
    a.handle_key(code(KeyCode::Backspace));
    assert_eq!(a.new_row.as_ref().unwrap().title, "Call th");
    a.handle_key(ctrl('u'));
    assert_eq!(a.new_row.as_ref().unwrap().title, "");
    assert_eq!(a.paste("Buy\nmilk\n"), Outcome::Continue);
    assert_eq!(a.new_row.as_ref().unwrap().title, "Buy milk");
    // Normal-mode keys type into the row; they never act on the list.
    type_str(&mut a, " x dd");
    assert_eq!(a.new_row.as_ref().unwrap().title, "Buy milk x dd");
}

fn kind_save(uid: &str, kind: TaskKind) -> Op {
    Op::Save {
        uid: uid.into(),
        changes: vec![Change::Kind(kind)],
        to: None,
    }
}

#[test]
fn ct_cp_cc_emit_kind_change() {
    let mut a = app();
    select(&mut a, "Call the dentist");
    let uid = selected_uid(&a);
    for (keys, kind) in [("cp", TaskKind::Project), ("cc", TaskKind::Checklist)] {
        let mut a = app();
        select(&mut a, "Call the dentist");
        assert_eq!(a.handle_key(key('c')), Outcome::Continue);
        assert_eq!(
            write_op(a.handle_key(key(keys.chars().nth(1).unwrap()))),
            kind_save(&uid, kind)
        );
    }
    // `ct` on a checklist turns it back into a task; the Details pane acts too.
    let mut a = app();
    in_all(&mut a, "Packing list");
    a.handle_key(key('c'));
    assert_eq!(
        write_op(a.handle_key(key('t'))),
        kind_save(PACKING, TaskKind::Task)
    );
    let mut a = app();
    select(&mut a, "Call the dentist");
    a.focus = Pane::Details;
    a.handle_key(key('c'));
    assert_eq!(
        write_op(a.handle_key(key('p'))),
        kind_save(&uid, TaskKind::Project)
    );
}

#[test]
fn same_kind_writes_nothing_and_says_so() {
    let mut rep = report();
    rep.tasks
        .retain(|t| t.parent.as_deref() != Some("sv-bills-4"));
    let mut a = App::new(snapshot_of(rep), now());
    in_all(&mut a, "Prepare tax return");
    a.handle_key(key('c'));
    assert_eq!(a.handle_key(key('p')), Outcome::Continue);
    assert_eq!(
        a.message.as_deref(),
        Some("\"Prepare tax return\" is already a project")
    );
    let mut a = app();
    select(&mut a, "Call the dentist");
    a.handle_key(key('c'));
    assert_eq!(a.handle_key(key('t')), Outcome::Continue);
    assert_eq!(
        a.message.as_deref(),
        Some("\"Call the dentist\" is already a task")
    );
}

#[test]
fn cc_on_project_with_items_shows_the_refusal() {
    let mut a = app();
    in_all(&mut a, "Prepare tax return");
    let uid = selected_uid(&a);
    a.handle_key(key('c'));
    assert_eq!(
        write_op(a.handle_key(key('c'))),
        kind_save(&uid, TaskKind::Checklist)
    );
    a.write_finished(Err("project-to-checklist is not supported".into()));
    assert_eq!(
        a.message.as_deref(),
        Some("project-to-checklist is not supported")
    );
}

#[test]
fn c_then_other_key_cancels() {
    let mut a = app();
    select(&mut a, "Call the dentist");
    a.handle_key(key('c'));
    assert_eq!(a.handle_key(key('j')), Outcome::Continue);
    // The pending `c` is gone: a lone `p` does nothing.
    assert_eq!(a.handle_key(key('p')), Outcome::Continue);
}

#[test]
fn c_is_ignored_under_overlay() {
    let mut a = app();
    select(&mut a, "Call the dentist");
    a.handle_key(key('?'));
    a.handle_key(key('c'));
    assert_eq!(a.handle_key(key('p')), Outcome::Continue);
}

/// A plain task with an item shows as a project but its file declares a task: `ct` would
/// write only new stamps, so it says so instead; `cp` writes the declared type.
#[test]
fn ct_on_an_untyped_parent_writes_nothing() {
    let mut rep = report();
    let d = rep
        .tasks
        .iter()
        .find(|t| t.summary == "Call the dentist")
        .unwrap();
    let (dentist, list) = (d.uid.clone(), d.collection.clone());
    let other = rep
        .tasks
        .iter_mut()
        .find(|t| t.parent.is_none() && t.uid != dentist && t.collection == list)
        .unwrap();
    other.parent = Some(dentist.clone());
    let snap = snapshot_of(rep);
    let i = snap.find(&dentist).unwrap();
    assert_eq!(snap.tasks[i].kind, TaskKind::Project, "shown as a project");
    let mut a = App::new(snap, now());
    in_all(&mut a, "Call the dentist");
    a.handle_key(key('c'));
    assert_eq!(a.handle_key(key('t')), Outcome::Continue);
    assert_eq!(
        a.message.as_deref(),
        Some("\"Call the dentist\" is already a task")
    );
    a.handle_key(key('c'));
    assert_eq!(
        write_op(a.handle_key(key('p'))),
        kind_save(&dentist, TaskKind::Project)
    );
}

#[test]
fn list_completion_and_reopen_keep_selection_as_the_task_changes_section() {
    let mut a = app();
    type_str(&mut a, ":list inbox");
    a.handle_key(code(KeyCode::Enter));
    a.cursor = titles(&a)
        .iter()
        .position(|t| t == "Call the dentist")
        .unwrap();
    let uid = selected_uid(&a);
    assert_eq!(
        write_op(a.handle_key(key('x'))),
        Op::Complete { uid: uid.clone() }
    );
    let mut r = report();
    let t = r.tasks.iter_mut().find(|t| t.uid == uid).unwrap();
    t.status = twydo_core::model::Status::Completed;
    t.completed = Some(twydo_core::ical::value::IcalDateTime::Utc(now().to_utc()));
    a.reload(snapshot_of(r));
    assert_eq!(selected_uid(&a), uid);
    let rows = a.visible_rows();
    assert_eq!(
        a.snapshot.tasks[rows[a.cursor + 1].task].summary,
        "Buy stamps"
    );
    assert_eq!(
        write_op(a.handle_key(key('x'))),
        Op::Reopen { uid: uid.clone() }
    );
    a.reload(snapshot_of(report()));
    assert_eq!(selected_uid(&a), uid);
    assert_eq!(a.cursor, 1); // Due today, following the overdue task.
}

#[test]
fn reopening_detached_child_keeps_selection_under_a_collapsed_parent() {
    for view in [View::All, View::List("bills".into())] {
        for kind in [TaskKind::Project, TaskKind::Checklist] {
            let mut r = report();
            let parent = r
                .tasks
                .iter_mut()
                .find(|t| t.summary == "Prepare tax return")
                .unwrap();
            parent.kind = kind;
            let parent_uid = parent.uid.clone();
            let child = r
                .tasks
                .iter_mut()
                .find(|t| t.parent.as_deref() == Some(&parent_uid))
                .unwrap();
            let uid = child.uid.clone();
            child.status = twydo_core::model::Status::Completed;
            child.completed = Some(twydo_core::ical::value::IcalDateTime::Utc(now().to_utc()));
            let completed = snapshot_of(r);
            let mut a = App::new(completed.clone(), now());
            a.view = view.clone();
            a.cursor = a
                .visible_rows()
                .iter()
                .position(|r| a.snapshot.tasks[r.task].uid == uid)
                .unwrap();

            // An unchanged reload keeps the child detached and its parent collapsed.
            a.reload(completed.clone());
            assert_eq!(selected_uid(&a), uid);
            assert!(!a.expanded.contains(&parent_uid));
            assert_eq!(
                write_op(a.handle_key(key('x'))),
                Op::Reopen { uid: uid.clone() }
            );
            let mut reopened = completed;
            let child = reopened.find(&uid).unwrap();
            reopened.tasks[child].status = twydo_core::model::Status::Open;
            reopened.tasks[child].completed = None;
            a.reload(reopened);

            assert_eq!(selected_uid(&a), uid, "{view:?}, {kind:?}");
            assert_eq!(a.visible_rows()[a.cursor].depth, 1);
            assert!(a.expanded.contains(&parent_uid));
        }
    }
}
