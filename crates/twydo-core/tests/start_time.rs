//! Start times against the start-time round (`fixtures/set-start/phone-encodings`): reading
//! what 2Do wrote, and writing the bytes 2Do writes, apart from the two sync stamps.

mod common;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use twydo_core::edit::{Change, Due, NewTask, apply_changes, new_task};
use twydo_core::ical::value::IcalDateTime;
use twydo_core::ical::{Calendar, parse};
use twydo_core::model::{Task, tasks_from_calendar};

const CASE: &str = "set-start/phone-encodings";
const S1: &str = "0f8195e0cd3947e4809d1b5d5015525a";
const S2: &str = "d9ae60e7b31b4ff8b4d4249b5499cb59";
const S3: &str = "0907b521c5fd473193319a18076365c2";
const S7: &str = "b42a79b674184d66bded898223236ed1";
const S8: &str = "d3afe2df1f4b47848edec337dc134f7a";
const S9: &str = "41d4356a9ba54e9c972655340f050c6d";

fn phone(state: &str, uid: &str) -> Calendar {
    parse(&common::fixture(CASE, &format!("{state}/list1"), uid)).unwrap()
}

fn task(cal: &Calendar) -> Task {
    let mut v = tasks_from_calendar(cal, "list1", std::path::Path::new("list1/t.ics"));
    assert_eq!(v.len(), 1);
    v.remove(0)
}

fn at(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
}

fn day(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-21T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn start_of(state: &str, uid: &str) -> (Option<IcalDateTime>, Vec<String>) {
    let t = task(&phone(state, uid));
    (t.start, t.warnings)
}

#[test]
fn reads_phone_start_times() {
    let timed = |s: &str| (Some(IcalDateTime::Floating(at(s))), vec![]);
    for uid in [S1, S2, S3] {
        assert_eq!(
            start_of("phone-a-batch1", uid),
            timed("2026-11-25 10:00"),
            "{uid}"
        );
    }
    assert_eq!(start_of("phone-a-batch1", S9), timed("2027-04-06 10:00"));
    assert_eq!(start_of("phone-a-batch1", S7), timed("2026-11-23 09:00"));
    assert_eq!(start_of("phone-a-batch1", S8), timed("2026-11-23 09:00"));
    assert_eq!(start_of("phone-a-batch2", S1), timed("2026-11-25 14:30"));
    assert_eq!(
        start_of("phone-a-batch2", S2),
        (Some(IcalDateTime::Date(day("2026-11-25"))), vec![])
    );
    assert_eq!(start_of("phone-a-batch2", S3), (None, vec![]));
}

/// Review focus 2: an edit of another field must not rewrite a timed start to midnight.
#[test]
fn title_edit_keeps_timed_start() {
    let mut cal = phone("phone-a-batch1", S1);
    let meta = |c: &Calendar| {
        common::lines(c)
            .into_iter()
            .filter(|l| l.1 == "X-2DOAPP-METADATA")
            .collect::<Vec<_>>()
    };
    let before = meta(&cal);
    apply_changes(&mut cal, &[Change::Summary("S1 renamed".into())], now()).unwrap();
    assert_eq!(meta(&cal), before);
    assert_eq!(
        task(&cal).start,
        Some(IcalDateTime::Floating(at("2026-11-25 10:00")))
    );
}

/// Every property line in document order, without the two sync stamps.
fn norm(cal: &Calendar) -> Vec<common::Line> {
    common::lines(cal)
        .into_iter()
        .filter(|l| !common::is_stamp(&l.1))
        .collect()
}

fn edited(mut cal: Calendar, change: Change) -> Calendar {
    apply_changes(&mut cal, &[change], now()).unwrap();
    cal
}

fn timed(s: &str) -> Option<Due> {
    Some(Due::Time(at(s)))
}

#[test]
fn set_start_time_matches_phone_bytes() {
    // S3 without its start (phone batch 2) gets the start back: the phone's batch 1 bytes.
    let got = edited(
        phone("phone-a-batch2", S3),
        Change::Start(timed("2026-11-25 10:00")),
    );
    assert_eq!(norm(&got), norm(&phone("phone-a-batch1", S3)));
    // S9, summer time: removed and set again gives the phone's value back.
    let cleared = edited(phone("phone-a-batch1", S9), Change::Start(None));
    let got = edited(cleared, Change::Start(timed("2027-04-06 10:00")));
    assert_eq!(norm(&got), norm(&phone("phone-a-batch1", S9)));
}

#[test]
fn change_start_time_matches_phone_bytes() {
    let got = edited(
        phone("phone-a-batch1", S1),
        Change::Start(timed("2026-11-25 14:30")),
    );
    assert_eq!(norm(&got), norm(&phone("phone-a-batch2", S1)));
}

#[test]
fn drop_start_time_matches_phone_bytes() {
    let got = edited(
        phone("phone-a-batch1", S2),
        Change::Start(Some(Due::Date(day("2026-11-25")))),
    );
    assert_eq!(norm(&got), norm(&phone("phone-a-batch2", S2)));
}

#[test]
fn remove_timed_start_matches_phone_bytes() {
    let got = edited(phone("phone-a-batch1", S3), Change::Start(None));
    assert_eq!(norm(&got), norm(&phone("phone-a-batch2", S3)));
}

#[test]
fn creates_task_with_start_time() {
    let mut t = NewTask::plain("list1", "S1 start");
    t.start = timed("2026-11-25 10:00");
    let cal = new_task(&t, S1, None, now()).unwrap();
    assert_eq!(
        task(&cal).start,
        Some(IcalDateTime::Floating(at("2026-11-25 10:00")))
    );
    let raw = String::from_utf8(twydo_core::ical::serialize(&cal)).unwrap();
    assert!(
        raw.replace("\r\n ", "")
            .contains("StartDate%22%3A1795600800"),
        "{raw}"
    );
}
