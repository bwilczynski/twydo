//! Field edits against the iPhone captures: each edit must change exactly the lines 2Do
//! changed, to the same values, and keep every other line's bytes.

use std::collections::BTreeSet;
use std::path::Path;

use chrono::{DateTime, NaiveDate, Utc};
use twydo_core::edit::{Change, Due, EditError, apply_changes, touch};
use twydo_core::ical::{Calendar, Component, Param, parse, serialize};
use twydo_core::model::meta_edit::MetaJson;

const A: &str = "0a04f22aeb294b719633c978fe1975e5";
const B: &str = "41b9cdfea48e46fd9da52f9337c3f6c4";
const C: &str = "3b93fba16ec74e24b2da63a73c29ca8c";
const D: &str = "855a275473c34da9840a6a76c5f18385";
const MONTHLY: &str = "5c072ad90a8f44f1a974e170a8a950e7";
const YEARLY: &str = "c3e7283649dd413abf3f73745121bce5";

fn fixture(scenario: &str, step: &str, uid: &str) -> Vec<u8> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/phase0/v1")
        .join(scenario)
        .join(step)
        .join(format!("{uid}.ics"));
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

type Line = (String, String, Vec<Param>, String);

/// Every property as (component path, name, params, value), in document order.
fn flat(c: &Component, path: &str, out: &mut Vec<Line>) {
    let path = format!("{path}/{}", c.name);
    for p in &c.props {
        out.push((
            path.clone(),
            p.name.clone(),
            p.params.clone(),
            p.value.clone(),
        ));
    }
    for ch in &c.children {
        flat(ch, &path, out);
    }
}

fn lines(cal: &Calendar) -> Vec<Line> {
    let mut out = Vec::new();
    flat(&cal.root, "", &mut out);
    out
}

fn is_stamp(name: &str) -> bool {
    name == "DTSTAMP" || name == "LAST-MODIFIED"
}

/// Raw logical lines (folds and terminator kept).
fn raw_logical(b: &[u8]) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    for phys in b.split_inclusive(|&c| c == b'\n') {
        match (phys.first(), out.last_mut()) {
            (Some(b' ' | b'\t'), Some(prev)) => prev.extend_from_slice(phys),
            _ => out.push(phys.to_vec()),
        }
    }
    out
}

/// Property name of a raw line; BEGIN/END lines are named by their whole content.
fn raw_name(l: &[u8]) -> String {
    let s = String::from_utf8_lossy(l);
    let s = s.trim_end();
    let name = s
        .split([';', ':'])
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    if name == "BEGIN" || name == "END" {
        s.to_ascii_uppercase()
    } else {
        name
    }
}

fn dtstamp(cal: &Calendar) -> DateTime<Utc> {
    let t = cal.root.children_named("VTODO").next().unwrap();
    let v = &t.prop("DTSTAMP").unwrap().value;
    chrono::NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%SZ")
        .unwrap()
        .and_utc()
}

/// Apply `changes` to `<scenario>/<from>/<uid>.ics` with `now` = the evidence's DTSTAMP, and
/// check the result against `<scenario>/<to>/<uid>.ics`:
/// - logical lines equal the evidence's (position of DTSTAMP/LAST-MODIFIED ignored);
/// - the lines 2Do changed are exactly `changed` (plus the two stamps);
/// - every other line keeps its original bytes and order.
fn check(scenario: &str, from: &str, to: &str, uid: &str, changes: &[Change], changed: &[&str]) {
    let input = fixture(scenario, from, uid);
    let evidence = parse(&fixture(scenario, to, uid)).unwrap();
    let mut cal = parse(&input).unwrap();
    apply_changes(&mut cal, changes, dtstamp(&evidence)).unwrap();
    let out = serialize(&cal);
    let got = parse(&out).unwrap();
    let ctx = format!("{scenario} {from}->{to} {uid}");

    // The fixtures themselves differ exactly in `changed` + stamps.
    let (before, want) = (lines(&parse(&input).unwrap()), lines(&evidence));
    let diff_names: BTreeSet<String> = before
        .iter()
        .filter(|l| !want.contains(l))
        .chain(want.iter().filter(|l| !before.contains(l)))
        .map(|l| l.1.clone())
        .collect();
    let mut expect_names: BTreeSet<String> = changed
        .iter()
        .filter(|n| !n.starts_with("BEGIN:") && !n.starts_with("END:"))
        .map(|n| n.to_string())
        .collect();
    expect_names.extend(["DTSTAMP".to_string(), "LAST-MODIFIED".to_string()]);
    assert_eq!(diff_names, expect_names, "{ctx}: fixture changes");

    // Same logical lines as 2Do wrote.
    let got_l = lines(&got);
    let non_stamp =
        |v: &[Line]| -> Vec<Line> { v.iter().filter(|l| !is_stamp(&l.1)).cloned().collect() };
    let stamps = |v: &[Line]| -> Vec<Line> {
        let mut s: Vec<Line> = v.iter().filter(|l| is_stamp(&l.1)).cloned().collect();
        s.sort_by(|a, b| a.1.cmp(&b.1));
        s
    };
    assert_eq!(non_stamp(&got_l), non_stamp(&want), "{ctx}: lines");
    assert_eq!(stamps(&got_l), stamps(&want), "{ctx}: stamps");

    // Untouched lines keep their bytes and order.
    let keep = |v: Vec<Vec<u8>>| -> Vec<Vec<u8>> {
        v.into_iter()
            .filter(|l| {
                let n = raw_name(l);
                !is_stamp(&n) && !changed.contains(&n.as_str())
            })
            .collect()
    };
    assert_eq!(
        keep(raw_logical(&out)),
        keep(raw_logical(&input)),
        "{ctx}: preserved bytes"
    );
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

const L1: &str = "plan4-list1";

#[test]
fn summary_edit_matches_2do() {
    check(
        L1,
        "01-create",
        "02-edit",
        A,
        &[Change::Summary("A2".into())],
        &["SUMMARY"],
    );
}

#[test]
fn notes_edit_and_clear_match_2do() {
    check(
        L1,
        "01-create",
        "02-edit",
        B,
        &[Change::Description(Some("x, y\nz".into()))],
        &["DESCRIPTION"],
    );
    check(
        L1,
        "02-edit",
        "03-complete",
        B,
        &[Change::Description(None)],
        &["DESCRIPTION"],
    );
}

#[test]
fn due_edit_and_remove_match_2do() {
    check(
        L1,
        "03-complete",
        "04-reopen",
        B,
        &[Change::Due(Some(Due::Date(date(2026, 12, 5))))],
        &["DUE", "DTSTART"],
    );
    check(
        L1,
        "04-reopen",
        "05-move",
        B,
        &[Change::Due(None)],
        &["DUE", "DTSTART"],
    );
}

#[test]
fn timed_due_edit_matches_2do() {
    let t = date(2026, 12, 3).and_hms_opt(18, 45, 0).unwrap();
    check(
        L1,
        "01-create",
        "02-edit",
        C,
        &[Change::Due(Some(Due::Time(t)))],
        &["DUE", "DTSTART", "X-2DOAPP-METADATA"],
    );
    check(
        L1,
        "02-edit",
        "03-complete",
        C,
        &[Change::Due(Some(Due::Date(date(2026, 12, 3))))],
        &[
            "DUE",
            "DTSTART",
            "X-2DOAPP-METADATA",
            "BEGIN:VALARM",
            "END:VALARM",
            "TRIGGER",
            "ACTION",
            "DESCRIPTION",
            "X-2DOAPP-SOUND",
            "X-2DOAPP-ISAUTO",
            "X-2DOAPP-RELDUETIME",
        ],
    );
}

#[test]
fn start_edit_and_remove_match_2do() {
    check(
        L1,
        "01-create",
        "02-edit",
        D,
        &[Change::Start(Some(date(2026, 12, 2)))],
        &["X-2DOAPP-METADATA"],
    );
    check(
        L1,
        "02-edit",
        "03-complete",
        D,
        &[Change::Start(None)],
        &["X-2DOAPP-METADATA"],
    );
}

#[test]
fn repeat_interval_edit_matches_2do() {
    for (scenario, uid) in [("rule-months-day-1-28", MONTHLY), ("rule-years", YEARLY)] {
        check(
            scenario,
            "02-complete",
            "03-edit",
            uid,
            &[Change::RepeatInterval(2)],
            &["RRULE", "X-2DOAPP-METADATA"],
        );
    }
}

fn todo(cal: &Calendar) -> &Component {
    cal.root.children_named("VTODO").next().unwrap()
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-01T15:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn midnight_due_time_writes_duetime_0() {
    let mut cal = parse(&fixture(L1, "01-create", A)).unwrap();
    let t = date(2026, 12, 3).and_hms_opt(0, 0, 0).unwrap();
    apply_changes(&mut cal, &[Change::Due(Some(Due::Time(t)))], now()).unwrap();
    let cal = parse(&serialize(&cal)).unwrap();
    let todo = todo(&cal);
    for name in ["DUE", "DTSTART"] {
        let p = todo.prop(name).unwrap();
        assert_eq!(p.value, "20261203T000000", "{name}");
        assert!(p.params.is_empty(), "{name}");
    }
    let meta = MetaJson::parse(&todo.prop("X-2DOAPP-METADATA").unwrap().value).unwrap();
    assert_eq!(meta.get_i64("duetime"), Some(0));
    // A timed due gets 2Do's automatic alert (user decision, 2026-11-01).
    assert_eq!(auto_alarms(todo), vec![auto_alarm("A")]);
    assert_eq!(todo.prop("DTSTAMP").unwrap().value, "20261101T150000Z");
    assert_eq!(
        todo.prop("LAST-MODIFIED").unwrap().value,
        "20261101T150000Z"
    );
}

#[test]
fn empty_title_rejected() {
    let input = fixture(L1, "01-create", A);
    let mut cal = parse(&input).unwrap();
    let before = cal.clone();
    let err = apply_changes(
        &mut cal,
        &[
            Change::Description(Some("n".into())),
            Change::Summary("  ".into()),
        ],
        now(),
    )
    .unwrap_err();
    assert!(matches!(err, EditError::EmptyTitle), "{err:?}");
    assert_eq!(cal, before);
    assert_eq!(serialize(&cal), input);
}

const SYNTHETIC: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:synthetic-1\r\nSUMMARY:Synthetic\r\nCREATED:20261101T100000Z\r\nDTSTART;VALUE=DATE:20261120\r\nDUE;VALUE=DATE:20261125\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";

#[test]
fn dtstart_without_json_meta_is_left_alone() {
    let mut cal = parse(SYNTHETIC.as_bytes()).unwrap();
    apply_changes(
        &mut cal,
        &[Change::Due(Some(Due::Date(date(2026, 11, 28))))],
        now(),
    )
    .unwrap();
    let out = String::from_utf8(serialize(&cal)).unwrap();
    assert!(out.contains("\r\nDTSTART;VALUE=DATE:20261120\r\n"), "{out}");
    assert!(out.contains("\r\nDUE;VALUE=DATE:20261128\r\n"), "{out}");
    assert!(!out.contains("X-2DOAPP-METADATA"), "{out}");
}

#[test]
fn touch_inserts_missing_stamps_after_created() {
    let mut cal = parse(SYNTHETIC.as_bytes()).unwrap();
    let t = cal.root.children_named_mut("VTODO").next().unwrap();
    touch(t, now());
    let out = String::from_utf8(serialize(&cal)).unwrap();
    assert!(
        out.contains(
            "CREATED:20261101T100000Z\r\nDTSTAMP:20261101T150000Z\r\nLAST-MODIFIED:20261101T150000Z\r\nDTSTART"
        ),
        "{out}"
    );
}

#[test]
fn start_with_token_metadata_is_unsupported() {
    let s = SYNTHETIC.replace("END:VTODO", "X-2DOAPP-METADATA:@starred\r\nEND:VTODO");
    let mut cal = parse(s.as_bytes()).unwrap();
    let before = cal.clone();
    let err =
        apply_changes(&mut cal, &[Change::Start(Some(date(2026, 11, 21)))], now()).unwrap_err();
    assert!(
        matches!(&err, EditError::Unsupported(m) if m == "this task's 2Do metadata format is read-only"),
        "{err:?}"
    );
    assert_eq!(cal, before);
}

#[test]
fn start_without_metadata_inserts_default_meta() {
    let no_start = SYNTHETIC.replace("DTSTART;VALUE=DATE:20261120\r\n", "");
    let mut cal = parse(no_start.as_bytes()).unwrap();
    apply_changes(&mut cal, &[Change::Start(Some(date(2026, 12, 2)))], now()).unwrap();
    let cal = parse(&serialize(&cal)).unwrap();
    let meta = MetaJson::parse(&todo(&cal).prop("X-2DOAPP-METADATA").unwrap().value).unwrap();
    assert_eq!(meta.get_i64("StartDate"), Some(1_796_169_600));
    assert_eq!(meta.get_i64("RecurrenceValue"), Some(0));
}

#[test]
fn repeat_interval_one_removes_interval() {
    let s = SYNTHETIC.replace(
        "END:VTODO",
        "RRULE:FREQ=WEEKLY;INTERVAL=3;BYDAY=MO\r\nEND:VTODO",
    );
    let mut cal = parse(s.as_bytes()).unwrap();
    apply_changes(&mut cal, &[Change::RepeatInterval(1)], now()).unwrap();
    assert_eq!(
        todo(&cal).prop("RRULE").unwrap().value,
        "FREQ=WEEKLY;BYDAY=MO"
    );
    apply_changes(&mut cal, &[Change::RepeatInterval(4)], now()).unwrap();
    assert_eq!(
        todo(&cal).prop("RRULE").unwrap().value,
        "FREQ=WEEKLY;BYDAY=MO;INTERVAL=4"
    );
    let before = cal.clone();
    assert!(apply_changes(&mut cal, &[Change::RepeatInterval(0)], now()).is_err());
    assert_eq!(cal, before);
}

#[test]
fn start_on_task_with_own_dtstart_and_no_meta_is_unsupported() {
    let msg =
        "setting a start date on a task with its own DTSTART and no 2Do metadata is not supported";
    let no_due = SYNTHETIC.replace("DUE;VALUE=DATE:20261125\r\n", "");
    let start = Change::Start(Some(date(2026, 11, 21)));
    let due = Change::Due(Some(Due::Date(date(2026, 11, 28))));
    for (src, changes) in [
        (SYNTHETIC.to_string(), vec![start.clone()]),
        (SYNTHETIC.to_string(), vec![due.clone(), start.clone()]),
        (no_due, vec![start.clone()]),
    ] {
        let mut cal = parse(src.as_bytes()).unwrap();
        let before = cal.clone();
        let err = apply_changes(&mut cal, &changes, now()).unwrap_err();
        assert!(
            matches!(&err, EditError::Unsupported(m) if m == msg),
            "{changes:?}: {err:?}"
        );
        assert_eq!(cal, before, "{changes:?}");
    }
    // A DTSTART that mirrors DUE is not a real start: allowed.
    let mirror = SYNTHETIC.replace("DTSTART;VALUE=DATE:20261120", "DTSTART;VALUE=DATE:20261125");
    let mut cal = parse(mirror.as_bytes()).unwrap();
    apply_changes(&mut cal, &[start], now()).unwrap();
}

#[test]
fn due_time_seconds_are_zeroed() {
    let mut cal = parse(&fixture(L1, "01-create", C)).unwrap();
    let t = date(2026, 12, 3).and_hms_opt(18, 45, 30).unwrap();
    apply_changes(&mut cal, &[Change::Due(Some(Due::Time(t)))], now()).unwrap();
    let todo = todo(&cal);
    assert_eq!(todo.prop("DUE").unwrap().value, "20261203T184500");
    assert_eq!(todo.prop("DTSTART").unwrap().value, "20261203T184500");
}

/// 2Do's automatic at-due-time alert for a task titled `summary` (H8, `01-create` C).
fn auto_alarm(summary: &str) -> Vec<(String, Vec<Param>, String)> {
    let g = || {
        vec![Param {
            name: "SHARE-SCOPE".into(),
            values: vec!["GLOBAL".into()],
        }]
    };
    vec![
        ("TRIGGER".into(), vec![], "PT0S".into()),
        ("ACTION".into(), vec![], "AUDIO".into()),
        ("DESCRIPTION".into(), vec![], summary.into()),
        ("X-2DOAPP-SOUND".into(), g(), "Basso".into()),
        ("X-2DOAPP-ISAUTO".into(), g(), "1".into()),
        ("X-2DOAPP-RELDUETIME".into(), g(), "1".into()),
    ]
}

/// Every VALARM child as (name, params, value) lines.
fn auto_alarms(todo: &Component) -> Vec<Vec<(String, Vec<Param>, String)>> {
    todo.children_named("VALARM")
        .map(|a| {
            a.props
                .iter()
                .map(|p| (p.name.clone(), p.params.clone(), p.value.clone()))
                .collect()
        })
        .collect()
}

/// Date-only -> timed adds exactly 2Do's automatic alert: C `03-complete` (date-only) back
/// to its `02-edit` state (timed 18:45, with the alarm 2Do added).
#[test]
fn date_only_to_timed_adds_auto_alarm_like_2do() {
    let t = date(2026, 12, 3).and_hms_opt(18, 45, 0).unwrap();
    check(
        L1,
        "03-complete",
        "02-edit",
        C,
        &[Change::Due(Some(Due::Time(t)))],
        &[
            "DUE",
            "DTSTART",
            "X-2DOAPP-METADATA",
            "BEGIN:VALARM",
            "END:VALARM",
            "TRIGGER",
            "ACTION",
            "DESCRIPTION",
            "X-2DOAPP-SOUND",
            "X-2DOAPP-ISAUTO",
            "X-2DOAPP-RELDUETIME",
        ],
    );
}

/// Timed -> timed keeps the one existing auto alarm as is (2Do did, `01-create` -> `02-edit`).
#[test]
fn timed_to_timed_keeps_one_auto_alarm() {
    let input = fixture(L1, "01-create", C);
    let mut cal = parse(&input).unwrap();
    let t = date(2026, 12, 4).and_hms_opt(9, 15, 0).unwrap();
    apply_changes(&mut cal, &[Change::Due(Some(Due::Time(t)))], now()).unwrap();
    let orig = parse(&input).unwrap();
    assert_eq!(auto_alarms(todo(&cal)), auto_alarms(todo(&orig)));
    assert_eq!(auto_alarms(todo(&cal)), vec![auto_alarm("C")]);
}

/// Ruling R14: a title edit leaves an existing auto alarm's DESCRIPTION unchanged.
#[test]
fn retitle_keeps_auto_alarm_description() {
    let mut cal = parse(&fixture(L1, "01-create", C)).unwrap();
    apply_changes(&mut cal, &[Change::Summary("C2".into())], now()).unwrap();
    assert_eq!(todo(&cal).prop("SUMMARY").unwrap().value, "C2");
    assert_eq!(auto_alarms(todo(&cal)), vec![auto_alarm("C")]);
}

/// A task without `<2Do Meta>` JSON gets no auto alarm.
#[test]
fn timed_due_without_json_meta_adds_no_alarm() {
    let mut cal = parse(SYNTHETIC.as_bytes()).unwrap();
    let t = date(2026, 11, 28).and_hms_opt(10, 0, 0).unwrap();
    apply_changes(&mut cal, &[Change::Due(Some(Due::Time(t)))], now()).unwrap();
    assert!(todo(&cal).children_named("VALARM").next().is_none());
}
