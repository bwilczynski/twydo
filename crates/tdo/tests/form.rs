//! The edit form's state machine, on synthetic sample tasks.

use std::path::PathBuf;

use chrono::{NaiveDate, NaiveTime};
use chrono_tz::Tz;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tdo::form::{EditForm, Field, FormAction};
use twydo_core::edit::{Change, Due};
use twydo_core::model::Task;

const NY: Tz = chrono_tz::America::New_York;

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 4).unwrap()
}

fn task(summary: &str) -> Task {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../twydo-core/tests/data/sample-vdir");
    twydo_core::vdir::load(&root)
        .unwrap()
        .tasks
        .into_iter()
        .find(|t| t.summary == summary)
        .unwrap()
}

fn form(summary: &str) -> EditForm {
    EditForm::open(&task(summary), NY)
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

fn type_str(f: &mut EditForm, s: &str) {
    for c in s.chars() {
        f.key(key(c));
    }
}

/// Move focus to `field` with Tab.
fn focus(f: &mut EditForm, field: Field) {
    for _ in 0..10 {
        if f.focus == field {
            return;
        }
        f.key(code(KeyCode::Tab));
    }
    panic!("cannot reach {field:?}");
}

#[test]
fn open_fills_fields() {
    let f = form("Pay water bill");
    assert_eq!(f.title, "Pay water bill");
    assert_eq!(f.due, "2026-11-06 09:00");
    assert_eq!(f.start, "");
    assert_eq!(f.notes, "");
    let mut t = task("Pay water bill");
    t.description = Some("line 1\nline 2".into());
    assert_eq!(EditForm::open(&t, NY).notes, "line 1\nline 2");
    assert_eq!(f.repeat, None);
    assert_eq!(f.focus, Field::Title);
    assert_eq!(
        f.fields(),
        vec![Field::Title, Field::Due, Field::Start, Field::Notes]
    );

    let f = form("Pay rent");
    assert_eq!(f.repeat.as_deref(), Some("1"));
    assert_eq!(f.repeat_unit, "months");
    assert!(f.fields().contains(&Field::Repeat));
    assert_eq!(form("Read the saved article").start, "2026-11-10");
}

#[test]
fn unchanged_form_has_no_changes() {
    for s in [
        "Pay water bill",
        "Pay rent",
        "Read the saved article",
        "Packing list",
    ] {
        assert_eq!(form(s).changes(today()), Ok(vec![]), "{s}");
    }
}

#[test]
fn typed_due_becomes_change() {
    let mut f = form("Pay water bill");
    focus(&mut f, Field::Due);
    f.key(ctrl('u'));
    type_str(&mut f, "fri 5pm");
    let at = NaiveDate::from_ymd_opt(2026, 11, 6)
        .unwrap()
        .and_time(NaiveTime::from_hms_opt(17, 0, 0).unwrap());
    assert_eq!(
        f.changes(today()),
        Ok(vec![Change::Due(Some(Due::Time(at)))])
    );
}

#[test]
fn cleared_due_is_none() {
    let mut f = form("Buy stamps");
    focus(&mut f, Field::Due);
    f.key(ctrl('u'));
    assert_eq!(f.changes(today()), Ok(vec![Change::Due(None)]));
}

#[test]
fn bad_due_is_error() {
    let mut f = form("Buy stamps");
    focus(&mut f, Field::Due);
    f.key(ctrl('u'));
    type_str(&mut f, "someday");
    assert_eq!(
        f.changes(today()),
        Err("due: can't read date \"someday\"".into())
    );
}

#[test]
fn start_with_time_is_error() {
    let mut f = form("Buy stamps");
    focus(&mut f, Field::Start);
    type_str(&mut f, "fri 5pm");
    assert_eq!(
        f.changes(today()),
        Err("start: start dates have no time".into())
    );
    f.key(ctrl('u'));
    type_str(&mut f, "fri");
    assert_eq!(
        f.changes(today()),
        Ok(vec![Change::Start(NaiveDate::from_ymd_opt(2026, 11, 6))])
    );
}

#[test]
fn empty_title_is_error() {
    let mut f = form("Buy stamps");
    f.key(ctrl('u'));
    assert_eq!(f.changes(today()), Err("title must not be empty".into()));
    type_str(&mut f, "  ");
    assert_eq!(f.changes(today()), Err("title must not be empty".into()));
}

#[test]
fn title_and_notes_changes_in_field_order() {
    let mut t = task("Pay water bill");
    t.description = Some("n".into());
    let mut f = EditForm::open(&t, NY);
    f.set_notes(String::new());
    type_str(&mut f, "!");
    assert_eq!(
        f.changes(today()),
        Ok(vec![
            Change::Summary("Pay water bill!".into()),
            Change::Description(None)
        ])
    );
    f.set_notes("a\nb".into());
    assert_eq!(
        f.changes(today()).unwrap()[1],
        Change::Description(Some("a\nb".into()))
    );
}

#[test]
fn repeat_interval_change() {
    let mut f = form("Pay rent");
    focus(&mut f, Field::Repeat);
    f.key(ctrl('u'));
    type_str(&mut f, "x2");
    assert_eq!(f.repeat.as_deref(), Some("2"), "digits only");
    assert_eq!(f.changes(today()), Ok(vec![Change::RepeatInterval(2)]));
    f.key(ctrl('u'));
    type_str(&mut f, "0");
    assert_eq!(
        f.changes(today()),
        Err("repeat: enter a number of months".into())
    );
}

#[test]
fn backspace_and_ctrl_w_are_char_aware() {
    let mut f = form("Buy stamps");
    f.key(ctrl('u'));
    type_str(&mut f, "Zażółć gęślą");
    f.key(code(KeyCode::Backspace));
    assert_eq!(f.title, "Zażółć gęśl");
    f.key(ctrl('w'));
    assert_eq!(f.title, "Zażółć");
    f.key(ctrl('w'));
    assert_eq!(f.title, "");
    f.key(code(KeyCode::Backspace));
    assert_eq!(f.title, "");
}

#[test]
fn tab_cycles_and_skips_absent_repeat() {
    let mut f = form("Buy stamps");
    let mut seen = vec![f.focus];
    for _ in 0..4 {
        f.key(code(KeyCode::Tab));
        seen.push(f.focus);
    }
    assert_eq!(
        seen,
        vec![
            Field::Title,
            Field::Due,
            Field::Start,
            Field::Notes,
            Field::Title
        ]
    );
    f.key(code(KeyCode::BackTab));
    assert_eq!(f.focus, Field::Notes);
    f.key(code(KeyCode::Up));
    assert_eq!(f.focus, Field::Start);
    f.key(code(KeyCode::Down));
    assert_eq!(f.focus, Field::Notes);
}

#[test]
fn enter_saves_or_asks_for_editor() {
    let mut f = form("Buy stamps");
    assert_eq!(f.key(code(KeyCode::Enter)), FormAction::Save);
    assert_eq!(f.key(ctrl('s')), FormAction::Save);
    assert_eq!(f.key(code(KeyCode::Esc)), FormAction::Cancel);
    focus(&mut f, Field::Notes);
    assert_eq!(f.key(code(KeyCode::Enter)), FormAction::EditNotes);
    assert_eq!(f.key(key('q')), FormAction::None);
    assert_eq!(f.notes, "", "typing does not edit notes");
}

#[test]
fn preview_shows_parsed_or_error() {
    let mut f = form("Buy stamps");
    assert_eq!(
        f.preview(Field::Due, today()).as_deref(),
        Some("→ Mon 2 Nov")
    );
    assert_eq!(f.preview(Field::Start, today()).as_deref(), Some("→ none"));
    assert_eq!(f.preview(Field::Title, today()), None);
    focus(&mut f, Field::Due);
    f.key(ctrl('u'));
    type_str(&mut f, "fri 5pm");
    assert_eq!(
        f.preview(Field::Due, today()).as_deref(),
        Some("→ Fri 6 Nov 17:00")
    );
    type_str(&mut f, "x");
    assert_eq!(
        f.preview(Field::Due, today()).as_deref(),
        Some("can't read date \"fri 5pmx\"")
    );
}

#[test]
fn paste_inserts_one_line_into_focused_field() {
    let mut f = form("Buy stamps");
    f.key(ctrl('u'));
    f.paste("fix door\nand\twindow\r\n");
    assert_eq!(f.title, "fix door and window");
    focus(&mut f, Field::Notes);
    f.paste("ignored");
    assert_eq!(f.notes, "");
}

#[test]
fn title_is_saved_trimmed() {
    let mut f = form("Buy stamps");
    f.key(ctrl('u'));
    type_str(&mut f, "  New title  ");
    assert_eq!(
        f.changes(today()),
        Ok(vec![Change::Summary("New title".into())])
    );
}

#[test]
fn title_with_only_added_spaces_is_no_change() {
    let mut f = form("Buy stamps");
    type_str(&mut f, "  ");
    assert_eq!(f.changes(today()), Ok(vec![]));
}

#[test]
fn repeat_with_leading_zero_of_same_number_is_no_change() {
    let mut f = form("Pay rent");
    focus(&mut f, Field::Repeat);
    f.key(ctrl('u'));
    type_str(&mut f, "01");
    assert_eq!(f.changes(today()), Ok(vec![]));
}
