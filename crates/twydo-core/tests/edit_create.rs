//! New tasks follow the layout of the iPhone's plain-task capture.

use std::path::Path;

use chrono::{NaiveDate, TimeZone, Utc};
use twydo_core::edit::{Due, EditError, NewTask, file_name, new_task, new_uid};
use twydo_core::ical::value::IcalDateTime;
use twydo_core::ical::{Calendar, parse, serialize};
use twydo_core::model::meta_edit::MetaJson;
use twydo_core::model::{TaskKind, tasks_from_calendar};

const A: &str = "0a04f22aeb294b719633c978fe1975e5";

fn reference() -> Calendar {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/phase0/v1/plan4-list1/01-create")
        .join(format!("{A}.ics"));
    parse(&std::fs::read(&p).unwrap()).unwrap()
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn nt(summary: &str) -> NewTask {
    NewTask {
        collection: "l".into(),
        summary: summary.into(),
        description: None,
        due: Some(Due::Date(d(2026, 12, 3))),
        start: None,
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
    let cal = new_task(&nt("A"), A, now()).unwrap();
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
    let cal = new_task(&t, A, now()).unwrap();
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
        new_task(&nt("  "), A, now()),
        Err(EditError::EmptyTitle)
    ));
}
