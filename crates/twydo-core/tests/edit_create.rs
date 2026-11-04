//! New tasks follow the layout of the iPhone's plain-task capture.

use std::path::Path;

use chrono::{NaiveDate, TimeZone, Utc};
use twydo_core::edit::{
    Change, Due, EditError, NewTask, apply_changes, file_name, new_task, new_uid,
};
use twydo_core::ical::value::IcalDateTime;
use twydo_core::ical::{Calendar, parse, serialize};
use twydo_core::model::meta_edit::MetaJson;
use twydo_core::model::{TaskKind, tasks_from_calendar};
use twydo_core::repeatinput::{self, RepeatSpec};

const A: &str = "0a04f22aeb294b719633c978fe1975e5";
const C: &str = "3b93fba16ec74e24b2da63a73c29ca8c";

fn reference() -> Calendar {
    capture(A)
}

fn capture(uid: &str) -> Calendar {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/phase0/v1/plan4-list1/01-create")
        .join(format!("{uid}.ics"));
    parse(&std::fs::read(&p).unwrap()).unwrap()
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn nt(summary: &str) -> NewTask {
    NewTask {
        due: Some(Due::Date(d(2026, 12, 3))),
        ..NewTask::plain("l", summary)
    }
}

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 1, 14, 4, 51).unwrap()
}

fn names(c: &twydo_core::ical::Component) -> Vec<String> {
    c.props.iter().map(|p| p.name.clone()).collect()
}

#[test]
fn new_uid_shape() {
    let a = new_uid();
    assert_eq!(a.len(), 32);
    assert!(a.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
    assert_ne!(a, new_uid());
    assert_eq!(file_name("abc"), "abc.ics");
}

#[test]
fn new_task_matches_2do_layout() {
    let r = reference();
    let cal = new_task(&nt("A"), A, None, now()).unwrap();
    assert_eq!(names(&cal.root), ["VERSION", "PRODID", "CALSCALE"]);
    let skip = |n: &String| !(n.contains("SYNC") || n == "X-2DOAPP-AUDIO-DURATION");
    let want: Vec<String> = names(r.root.children_named("VTODO").next().unwrap())
        .into_iter()
        .filter(skip)
        .collect();
    let got = names(cal.root.children_named("VTODO").next().unwrap());
    assert_eq!(got, want);
    let m = |c: &Calendar| {
        let p = c
            .root
            .children_named("VTODO")
            .next()
            .unwrap()
            .prop("X-2DOAPP-METADATA")
            .unwrap()
            .clone();
        (p.params.clone(), MetaJson::parse(&p.value).unwrap())
    };
    assert_eq!(m(&cal), m(&r));
    let text = String::from_utf8(serialize(&cal)).unwrap();
    assert!(text.contains("\r\nPRODID:-//Twydo//tdo "));
    assert!(text.contains("\r\nDESCRIPTION:\r\n"));
    assert!(text.contains("\r\nDTSTART;VALUE=DATE:20261203\r\n"));
    assert!(text.contains("\r\nCREATED:20261101T140451Z\r\n"));
    assert!(!text.contains("VALARM"));
}

#[test]
fn new_task_reads_back() {
    let mut t = nt("  Buy milk ");
    t.start = Some(d(2026, 12, 1));
    let cal = new_task(&t, A, None, now()).unwrap();
    let cal = parse(&serialize(&cal)).unwrap();
    let tasks = tasks_from_calendar(&cal, "l", Path::new("x.ics"));
    assert_eq!(tasks.len(), 1);
    let k = &tasks[0];
    assert_eq!(k.summary, "Buy milk");
    assert_eq!(k.kind, TaskKind::Task);
    assert!(k.warnings.is_empty(), "{:?}", k.warnings);
    assert_eq!(k.start, Some(IcalDateTime::Date(d(2026, 12, 1))));
    assert_eq!(k.due, Some(IcalDateTime::Date(d(2026, 12, 3))));
}

#[test]
fn new_task_empty_title_rejected() {
    assert!(matches!(
        new_task(&nt("  "), A, None, now()),
        Err(EditError::EmptyTitle)
    ));
}

/// Every line of the VTODO and its children as (path, name, params, value), without
/// `skip`ped names; the metadata decoded.
fn vtodo_lines(cal: &Calendar, skip: &[&str]) -> Vec<String> {
    fn walk(c: &twydo_core::ical::Component, path: &str, skip: &[&str], out: &mut Vec<String>) {
        let path = format!("{path}/{}", c.name);
        for p in &c.props {
            if skip.contains(&p.name.as_str())
                || p.name.contains("SYNC")
                || p.name == "X-2DOAPP-AUDIO-DURATION"
            {
                continue;
            }
            let v = match MetaJson::parse(&p.value) {
                Some(m) if p.name == "X-2DOAPP-METADATA" => format!("{m:?}"),
                _ => p.value.clone(),
            };
            out.push(format!("{path} {} {:?} {v}", p.name, p.params));
        }
        for ch in &c.children {
            walk(ch, &path, skip, out);
        }
    }
    let mut out = Vec::new();
    walk(
        cal.root.children_named("VTODO").next().unwrap(),
        "",
        skip,
        &mut out,
    );
    out
}

/// A timed new task equals 2Do's timed capture C (`01-create`) minus the SYNC /
/// AUDIO-DURATION properties, including 2Do's automatic alert (user decision 2026-11-01).
#[test]
fn new_timed_task_matches_2do_with_auto_alarm() {
    let r = capture(C);
    let t = NewTask {
        due: Some(Due::Time(d(2026, 12, 3).and_hms_opt(17, 30, 0).unwrap())),
        ..nt("C")
    };
    let cal = new_task(&t, C, None, now()).unwrap();
    let skip = ["DTSTAMP", "LAST-MODIFIED", "CREATED"];
    assert_eq!(vtodo_lines(&cal, &skip), vtodo_lines(&r, &skip));
    let text = String::from_utf8(serialize(&cal)).unwrap();
    assert_eq!(text.matches("BEGIN:VALARM").count(), 1);
}

fn fixture(scenario: &str, step: &str, uid: &str) -> Calendar {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/phase0/v1")
        .join(scenario)
        .join(step)
        .join(format!("{uid}.ics"));
    parse(&std::fs::read(&p).unwrap()).unwrap()
}

const STAMPS: [&str; 3] = ["DTSTAMP", "LAST-MODIFIED", "CREATED"];

fn repeat_spec(input: &str) -> RepeatSpec {
    use chrono::Weekday::*;
    repeatinput::parse(input, d(2026, 11, 3), &[Mon, Tue, Wed, Thu, Fri])
        .unwrap()
        .unwrap()
}

/// `t` built as UID `uid` equals the phone's capture, apart from stamps and the properties
/// Twydo leaves out.
fn assert_like_phone(t: &NewTask, uid: &str, parent_json_uid: Option<&str>, phone: &Calendar) {
    let cal = new_task(t, uid, parent_json_uid, now()).unwrap();
    assert_eq!(vtodo_lines(&cal, &STAMPS), vtodo_lines(phone, &STAMPS));
}

fn dated(summary: &str, due: NaiveDate) -> NewTask {
    NewTask {
        due: Some(Due::Date(due)),
        ..NewTask::plain("l", summary)
    }
}

const P6: &str = "plan6-list1";
const PARENT_PROJECT: &str = "09c49c2cd76f483498fc2ff3a0f66160";
const PARENT_CHECKLIST: &str = "a8dcf93bcaa14211a07481eb87af8f11";

#[test]
fn creates_a_project_like_the_phone() {
    let t = NewTask {
        kind: TaskKind::Project,
        ..NewTask::plain("l", "P")
    };
    assert_like_phone(
        &t,
        PARENT_PROJECT,
        None,
        &fixture(P6, "01-create", PARENT_PROJECT),
    );
}

#[test]
fn creates_a_checklist_like_the_phone() {
    let t = NewTask {
        kind: TaskKind::Checklist,
        ..dated("K", d(2026, 12, 10))
    };
    assert_like_phone(
        &t,
        PARENT_CHECKLIST,
        None,
        &fixture(P6, "01-create", PARENT_CHECKLIST),
    );
}

#[test]
fn creates_a_checklist_item_like_the_phone() {
    let uid = "28ae80e7369d40feaba54fe228eda2c6";
    let t = NewTask {
        parent: Some(PARENT_CHECKLIST.into()),
        ..NewTask::plain("l", "k1")
    };
    assert_like_phone(&t, uid, None, &fixture(P6, "01-create", uid));
}

#[test]
fn creates_a_project_item_like_the_phone() {
    let uid = "e668818024cc48f4a404fb9ced368789";
    let t = NewTask {
        parent: Some(PARENT_PROJECT.into()),
        ..dated("p1", d(2026, 12, 5))
    };
    assert_like_phone(&t, uid, None, &fixture(P6, "01-create", uid));
}

#[test]
fn an_item_points_at_the_parents_json_uid_when_it_differs() {
    let t = NewTask {
        parent: Some("ical".into()),
        ..NewTask::plain("l", "i")
    };
    let cal = new_task(&t, A, Some("internal"), now()).unwrap();
    let todo = cal.root.children_named("VTODO").next().unwrap();
    assert_eq!(todo.prop("RELATED-TO").unwrap().value, "ical");
    let m = MetaJson::parse(&todo.prop("X-2DOAPP-METADATA").unwrap().value).unwrap();
    assert_eq!(m.get_str("parentuid"), Some("internal"));
}

#[test]
fn creates_a_recurring_project_like_the_phone() {
    let uid = "8f603cfaa41f4c229b9adcd6223e7393";
    let t = NewTask {
        kind: TaskKind::Project,
        repeat: Some(repeat_spec("monthly")),
        ..dated("R", d(2026, 12, 15))
    };
    assert_like_phone(&t, uid, None, &fixture(P6, "01-create", uid));
}

#[test]
fn creates_a_repeating_task_like_the_phone_monthly() {
    let uid = "5c072ad90a8f44f1a974e170a8a950e7";
    let t = NewTask {
        repeat: Some(repeat_spec("monthly")),
        ..dated("Test rule months day 1-28", d(2026, 11, 10))
    };
    assert_like_phone(
        &t,
        uid,
        None,
        &fixture("rule-months-day-1-28", "01-create", uid),
    );
}

#[test]
fn creates_a_repeating_task_like_the_phone_yearly() {
    let uid = "c3e7283649dd413abf3f73745121bce5";
    let t = NewTask {
        repeat: Some(repeat_spec("yearly")),
        ..dated("Test rule years", d(2026, 11, 15))
    };
    assert_like_phone(&t, uid, None, &fixture("rule-years", "01-create", uid));
}

/// No phone create exists for the other classes: a create with a repeat equals a plain create
/// followed by `Change::Repeat` (which matches the phone's edit captures).
#[test]
fn create_with_repeat_equals_create_then_set_repeat() {
    for input in [
        "daily",
        "3d",
        "weekly",
        "2w",
        "mon thu",
        "workdays",
        "2nd tue",
        "monthly after done",
        "monthly 3 times",
        "monthly until 2027-06-30",
    ] {
        let spec = repeat_spec(input);
        let plain = dated("T", d(2026, 12, 3));
        let mut want = new_task(&plain, A, None, now()).unwrap();
        apply_changes(&mut want, &[Change::Repeat(Some(spec.clone()))], now()).unwrap();
        let t = NewTask {
            repeat: Some(spec),
            ..plain
        };
        let got = new_task(&t, A, None, now()).unwrap();
        let skip = ["DTSTAMP", "LAST-MODIFIED"];
        assert_eq!(
            vtodo_lines(&got, &skip),
            vtodo_lines(&want, &skip),
            "{input}"
        );
        let todo = got.root.children_named("VTODO").next().unwrap();
        assert!(todo.prop("RRULE").is_some(), "{input}");
    }
}

#[test]
fn a_repeat_without_a_due_is_refused() {
    let t = NewTask {
        repeat: Some(repeat_spec("daily")),
        ..NewTask::plain("l", "T")
    };
    let e = new_task(&t, A, None, now()).unwrap_err();
    assert_eq!(e.to_string(), "a repeating task needs a due date");
}

#[test]
fn an_item_of_a_checklist_with_dates_does_not_crash() {
    let t = NewTask {
        parent: Some(PARENT_CHECKLIST.into()),
        repeat: Some(repeat_spec("daily")),
        ..dated("i", d(2026, 12, 3))
    };
    assert!(new_task(&t, A, None, now()).is_ok());
}

fn end_keys(cal: &Calendar) -> (Option<i64>, Option<i64>, Option<i64>, String) {
    let todo = cal.root.children_named("VTODO").next().unwrap();
    let m = MetaJson::parse(&todo.prop("X-2DOAPP-METADATA").unwrap().value).unwrap();
    (
        m.get_i64("RecurrenceEndType"),
        m.get_i64("RecurrenceEndRepetitions"),
        m.get_i64("RecurrenceEndRepetitionsOrig"),
        todo.prop("RRULE").unwrap().value.clone(),
    )
}

#[test]
fn repeat_edit_on_an_advanced_count_keeps_the_original_count() {
    let t = NewTask {
        repeat: Some(repeat_spec("monthly 3 times")),
        ..nt("T")
    };
    let created = new_task(&t, A, None, now()).unwrap();
    let (mut live, _copy) = twydo_core::edit::complete_recurring(
        &created,
        now(),
        now().date_naive(),
        "ffffffffffffffffffffffffffffffff",
    )
    .unwrap();
    let (ty, reps, orig, rule) = end_keys(&live);
    assert_eq!((ty, reps, orig), (Some(2), Some(2), Some(3)), "{rule}");

    // Interval-only change: the ending condition is unchanged, so the original count stays.
    apply_changes(
        &mut live,
        &[Change::Repeat(Some(repeat_spec("2m 2 times")))],
        now(),
    )
    .unwrap();
    let (ty, reps, orig, rule) = end_keys(&live);
    assert_eq!((ty, reps, orig), (Some(2), Some(2), Some(3)), "{rule}");
    assert!(
        rule.contains("COUNT=2") && rule.contains("INTERVAL=2"),
        "{rule}"
    );

    // A different count is a new ending: both reset.
    apply_changes(
        &mut live,
        &[Change::Repeat(Some(repeat_spec("2m 5 times")))],
        now(),
    )
    .unwrap();
    let (ty, reps, orig, rule) = end_keys(&live);
    assert_eq!((ty, reps, orig), (Some(2), Some(5), Some(5)), "{rule}");
}
