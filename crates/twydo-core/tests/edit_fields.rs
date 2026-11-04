//! Field edits against the iPhone captures: each edit must change exactly the lines 2Do
//! changed, to the same values, and keep every other line's bytes.

use std::collections::BTreeSet;
use std::path::Path;

use chrono::{DateTime, NaiveDate, Utc};
use twydo_core::edit::{Change, Due, EditError, apply_changes, touch};
use twydo_core::ical::{Calendar, Component, Param, parse, serialize};
use twydo_core::model::TaskKind;
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

// ---- Repeat rules (Plan 7 Task 4), against the iPhone's repeat picker ----

const P7: &str = "plan7-list1";

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

/// The JSON keys and values in order, with the phone's own `RUID` id replaced by a marker
/// (2Do writes its internal id there, which Twydo cannot know; Twydo writes the UID) and
/// `TaskType` / `TaskDuration` in one order (the phone swapped them in R16 only).
fn meta_pairs(value: &str) -> String {
    let start = value.find("<2Do Meta>").unwrap() + "<2Do Meta>".len();
    let end = value.find("</2Do Meta>").unwrap();
    let serde_json::Value::Object(obj) =
        serde_json::from_str(&percent_decode(&value[start..end])).unwrap()
    else {
        panic!("not an object");
    };
    let mut pairs: Vec<(String, serde_json::Value)> = obj.into_iter().collect();
    for (k, v) in &mut pairs {
        if k == "RUID" && v != "" {
            *v = serde_json::Value::from("<own>");
        }
    }
    let pos = |pairs: &[(String, serde_json::Value)], k: &str| {
        pairs.iter().position(|(n, _)| n == k).unwrap()
    };
    let (t, d) = (pos(&pairs, "TaskType"), pos(&pairs, "TaskDuration"));
    if t < d {
        pairs.swap(t, d);
    }
    format!("{pairs:?}")
}

/// Every property as one sortable string. Left out, because they are not part of setting a
/// repeat: `DTSTAMP`/`LAST-MODIFIED` (the stamps), and what the phone adds when it rewrites a
/// Twydo-created file wholesale in its own layout: `PRODID`, `X-2DOAPP-SYNC-IMAGE`,
/// `X-2DOAPP-SYNC-AUDIO` and `X-2DOAPP-AUDIO-DURATION`.
fn repeat_lines(cal: &Calendar) -> Vec<String> {
    const PHONE_LAYOUT: [&str; 4] = [
        "PRODID",
        "X-2DOAPP-SYNC-IMAGE",
        "X-2DOAPP-SYNC-AUDIO",
        "X-2DOAPP-AUDIO-DURATION",
    ];
    let mut out: Vec<String> = lines(cal)
        .into_iter()
        .filter(|l| !is_stamp(&l.1) && !PHONE_LAYOUT.contains(&l.1.as_str()))
        .map(|(path, name, params, value)| {
            let value = if name == "X-2DOAPP-METADATA" {
                meta_pairs(&value)
            } else {
                value
            };
            format!("{path}|{name}|{params:?}|{value}")
        })
        .collect();
    out.sort();
    out
}

fn work_days() -> Vec<chrono::Weekday> {
    use chrono::Weekday::*;
    vec![Mon, Tue, Wed, Thu, Fri]
}

fn spec(input: &str) -> twydo_core::repeatinput::RepeatSpec {
    twydo_core::repeatinput::parse(input, date(2026, 11, 3), &work_days())
        .unwrap()
        .unwrap()
}

/// Apply `Change::Repeat(spec(input))` to the plain `00-seed` task `uid` and compare with the
/// phone's `01-phone-repeat` file; `RRULE` must sit right after `DUE`.
fn check_repeat(uid: &str, input: &str) {
    let mut cal = parse(&fixture(P7, "00-seed", uid)).unwrap();
    let want = parse(&fixture(P7, "01-phone-repeat", uid)).unwrap();
    apply_changes(&mut cal, &[Change::Repeat(Some(spec(input)))], now()).unwrap();
    let got = parse(&serialize(&cal)).unwrap();
    assert_eq!(repeat_lines(&got), repeat_lines(&want), "{input}");
    let names: Vec<&str> = todo(&got).props.iter().map(|p| p.name.as_str()).collect();
    let due = names.iter().position(|n| *n == "DUE").unwrap();
    assert_eq!(names[due + 1], "RRULE", "{input}");
    let meta = &todo(&got).prop("X-2DOAPP-METADATA").unwrap().value;
    assert!(percent_decode(meta).contains(&format!("\"RUID\":\"{uid}\"")));
}

#[test]
fn set_repeat_matches_phone_edit_daily() {
    check_repeat("a8d833aa427849239c35cb2d8cfef721", "daily"); // R01
    check_repeat("5965cd6bfaa64fc2960f51449c63f5bd", "daily"); // R02
}

#[test]
fn set_repeat_matches_phone_edit_every_n_days() {
    check_repeat("f7f92e74cc4746a5a9ae41eaae87a54e", "3d"); // R03
}

#[test]
fn set_repeat_matches_phone_edit_weekly() {
    check_repeat("ce75b21d989147aab17422d262bf0acd", "weekly"); // R04
    check_repeat("f07c68d24c4f476f81b81731141a3192", "2w"); // R05
}

#[test]
fn set_repeat_matches_phone_edit_weekday_set() {
    check_repeat("391a95aa2b524602847685a7276aefab", "mon thu"); // R06
    check_repeat("5864b3930d9f4959a451f8042f538244", "workdays"); // R07
}

#[test]
fn set_repeat_matches_phone_edit_monthly() {
    check_repeat("992fea854c4d42baaa0ad2d20f3f65a5", "monthly"); // R08
}

#[test]
fn set_repeat_matches_phone_edit_nth_weekday() {
    check_repeat("08aa0eefe10e484d8221dcda095a911b", "2nd tue"); // R10
}

#[test]
fn set_repeat_matches_phone_edit_yearly() {
    check_repeat("55bab4f5add845729158a006077fa70d", "yearly"); // R13
}

#[test]
fn set_repeat_matches_phone_edit_from_completion() {
    check_repeat("18669d33edad43c7a70152cd1b49fc0c", "monthly after done"); // R14
}

#[test]
fn set_repeat_matches_phone_edit_end_after_count() {
    check_repeat("921f88b27e2d4563bb29a773cbfe4262", "monthly 3 times"); // R15
}

#[test]
fn set_repeat_matches_phone_edit_end_on_date() {
    check_repeat(
        "7ca468eaff814c459baf0667fe9ba1b3",
        "monthly until 2027-06-30",
    ); // R16
}

#[test]
fn repeat_without_due_matches_phone() {
    // R17: the caller gives a due-less task today's date first (2Do's repeat picker does).
    let uid = "cfd0a124f69a4f55a8180158fb118d6d";
    let mut cal = parse(&fixture(P7, "00-seed", uid)).unwrap();
    let want = parse(&fixture(P7, "02-phone-repeat-no-due", uid)).unwrap();
    let changes = [
        Change::Due(Some(Due::Date(date(2026, 11, 3)))),
        Change::Repeat(Some(spec("weekly"))),
    ];
    apply_changes(&mut cal, &changes, now()).unwrap();
    let got = parse(&serialize(&cal)).unwrap();
    assert_eq!(repeat_lines(&got), repeat_lines(&want));

    // Without a due date the repeat alone is refused, and nothing changes.
    let mut cal = parse(&fixture(P7, "00-seed", uid)).unwrap();
    let before = cal.clone();
    let err = apply_changes(&mut cal, &[Change::Repeat(Some(spec("weekly")))], now());
    assert!(matches!(err, Err(EditError::Unsupported(_))));
    assert_eq!(cal, before);
}

#[test]
fn remove_repeat_matches_phone() {
    // R09 (the picker set back to none) shows what the phone leaves: no RRULE, `RecurrenceType`
    // 0, and `RecurrenceValue`, `RecurrenceFrom` and `RUID` as the repeat had them.
    let uid = "a8d833aa427849239c35cb2d8cfef721"; // R01, daily
    let mut cal = parse(&fixture(P7, "01-phone-repeat", uid)).unwrap();
    let before = cal.clone();
    apply_changes(&mut cal, &[Change::Repeat(None)], now()).unwrap();
    assert!(todo(&cal).prop("RRULE").is_none());
    let json = |c: &Calendar| meta_pairs(&todo(c).prop("X-2DOAPP-METADATA").unwrap().value);
    let want = json(&before).replace(
        "(\"RecurrenceType\", Number(256))",
        "(\"RecurrenceType\", Number(0))",
    );
    assert_ne!(want, json(&before));
    assert_eq!(json(&cal), want);
    // Every other line keeps its bytes.
    let rest = |c: &Calendar| {
        lines(c)
            .into_iter()
            .filter(|l| !is_stamp(&l.1) && l.1 != "RRULE" && l.1 != "X-2DOAPP-METADATA")
            .collect::<Vec<_>>()
    };
    assert_eq!(rest(&cal), rest(&before));

    // Removing a repeat that is not there changes nothing but the stamps.
    let seed = parse(&fixture(P7, "00-seed", uid)).unwrap();
    let mut plain = seed.clone();
    apply_changes(&mut plain, &[Change::Repeat(None)], now()).unwrap();
    assert_eq!(rest(&plain), rest(&seed));
    assert_eq!(json(&plain), json(&seed));
}

#[test]
fn codes_follow_the_2do_table() {
    use twydo_core::edit::repeat::twodo_codes;
    use twydo_core::model::repeat::parse_rrule;
    let codes = |s: &str| twodo_codes(&parse_rrule(s).unwrap());
    assert_eq!(codes("FREQ=DAILY"), (256, 1));
    assert_eq!(codes("FREQ=DAILY;INTERVAL=3"), (256, 3));
    assert_eq!(codes("FREQ=WEEKLY"), (257, 1));
    assert_eq!(codes("FREQ=WEEKLY;INTERVAL=2"), (257, 2));
    assert_eq!(codes("FREQ=MONTHLY"), (258, 1));
    assert_eq!(codes("FREQ=MONTHLY;INTERVAL=3"), (258, 3));
    assert_eq!(codes("FREQ=YEARLY"), (259, 1));
    // Weekday bitmask: Sun 1, Sat 2, Fri 4, Thu 8, Wed 16, Tue 32, Mon 64.
    assert_eq!(codes("FREQ=WEEKLY;BYDAY=SU"), (1, 1));
    assert_eq!(codes("FREQ=WEEKLY;BYDAY=SA"), (2, 1));
    assert_eq!(codes("FREQ=WEEKLY;BYDAY=FR"), (4, 1));
    assert_eq!(codes("FREQ=WEEKLY;BYDAY=TH"), (8, 1));
    assert_eq!(codes("FREQ=WEEKLY;BYDAY=WE"), (16, 1));
    assert_eq!(codes("FREQ=WEEKLY;BYDAY=TU"), (32, 1));
    assert_eq!(codes("FREQ=WEEKLY;BYDAY=MO"), (64, 1));
    assert_eq!(codes("FREQ=WEEKLY;BYDAY=MO,TH"), (72, 1));
    assert_eq!(codes("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"), (124, 1));
    assert_eq!(codes("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO"), (64, 2));
    // Nth weekday of the month: 0x104 Monday .. 0x10A Sunday, value = the ordinal; "last" is
    // 6, the phone's own code (session A `R09`, session C `LF`), not -1.
    assert_eq!(codes("FREQ=MONTHLY;BYDAY=1MO"), (0x104, 1));
    assert_eq!(codes("FREQ=MONTHLY;BYDAY=2TU"), (0x105, 2));
    assert_eq!(codes("FREQ=MONTHLY;BYDAY=3WE"), (0x106, 3));
    assert_eq!(codes("FREQ=MONTHLY;BYDAY=4TH"), (0x107, 4));
    assert_eq!(codes("FREQ=MONTHLY;BYDAY=5FR"), (0x108, 5));
    assert_eq!(codes("FREQ=MONTHLY;BYDAY=-1SA"), (0x109, 6));
    assert_eq!(codes("FREQ=MONTHLY;BYDAY=-1FR"), (0x108, 6));
    assert_eq!(codes("FREQ=MONTHLY;BYDAY=1SU"), (0x10A, 1));
    // Last day of the month, every N months.
    assert_eq!(codes("FREQ=MONTHLY;BYMONTHDAY=-1"), (0x10B, 1));
    assert_eq!(codes("FREQ=MONTHLY;INTERVAL=2;BYMONTHDAY=-1"), (0x10B, 2));
    // Nth weekday (Mon-Fri) and Nth weekend day of the month.
    assert_eq!(
        codes("FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1"),
        (0x10C, -1)
    );
    assert_eq!(codes("FREQ=MONTHLY;BYDAY=SA,SU;BYSETPOS=1"), (0x10D, 1));
}

#[test]
fn set_repeat_keeps_a_foreign_ruid_and_fills_an_empty_one() {
    let uid = "a8d833aa427849239c35cb2d8cfef721"; // R01
    let ruid = |c: &Calendar| {
        let v = &todo(c).prop("X-2DOAPP-METADATA").unwrap().value;
        MetaJson::parse(v)
            .unwrap()
            .get_str("RUID")
            .unwrap()
            .to_string()
    };
    // Already repeating (phone-written series id): changing the rule keeps it.
    let mut cal = parse(&fixture(P7, "01-phone-repeat", uid)).unwrap();
    let series = ruid(&cal);
    assert!(!series.is_empty() && series != uid);
    apply_changes(&mut cal, &[Change::Repeat(Some(spec("weekly")))], now()).unwrap();
    assert_eq!(ruid(&cal), series);
    // A plain task (empty RUID) gets its own UID.
    let mut cal = parse(&fixture(P7, "00-seed", uid)).unwrap();
    apply_changes(&mut cal, &[Change::Repeat(Some(spec("weekly")))], now()).unwrap();
    assert_eq!(ruid(&cal), uid);
}

// ---- Change type (Plan 7 Task 6), against the iPhone's type selector ----

const K1: &str = "962ea2a85f75455aafa1102e68e1b92a";
const K2: &str = "b783e95fdb40450aa81cab922cb114f4";
const CHECKLIST_C: &str = "f45f2dab11144ad095882c25578a2958";

/// The JSON keys and values in order. The phone's own `uid` (2Do's internal id for a
/// Twydo-created parent) is replaced by a marker, since Twydo writes the task's UID there.
fn kind_pairs(cal: &Calendar) -> Vec<(String, serde_json::Value)> {
    let value = &todo(cal).prop("X-2DOAPP-METADATA").unwrap().value;
    let start = value.find("<2Do Meta>").unwrap() + "<2Do Meta>".len();
    let end = value.find("</2Do Meta>").unwrap();
    let serde_json::Value::Object(obj) =
        serde_json::from_str(&percent_decode(&value[start..end])).unwrap()
    else {
        panic!("not an object");
    };
    obj.into_iter()
        .map(|(k, v)| {
            let v = if k == "uid" { "<own>".into() } else { v };
            (k, v)
        })
        .collect()
}

/// Apply `Change::Kind(kind)` to `from`'s file and compare with the phone's `to` file: same
/// lines (leaving out the stamps and the phone's own layout), the JSON in the phone's key
/// order, and JSON `uid` = the task's UID when there is one.
fn check_kind(from: &str, to: &str, uid: &str, kind: twydo_core::model::TaskKind) {
    let mut cal = parse(&fixture(P7, from, uid)).unwrap();
    let want = parse(&fixture(P7, to, uid)).unwrap();
    apply_changes(&mut cal, &[Change::Kind(kind)], now()).unwrap();
    let got = parse(&serialize(&cal)).unwrap();
    let ctx = format!("{from}->{to} {uid}");
    let strip = |c: &Calendar| -> Vec<String> {
        let mut l = repeat_lines(c);
        l.retain(|x| !x.contains("|X-2DOAPP-METADATA|"));
        l
    };
    assert_eq!(strip(&got), strip(&want), "{ctx}: lines");
    assert_eq!(kind_pairs(&got), kind_pairs(&want), "{ctx}: JSON");
    let json = percent_decode(&todo(&got).prop("X-2DOAPP-METADATA").unwrap().value);
    let own = format!("\"uid\":\"{uid}\"");
    assert_eq!(
        json.contains(&own),
        kind_pairs(&want).iter().any(|(k, _)| k == "uid")
    );
}

#[test]
fn task_to_checklist() {
    check_kind(
        "02-phone-repeat-no-due",
        "03-phone-convert",
        K1,
        TaskKind::Checklist,
    );
}

#[test]
fn task_to_project() {
    check_kind(
        "02-phone-repeat-no-due",
        "03-phone-convert",
        K2,
        TaskKind::Project,
    );
}

#[test]
fn checklist_to_project() {
    check(
        P7,
        "02-phone-repeat-no-due",
        "03-phone-convert",
        CHECKLIST_C,
        &[Change::Kind(TaskKind::Project)],
        &["X-2DOAPP-METADATA"],
    );
}

#[test]
fn empty_parent_to_task() {
    check_kind(
        "03-phone-convert",
        "04-phone-convert-empty",
        K1,
        TaskKind::Task,
    );
}

#[test]
fn empty_project_to_checklist() {
    // No capture: the mirror of checklist -> project. Only `TaskType` changes, `uid` and the
    // parent key order stay.
    let mut cal = parse(&fixture(P7, "03-phone-convert", K2)).unwrap();
    let before = kind_pairs(&cal);
    apply_changes(&mut cal, &[Change::Kind(TaskKind::Checklist)], now()).unwrap();
    let after = kind_pairs(&cal);
    assert_eq!(after.len(), before.len());
    for ((bk, bv), (ak, av)) in before.iter().zip(&after) {
        assert_eq!(bk, ak);
        if bk == "TaskType" {
            assert_eq!((bv, av), (&2.into(), &1.into()));
        } else {
            assert_eq!(bv, av, "{bk}");
        }
    }
}

#[test]
fn same_kind_changes_only_the_stamps() {
    let input = fixture(P7, "03-phone-convert", K1);
    let mut cal = parse(&input).unwrap();
    let before = cal.clone();
    apply_changes(&mut cal, &[Change::Kind(TaskKind::Checklist)], now()).unwrap();
    let rest = |c: &Calendar| {
        lines(c)
            .into_iter()
            .filter(|l| !is_stamp(&l.1))
            .collect::<Vec<_>>()
    };
    assert_eq!(rest(&cal), rest(&before));
}

#[test]
fn tokens_metadata_refused() {
    let mut cal = parse(&fixture(P7, "02-phone-repeat-no-due", K1)).unwrap();
    let meta = cal.root.children_named_mut("VTODO").next().unwrap();
    meta.prop_mut("X-2DOAPP-METADATA")
        .unwrap()
        .set_value("@tasktype=0");
    let before = cal.clone();
    let err = apply_changes(&mut cal, &[Change::Kind(TaskKind::Project)], now()).unwrap_err();
    assert_eq!(err.to_string(), READ_ONLY);
    assert_eq!(cal, before);
}

const READ_ONLY: &str = "this task's 2Do metadata format is read-only";

/// Writing a repeat updates 2Do's codes in the JSON metadata; with tokens or no metadata the
/// rule would go stale beside them, so it is refused and nothing changes.
#[test]
fn repeat_refused_without_json_metadata() {
    let base = parse(&fixture(P7, "02-phone-repeat-no-due", K1)).unwrap();
    let mut tokens = base.clone();
    let todo = tokens.root.children_named_mut("VTODO").next().unwrap();
    todo.prop_mut("X-2DOAPP-METADATA")
        .unwrap()
        .set_value("@tasktype=0");
    let mut absent = base.clone();
    let todo = absent.root.children_named_mut("VTODO").next().unwrap();
    todo.remove_props("X-2DOAPP-METADATA");
    for (what, cal) in [("tokens", tokens), ("absent", absent)] {
        for change in [
            vec![
                Change::Due(Some(Due::Date(date(2026, 12, 7)))),
                Change::Repeat(Some(spec("weekly"))),
            ],
            vec![Change::Repeat(None)],
        ] {
            let mut c = cal.clone();
            let err = apply_changes(&mut c, &change, now()).unwrap_err();
            assert_eq!(err.to_string(), READ_ONLY, "{what} {change:?}");
            assert_eq!(c, cal, "{what}");
        }
    }
}

/// An Nth-weekday repeat keeps its ordinal in JSON `RecurrenceValue` (2Do has no interval for
/// it), so a new interval would overwrite the ordinal: refused, nothing changed (`N2b`, 2nd
/// Tuesday, session C seed).
#[test]
fn interval_of_an_nth_weekday_repeat_is_refused() {
    let input = fixture(
        "plan7-twydo-list2",
        "00-seed",
        "200243c3e116428e8eb384093bd6d3a4",
    );
    for n in [1, 2] {
        let mut cal = parse(&input).unwrap();
        let before = cal.clone();
        let err = apply_changes(&mut cal, &[Change::RepeatInterval(n)], now()).unwrap_err();
        match err {
            EditError::Unsupported(m) => assert_eq!(
                m, "changing the interval of an Nth-weekday repeat is not supported",
                "{n}"
            ),
            e => panic!("{n}: {e:?}"),
        }
        assert_eq!(cal, before);
    }
}
