//! Reminder trigger instants and eligibility. All content is synthetic.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, NaiveDateTime, NaiveTime, Utc, Weekday};
use chrono_tz::Tz;
use twydo_core::config::{Config, ListPrefs};
use twydo_core::ical::parse;
use twydo_core::model::{Task, tasks_from_calendar};
use twydo_core::remind::{Cause, Plan, Sent, Settings, Timed, eligible, plan, timed};
use twydo_core::vdir::{Collection, LoadReport};
use twydo_core::views::Snapshot;

const WARSAW: Tz = chrono_tz::Europe::Warsaw;

fn task_in(list: &str, uid: &str, body: &str) -> Task {
    let ics = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//t//t//EN\r\nBEGIN:VTODO\r\nUID:{uid}\r\n\
         SUMMARY:{uid}\r\n{body}END:VTODO\r\nEND:VCALENDAR\r\n"
    );
    let cal = parse(ics.as_bytes()).unwrap();
    tasks_from_calendar(&cal, list, Path::new(&format!("{list}/{uid}.ics")))
        .pop()
        .unwrap()
}

fn task(body: &str) -> Task {
    task_in("l", "t", body)
}

fn alarm(trigger: &str) -> String {
    format!("BEGIN:VALARM\r\nACTION:DISPLAY\r\n{trigger}\r\nEND:VALARM\r\n")
}

fn utc(s: &str) -> DateTime<Utc> {
    NaiveDateTime::parse_from_str(s, "%Y%m%dT%H%M%SZ")
        .unwrap()
        .and_utc()
}

fn entry(at: &str, causes: Vec<Cause>) -> Timed {
    Timed {
        at: utc(at),
        causes,
    }
}

#[test]
fn due_time_and_auto_alert_coalesce() {
    let t = task(&format!(
        "DUE:20261116T143000\r\n{}",
        alarm("TRIGGER;RELATED=END:PT0S")
    ));
    assert_eq!(
        timed(&t, WARSAW),
        vec![entry(
            "20261116T133000Z",
            vec![Cause::Due, Cause::Alert { index: 0 }]
        )]
    );
}

#[test]
fn alerts_at_other_times_stay_separate() {
    let t = task(&format!(
        "DUE:20261116T143000\r\n{}{}",
        alarm("TRIGGER;RELATED=END:PT0S"),
        alarm("TRIGGER;RELATED=END:-PT15M")
    ));
    assert_eq!(
        timed(&t, WARSAW),
        vec![
            entry("20261116T131500Z", vec![Cause::Alert { index: 1 }]),
            entry(
                "20261116T133000Z",
                vec![Cause::Due, Cause::Alert { index: 0 }]
            ),
        ]
    );
}

#[test]
fn relative_alert_uses_start_when_not_related_to_end() {
    let t = task(&format!(
        "DTSTART:20261116T090000\r\nDUE:20261116T170000\r\n{}",
        alarm("TRIGGER:PT0S")
    ));
    let got = timed(&t, WARSAW);
    assert_eq!(got.len(), 2);
    assert_eq!(
        got[0],
        entry("20261116T080000Z", vec![Cause::Alert { index: 0 }])
    );
    assert_eq!(got[1], entry("20261116T160000Z", vec![Cause::Due]));
}

#[test]
fn relative_alert_without_start_uses_due() {
    let t = task(&format!(
        "DUE:20261116T170000\r\n{}",
        alarm("TRIGGER:-PT1H")
    ));
    assert_eq!(
        timed(&t, WARSAW)[0],
        entry("20261116T150000Z", vec![Cause::Alert { index: 0 }])
    );
}

#[test]
fn relative_alert_on_date_only_due_counts_from_local_midnight() {
    let t = task(&format!(
        "DUE;VALUE=DATE:20261116\r\n{}",
        alarm("TRIGGER;RELATED=END:PT9H")
    ));
    assert_eq!(
        timed(&t, WARSAW),
        vec![entry("20261116T080000Z", vec![Cause::Alert { index: 0 }])]
    );
}

#[test]
fn absolute_alert_without_dates() {
    let t = task(&alarm("TRIGGER;VALUE=DATE-TIME:20261116T100000Z"));
    assert_eq!(
        timed(&t, WARSAW),
        vec![entry("20261116T100000Z", vec![Cause::Alert { index: 0 }])]
    );
}

#[test]
fn relative_alert_without_anchor_yields_nothing() {
    let t = task(&alarm("TRIGGER:-PT15M"));
    assert!(timed(&t, WARSAW).is_empty());
}

#[test]
fn zoned_and_utc_due() {
    let t = task("DUE;TZID=America/New_York:20261116T090000\r\n");
    assert_eq!(
        timed(&t, WARSAW),
        vec![entry("20261116T140000Z", vec![Cause::Due])]
    );
    let t = task("DUE:20261116T090000Z\r\n");
    assert_eq!(
        timed(&t, WARSAW),
        vec![entry("20261116T090000Z", vec![Cause::Due])]
    );
}

#[test]
fn dst_gap_local_midnight() {
    let havana: Tz = chrono_tz::America::Havana;
    let t = task(&format!(
        "DUE;VALUE=DATE:20260308\r\n{}",
        alarm("TRIGGER;RELATED=END:PT0S")
    ));
    // Midnight does not exist; it falls forward to 01:00 CDT (UTC-4).
    assert_eq!(
        timed(&t, havana),
        vec![entry("20260308T050000Z", vec![Cause::Alert { index: 0 }])]
    );
}

fn config(lists: &[(&str, ListPrefs)]) -> Config {
    Config {
        vdir: PathBuf::from("/nonexistent"),
        timezone: Some("Europe/Warsaw".to_string()),
        work_days: vec![Weekday::Mon],
        week_start: Weekday::Mon,
        morning_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        default_list: None,
        lists: lists
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect::<BTreeMap<_, _>>(),
        groups: vec![],
        writes: Default::default(),
        state_dir: None,
        sync: None,
        daemon: Default::default(),
        notifications: Default::default(),
    }
}

#[test]
fn eligibility() {
    let hidden = ListPrefs {
        hidden: true,
        ..ListPrefs::default()
    };
    let archived = ListPrefs {
        archived: true,
        ..ListPrefs::default()
    };
    let tasks = vec![
        task_in("open", "open", ""),
        task_in("open", "done", "STATUS:COMPLETED\r\n"),
        task_in("open", "cancelled", "STATUS:CANCELLED\r\n"),
        task_in("hid", "hid", ""),
        task_in("arc", "arc", ""),
        task_in("open", "list", ""),
        task_in("open", "item", "RELATED-TO;RELTYPE=PARENT:list\r\n"),
        task_in("open", "proj", ""),
        task_in("open", "child", "RELATED-TO;RELTYPE=PARENT:proj\r\n"),
    ];
    let mut tasks = tasks;
    tasks[5].kind = twydo_core::model::TaskKind::Checklist;
    tasks[7].kind = twydo_core::model::TaskKind::Project;
    let mut ids: Vec<String> = tasks.iter().map(|t| t.collection.clone()).collect();
    ids.sort();
    ids.dedup();
    let collections = ids
        .into_iter()
        .map(|id| Collection {
            path: PathBuf::from(&id),
            id,
            display_name: None,
            color: None,
        })
        .collect();
    let snap = Snapshot::new(
        LoadReport {
            collections,
            tasks,
            ..Default::default()
        },
        config(&[("hid", hidden), ("arc", archived)]),
    );
    let ok = |uid: &str| eligible(&snap, snap.find(uid).unwrap());
    assert!(ok("open"));
    assert!(!ok("done"));
    assert!(!ok("cancelled"));
    assert!(!ok("hid"));
    assert!(!ok("arc"));
    assert!(ok("list"));
    assert!(!ok("item"));
    assert!(ok("child"));
}

#[test]
fn alerts_at_one_instant_coalesce_with_both_indices() {
    let t = task(&format!(
        "DUE:20261116T143000\r\n{}{}",
        alarm("TRIGGER;RELATED=END:-PT15M"),
        alarm("TRIGGER;VALUE=DATE-TIME:20261116T131500Z")
    ));
    assert_eq!(
        timed(&t, WARSAW)[0],
        entry(
            "20261116T131500Z",
            vec![Cause::Alert { index: 0 }, Cause::Alert { index: 1 }]
        )
    );
}

#[test]
fn date_only_due_without_alert_yields_no_triggers() {
    let t = task("DUE;VALUE=DATE:20261116\r\n");
    assert!(timed(&t, WARSAW).is_empty());
}

// ---- plan ----

fn settings() -> Settings {
    Settings {
        tz: WARSAW,
        morning: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        catch_up: Duration::hours(24),
    }
}

fn with_title(mut t: Task, title: &str) -> Task {
    t.summary = title.to_string();
    t
}

fn snap_of(tasks: Vec<Task>) -> Snapshot {
    snap_with(tasks, &[])
}

fn snap_with(tasks: Vec<Task>, lists: &[(&str, ListPrefs)]) -> Snapshot {
    let mut ids: Vec<String> = tasks.iter().map(|t| t.collection.clone()).collect();
    ids.push("c1".to_string());
    ids.sort();
    ids.dedup();
    let collections = ids
        .into_iter()
        .map(|id| Collection {
            path: PathBuf::from(&id),
            display_name: (id == "c1").then(|| "Home".to_string()),
            id,
            color: None,
        })
        .collect();
    Snapshot::new(
        LoadReport {
            collections,
            tasks,
            ..Default::default()
        },
        config(lists),
    )
}

fn home(uid: &str, body: &str) -> Task {
    task_in("c1", uid, body)
}

fn no_sent() -> Sent {
    Sent::new()
}

fn run(snap: &Snapshot, sent: &Sent, now: &str) -> Plan {
    plan(snap, sent, utc(now), &settings())
}

const AUTO: &str = "TRIGGER;RELATED=END:PT0S";

#[test]
fn delivers_a_trigger_at_its_instant() {
    let t = with_title(
        home("u1", &format!("DUE:20261116T143000\r\n{}", alarm(AUTO))),
        "Call",
    );
    let p = run(&snap_of(vec![t]), &no_sent(), "20261116T133000Z");
    assert_eq!(p.due.len(), 1);
    let d = &p.due[0];
    assert_eq!(d.key, "t:u1@2026-11-16T13:30:00Z");
    assert_eq!(d.uid.as_deref(), Some("u1"));
    assert_eq!(d.title, "Call");
    assert_eq!(d.body, "Due Today 14:30 · Home");
}

#[test]
fn alert_before_due_names_the_alert() {
    let t = home(
        "u1",
        &format!(
            "DUE:20261116T143000\r\n{}",
            alarm("TRIGGER;RELATED=END:-PT15M")
        ),
    );
    let p = run(&snap_of(vec![t]), &no_sent(), "20261116T131500Z");
    assert_eq!(p.due[0].body, "15 min before due · Due Today 14:30 · Home");
}

#[test]
fn absolute_alert_without_due_shows_only_the_list() {
    let t = home("u1", &alarm("TRIGGER;VALUE=DATE-TIME:20261116T133000Z"));
    let p = run(&snap_of(vec![t]), &no_sent(), "20261116T133000Z");
    assert_eq!(p.due[0].body, "Home");
}

#[test]
fn body_falls_back_to_the_list_id() {
    let t = task_in(
        "c2",
        "u1",
        &alarm("TRIGGER;VALUE=DATE-TIME:20261116T133000Z"),
    );
    let p = run(&snap_of(vec![t]), &no_sent(), "20261116T133000Z");
    assert_eq!(p.due[0].body, "c2");
}

#[test]
fn sent_key_is_not_delivered_again() {
    let t = home("u1", &format!("DUE:20261116T143000\r\n{}", alarm(AUTO)));
    let mut sent = Sent::new();
    sent.insert(
        "t:u1@2026-11-16T13:30:00Z".to_string(),
        utc("20261116T133000Z"),
    );
    let p = run(&snap_of(vec![t]), &sent, "20261116T133500Z");
    assert!(p.due.iter().all(|d| d.uid.is_none()), "{:?}", p.due);
}

#[test]
fn catch_up_window_edges() {
    let t = home("u1", &alarm("TRIGGER;VALUE=DATE-TIME:20261115T133000Z"));
    let snap = snap_of(vec![t]);
    assert!(run(&snap, &no_sent(), "20261116T133000Z").due.is_empty());
    let p = run(&snap, &no_sent(), "20261116T132959Z");
    assert_eq!(p.due.len(), 1);
}

#[test]
fn huge_catch_up_does_not_panic() {
    let t = home("u1", &alarm("TRIGGER;VALUE=DATE-TIME:20261110T133000Z"));
    let snap = snap_of(vec![t]);
    let mut s = settings();
    s.catch_up = Duration::hours(i64::from(u32::MAX));
    let p = plan(&snap, &no_sent(), utc("20261116T133000Z"), &s);
    assert_eq!(
        p.due.len(),
        1,
        "no lower bound: the old trigger still fires"
    );
}

#[test]
fn not_yet_due_is_next_wake() {
    let t = home("u1", &alarm("TRIGGER;VALUE=DATE-TIME:20261116T140000Z"));
    let p = run(&snap_of(vec![t]), &no_sent(), "20261116T133000Z");
    assert!(p.due.is_empty());
    assert_eq!(p.next_wake, utc("20261116T140000Z"));
}

#[test]
fn next_wake_is_the_digest_when_nothing_else() {
    let snap = snap_of(vec![]);
    assert_eq!(
        run(&snap, &no_sent(), "20261116T060000Z").next_wake,
        utc("20261116T080000Z")
    );
    assert_eq!(
        run(&snap, &no_sent(), "20261116T133000Z").next_wake,
        utc("20261117T080000Z")
    );
}

#[test]
fn moved_due_notifies_again() {
    let mut sent = Sent::new();
    sent.insert(
        "t:u1@2026-11-16T13:30:00Z".to_string(),
        utc("20261116T133000Z"),
    );
    let t = home("u1", &format!("DUE:20261116T153000\r\n{}", alarm(AUTO)));
    let p = run(&snap_of(vec![t]), &sent, "20261116T143000Z");
    assert_eq!(p.due.len(), 1);
    assert_eq!(p.due[0].key, "t:u1@2026-11-16T14:30:00Z");
}

#[test]
fn ineligible_tasks_are_silent() {
    let fire = alarm("TRIGGER;VALUE=DATE-TIME:20261116T133000Z");
    let hidden = ListPrefs {
        hidden: true,
        ..ListPrefs::default()
    };
    let archived = ListPrefs {
        archived: true,
        ..ListPrefs::default()
    };
    let mut list = task_in("c1", "list", "");
    list.kind = twydo_core::model::TaskKind::Checklist;
    let tasks = vec![
        task_in("c1", "done", &format!("STATUS:COMPLETED\r\n{fire}")),
        task_in("hid", "hid", &fire),
        task_in("arc", "arc", &fire),
        list,
        task_in(
            "c1",
            "item",
            &format!("RELATED-TO;RELTYPE=PARENT:list\r\n{fire}"),
        ),
    ];
    let snap = snap_with(tasks, &[("hid", hidden), ("arc", archived)]);
    let p = run(&snap, &no_sent(), "20261116T133000Z");
    assert!(p.due.is_empty(), "{:?}", p.due);
}

fn date_task(uid: &str, date: &str) -> Task {
    home(uid, &format!("DUE;VALUE=DATE:{date}\r\n"))
}

#[test]
fn digest_lists_due_today_and_overdue() {
    let tasks = vec![
        with_title(date_task("a", "20261116"), "Beta"),
        with_title(date_task("b", "20261116"), "Alpha"),
        with_title(date_task("c", "20261114"), "Old"),
    ];
    let p = run(&snap_of(tasks), &no_sent(), "20261116T080000Z");
    let d = p.due.last().unwrap();
    assert_eq!(d.key, "d:2026-11-16");
    assert_eq!(d.uid, None);
    assert_eq!(d.title, "2 due today, 1 overdue");
    assert_eq!(d.body, "Alpha\nBeta\nOld");
}

#[test]
fn digest_only_once_a_day() {
    let snap = snap_of(vec![date_task("a", "20261116")]);
    let mut sent = Sent::new();
    sent.insert("d:2026-11-16".to_string(), utc("20261116T080000Z"));
    assert!(run(&snap, &sent, "20261116T100000Z").due.is_empty());
}

#[test]
fn digest_late_when_missed() {
    let snap = snap_of(vec![date_task("a", "20261116")]);
    let p = run(&snap, &no_sent(), "20261116T150000Z");
    assert_eq!(p.due[0].key, "d:2026-11-16");
}

#[test]
fn digest_not_before_morning() {
    let snap = snap_of(vec![date_task("a", "20261116")]);
    let p = run(&snap, &no_sent(), "20261116T075900Z");
    assert!(p.due.is_empty());
    assert_eq!(p.next_wake, utc("20261116T080000Z"));
}

#[test]
fn digest_omitted_when_empty() {
    let snap = snap_of(vec![date_task("a", "20261120")]);
    assert!(run(&snap, &no_sent(), "20261116T100000Z").due.is_empty());
}

#[test]
fn digest_title_omits_empty_half() {
    let today = snap_of(vec![date_task("a", "20261116")]);
    assert_eq!(
        run(&today, &no_sent(), "20261116T100000Z").due[0].title,
        "1 due today"
    );
    let late = snap_of(vec![date_task("a", "20261110")]);
    assert_eq!(
        run(&late, &no_sent(), "20261116T100000Z").due[0].title,
        "1 overdue"
    );
}

#[test]
fn digest_caps_at_five() {
    let tasks = (1..=7)
        .map(|i| with_title(date_task(&format!("u{i}"), "20261116"), &format!("T{i}")))
        .collect();
    let p = run(&snap_of(tasks), &no_sent(), "20261116T100000Z");
    assert_eq!(p.due[0].body, "T1\nT2\nT3\nT4\nT5\n+2 more");
}

#[test]
fn digest_counts_a_timed_due_before_today_as_overdue() {
    let t = home("a", "DUE:20261115T100000\r\n");
    let p = run(&snap_of(vec![t]), &no_sent(), "20261116T133000Z");
    let d = p.due.last().unwrap();
    assert_eq!(d.title, "1 overdue");
}

#[test]
fn digest_counts_a_timed_due_before_the_morning_as_overdue() {
    let t = home("a", "DUE:20261116T090000\r\n");
    let p = run(&snap_of(vec![t]), &no_sent(), "20261116T080000Z");
    // Due 09:00 local is 08:00Z, not before the morning instant: not overdue.
    assert!(p.due.iter().all(|d| d.uid.is_some()), "{:?}", p.due);
    let t = home("a", "DUE:20261116T080000\r\n");
    let p = run(&snap_of(vec![t]), &no_sent(), "20261116T080000Z");
    let d = p.due.last().unwrap();
    assert_eq!(d.uid, None);
    assert_eq!(d.title, "1 overdue");
}

#[test]
fn empty_digest_is_skipped_once() {
    let snap = snap_of(vec![]);
    let p = run(&snap, &no_sent(), "20261116T080000Z");
    assert!(p.due.is_empty());
    assert_eq!(p.skip, vec!["d:2026-11-16".to_string()]);
    let mut sent = Sent::new();
    sent.insert("d:2026-11-16".to_string(), utc("20261116T080000Z"));
    assert!(run(&snap, &sent, "20261116T080000Z").skip.is_empty());
}

#[test]
fn digest_is_not_skipped_before_morning() {
    let p = run(&snap_of(vec![]), &no_sent(), "20261116T070000Z");
    assert!(p.skip.is_empty());
}

#[test]
fn a_task_overdue_after_the_morning_is_not_in_the_digest() {
    let t = home("u1", "DUE:20261116T143000\r\n");
    let p = run(&snap_of(vec![t]), &no_sent(), "20261116T133000Z");
    assert_eq!(p.due.len(), 1, "{:?}", p.due);
    assert_eq!(p.due[0].key, "t:u1@2026-11-16T13:30:00Z");
    assert!(p.due.iter().all(|d| d.uid.is_some()));
    assert!(p.skip.contains(&"d:2026-11-16".to_string()));
}

#[test]
fn late_digest_ignores_dues_after_the_morning() {
    let old = with_title(date_task("a", "20261114"), "Old");
    let timed = with_title(home("b", "DUE:20261116T100000\r\n"), "Timed");
    let p = run(&snap_of(vec![old, timed]), &no_sent(), "20261116T150000Z");
    let d = p.due.iter().find(|d| d.uid.is_none()).unwrap();
    assert_eq!(d.title, "1 overdue");
    assert!(!d.body.contains("Timed"), "{}", d.body);
}

#[test]
fn empty_title_becomes_untitled() {
    let t = with_title(
        home("u1", &alarm("TRIGGER;VALUE=DATE-TIME:20261116T133000Z")),
        "  ",
    );
    let d = with_title(date_task("u2", "20261116"), "");
    let p = run(&snap_of(vec![t, d]), &no_sent(), "20261116T133000Z");
    assert_eq!(p.due[0].title, "(untitled)");
    assert_eq!(p.due.last().unwrap().body, "(untitled)");
}

#[test]
fn repeating_task_notifies_for_the_next_occurrence() {
    let mut sent = Sent::new();
    sent.insert(
        "t:u1@2026-11-16T13:30:00Z".to_string(),
        utc("20261116T133000Z"),
    );
    let t = home("u1", &format!("DUE:20261117T143000\r\n{}", alarm(AUTO)));
    let snap = snap_of(vec![t]);
    let before = run(&snap, &sent, "20261117T130000Z");
    assert!(before.due.is_empty());
    assert_eq!(before.next_wake, utc("20261117T133000Z"));
    let p = run(&snap, &sent, "20261117T133000Z");
    assert_eq!(p.due[0].key, "t:u1@2026-11-17T13:30:00Z");
}

#[test]
fn absurd_alert_offset_is_dropped_and_others_still_fire() {
    let bad = task_in(
        "l",
        "bad",
        &format!(
            "DUE:20261116T143000\r\n{}{}",
            alarm("TRIGGER;RELATED=END:-P99999999W"),
            alarm("TRIGGER;RELATED=END:PT0S")
        ),
    );
    // Only the due time and the normal alert survive; the overflowing one is dropped.
    assert_eq!(
        timed(&bad, WARSAW),
        vec![entry(
            "20261116T133000Z",
            vec![Cause::Due, Cause::Alert { index: 1 }]
        )]
    );
    let other = task_in(
        "l",
        "other",
        &format!(
            "DUE:20261116T150000\r\n{}",
            alarm("TRIGGER;RELATED=END:PT0S")
        ),
    );
    assert_eq!(timed(&other, WARSAW).len(), 1);
}

/// A8 from the alerts round: 2Do keeps the real start (2026-11-18) only in its JSON metadata and
/// mirrors `DUE` into `DTSTART`, so a relative alert counts from the due time, as on the phone.
#[test]
fn two_do_task_with_start_counts_relative_alert_from_due() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../fixtures/set-alert/phone-encodings/phone-a-batch1/list1/42200c41e91e4e289db9aed8e481d356.ics",
    );
    let cal = parse(&std::fs::read(&path).unwrap()).unwrap();
    let t = tasks_from_calendar(&cal, "list1", &path).pop().unwrap();
    assert!(t.start.is_some(), "the JSON start date is read");
    assert_eq!(
        timed(&t, WARSAW),
        vec![
            entry("20261120T084500Z", vec![Cause::Alert { index: 0 }]),
            entry("20261120T090000Z", vec![Cause::Due]),
        ]
    );
    let tz = WARSAW;
    assert_eq!(
        twydo_core::format::describe_alarm(&t.alarms[0], &t, tz),
        "15 min before due"
    );
}
