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
