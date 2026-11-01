//! New tasks follow the layout of the iPhone's plain-task capture.

use std::path::Path;

use chrono::{NaiveDate, TimeZone, Utc};
use twydo_core::edit::{Due, EditError, NewTask, file_name, new_task, new_uid};
use twydo_core::ical::value::IcalDateTime;
use twydo_core::ical::{Calendar, parse, serialize};
use twydo_core::model::meta_edit::MetaJson;
use twydo_core::model::{TaskKind, tasks_from_calendar};

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
    let cal = new_task(&t, C, now()).unwrap();
    let skip = ["DTSTAMP", "LAST-MODIFIED", "CREATED"];
    assert_eq!(vtodo_lines(&cal, &skip), vtodo_lines(&r, &skip));
    let text = String::from_utf8(serialize(&cal)).unwrap();
    assert_eq!(text.matches("BEGIN:VALARM").count(), 1);
}
