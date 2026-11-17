//! `views.get`: the Inbox and Today views for thin clients. Synthetic fixture content.

mod common;

use chrono::{NaiveDate, TimeZone, Utc};
use common::{FakeClock, Fixture, TestDaemon, empty, kind};
use serde_json::json;
use twydo_core::conflicts::{ConflictKind, ConflictStore, NewConflict, Side};
use twydo_core::edit::{Change, Op};
use twydo_core::model::TaskKind;
use twydo_core::store::bind_vdir_state;
use twydo_core::store::fsio::VdirLock;
use twydo_core::views::{Expansion, View, build_view, group_label};
use twydo_daemon::protocol::methods::DaemonInfo;
use twydo_daemon::protocol::rpc::ErrorKind;
use twydo_daemon::protocol::views::{SectionKey, ViewsGetResult};
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
        "inbox/late.ics",
        &ics(
            "late",
            "Late one",
            "DUE;VALUE=DATE:20261116\r\nDESCRIPTION:Bring the form\r\n",
        ),
    );
    fx.write(
        "inbox/now.ics",
        &ics("now", "Due today", "DUE:20261117T170000\r\n"),
    );
    fx.write(
        "work/next.ics",
        &ics("next", "Due tomorrow", "DUE;VALUE=DATE:20261118\r\n"),
    );
    fx.write("work/loose.ics", &ics("loose", "No date", ""));
    fx
}

fn start(fx: &Fixture, f: impl FnOnce(&mut twydo_core::config::Config)) -> TestDaemon {
    let mut cfg = fx.config();
    cfg.timezone = Some("UTC".into());
    f(&mut cfg);
    TestDaemon::start_cfg(fx, cfg, |o| o.clock = morning())
}

fn get(td: &TestDaemon, view: &str) -> ViewsGetResult {
    td.client()
        .call("views.get", json!({ "view": view }))
        .unwrap()
}

fn uids(v: &ViewsGetResult) -> Vec<String> {
    v.sections
        .iter()
        .flat_map(|s| s.rows.iter().map(|r| r.uid.clone()))
        .collect()
}

fn row<'a>(v: &'a ViewsGetResult, uid: &str) -> &'a twydo_daemon::protocol::views::ViewRow {
    v.sections
        .iter()
        .flat_map(|s| &s.rows)
        .find(|r| r.uid == uid)
        .unwrap_or_else(|| panic!("no row {uid}"))
}

#[test]
fn today_matches_build_view() {
    let fx = fx();
    let td = start(&fx, |_| {});
    let v = get(&td, "today");
    let snap = fx.snapshot();
    let now = Utc
        .with_ymd_and_hms(2026, 11, 17, 9, 0, 0)
        .unwrap()
        .with_timezone(&snap.tz());
    let want = build_view(&snap, &View::Today, now, &Expansion::default());
    let want_uids: Vec<_> = want
        .groups
        .iter()
        .flat_map(|(_, rows)| rows.iter().map(|r| snap.tasks[r.task].uid.clone()))
        .collect();
    assert_eq!(uids(&v), want_uids);
    assert!(!want_uids.is_empty());
    assert_eq!(v.sections.len(), want.groups.len());
    for (sec, (g, _)) in v.sections.iter().zip(&want.groups) {
        assert_eq!(sec.label, group_label(*g, now.date_naive()));
    }
    assert_eq!(v.sections[0].group, SectionKey::Overdue);
    assert_eq!(v.title, "Today");
    assert_eq!(v.open_count, want.counts.total);
    assert_eq!(v.today, NaiveDate::from_ymd_opt(2026, 11, 17).unwrap());
}

#[test]
fn inbox_lists_the_inbox_collection() {
    let td = start(&fx(), |_| {});
    let v = get(&td, "inbox");
    let mut got = uids(&v);
    got.sort();
    assert_eq!(got, ["late", "now"]);
    assert_eq!(v.title, "inbox");
    assert_eq!(row(&v, "late").list.id, "inbox");
}

#[test]
fn inbox_uses_configured_default_list() {
    let td = start(&fx(), |c| c.default_list = Some("work".into()));
    let v = get(&td, "inbox");
    let mut got = uids(&v);
    got.sort();
    assert_eq!(got, ["loose", "next"]);
    assert_eq!(v.capture.unwrap().collection, "work");
}

#[test]
fn capture_targets() {
    let td = start(&fx(), |_| {});
    let today = get(&td, "today").capture.unwrap();
    assert_eq!(today.collection, "inbox");
    assert_eq!(today.due, NaiveDate::from_ymd_opt(2026, 11, 17));
    let inbox = get(&td, "inbox").capture.unwrap();
    assert_eq!(inbox.collection, "inbox");
    assert_eq!(inbox.due, None);
}

#[test]
fn row_fields() {
    let fx = fx();
    fx.write(
        "inbox/rep.ics",
        &ics(
            "rep",
            "Weekly",
            "DUE:20261117T170000\r\nRRULE:FREQ=WEEKLY\r\nDTSTART:20261125T090000\r\n",
        ),
    );
    let td = start(&fx, |_| {});
    let v = get(&td, "inbox");
    let late = row(&v, "late");
    assert!(late.overdue);
    assert!(!late.due_today);
    assert_eq!(late.due.as_deref(), Some("Yesterday"));
    assert_eq!(late.note.as_deref(), Some("Bring the form"));
    assert_eq!(late.kind, TaskKind::Task);
    assert!(!late.repeats);
    let now = row(&v, "now");
    assert!(now.due_today && !now.overdue);
    let rep = row(&v, "rep");
    assert!(rep.repeats);
    assert!(rep.start.is_some());
}

#[test]
fn completed_rows_are_flagged() {
    let fx = fx();
    fx.write(
        "inbox/fin.ics",
        &ics(
            "fin",
            "Finished today",
            "DUE:20261117T080000Z\r\nSTATUS:COMPLETED\r\nCOMPLETED:20261117T080000Z\r\n",
        ),
    );
    let td = start(&fx, |_| {});
    let v = get(&td, "inbox");
    let done = v
        .sections
        .iter()
        .find(|s| s.group == SectionKey::Done)
        .expect("done section");
    assert!(done.rows.iter().any(|r| r.uid == "fin" && r.completed));
    assert!(
        v.sections
            .iter()
            .filter(|s| s.group != SectionKey::Done)
            .all(|s| s.rows.iter().all(|r| !r.completed))
    );
    assert!(!row(&v, "now").completed);
}

#[test]
fn queued_write_shows_pending() {
    let fx = fx();
    let cfg = fx.config();
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let _lock = VdirLock::exclusive(&state).unwrap();
    let op = Op::Edit {
        uid: "now".into(),
        changes: vec![Change::Summary("Due today, queued".into())],
    };
    let at = Utc.with_ymd_and_hms(2026, 11, 9, 12, 0, 0).unwrap();
    common::queue_op(&cfg, &fx.snapshot(), &op, "edit", None, at);
    drop(_lock);
    let _sync = VdirLock::shared(&state).unwrap();
    let td = start(&fx, |_| {});
    let v = get(&td, "inbox");
    let r = row(&v, "now");
    assert!(r.pending);
    assert_eq!(r.title, "Due today, queued");
    assert!(!row(&v, "late").pending);
}

#[test]
fn conflict_flag_set_for_unresolved_conflict() {
    let fx = fx();
    let text = ics("late", "Late one", "DUE;VALUE=DATE:20261116\r\n");
    ConflictStore::open(&fx.state())
        .unwrap()
        .record(
            NewConflict {
                uid: "late".into(),
                collection: "inbox".into(),
                path: "inbox/late.ics".into(),
                kind: ConflictKind::BothChanged,
                held: false,
                vdir_holds: Side::Remote,
                local: Some(text.clone()),
                remote: Some(text),
            },
            Utc::now(),
        )
        .unwrap();
    let td = start(&fx, |_| {});
    let v = get(&td, "inbox");
    assert!(row(&v, "late").conflict);
    assert!(!row(&v, "now").conflict);
}

#[test]
fn today_follows_daemon_timezone() {
    let fx = fx();
    let mut cfg = fx.config();
    cfg.timezone = Some("Pacific/Auckland".into());
    let td = TestDaemon::start_cfg(&fx, cfg, |o| {
        o.clock = FakeClock::at(Utc.with_ymd_and_hms(2026, 11, 17, 13, 0, 0).unwrap());
    });
    let v = get(&td, "today");
    assert_eq!(v.today, NaiveDate::from_ymd_opt(2026, 11, 18).unwrap());
    assert_eq!(v.now.offset().local_minus_utc(), 13 * 3600);
}

#[test]
fn no_lists_gives_empty_inbox_without_capture() {
    let fx = Fixture::new(&[]);
    let td = start(&fx, |_| {});
    let v = get(&td, "inbox");
    assert!(v.sections.is_empty());
    assert_eq!(v.capture, None);
    assert_eq!(v.title, "Inbox");
    assert_eq!(v.open_count, 0);
}

#[test]
fn unknown_view_is_invalid_params() {
    let td = start(&fx(), |_| {});
    let e = td
        .client()
        .call::<_, ViewsGetResult>("views.get", json!({ "view": "all" }))
        .unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Invalid);
}

#[test]
fn revision_and_epoch_match_snapshot_get() {
    let td = start(&fx(), |_| {});
    let c = td.client();
    let snap: WireSnapshot = c.call("snapshot.get", empty()).unwrap();
    let info: DaemonInfo = c.call("daemon.info", empty()).unwrap();
    let v: ViewsGetResult = c.call("views.get", json!({ "view": "today" })).unwrap();
    assert_eq!(v.revision, snap.revision);
    assert_eq!(v.epoch, info.epoch);
}
