//! `tasks.get`: one task and its children for a thin client. Synthetic fixture content.

mod common;

use chrono::{TimeZone, Utc};
use common::{FakeClock, Fixture, TestDaemon, empty, kind};
use serde_json::json;
use twydo_core::conflicts::{ConflictKind, ConflictStore, NewConflict, Side};
use twydo_core::details;
use twydo_core::edit::{Change, Op};
use twydo_core::model::TaskKind;
use twydo_core::store::bind_vdir_state;
use twydo_core::store::fsio::VdirLock;
use twydo_daemon::protocol::methods::DaemonInfo;
use twydo_daemon::protocol::rpc::ErrorKind;
use twydo_daemon::protocol::tasks::TasksGetResult;
use twydo_daemon::protocol::wire::WireSnapshot;

fn ics(uid: &str, summary: &str, extra: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\nUID:{uid}\r\n\
         SUMMARY:{summary}\r\n{extra}END:VTODO\r\nEND:VCALENDAR\r\n"
    )
}

/// 2026-11-17 09:00 UTC.
fn morning() -> std::sync::Arc<FakeClock> {
    FakeClock::at(Utc.with_ymd_and_hms(2026, 11, 17, 9, 0, 0).unwrap())
}

fn fx() -> Fixture {
    let fx = Fixture::new(&[]);
    fx.write(
        "inbox/chk.ics",
        &ics(
            "chk",
            "Pack",
            "X-2DOAPP-METADATA:@tasktype:checklist\r\nDUE:20261117T170000\r\n\
             DESCRIPTION:Bring the form\r\n",
        ),
    );
    for (uid, extra) in [
        ("i1", "RELATED-TO;RELTYPE=PARENT:chk\r\n"),
        (
            "i2",
            "RELATED-TO;RELTYPE=PARENT:chk\r\nDESCRIPTION:Mind the size\r\n",
        ),
        (
            "i3",
            "RELATED-TO;RELTYPE=PARENT:chk\r\nSTATUS:COMPLETED\r\nCOMPLETED:20261116T100000Z\r\n",
        ),
    ] {
        fx.write(
            &format!("inbox/{uid}.ics"),
            &ics(uid, &format!("Item {uid}"), extra),
        );
    }
    fx.write(
        "inbox/rep.ics",
        &ics(
            "rep",
            "Water plants",
            "DUE:20261117T180000\r\nRRULE:FREQ=DAILY\r\n\
             BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\n",
        ),
    );
    fx
}

fn start(fx: &Fixture) -> TestDaemon {
    let mut cfg = fx.config();
    cfg.timezone = Some("UTC".into());
    TestDaemon::start_cfg(fx, cfg, |o| o.clock = morning())
}

fn get(td: &TestDaemon, uid: &str) -> TasksGetResult {
    td.client()
        .call("tasks.get", json!({ "uid": uid }))
        .unwrap()
}

#[test]
fn checklist_with_children() {
    let fx = fx();
    let td = start(&fx);
    let r = get(&td, "chk");
    assert_eq!(r.task.kind, TaskKind::Checklist);
    assert_eq!(r.task.due.as_deref(), Some("Today 17:00"));
    assert!(r.task.due_today);
    assert!(!r.task.overdue);
    assert_eq!(r.task.note.as_deref(), Some("Bring the form"));
    assert!(r.task.parent.is_none());
    let uids: Vec<_> = r.children.iter().map(|c| c.uid.as_str()).collect();
    assert_eq!(uids, ["i1", "i2", "i3"]);
    assert!(r.children[2].completed);
    assert!(!r.children[0].completed);
    assert!(r.children[1].has_note);
    assert!(!r.children[0].has_note);
    let c = td.client();
    let snap: WireSnapshot = c.call("snapshot.get", empty()).unwrap();
    let info: DaemonInfo = c.call("daemon.info", empty()).unwrap();
    assert_eq!(r.revision, snap.revision);
    assert_eq!(r.epoch, info.epoch);
}

#[test]
fn item_names_its_parent() {
    let td = start(&fx());
    let r = get(&td, "i1");
    let p = r.task.parent.expect("parent");
    assert_eq!(p.uid, "chk");
    assert_eq!(p.title, "Pack");
    assert_eq!(p.kind, TaskKind::Checklist);
    assert!(r.children.is_empty());
}

#[test]
fn repeat_and_alerts_worded_like_the_tui() {
    let fx = fx();
    let td = start(&fx);
    let r = get(&td, "rep");
    let snap = fx.snapshot();
    let idx = snap.tasks.iter().position(|t| t.uid == "rep").unwrap();
    let t = &snap.tasks[idx];
    assert!(details::repeat_text(t, snap.tz()).is_some());
    assert_eq!(r.task.repeat, details::repeat_text(t, snap.tz()));
    assert_eq!(r.task.alerts, details::alert_lines(t, snap.tz()));
    assert_eq!(r.task.alerts.len(), 1);
}

#[test]
fn conflict_and_pending_flags() {
    let fx = fx();
    let text = ics("rep", "Water plants", "DUE:20261117T180000\r\n");
    ConflictStore::open(&fx.state())
        .unwrap()
        .record(
            NewConflict {
                uid: "rep".into(),
                collection: "inbox".into(),
                path: "inbox/rep.ics".into(),
                kind: ConflictKind::BothChanged,
                held: false,
                vdir_holds: Side::Remote,
                local: Some(text.clone()),
                remote: Some(text),
            },
            Utc::now(),
        )
        .unwrap();
    let cfg = fx.config();
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let lock = VdirLock::exclusive(&state).unwrap();
    let op = Op::Edit {
        uid: "chk".into(),
        changes: vec![Change::Summary("Pack, queued".into())],
    };
    let at = Utc.with_ymd_and_hms(2026, 11, 9, 12, 0, 0).unwrap();
    common::queue_op(&cfg, &fx.snapshot(), &op, "edit", None, at);
    drop(lock);
    let _sync = VdirLock::shared(&state).unwrap();
    let td = start(&fx);
    assert!(get(&td, "rep").task.conflict);
    assert!(!get(&td, "rep").task.pending);
    let chk = get(&td, "chk");
    assert!(chk.task.pending);
    assert!(!chk.task.conflict);
    assert_eq!(chk.task.title, "Pack, queued");
}

#[test]
fn unknown_uid_is_invalid() {
    let td = start(&fx());
    let err = td
        .client()
        .call::<_, TasksGetResult>("tasks.get", json!({ "uid": "nope" }))
        .unwrap_err();
    assert_eq!(kind(&err), ErrorKind::Invalid);
}
