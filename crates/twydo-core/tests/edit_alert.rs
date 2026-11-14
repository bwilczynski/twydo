//! Custom alert edits against the alerts round (`fixtures/set-alert/phone-encodings`): each
//! edit must produce the bytes 2Do wrote, apart from the two sync stamps.

mod common;

use chrono::{DateTime, NaiveDate, Utc};
use twydo_core::edit::{Change, Due, EditError, apply_changes};
use twydo_core::ical::{Calendar, parse, serialize};
use twydo_core::model::{AlertOffset, CustomAlert, custom_alert, tasks_from_calendar};

const CASE: &str = "set-alert/phone-encodings";
const A1: &str = "ae49d9b7655647d5aa1788c1bff3ac8f";
const A2: &str = "981428f9f4604164ad5acffd30bf54d5";
const A3: &str = "92c11511b21b49288ff0ce7ac11c0313";
const A4: &str = "b1e7bcf4b25c46f3b91c4aff8e812e63";
const A8: &str = "42200c41e91e4e289db9aed8e481d356";

fn phone(state: &str, uid: &str) -> Calendar {
    parse(&common::fixture(CASE, &format!("{state}/list1"), uid)).unwrap()
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-15T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn minutes(n: u32) -> Option<AlertOffset> {
    Some(AlertOffset::new(n).unwrap())
}

/// Every property line in document order, without the two sync stamps: 2Do moves the stamps
/// and refolds lines when it saves, which the other writer tests also leave out.
fn norm(cal: &Calendar) -> Vec<common::Line> {
    common::lines(cal)
        .into_iter()
        .filter(|l| !common::is_stamp(&l.1))
        .collect()
}

/// `cal` with its VALARM's TRIGGER replaced: rebuilds the pre-edit default alert.
fn with_trigger(mut cal: Calendar, trigger: &str) -> Calendar {
    let todo = cal.root.children_named_mut("VTODO").next().unwrap();
    let alarm = todo.children_named_mut("VALARM").next().unwrap();
    alarm.prop_mut("TRIGGER").unwrap().set_value(trigger);
    cal
}

fn without_alarms(mut cal: Calendar) -> Calendar {
    let todo = cal.root.children_named_mut("VTODO").next().unwrap();
    todo.remove_children(|c| c.name == "VALARM");
    cal
}

fn edited(mut cal: Calendar, changes: &[Change]) -> Calendar {
    apply_changes(&mut cal, changes, now()).unwrap();
    cal
}

fn classify(cal: &Calendar) -> CustomAlert {
    custom_alert(cal.root.children_named("VTODO").next().unwrap())
}

#[test]
fn set_alert_matches_phone_bytes_15m_4h_1d() {
    // In place over the default alert, as the user did for A1 and A8.
    for uid in [A1, A8] {
        let want = phone("phone-a-batch1", uid);
        let before = with_trigger(want.clone(), "PT0S");
        assert_eq!(
            classify(&before),
            CustomAlert::AtDue,
            "{uid}: default alert only"
        );
        let got = edited(before, &[Change::Alert(minutes(15))]);
        assert_eq!(norm(&got), norm(&want), "{uid}");
    }
    // A2 kept the default alert's other lines; 2Do's RELDUETIME:0 came from picking an exact
    // time, which Twydo never does, so only the trigger is compared there.
    let a2 = phone("phone-a-batch1", A2);
    let got = edited(
        with_trigger(a2.clone(), "PT0S"),
        &[Change::Alert(minutes(240))],
    );
    let trigger = |c: &Calendar| {
        let todo = c.root.children_named("VTODO").next().unwrap();
        todo.children_named("VALARM")
            .next()
            .unwrap()
            .prop("TRIGGER")
            .unwrap()
            .value
            .clone()
    };
    assert_eq!(trigger(&got), "-PT4H");
    assert_eq!(trigger(&a2), "-PT4H");
    // Added as a new alert where there is none: A3 (timed) and A4 (date-only).
    for uid in [A3, A4] {
        let want = phone("phone-a-batch1", uid);
        let got = edited(
            without_alarms(want.clone()),
            &[Change::Alert(minutes(1440))],
        );
        assert_eq!(norm(&got), norm(&want), "{uid}");
    }
}

#[test]
fn change_offset_matches_phone_bytes() {
    let got = edited(phone("phone-a-batch1", A1), &[Change::Alert(minutes(30))]);
    assert_eq!(norm(&got), norm(&phone("phone-a-batch2", A1)));
}

#[test]
fn remove_alert_matches_phone_bytes() {
    let got = edited(phone("phone-a-batch1", A2), &[Change::Alert(None)]);
    assert_eq!(norm(&got), norm(&phone("phone-a-batch2", A2)));
}

#[test]
fn due_removal_drops_custom_alert_like_the_phone() {
    let got = edited(phone("phone-a-batch1", A3), &[Change::Due(None)]);
    assert_eq!(norm(&got), norm(&phone("phone-a-batch2", A3)));
}

#[test]
fn due_edit_keeps_custom_alert() {
    let before = phone("phone-a-batch2", A1);
    let alarm = |c: &Calendar| {
        let todo = c.root.children_named("VTODO").next().unwrap();
        todo.children_named("VALARM")
            .map(|a| a.prop("TRIGGER").unwrap().value.clone())
            .collect::<Vec<_>>()
    };
    let moved = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
    let later = edited(
        before.clone(),
        &[Change::Due(Some(Due::Time(
            moved.and_hms_opt(10, 0, 0).unwrap(),
        )))],
    );
    assert_eq!(
        alarm(&later),
        ["-PT30M"],
        "no automatic alert added beside it"
    );
    assert_eq!(
        classify(&later),
        CustomAlert::One(AlertOffset::new(30).unwrap())
    );
    // Dropping the time keeps an edited default alert (it carries ISAUTO:1).
    let dated = edited(before, &[Change::Due(Some(Due::Date(moved)))]);
    assert_eq!(alarm(&dated), ["-PT30M"]);
}

#[test]
fn custom_alert_classifies_shapes() {
    let a1 = phone("phone-a-batch1", A1);
    assert_eq!(
        classify(&a1),
        CustomAlert::One(AlertOffset::new(15).unwrap())
    );
    assert_eq!(
        classify(&with_trigger(a1.clone(), "PT0S")),
        CustomAlert::AtDue
    );
    assert_eq!(classify(&phone("phone-a-batch2", A2)), CustomAlert::None);
    assert_eq!(
        classify(&phone("phone-a-batch1", A3)),
        CustomAlert::One(AlertOffset::new(1440).unwrap())
    );
    for trigger in [
        "PT15M",
        ";RELATED=END:-PT15M",
        ";VALUE=DATE-TIME:20261120T090000Z",
    ] {
        let text = String::from_utf8(serialize(&a1)).unwrap().replace(
            "TRIGGER:-PT15M",
            &format!(
                "TRIGGER{}",
                if trigger.starts_with(';') {
                    trigger.to_string()
                } else {
                    format!(":{trigger}")
                }
            ),
        );
        assert_eq!(
            classify(&parse(text.as_bytes()).unwrap()),
            CustomAlert::Unsupported,
            "{trigger}"
        );
    }
    let mut two = a1.clone();
    {
        let todo = two.root.children_named_mut("VTODO").next().unwrap();
        let copy = todo.children_named("VALARM").next().unwrap().clone();
        todo.push_child(copy);
    }
    assert_eq!(classify(&two), CustomAlert::Unsupported, "two alerts");
    let foreign = String::from_utf8(serialize(&without_alarms(a1.clone())))
        .unwrap()
        .replace(
            "END:VTODO",
            "BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VTODO",
        );
    assert_eq!(
        classify(&parse(foreign.as_bytes()).unwrap()),
        CustomAlert::Unsupported,
        "foreign"
    );
    // The parsed task carries the same verdict.
    let task = tasks_from_calendar(&a1, "list1", std::path::Path::new("list1/a1.ics"))
        .pop()
        .unwrap();
    assert_eq!(
        task.custom_alert,
        CustomAlert::One(AlertOffset::new(15).unwrap())
    );
}

#[test]
fn alert_needs_a_due_date_and_editable_alerts() {
    let mut undated = phone("phone-a-batch2", A3);
    assert!(matches!(
        apply_changes(&mut undated, &[Change::Alert(minutes(15))], now()),
        Err(EditError::Unsupported(m)) if m == "an alert needs a due date"
    ));
    let mut two = phone("phone-a-batch1", A1);
    {
        let todo = two.root.children_named_mut("VTODO").next().unwrap();
        let copy = todo.children_named("VALARM").next().unwrap().clone();
        todo.push_child(copy);
    }
    assert!(matches!(
        apply_changes(&mut two, &[Change::Alert(minutes(15))], now()),
        Err(EditError::Unsupported(m)) if m == "this task has alerts Twydo can't edit"
    ));
}

#[test]
fn offsets_use_the_units_2do_writes() {
    for (n, want) in [
        (15, "-PT15M"),
        (90, "-PT1H30M"),
        (240, "-PT4H"),
        (1440, "-P1D"),
        (2880, "-P2D"),
        (10080, "-P7D"),
        (1500, "-P1DT1H"),
    ] {
        let got = edited(phone("phone-a-batch1", A1), &[Change::Alert(minutes(n))]);
        let todo = got.root.children_named("VTODO").next().unwrap();
        let t = todo
            .children_named("VALARM")
            .next()
            .unwrap()
            .prop("TRIGGER")
            .unwrap()
            .value
            .clone();
        assert_eq!(t, want, "{n} min");
        assert_eq!(
            classify(&got),
            CustomAlert::One(AlertOffset::new(n).unwrap())
        );
    }
    assert!(AlertOffset::new(0).is_none());
}

/// Review finding I1: a removed alert stays removed when the due time moves; only a task that
/// gains a due time gets 2Do's automatic alert.
#[test]
fn moving_a_due_time_does_not_restore_a_removed_alert() {
    let removed = phone("phone-a-batch2", A2);
    let moved = NaiveDate::from_ymd_opt(2026, 11, 22).unwrap();
    let got = edited(
        removed,
        &[Change::Due(Some(Due::Time(
            moved.and_hms_opt(9, 0, 0).unwrap(),
        )))],
    );
    let todo = got.root.children_named("VTODO").next().unwrap();
    assert_eq!(todo.children_named("VALARM").count(), 0);
    // A date-only task given a time still gets the automatic alert.
    let dated = without_alarms(phone("phone-a-batch1", A4));
    let got = edited(
        dated,
        &[Change::Due(Some(Due::Time(
            moved.and_hms_opt(9, 0, 0).unwrap(),
        )))],
    );
    let todo = got.root.children_named("VTODO").next().unwrap();
    let alarms: Vec<_> = todo.children_named("VALARM").collect();
    assert_eq!(alarms.len(), 1);
    assert!(twydo_core::model::alert::is_auto_alarm(alarms[0]));
}

/// Review finding 2: the automatic alert can be removed, as on the phone (A2: the VALARM goes).
#[test]
fn removing_the_automatic_alert_matches_the_phone() {
    let auto = with_trigger(phone("phone-a-batch1", A2), "PT0S");
    let got = edited(auto, &[Change::Alert(None)]);
    assert_eq!(norm(&got), norm(&phone("phone-a-batch2", A2)));
    assert_eq!(classify(&got), CustomAlert::None);
}

/// Review finding 5: only a single 2Do-shaped VALARM is editable, so a write never leaves a
/// shape Twydo then reads as unsupported.
#[test]
fn malformed_or_doubled_automatic_alerts_are_unsupported() {
    let auto = with_trigger(phone("phone-a-batch1", A1), "PT0S");
    let mut two = auto.clone();
    {
        let todo = two.root.children_named_mut("VTODO").next().unwrap();
        let copy = todo.children_named("VALARM").next().unwrap().clone();
        todo.push_child(copy);
    }
    assert_eq!(
        classify(&two),
        CustomAlert::Unsupported,
        "two automatic alerts"
    );
    let mut bare = auto;
    {
        let todo = bare.root.children_named_mut("VTODO").next().unwrap();
        let alarm = todo.children_named_mut("VALARM").next().unwrap();
        alarm.remove_props("X-2DOAPP-SOUND");
    }
    assert_eq!(classify(&bare), CustomAlert::Unsupported, "not 2Do-shaped");
    let mut cal = bare;
    assert!(matches!(
        apply_changes(&mut cal, &[Change::Alert(minutes(15))], now()),
        Err(EditError::Unsupported(m)) if m == "this task has alerts Twydo can't edit"
    ));
}
