//! Relative dates and alert descriptions. All content is synthetic.

use std::path::PathBuf;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;

use twydo_core::format::{describe_alarm, relative_date};
use twydo_core::ical::value::IcalDateTime;
use twydo_core::model::{Alarm, Metadata, Status, Task, TaskKind, Trigger};

const NY: Tz = chrono_tz::America::New_York;

fn now() -> DateTime<Tz> {
    NY.with_ymd_and_hms(2026, 11, 4, 10, 0, 0).unwrap()
}

fn date(s: &str) -> IcalDateTime {
    IcalDateTime::Date(NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap())
}

fn timed(s: &str) -> IcalDateTime {
    IcalDateTime::Zoned {
        local: NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M").unwrap(),
        tzid: "America/New_York".into(),
    }
}

#[test]
fn relative_dates() {
    let cases = [
        (date("2026-10-28"), "7 days ago"),
        (date("2026-11-02"), "2 days ago"),
        (date("2026-11-03"), "Yesterday"),
        (date("2026-11-04"), "Today"),
        (date("2026-11-05"), "Tomorrow"),
        (date("2026-11-06"), "Fri"),
        (date("2026-11-10"), "Tue"),
        (date("2026-11-11"), "11 Nov"),
        (date("2027-11-06"), "6 Nov 2027"),
        (timed("2026-11-04T15:00"), "Today 15:00"),
        (timed("2026-11-06T09:05"), "Fri 09:05"),
        (timed("2026-11-03T08:00"), "Yesterday 08:00"),
    ];
    for (d, want) in cases {
        assert_eq!(relative_date(&d, now()), want, "{d:?}");
    }
}

#[test]
fn utc_values_use_the_local_zone() {
    // 2026-11-04T23:30Z is 18:30 in New York, still Today.
    let d = IcalDateTime::Utc(Utc.with_ymd_and_hms(2026, 11, 4, 23, 30, 0).unwrap());
    assert_eq!(relative_date(&d, now()), "Today 18:30");
}

fn task(start: Option<IcalDateTime>) -> Task {
    Task {
        uid: "u".into(),
        collection: "c".into(),
        path: PathBuf::new(),
        summary: "t".into(),
        description: None,
        due: Some(timed("2026-11-06T09:00")),
        start,
        status: Status::Open,
        completed: None,
        created: None,
        last_modified: None,
        rrule: None,
        parent: None,
        kind: TaskKind::Task,
        alarms: vec![],
        meta: Metadata::default(),
        from_completion: false,
        warnings: vec![],
        blocked: None,
    }
}

fn rel(mins: i64, end: bool) -> Alarm {
    Alarm {
        trigger: Trigger::Relative {
            offset: Duration::minutes(mins),
            related_end: end,
        },
        index: 0,
    }
}

#[test]
fn alarm_descriptions() {
    let t = task(None);
    assert_eq!(
        describe_alarm(&rel(-15, false), &t, NY),
        "15 min before due"
    );
    assert_eq!(describe_alarm(&rel(-15, true), &t, NY), "15 min before due");
    assert_eq!(describe_alarm(&rel(0, true), &t, NY), "at due");
    assert_eq!(describe_alarm(&rel(-60, true), &t, NY), "1 h before due");
    assert_eq!(describe_alarm(&rel(-90, true), &t, NY), "90 min before due");
    assert_eq!(
        describe_alarm(&rel(-2 * 1440, true), &t, NY),
        "2 d before due"
    );
    assert_eq!(describe_alarm(&rel(30, true), &t, NY), "30 min after due");
    let s = task(Some(timed("2026-11-05T09:00")));
    assert_eq!(describe_alarm(&rel(-60, false), &s, NY), "1 h before start");
    assert_eq!(describe_alarm(&rel(0, false), &s, NY), "at start");
    let abs = Alarm {
        trigger: Trigger::Absolute(Utc.with_ymd_and_hms(2026, 11, 6, 14, 0, 0).unwrap()),
        index: 0,
    };
    assert_eq!(describe_alarm(&abs, &t, NY), "at 09:00 6 Nov");
}
