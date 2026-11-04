//! The edit form's state machine, on synthetic sample tasks.

use std::path::PathBuf;

use chrono::{NaiveDate, NaiveTime, Weekday};
use chrono_tz::Tz;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tdo::form::{EditForm, Field, FormAction, NO_CHANGES, NewTarget};
use twydo_core::edit::{Change, Due, NewTask, Op};
use twydo_core::model::{Task, TaskKind};
use twydo_core::repeatinput::{self, EXAMPLES};

const NY: Tz = chrono_tz::America::New_York;
const WD: [Weekday; 5] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
];

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 4).unwrap()
}

fn tasks() -> Vec<Task> {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../twydo-core/tests/data/sample-vdir");
    twydo_core::vdir::load(&root).unwrap().tasks
}

fn task(summary: &str) -> Task {
    tasks().into_iter().find(|t| t.summary == summary).unwrap()
}

/// The kind of the parent `t` is an item of, as the app looks it up.
fn item_of(t: &Task) -> Option<TaskKind> {
    let p = t.parent.as_ref()?;
    tasks().into_iter().find(|x| &x.uid == p).map(|x| x.kind)
}

fn form(summary: &str) -> EditForm {
    let t = task(summary);
    EditForm::open(&t, item_of(&t), NY)
}

fn changes(f: &EditForm) -> Result<Vec<Change>, String> {
    f.changes(today(), &WD)
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
    for _ in 0..12 {
        if f.focus == field {
            return;
        }
        f.key(code(KeyCode::Tab));
    }
    panic!("cannot reach {field:?}");
}

#[test]
fn unchanged_form_has_no_changes() {
    for s in [
        "Pay water bill",
        "Pay rent",
        "Read the saved article",
        "Packing list",
    ] {
        assert_eq!(changes(&form(s)), Ok(vec![]), "{s}");
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
    assert_eq!(changes(&f), Ok(vec![Change::Due(Some(Due::Time(at)))]));
}

#[test]
fn cleared_due_is_none() {
    let mut f = form("Buy stamps");
    focus(&mut f, Field::Due);
    f.key(ctrl('u'));
    assert_eq!(changes(&f), Ok(vec![Change::Due(None)]));
}

#[test]
fn bad_due_is_error() {
    let mut f = form("Buy stamps");
    focus(&mut f, Field::Due);
    f.key(ctrl('u'));
    type_str(&mut f, "someday");
    assert_eq!(changes(&f), Err("due: can't read date \"someday\"".into()));
}

#[test]
fn start_with_time_is_error() {
    let mut f = form("Buy stamps");
    focus(&mut f, Field::Start);
    type_str(&mut f, "fri 5pm");
    assert_eq!(changes(&f), Err("start: start dates have no time".into()));
    f.key(ctrl('u'));
    type_str(&mut f, "fri");
    assert_eq!(
        changes(&f),
        Ok(vec![Change::Start(NaiveDate::from_ymd_opt(2026, 11, 6))])
    );
}

#[test]
fn empty_title_is_error() {
    let mut f = form("Buy stamps");
    f.key(ctrl('u'));
    assert_eq!(changes(&f), Err("title must not be empty".into()));
    type_str(&mut f, "  ");
    assert_eq!(changes(&f), Err("title must not be empty".into()));
}

#[test]
fn title_and_notes_changes_in_field_order() {
    let mut t = task("Pay water bill");
    t.description = Some("n".into());
    let mut f = EditForm::open(&t, None, NY);
    f.set_notes(String::new());
    type_str(&mut f, "!");
    assert_eq!(
        changes(&f),
        Ok(vec![
            Change::Summary("Pay water bill!".into()),
            Change::Description(None)
        ])
    );
    f.set_notes("a\nb".into());
    assert_eq!(
        changes(&f).unwrap()[1],
        Change::Description(Some("a\nb".into()))
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
        f.preview(Field::Due, today(), &WD).as_deref(),
        Some("→ Mon 2 Nov")
    );
    assert_eq!(
        f.preview(Field::Start, today(), &WD).as_deref(),
        Some("→ none")
    );
    assert_eq!(f.preview(Field::Title, today(), &WD), None);
    focus(&mut f, Field::Due);
    f.key(ctrl('u'));
    type_str(&mut f, "fri 5pm");
    assert_eq!(
        f.preview(Field::Due, today(), &WD).as_deref(),
        Some("→ Fri 6 Nov 17:00")
    );
    type_str(&mut f, "x");
    assert_eq!(
        f.preview(Field::Due, today(), &WD).as_deref(),
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
    assert_eq!(changes(&f), Ok(vec![Change::Summary("New title".into())]));
}

#[test]
fn title_with_only_added_spaces_is_no_change() {
    let mut f = form("Buy stamps");
    type_str(&mut f, "  ");
    assert_eq!(changes(&f), Ok(vec![]));
}

#[test]
fn open_fills_fields() {
    let f = form("Pay water bill");
    assert_eq!(f.uid.as_deref(), Some(task("Pay water bill").uid.as_str()));
    assert_eq!(f.title, "Pay water bill");
    assert_eq!(f.list, "bills");
    assert_eq!(f.kind, TaskKind::Task);
    assert_eq!(f.due, "2026-11-06 09:00");
    assert_eq!(f.start, "");
    assert_eq!(f.repeat, "");
    assert_eq!(f.notes, "");
    let mut t = task("Pay water bill");
    t.description = Some("line 1\nline 2".into());
    assert_eq!(EditForm::open(&t, None, NY).notes, "line 1\nline 2");
    assert_eq!(f.focus, Field::Title);
    assert_eq!(
        f.fields(),
        vec![
            Field::Title,
            Field::List,
            Field::Type,
            Field::Due,
            Field::Start,
            Field::Repeat,
            Field::Notes
        ]
    );
    assert_eq!(form("Read the saved article").start, "2026-11-10");
    assert_eq!(form("Packing list").kind, TaskKind::Checklist);
}

#[test]
fn tab_cycles_through_every_field() {
    let mut f = form("Buy stamps");
    let mut seen = vec![f.focus];
    for _ in 0..7 {
        f.key(code(KeyCode::Tab));
        seen.push(f.focus);
    }
    assert_eq!(
        seen,
        vec![
            Field::Title,
            Field::List,
            Field::Type,
            Field::Due,
            Field::Start,
            Field::Repeat,
            Field::Notes,
            Field::Title
        ]
    );
    f.key(code(KeyCode::BackTab));
    assert_eq!(f.focus, Field::Notes);
    f.key(code(KeyCode::Up));
    assert_eq!(f.focus, Field::Repeat);
    f.key(code(KeyCode::Down));
    assert_eq!(f.focus, Field::Notes);
}

#[test]
fn repeat_field_prefills_rule_text() {
    assert_eq!(form("Water plants").repeat, "wed");
    assert_eq!(form("Renew car insurance").repeat, "yearly");
    assert_eq!(form("Buy stamps").repeat, "", "no repeat: empty field");
    for s in ["Water plants", "Renew car insurance"] {
        assert_eq!(changes(&form(s)), Ok(vec![]), "{s}");
    }

    // A rule the syntax cannot say (monthly on a fixed day) is kept as it is: the field
    // starts empty, saving writes no repeat change, and the preview names the rule.
    let mut f = form("Pay rent");
    assert_eq!(f.repeat, "");
    assert_eq!(changes(&f), Ok(vec![]));
    let p = f.preview(Field::Repeat, today(), &WD).unwrap();
    assert!(p.starts_with("→ ") && p.contains("kept"), "{p}");
    // Typing `none` removes it.
    focus(&mut f, Field::Repeat);
    type_str(&mut f, "none");
    assert_eq!(changes(&f), Ok(vec![Change::Repeat(None)]));
}

#[test]
fn repeat_typed_text_becomes_one_change() {
    let mut f = form("Water plants");
    focus(&mut f, Field::Repeat);
    f.key(ctrl('u'));
    type_str(&mut f, "2w after done");
    let want = repeatinput::parse("2w after done", today(), &WD).unwrap();
    assert_eq!(changes(&f), Ok(vec![Change::Repeat(want)]));
    // The same rule in other words is no change.
    f.key(ctrl('u'));
    type_str(&mut f, "every wed");
    assert_eq!(changes(&f), Ok(vec![]));
    f.key(ctrl('u'));
    type_str(&mut f, "fortnightly");
    assert!(changes(&f).unwrap_err().starts_with("repeat: "));
    f.key(ctrl('u'));
    assert_eq!(
        changes(&f),
        Ok(vec![Change::Repeat(None)]),
        "emptied: removed"
    );
}

#[test]
fn repeat_without_due_makes_it_due_today() {
    let mut f = form("Packing list");
    assert_eq!(f.due, "");
    focus(&mut f, Field::Repeat);
    type_str(&mut f, "weekly");
    let p = f.preview(Field::Repeat, today(), &WD).unwrap();
    assert!(p.ends_with("· due today"), "{p}");
    let spec = repeatinput::parse("weekly", today(), &WD).unwrap();
    assert_eq!(
        changes(&f),
        Ok(vec![
            Change::Due(Some(Due::Date(today()))),
            Change::Repeat(spec)
        ])
    );
}

#[test]
fn repeat_preview_and_examples() {
    let mut f = form("Buy stamps");
    assert_eq!(
        f.preview(Field::Repeat, today(), &WD),
        None,
        "unfocused: none"
    );
    focus(&mut f, Field::Repeat);
    assert_eq!(
        f.preview(Field::Repeat, today(), &WD).as_deref(),
        Some(EXAMPLES)
    );
    type_str(&mut f, "monthly");
    assert_eq!(
        f.preview(Field::Repeat, today(), &WD).as_deref(),
        Some("→ Monthly on the 2nd · from due date"),
        "anchored on the due date"
    );
    f.key(ctrl('u'));
    type_str(&mut f, "none");
    assert_eq!(
        f.preview(Field::Repeat, today(), &WD).as_deref(),
        Some("→ none")
    );
    f.key(ctrl('u'));
    type_str(&mut f, "15th");
    assert_eq!(
        f.preview(Field::Repeat, today(), &WD),
        repeatinput::parse("15th", today(), &WD).err(),
        "the parse error"
    );
}

#[test]
fn type_field_cycles_with_space_and_hl() {
    let mut f = form("Buy stamps");
    focus(&mut f, Field::Type);
    assert_eq!(f.key(key(' ')), FormAction::None);
    assert_eq!(f.kind, TaskKind::Project);
    f.key(key('l'));
    assert_eq!(f.kind, TaskKind::Checklist);
    f.key(key('l'));
    assert_eq!(f.kind, TaskKind::Task);
    f.key(key('h'));
    assert_eq!(f.kind, TaskKind::Checklist);
    f.key(key('x'));
    assert_eq!(f.kind, TaskKind::Checklist, "other keys do nothing");
    assert_eq!(changes(&f), Ok(vec![Change::Kind(TaskKind::Checklist)]));
    f.key(key('l'));
    assert_eq!(changes(&f), Ok(vec![]), "back to Task: no change");
}

#[test]
fn checklist_item_has_title_and_notes_only() {
    let f = form("Passport");
    assert_eq!(f.fields(), vec![Field::Title, Field::Notes]);
    let f = form("Review draft forms");
    assert_eq!(
        f.fields(),
        vec![
            Field::Title,
            Field::Type,
            Field::Due,
            Field::Start,
            Field::Repeat,
            Field::Notes
        ],
        "a project item: no List"
    );
}

#[test]
fn enter_on_list_picks_and_set_list_moves() {
    let mut f = form("Buy stamps");
    focus(&mut f, Field::List);
    assert_eq!(f.key(code(KeyCode::Enter)), FormAction::PickList);
    f.key(key('x'));
    assert_eq!(f.list, "inbox", "the list is not typed");
    f.set_list("personal".into());
    assert_eq!(f.list, "personal");
}

#[test]
fn save_builds_one_op_with_list_change() {
    let t = task("Buy stamps");
    let mut f = form("Buy stamps");
    assert_eq!(f.save(today(), &WD), Err(NO_CHANGES.to_string()));
    type_str(&mut f, "!");
    assert_eq!(
        f.save(today(), &WD),
        Ok(Op::Save {
            uid: t.uid.clone(),
            changes: vec![Change::Summary("Buy stamps!".into())],
            to: None,
        }),
        "an unchanged list sends no move"
    );
    f.set_list("personal".into());
    focus(&mut f, Field::Type);
    f.key(key('l'));
    assert_eq!(
        f.save(today(), &WD),
        Ok(Op::Save {
            uid: t.uid.clone(),
            changes: vec![
                Change::Summary("Buy stamps!".into()),
                Change::Kind(TaskKind::Project)
            ],
            to: Some("personal".into()),
        })
    );
    // A move alone is a save too; moving back is none.
    let mut f = form("Buy stamps");
    f.set_list("personal".into());
    assert_eq!(
        f.save(today(), &WD),
        Ok(Op::Save {
            uid: t.uid.clone(),
            changes: vec![],
            to: Some("personal".into()),
        })
    );
    f.set_list("inbox".into());
    assert_eq!(f.save(today(), &WD), Err(NO_CHANGES.to_string()));
    f.key(ctrl('u'));
    assert_eq!(
        f.save(today(), &WD),
        Err("title must not be empty".to_string())
    );
}

#[test]
fn new_task_saves_as_create() {
    let target = NewTarget {
        collection: "inbox".into(),
        parent: None,
        parent_kind: None,
        due_today: true,
    };
    let mut f = EditForm::new_task(target, "Plan the garden".into(), NY);
    assert_eq!(f.uid, None);
    assert_eq!(f.due, "today");
    assert_eq!(f.focus, Field::Title);
    focus(&mut f, Field::Repeat);
    type_str(&mut f, "monthly");
    let repeat = repeatinput::parse("monthly", today(), &WD).unwrap();
    assert_eq!(
        f.save(today(), &WD),
        Ok(Op::Create(NewTask {
            due: Some(Due::Date(today())),
            repeat,
            ..NewTask::plain("inbox", "Plan the garden")
        }))
    );

    // An item with no dates; a repeat on an undated new task makes it due today.
    let target = NewTarget {
        collection: "personal".into(),
        parent: Some("sv-personal-2".into()),
        parent_kind: Some(TaskKind::Checklist),
        due_today: false,
    };
    let f = EditForm::new_task(target, "Socks".into(), NY);
    assert_eq!(f.fields(), vec![Field::Title, Field::Notes]);
    assert_eq!(
        f.save(today(), &WD),
        Ok(Op::Create(NewTask {
            parent: Some("sv-personal-2".into()),
            ..NewTask::plain("personal", "Socks")
        }))
    );
    let target = NewTarget {
        collection: "inbox".into(),
        parent: None,
        parent_kind: None,
        due_today: false,
    };
    let mut f = EditForm::new_task(target, "Stretch".into(), NY);
    focus(&mut f, Field::Repeat);
    type_str(&mut f, "daily");
    let Ok(Op::Create(t)) = f.save(today(), &WD) else {
        panic!("expected a create")
    };
    assert_eq!(t.due, Some(Due::Date(today())));
    let f = EditForm::new_task(
        NewTarget {
            collection: "inbox".into(),
            parent: None,
            parent_kind: None,
            due_today: false,
        },
        " ".into(),
        NY,
    );
    assert_eq!(
        f.save(today(), &WD),
        Err("title must not be empty".to_string())
    );
}

/// A new item's card shows only what its parent allows, and saves only that: a checklist item
/// added in Today gets no due date, a project item does.
#[test]
fn new_item_saves_only_the_fields_it_shows() {
    let item = |kind| NewTarget {
        collection: "personal".into(),
        parent: Some("sv-personal-2".into()),
        parent_kind: Some(kind),
        due_today: true,
    };
    let f = EditForm::new_task(item(TaskKind::Checklist), "Socks".into(), NY);
    assert_eq!(f.item_of, Some(TaskKind::Checklist));
    assert_eq!(f.fields(), vec![Field::Title, Field::Notes]);
    assert_eq!(
        f.save(today(), &WD),
        Ok(Op::Create(NewTask {
            parent: Some("sv-personal-2".into()),
            ..NewTask::plain("personal", "Socks")
        }))
    );
    let f = EditForm::new_task(item(TaskKind::Project), "Book flights".into(), NY);
    assert_eq!(
        f.save(today(), &WD),
        Ok(Op::Create(NewTask {
            parent: Some("sv-personal-2".into()),
            due: Some(Due::Date(today())),
            ..NewTask::plain("personal", "Book flights")
        }))
    );
}

#[test]
fn emptying_the_repeat_of_a_repeating_task_says_it_removes_it() {
    let mut f = form("Water plants");
    focus(&mut f, Field::Repeat);
    f.key(ctrl('u'));
    let removes = Some("→ none (removes the repeat)");
    assert_eq!(f.preview(Field::Repeat, today(), &WD).as_deref(), removes);
    assert_eq!(changes(&f), Ok(vec![Change::Repeat(None)]));
    type_str(&mut f, "none");
    assert_eq!(f.preview(Field::Repeat, today(), &WD).as_deref(), removes);
    // Unfocused, the cue stays.
    let mut g = form("Water plants");
    focus(&mut g, Field::Repeat);
    g.key(ctrl('u'));
    focus(&mut g, Field::Notes);
    assert_eq!(g.preview(Field::Repeat, today(), &WD).as_deref(), removes);
}
