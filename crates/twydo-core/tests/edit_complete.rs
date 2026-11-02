//! Completion, reopen and recurring copy-and-advance against the iPhone captures.

use std::collections::BTreeSet;
use std::path::Path;

use chrono::{DateTime, NaiveDate, Utc};
use twydo_core::edit::{
    EditError, complete_one_time, complete_recurring, next_due, reopen_one_time,
};
use twydo_core::ical::{Calendar, Component, Param, parse, serialize};
use twydo_core::model::meta_edit::MetaJson;
use twydo_core::model::repeat::parse_rrule;

const A: &str = "0a04f22aeb294b719633c978fe1975e5";
const E: &str = "44123b09407c4ea1adae8f2ecdb64d49";
const E_COPY: &str = "7d0349b58c3545d2ba6c8d87bcc45e85";
const MONTHLY: &str = "5c072ad90a8f44f1a974e170a8a950e7";
const MONTHLY_COPY: &str = "6f19ef6b404644a38451520da5804e9f";
const YEARLY: &str = "c3e7283649dd413abf3f73745121bce5";
const YEARLY_COPY: &str = "8c164aa9120c42df846a5680e1788e5f";
const L1: &str = "plan4-list1";
const P6: &str = "plan6-list1";
const L: &str = "441bfb76e9e14d93a024aceaebee05e7";
const L_COPY1: &str = "e6e12fe5069f4f7895a8d2f61f34e8fb";
const L_COPY2: &str = "8a33f149c12143b183bcdbf446cb5268";
const S: &str = "06c9664dd9144be185b7436957745800";
const S_COPY: &str = "108f857c7247424bae7a6a82fee5c778";
const T: &str = "094c6d401ec74cfd848329c3359e2283";
const T_COPY: &str = "cd6ab25e6f2643ec827a61ce798a119d";
const Y: &str = "d27d9dc0b8954b9687896bf711797856";
const Y_COPY: &str = "602bb1883b974d728f4dff6d8e5f8da0";

fn fixture(scenario: &str, step: &str, uid: &str) -> Vec<u8> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/phase0/v1")
        .join(scenario)
        .join(step)
        .join(format!("{uid}.ics"));
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

type Line = (String, String, Vec<Param>, String);

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

/// Every property as (component path, name, params, value), in document order.
fn lines(cal: &Calendar) -> Vec<Line> {
    let mut out = Vec::new();
    flat(&cal.root, "", &mut out);
    out
}

fn sorted(mut v: Vec<Line>) -> Vec<Line> {
    v.sort_by_cached_key(|l| format!("{l:?}"));
    v
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

fn todo(cal: &Calendar) -> &Component {
    cal.root.children_named("VTODO").next().unwrap()
}

fn dtstamp(cal: &Calendar) -> DateTime<Utc> {
    let v = &todo(cal).prop("DTSTAMP").unwrap().value;
    chrono::NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%SZ")
        .unwrap()
        .and_utc()
}

fn stamp(now: DateTime<Utc>) -> String {
    now.format("%Y%m%dT%H%M%SZ").to_string()
}

/// The evidence with `COMPLETED` set to `now`: 2Do's completion time is a few seconds
/// before its sync stamp, Twydo uses one `now` for both.
fn with_completed_now(mut v: Vec<Line>, now: DateTime<Utc>) -> Vec<Line> {
    for l in &mut v {
        if l.1 == "COMPLETED" {
            l.3 = stamp(now);
        }
    }
    v
}

/// Names whose lines differ between two property lists.
fn diff_names(a: &[Line], b: &[Line]) -> BTreeSet<String> {
    a.iter()
        .filter(|l| !b.contains(l))
        .chain(b.iter().filter(|l| !a.contains(l)))
        .map(|l| l.1.clone())
        .collect()
}

/// Check `out` (from `input`) against `evidence`:
/// - the fixtures differ exactly in `changed` + the two stamps;
/// - lines not in `changed` equal the evidence's, in order (stamps by value only);
/// - lines in `changed` equal the evidence's as a multiset (`COMPLETED` = `now`);
/// - every untouched line keeps its original bytes and order.
fn check_in_place(
    ctx: &str,
    input: &[u8],
    out: &[u8],
    evidence: &Calendar,
    now: DateTime<Utc>,
    changed: &[&str],
) {
    let before = lines(&parse(input).unwrap());
    let want = with_completed_now(lines(evidence), now);
    let got = lines(&parse(out).unwrap());

    let mut expect: BTreeSet<String> = changed.iter().map(|n| n.to_string()).collect();
    expect.extend(["DTSTAMP".to_string(), "LAST-MODIFIED".to_string()]);
    assert_eq!(
        diff_names(&before, &lines(evidence)),
        expect,
        "{ctx}: fixture changes"
    );

    let fixed = |v: &[Line]| -> Vec<Line> {
        v.iter()
            .filter(|l| !is_stamp(&l.1) && !changed.contains(&l.1.as_str()))
            .cloned()
            .collect()
    };
    let moved = |v: &[Line]| -> Vec<Line> {
        sorted(
            v.iter()
                .filter(|l| is_stamp(&l.1) || changed.contains(&l.1.as_str()))
                .cloned()
                .collect(),
        )
    };
    assert_eq!(fixed(&got), fixed(&want), "{ctx}: untouched lines");
    assert_eq!(moved(&got), moved(&want), "{ctx}: changed lines");

    let keep = |v: Vec<Vec<u8>>| -> Vec<Vec<u8>> {
        v.into_iter()
            .filter(|l| {
                let n = raw_name(l);
                !is_stamp(&n) && !changed.contains(&n.as_str())
            })
            .collect()
    };
    assert_eq!(
        keep(raw_logical(out)),
        keep(raw_logical(input)),
        "{ctx}: preserved bytes"
    );
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-01T15:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn unsupported(err: &EditError) -> &str {
    match err {
        EditError::Unsupported(m) => m,
        e => panic!("expected Unsupported, got {e:?}"),
    }
}

#[test]
fn one_time_complete_and_reopen_match_2do() {
    type Op = fn(&mut Calendar, DateTime<Utc>) -> Result<(), EditError>;
    let cases: [(&str, &str, Op, &[&str]); 2] = [
        (
            "02-edit",
            "03-complete",
            complete_one_time,
            &["STATUS", "PERCENT-COMPLETE", "COMPLETED"],
        ),
        (
            "03-complete",
            "04-reopen",
            reopen_one_time,
            &["STATUS", "PERCENT-COMPLETE", "COMPLETED"],
        ),
    ];
    for (from, to, op, changed) in cases {
        let input = fixture(L1, from, A);
        let evidence = parse(&fixture(L1, to, A)).unwrap();
        let now = dtstamp(&evidence);
        let mut cal = parse(&input).unwrap();
        op(&mut cal, now).unwrap();
        let out = serialize(&cal);
        check_in_place(
            &format!("A {from}->{to}"),
            &input,
            &out,
            &evidence,
            now,
            changed,
        );
    }
}

#[test]
fn complete_places_percent_and_completed_after_status() {
    let mut cal = parse(&fixture(L1, "02-edit", A)).unwrap();
    complete_one_time(&mut cal, now()).unwrap();
    let out = String::from_utf8(serialize(&cal)).unwrap();
    assert!(
        out.contains(
            "\nSTATUS:COMPLETED\nPERCENT-COMPLETE:100\nCOMPLETED:20261101T150000Z\nCREATED:"
        ),
        "{out}"
    );
}

#[test]
fn history_copy_reopen_matches_2do() {
    let input = fixture(L1, "03-complete", E_COPY);
    let evidence = parse(&fixture(L1, "04-reopen", E_COPY)).unwrap();
    let now = dtstamp(&evidence);
    let mut cal = parse(&input).unwrap();
    reopen_one_time(&mut cal, now).unwrap();
    let out = serialize(&cal);
    check_in_place(
        "E copy 03->04",
        &input,
        &out,
        &evidence,
        now,
        &["STATUS", "PERCENT-COMPLETE", "COMPLETED"],
    );
}

/// Complete `<scenario>/<from>/<uid>` with `now` = the evidence's live DTSTAMP and check the
/// live result and the copy against `<scenario>/<to>/`.
fn check_copy_and_advance(
    scenario: &str,
    from: &str,
    to: &str,
    uid: &str,
    copy_uid: &str,
    live_due: &str,
    changed: &[&str],
) {
    let ctx = format!("{scenario} {from}->{to} {uid}");
    let input = fixture(scenario, from, uid);
    let want_live = parse(&fixture(scenario, to, uid)).unwrap();
    let want_copy = parse(&fixture(scenario, to, copy_uid)).unwrap();
    let now = dtstamp(&want_live);
    let cal = parse(&input).unwrap();
    let before = cal.clone();

    let (live, copy) = complete_recurring(&cal, now, copy_uid).unwrap();
    assert_eq!(cal, before, "{ctx}: input untouched");

    // Live: only the stamps, DTSTART and DUE change, to 2Do's values.
    assert_eq!(todo(&want_live).prop("DUE").unwrap().value, live_due);
    let live_out = serialize(&live);
    check_in_place(
        &format!("{ctx} live"),
        &input,
        &live_out,
        &want_live,
        now,
        changed,
    );

    // Copy: same properties as 2Do's history copy (COMPLETED and the stamps = now; 2Do
    // stamps the copy and the live task a second apart in some captures).
    let copy = parse(&serialize(&copy)).unwrap();
    let mut want = with_completed_now(lines(&want_copy), now);
    for l in want.iter_mut().filter(|l| is_stamp(&l.1)) {
        l.3 = stamp(now);
    }
    assert_eq!(sorted(lines(&copy)), sorted(want), "{ctx}: copy");
    assert_eq!(todo(&copy).prop("UID").unwrap().value, copy_uid);

    // Child components (VALARM) are carried over untouched, live and copy.
    let input_cal = parse(&input).unwrap();
    let kids = &todo(&input_cal).children;
    assert_eq!(&todo(&live).children, kids, "{ctx}: live children");
    assert_eq!(&todo(&copy).children, kids, "{ctx}: copy children");
}

#[test]
fn monthly_copy_and_advance_matches_2do_plan4() {
    check_copy_and_advance(
        L1,
        "02-edit",
        "03-complete",
        E,
        E_COPY,
        "20270110",
        &["DTSTART", "DUE"],
    );
}

#[test]
fn monthly_copy_and_advance_matches_2do() {
    check_copy_and_advance(
        "rule-months-day-1-28",
        "01-create",
        "02-complete",
        MONTHLY,
        MONTHLY_COPY,
        "20261210",
        &["DTSTART", "DUE"],
    );
    let evidence = parse(&fixture("rule-months-day-1-28", "02-complete", MONTHLY)).unwrap();
    assert_eq!(stamp(dtstamp(&evidence)), "20261101T121650Z");
}

#[test]
fn yearly_copy_and_advance_matches_2do() {
    check_copy_and_advance(
        "rule-years",
        "01-create",
        "02-complete",
        YEARLY,
        YEARLY_COPY,
        "20271115",
        &["DTSTART", "DUE"],
    );
}

#[test]
fn next_due_intervals() {
    let monthly2 = parse_rrule("FREQ=MONTHLY;INTERVAL=2").unwrap();
    assert_eq!(
        next_due(date(2026, 12, 10), &monthly2).unwrap(),
        date(2027, 2, 10)
    );
    let yearly2 = parse_rrule("FREQ=YEARLY;INTERVAL=2").unwrap();
    assert_eq!(
        next_due(date(2026, 11, 15), &yearly2).unwrap(),
        date(2028, 11, 15)
    );
    let bymonthday = parse_rrule("FREQ=MONTHLY;BYMONTHDAY=10").unwrap();
    let err = next_due(date(2026, 12, 10), &bymonthday).unwrap_err();
    assert!(
        unsupported(&err).starts_with("repeat rule shape not verified: "),
        "{err:?}"
    );
}

#[test]
fn next_due_refuses_unverified_shapes_and_clamping() {
    for r in [
        "FREQ=WEEKLY",
        "FREQ=DAILY",
        "FREQ=MONTHLY;COUNT=3",
        "FREQ=MONTHLY;UNTIL=20271231",
        "FREQ=YEARLY;BYMONTH=11",
        "FREQ=MONTHLY;BYDAY=1MO",
        "FREQ=MONTHLY;WKST=MO",
    ] {
        let rule = parse_rrule(r).unwrap();
        let err = next_due(date(2026, 12, 10), &rule).unwrap_err();
        assert!(
            unsupported(&err).starts_with("repeat rule shape not verified: "),
            "{r}: {err:?}"
        );
    }
    // Day 31 / Feb 29 would clamp: a bug, not a result.
    let monthly = parse_rrule("FREQ=MONTHLY").unwrap();
    assert!(next_due(date(2027, 1, 31), &monthly).is_err());
    let yearly = parse_rrule("FREQ=YEARLY").unwrap();
    assert!(next_due(date(2028, 2, 29), &yearly).is_err());
}

const PLAIN: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:synthetic-1\r\nSUMMARY:Synthetic\r\nSTATUS:NEEDS-ACTION\r\nCREATED:20261101T100000Z\r\nDUE;VALUE=DATE:20261125\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";

#[test]
fn one_time_state_and_rrule_checks() {
    // Reopen of an open task.
    let mut cal = parse(PLAIN.as_bytes()).unwrap();
    let before = cal.clone();
    assert!(unsupported(&reopen_one_time(&mut cal, now()).unwrap_err()).contains("not completed"));
    assert_eq!(cal, before);

    // Complete twice.
    complete_one_time(&mut cal, now()).unwrap();
    let done = cal.clone();
    let err = complete_one_time(&mut cal, now()).unwrap_err();
    assert!(unsupported(&err).contains("already completed"), "{err:?}");
    assert_eq!(cal, done);

    // Repeating task.
    let rep = PLAIN.replace("END:VTODO", "RRULE:FREQ=MONTHLY\r\nEND:VTODO");
    let mut cal = parse(rep.as_bytes()).unwrap();
    let before = cal.clone();
    let err = complete_one_time(&mut cal, now()).unwrap_err();
    assert_eq!(
        unsupported(&err),
        "use complete_recurring for repeating tasks"
    );
    assert_eq!(cal, before);
}

#[test]
fn reopen_inserts_percent_after_status_when_absent() {
    let s = PLAIN.replace(
        "STATUS:NEEDS-ACTION\r\n",
        "STATUS:COMPLETED\r\nCOMPLETED:20261101T120000Z\r\n",
    );
    let mut cal = parse(s.as_bytes()).unwrap();
    reopen_one_time(&mut cal, now()).unwrap();
    let out = String::from_utf8(serialize(&cal)).unwrap();
    assert!(
        out.contains("\r\nSTATUS:NEEDS-ACTION\r\nPERCENT-COMPLETE:0\r\nCREATED:"),
        "{out}"
    );
    assert!(!out.contains("\r\nCOMPLETED:"), "{out}");
}

/// The monthly fixture with `f` applied to its text.
fn monthly_with(f: impl Fn(String) -> String) -> Calendar {
    let s = String::from_utf8(fixture("rule-months-day-1-28", "01-create", MONTHLY)).unwrap();
    parse(f(s).as_bytes()).unwrap()
}

fn set_meta(cal: &mut Calendar, key: &str, v: i64) {
    let t = cal.root.children_named_mut("VTODO").next().unwrap();
    let p = t.prop_mut("X-2DOAPP-METADATA").unwrap();
    let mut m = MetaJson::parse(&p.value).unwrap();
    m.set_i64(key, v);
    p.set_value(&m.encode());
}

#[test]
fn complete_recurring_refuses_unverified_cases() {
    let cases: Vec<(&str, Calendar)> = vec![
        ("no RRULE", parse(PLAIN.as_bytes()).unwrap()),
        (
            "BYMONTHDAY",
            monthly_with(|s| s.replace("RRULE:FREQ=MONTHLY", "RRULE:FREQ=MONTHLY;BYMONTHDAY=10")),
        ),
        (
            "UTC due time",
            monthly_with(|s| {
                s.replace("DTSTART;VALUE=DATE:20261110", "DTSTART:20261110T090000Z")
                    .replace("DUE;VALUE=DATE:20261110", "DUE:20261110T090000Z")
            }),
        ),
        (
            "own DTSTART",
            monthly_with(|s| {
                s.replace("DTSTART;VALUE=DATE:20261110", "DTSTART;VALUE=DATE:20261105")
            }),
        ),
        (
            "no metadata",
            monthly_with(|s| {
                let start = s.find("X-2DOAPP-METADATA").unwrap();
                let end = s.find("X-2DOAPP-SYNC-IMAGE").unwrap();
                format!("{}{}", &s[..start], &s[end..])
            }),
        ),
        (
            "no DUE",
            monthly_with(|s| s.replace("DUE;VALUE=DATE:20261110\n", "")),
        ),
        (
            "completed",
            monthly_with(|s| s.replace("STATUS:NEEDS-ACTION", "STATUS:COMPLETED")),
        ),
        ("from completion", {
            let mut c = monthly_with(|s| s);
            set_meta(&mut c, "RecurrenceFrom", 2);
            c
        }),
        ("type mismatch", {
            let mut c = monthly_with(|s| s);
            set_meta(&mut c, "RecurrenceType", 259);
            c
        }),
        ("value mismatch", {
            let mut c = monthly_with(|s| s);
            set_meta(&mut c, "RecurrenceValue", 3);
            c
        }),
        ("copy uid = uid", monthly_with(|s| s)),
    ];
    for (name, cal) in cases {
        let copy_uid = if name == "copy uid = uid" {
            MONTHLY
        } else {
            MONTHLY_COPY
        };
        let before = cal.clone();
        let Err(err) = complete_recurring(&cal, now(), copy_uid) else {
            panic!("{name}: accepted");
        };
        assert_eq!(cal, before, "{name}");
        assert!(matches!(err, EditError::Unsupported(_)), "{name}: {err:?}");
    }
}

#[test]
fn copy_without_ruid_gets_original_uid() {
    let mut cal = monthly_with(|s| s);
    {
        let t = cal.root.children_named_mut("VTODO").next().unwrap();
        let p = t.prop_mut("X-2DOAPP-METADATA").unwrap();
        let mut m = MetaJson::parse(&p.value).unwrap();
        m.remove("RUID");
        p.set_value(&m.encode());
    }
    let (_, copy) = complete_recurring(&cal, now(), MONTHLY_COPY).unwrap();
    let m = MetaJson::parse(&todo(&copy).prop("X-2DOAPP-METADATA").unwrap().value).unwrap();
    assert_eq!(m.get_str("RUID"), Some(MONTHLY));
    assert!(todo(&copy).prop("RRULE").is_none());
}

const META_ALL: &[&str] = &["DTSTART", "DUE", "X-2DOAPP-METADATA"];

#[test]
fn missed_monthly_advances_one_interval() {
    let d = &["DTSTART", "DUE"];
    check_copy_and_advance(
        P6,
        "02-edit",
        "03-complete-child",
        L,
        L_COPY1,
        "20260915",
        d,
    );
    check_copy_and_advance(
        P6,
        "03-complete-child",
        "04-reopen-child",
        L,
        L_COPY2,
        "20261015",
        d,
    );
}

#[test]
fn monthly_with_start_shifts_start_date() {
    check_copy_and_advance(
        P6,
        "02-edit",
        "03-complete-child",
        S,
        S_COPY,
        "20270105",
        META_ALL,
    );
}

#[test]
fn timed_monthly_with_start_and_alert_matches_2do() {
    check_copy_and_advance(
        P6,
        "02-edit",
        "03-complete-child",
        T,
        T_COPY,
        "20270120T090000",
        META_ALL,
    );
}

#[test]
fn timed_yearly_with_start_and_alert_matches_2do() {
    check_copy_and_advance(
        P6,
        "02-edit",
        "03-complete-child",
        Y,
        Y_COPY,
        "20280228T090000",
        META_ALL,
    );
}

fn p6_text(uid: &str) -> String {
    String::from_utf8(fixture(P6, "02-edit", uid)).unwrap()
}

#[test]
fn zoned_due_time_is_refused() {
    let t = p6_text(T);
    for (due, start) in [
        (
            "DUE;TZID=Europe/Warsaw:20261220T090000",
            "DTSTART;TZID=Europe/Warsaw:20261220T090000",
        ),
        ("DUE:20261220T080000Z", "DTSTART:20261220T080000Z"),
    ] {
        let s = t
            .replace("DUE:20261220T090000", due)
            .replace("DTSTART:20261220T090000", start);
        let cal = parse(s.as_bytes()).unwrap();
        let err = complete_recurring(&cal, now(), T_COPY).unwrap_err();
        assert_eq!(
            unsupported(&err),
            "completing a repeat with a zoned due time is not verified",
            "{due}"
        );
    }
}

#[test]
fn start_on_day_31_is_refused() {
    // StartDate 2026-10-31 (UTC midnight), due 2026-11-05.
    let s = p6_text(S)
        .replace(
            "%22StartDate%22%3A1796083200",
            "%22StartDate%22%3A1793404800",
        )
        .replace("20261205", "20261105");
    let cal = parse(s.as_bytes()).unwrap();
    let err = complete_recurring(&cal, now(), S_COPY).unwrap_err();
    assert_eq!(
        unsupported(&err),
        "next start date after 2026-10-31 would not fall on day 31 of the month"
    );
}

const R: &str = "8f603cfaa41f4c229b9adcd6223e7393";
const R_COPY: &str = "14b5a37a52224e8c959b5002e2275fc9";

fn json_uid(cal: &Calendar) -> Option<String> {
    let m = MetaJson::parse(&todo(cal).prop("X-2DOAPP-METADATA").unwrap().value).unwrap();
    m.get_str("uid").map(str::to_string)
}

#[test]
fn recurring_project_copy_rewrites_json_uid() {
    check_copy_and_advance(
        P6,
        "04-reopen-child",
        "05-complete-parent",
        R,
        R_COPY,
        "20270115",
        &["DTSTART", "DUE"],
    );
    let input = parse(&fixture(P6, "04-reopen-child", R)).unwrap();
    assert_eq!(json_uid(&input).as_deref(), Some(R), "fixture carries uid");
    let (live, copy) = complete_recurring(&input, now(), R_COPY).unwrap();
    assert_eq!(json_uid(&live).as_deref(), Some(R));
    assert_eq!(json_uid(&copy).as_deref(), Some(R_COPY));
}

#[test]
fn foreign_json_uid_is_kept_in_the_copy() {
    let mut cal = parse(&fixture(P6, "04-reopen-child", R)).unwrap();
    {
        let t = cal.root.children_named_mut("VTODO").next().unwrap();
        let p = t.prop_mut("X-2DOAPP-METADATA").unwrap();
        let mut m = MetaJson::parse(&p.value).unwrap();
        m.set_str("uid", "someone-else");
        p.set_value(&m.encode());
    }
    let (_, copy) = complete_recurring(&cal, now(), R_COPY).unwrap();
    assert_eq!(json_uid(&copy).as_deref(), Some("someone-else"));
}
