mod common;

use twydo_core::config::Config;
use twydo_core::edit::{Change, Op};
use twydo_core::model::TaskKind;
use twydo_daemon::protocol::wire::{from_wire, to_wire};

#[test]
fn wire_round_trip_keeps_what_views_need() {
    let fx = common::Fixture::new(&[
        ("inbox", "p1", "Synthetic project", None),
        ("inbox", "c1", "Synthetic item one", Some("p1")),
        ("inbox", "c2", "Synthetic item two", Some("p1")),
    ]);
    let snap = fx.snapshot().with_pending(["c1".to_string()].into());
    let back = from_wire(
        serde_json::from_str(&serde_json::to_string(&to_wire(&snap, 7)).unwrap()).unwrap(),
    );
    assert_eq!(
        back.config,
        fx.config(),
        "the client's configuration comes from the snapshot"
    );
    assert_eq!(back.tasks.len(), 3);
    let p = back.find("p1").unwrap();
    assert_eq!(
        back.children(p).len(),
        2,
        "parent links resolve on the client"
    );
    assert_eq!(back.tasks[p].kind, TaskKind::Project);
    assert!(back.is_pending(back.find("c1").unwrap()));
    assert_eq!(back.tasks[p].path, fx.vdir().join("inbox/p1.ics"));
    assert_eq!(back.rev(p), snap.rev(snap.find("p1").unwrap()));
}

#[test]
fn wire_paths_are_vdir_relative_and_carry_no_bytes() {
    let fx = common::Fixture::new(&[("inbox", "u1", "Synthetic", None)]);
    let json = serde_json::to_value(to_wire(&fx.snapshot(), 1)).unwrap();
    assert_eq!(json["tasks"][0]["path"], "inbox/u1.ics");
    assert!(!json.to_string().contains("BEGIN:VCALENDAR"));
    assert_eq!(json["tasks"][0]["rev"].as_str().unwrap().len(), 16);
}

#[test]
fn a_full_config_round_trips() {
    // every table set: lists prefs, groups, writes, sync, week_start, work_days, morning_time
    let cfg = common::full_config();
    let back: Config = serde_json::from_value(serde_json::to_value(&cfg).unwrap()).unwrap();
    assert_eq!(back, cfg);
}

#[test]
fn queued_op_encoding_is_unchanged() {
    // Queue files written before this change must still read.
    let op = Op::Edit {
        uid: "u1".into(),
        changes: vec![Change::Summary("X".into())],
    };
    assert_eq!(
        serde_json::to_string(&op).unwrap(),
        r#"{"Edit":{"uid":"u1","changes":[{"Summary":"X"}]}}"#
    );
}

const ALARM_ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\nUID:alarm\r\nSUMMARY:With alarm\r\nDUE:20261114T093000Z\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:x\r\nTRIGGER;VALUE=DATE-TIME:20261114T083000Z\r\nEND:VALARM\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
const RRULE_ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\nUID:until\r\nSUMMARY:Repeats\r\nDUE;VALUE=DATE:20261114\r\nRRULE:FREQ=WEEKLY;UNTIL=20271231T000000Z\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
const ZONED_ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\nUID:zoned\r\nSUMMARY:Zoned\r\nDUE;TZID=Europe/Berlin:20261114T093000\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
const DATE_ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\nUID:dateonly\r\nSUMMARY:Date only\r\nDUE;VALUE=DATE:20261114\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";

#[test]
fn wire_round_trip_keeps_alarms_rrule_and_zoned_due() {
    let fx = common::Fixture::new(&[]);
    fx.write("inbox/alarm.ics", ALARM_ICS);
    fx.write("inbox/until.ics", RRULE_ICS);
    fx.write("inbox/zoned.ics", ZONED_ICS);
    fx.write("inbox/dateonly.ics", DATE_ICS);
    let snap = fx.snapshot();
    assert_eq!(snap.tasks.len(), 4, "{:?}", snap.errors);
    let back = from_wire(
        serde_json::from_str(&serde_json::to_string(&to_wire(&snap, 3)).unwrap()).unwrap(),
    );
    for t in &snap.tasks {
        let b = &back.tasks[back.find(&t.uid).unwrap()];
        let mut want = t.clone();
        want.path = fx.vdir().join(&t.path);
        assert_eq!(b, &want, "task {} changed on the wire", t.uid);
        let i = back.find(&t.uid).unwrap();
        assert_eq!(
            back.rev(i),
            snap.rev(snap.find(&t.uid).unwrap()),
            "revision of {}",
            t.uid
        );
    }
    let alarm = &back.tasks[back.find("alarm").unwrap()];
    assert!(!alarm.alarms.is_empty(), "{alarm:?}");
    let until = &back.tasks[back.find("until").unwrap()];
    assert!(
        until.rrule.as_deref().is_some_and(|r| r.contains("UNTIL")),
        "{until:?}"
    );
}

#[test]
fn an_empty_wire_rev_stays_unknown() {
    let fx = common::Fixture::new(&[("inbox", "u1", "Synthetic", None)]);
    let mut wire = to_wire(&fx.snapshot(), 1);
    assert_eq!(wire.tasks[0].rev.len(), 16);
    wire.tasks[0].rev = String::new();
    let back = from_wire(wire);
    assert_eq!(back.rev(back.find("u1").unwrap()), None);
}

#[test]
fn wire_config_never_carries_the_daemon_table() {
    let fx = common::Fixture::new(&[("inbox", "u1", "Synthetic", None)]);
    let mut snap = fx.snapshot();
    snap.config.daemon.unit_warning = false;
    let wire = to_wire(&snap, 1);
    let json = serde_json::to_value(&wire.config).unwrap();
    assert!(json.get("daemon").is_none(), "{json}");
    assert!(!wire.unit_warning);
    let back = from_wire(serde_json::from_str(&serde_json::to_string(&wire).unwrap()).unwrap());
    assert!(!back.config.daemon.unit_warning);
}

#[test]
fn a_snapshot_without_unit_warning_decodes_as_true() {
    let fx = common::Fixture::new(&[("inbox", "u1", "Synthetic", None)]);
    let mut json = serde_json::to_value(to_wire(&fx.snapshot(), 1)).unwrap();
    json.as_object_mut()
        .unwrap()
        .remove("unit_warning")
        .unwrap();
    let wire: twydo_daemon::protocol::wire::WireSnapshot = serde_json::from_value(json).unwrap();
    assert!(wire.unit_warning);
    assert!(from_wire(wire).config.daemon.unit_warning);
}
