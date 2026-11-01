use std::path::Path;

use chrono::{Duration, NaiveDate, TimeZone, Utc};
use twydo_core::ical::parse;
use twydo_core::ical::value::IcalDateTime;
use twydo_core::model::{Status, Task, TaskKind, Trigger, parse_metadata, tasks_from_calendar};

fn load(src: &[u8]) -> Vec<Task> {
    let cal = parse(src).unwrap();
    tasks_from_calendar(&cal, "work", Path::new("work/t.ics"))
}

fn todo(lines: &str) -> Task {
    let src = format!(
        "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:u1\r\nSUMMARY:S\r\n{lines}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
    );
    let mut v = load(src.as_bytes());
    assert_eq!(v.len(), 1);
    v.remove(0)
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

#[test]
fn basic_fields() {
    let v = load(include_bytes!("data/model/basic.ics"));
    let t = &v[0];
    assert_eq!(t.uid, "t1");
    assert_eq!(t.collection, "work");
    assert_eq!(t.summary, "Water the plants, twice");
    assert_eq!(t.description.as_deref(), Some("Line one\nLine two"));
    assert_eq!(t.due, Some(IcalDateTime::Date(d(2026, 11, 5))));
    assert_eq!(t.status, Status::Open);
    assert_eq!(t.kind, TaskKind::Task);
    assert!(t.warnings.is_empty());
    assert!(t.blocked.is_none());
}

#[test]
fn parent_and_kind() {
    let child = todo("RELATED-TO;RELTYPE=PARENT:p1");
    assert_eq!(child.parent.as_deref(), Some("p1"));
    assert_eq!(todo("RELATED-TO:p2").parent.as_deref(), Some("p2"));
    assert_eq!(todo("RELATED-TO;RELTYPE=SIBLING:p3").parent, None);

    let p = todo("X-2DOAPP-METADATA:@tasktype:checklist @repeatfromcompletion");
    assert_eq!(p.kind, TaskKind::Checklist);
    assert!(p.from_completion);
    assert_eq!(
        todo("X-2DOAPP-METADATA:@tasktype:project").kind,
        TaskKind::Project
    );
    assert!(!todo("X-2DOAPP-METADATA:@starred").from_completion);
}

#[test]
fn metadata_tokens() {
    let m = parse_metadata("@taskid:abc @tags:a, b @starred @duetime:09:30");
    assert_eq!(m.value("taskid"), Some("abc"));
    assert_eq!(m.tokens["tags"], Some("a, b".to_string()));
    assert_eq!(m.tokens["starred"], None);
    assert!(m.has("starred"));
    assert_eq!(m.value("duetime"), Some("09:30"));
    // An @ inside a value (an e-mail address) is not a token.
    let m = parse_metadata("@note:mail me@example.com @starred");
    assert_eq!(m.value("note"), Some("mail me@example.com"));
    assert!(m.has("starred"));
}

#[test]
fn metadata_value_is_unescaped() {
    let t = todo("X-2DOAPP-METADATA:@tags:a\\, b @starred");
    assert_eq!(t.meta.value("tags"), Some("a, b"));
}

#[test]
fn start_from_metadata_and_conflict() {
    let t = todo("X-2DOAPP-METADATA:@startdate:2026-11-03 @starttime:08:15");
    assert_eq!(
        t.start,
        Some(IcalDateTime::Floating(
            d(2026, 11, 3).and_hms_opt(8, 15, 0).unwrap()
        ))
    );
    assert!(t.warnings.is_empty());
    assert_eq!(
        todo("X-2DOAPP-METADATA:@startdate:2026-11-03").start,
        Some(IcalDateTime::Date(d(2026, 11, 3)))
    );

    let ok = todo("DTSTART;VALUE=DATE:20261103\r\nX-2DOAPP-METADATA:@startdate:2026-11-03");
    assert!(ok.warnings.is_empty());
}

#[test]
fn start_conflict_warns() {
    let t = todo("DTSTART;VALUE=DATE:20261102\r\nX-2DOAPP-METADATA:@startdate:2026-11-03");
    assert_eq!(t.start, Some(IcalDateTime::Date(d(2026, 11, 2))));
    assert!(
        t.warnings
            .contains(&"start date: DTSTART and 2Do metadata disagree".to_string())
    );
}

#[test]
fn alarms() {
    let t = todo(
        "BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\n\
         BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER;RELATED=END:PT0S\r\nEND:VALARM\r\n\
         BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER;VALUE=DATE-TIME:20261104T140000Z\r\nEND:VALARM\r\n\
         BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:garbage\r\nEND:VALARM",
    );
    assert_eq!(t.alarms.len(), 3);
    assert_eq!(
        t.alarms[0].trigger,
        Trigger::Relative {
            offset: Duration::minutes(-15),
            related_end: false
        }
    );
    assert_eq!(
        t.alarms[1].trigger,
        Trigger::Relative {
            offset: Duration::zero(),
            related_end: true
        }
    );
    assert_eq!(t.alarms[1].index, 1);
    assert_eq!(
        t.alarms[2].trigger,
        Trigger::Absolute(Utc.with_ymd_and_hms(2026, 11, 4, 14, 0, 0).unwrap())
    );
    assert_eq!(t.alarms[2].index, 2);
    assert!(
        t.warnings
            .iter()
            .any(|w| w.starts_with("invalid TRIGGER: "))
    );
}

#[test]
fn recurrence_override_blocks() {
    let v = load(include_bytes!("data/model/override.ics"));
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].summary, "Weekly thing");
    assert_eq!(v[0].rrule.as_deref(), Some("FREQ=WEEKLY"));
    assert_eq!(
        v[0].blocked.as_deref(),
        Some("recurrence overrides are not supported")
    );
}

#[test]
fn completed_and_cancelled() {
    assert_eq!(todo("STATUS:COMPLETED").status, Status::Completed);
    let t = todo("COMPLETED:20261103T120000Z");
    assert_eq!(t.status, Status::Completed);
    assert!(t.completed.is_some());
    assert_eq!(todo("STATUS:CANCELLED").status, Status::Cancelled);
    assert_eq!(todo("STATUS:IN-PROCESS").status, Status::Open);
}

#[test]
fn invalid_date_warns_not_drops() {
    let t = todo("DUE:notadate\r\nCREATED:20261101T000000Z");
    assert_eq!(t.due, None);
    assert!(t.created.is_some());
    assert!(t.warnings.iter().any(|w| w.starts_with("invalid DUE: ")));
}

#[test]
fn missing_summary_and_non_todo_components() {
    let v = load(b"BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:x\r\nEND:VTODO\r\nEND:VCALENDAR\r\n");
    assert_eq!(v[0].summary, "");
    assert!(v[0].warnings.contains(&"missing SUMMARY".to_string()));
    let v = load(b"BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:e\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n");
    assert!(v.is_empty());
}

#[test]
fn explicit_open_status_wins_over_leftover_completed() {
    for status in ["NEEDS-ACTION", "IN-PROCESS"] {
        let t = todo(&format!("STATUS:{status}\r\nCOMPLETED:20261103T120000Z"));
        assert_eq!(t.status, Status::Open, "{status}");
        assert!(t.completed.is_some());
        assert!(
            t.warnings
                .contains(&"STATUS and COMPLETED disagree".to_string()),
            "{status}: {:?}",
            t.warnings
        );
    }
    // Without STATUS, COMPLETED alone still means completed, without a warning.
    let t = todo("COMPLETED:20261103T120000Z");
    assert_eq!(t.status, Status::Completed);
    assert!(t.warnings.is_empty(), "{:?}", t.warnings);
    // Agreeing STATUS:COMPLETED with COMPLETED is not a conflict.
    let t = todo("STATUS:COMPLETED\r\nCOMPLETED:20261103T120000Z");
    assert_eq!(t.status, Status::Completed);
    assert!(t.warnings.is_empty(), "{:?}", t.warnings);
}

#[test]
fn unrecognised_tasktype_warns() {
    let t = todo("X-2DOAPP-METADATA:@tasktype:folder");
    assert_eq!(t.kind, TaskKind::Task);
    assert!(
        t.warnings
            .contains(&"unrecognised @tasktype value: folder".to_string()),
        "{:?}",
        t.warnings
    );
    assert!(
        todo("X-2DOAPP-METADATA:@tasktype:Project")
            .warnings
            .is_empty()
    );
}

#[test]
fn invalid_metadata_start_warns() {
    let t = todo("X-2DOAPP-METADATA:@startdate:2026-13-40");
    assert_eq!(t.start, None);
    assert!(
        t.warnings
            .contains(&"invalid @startdate: 2026-13-40".to_string()),
        "{:?}",
        t.warnings
    );
    let t = todo("X-2DOAPP-METADATA:@startdate:2026-11-03 @starttime:25:99");
    assert_eq!(t.start, Some(IcalDateTime::Date(d(2026, 11, 3))));
    assert!(
        t.warnings
            .contains(&"invalid @starttime: 25:99".to_string()),
        "{:?}",
        t.warnings
    );
}
