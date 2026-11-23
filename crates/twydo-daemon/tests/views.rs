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
use twydo_core::views::{
    Expansion, SidebarEntry, View, build_view, counts, group_label, sidebar, view_name,
};
use twydo_daemon::protocol::methods::DaemonInfo;
use twydo_daemon::protocol::rpc::ErrorKind;
use twydo_daemon::protocol::views::{
    SectionKey, SettingsGetResult, ViewName, ViewsEntry, ViewsGetParams, ViewsGetResult,
    ViewsListItem, ViewsListResult,
};
use twydo_daemon::protocol::wire::WireSnapshot;
use twydo_daemon::server::schedule::Clock;

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
    let want_keys: Vec<SectionKey> = want
        .groups
        .iter()
        .map(|(g, _)| SectionKey::from(*g))
        .collect();
    let got_keys: Vec<SectionKey> = v.sections.iter().map(|s| s.group).collect();
    assert_eq!(got_keys, want_keys);
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
fn completed_days_are_told_apart_by_date() {
    let fx = fx();
    for (uid, day) in [("fin1", "20261117"), ("fin2", "20261116")] {
        fx.write(
            &format!("inbox/{uid}.ics"),
            &ics(
                uid,
                "Finished",
                &format!("STATUS:COMPLETED\r\nCOMPLETED:{day}T080000Z\r\n"),
            ),
        );
    }
    let td = start(&fx, |_| {});
    let v = get(&td, "done");
    let dates: Vec<_> = v
        .sections
        .iter()
        .filter(|s| s.group == SectionKey::Completed)
        .map(|s| s.date)
        .collect();
    let d = |day| chrono::NaiveDate::from_ymd_opt(2026, 11, day);
    assert_eq!(dates, vec![d(17), d(16)]);
    assert!(
        v.sections
            .iter()
            .filter(|s| s.group != SectionKey::Completed)
            .all(|s| s.date.is_none())
    );
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
fn zoned_due_is_dated_in_the_daemon_timezone() {
    let fx = fx();
    // 2026-11-18 00:30 in Auckland (UTC+13) is 2026-11-17 11:30 UTC: today, not tomorrow.
    fx.write(
        "inbox/nz.ics",
        &ics(
            "nz",
            "Zoned",
            "DUE;TZID=Pacific/Auckland:20261118T003000\r\n",
        ),
    );
    // 2026-11-17 22:00 in New York (UTC-5) is 2026-11-18 03:00 UTC: tomorrow, not today.
    fx.write(
        "inbox/ny.ics",
        &ics(
            "ny",
            "Zoned west",
            "DUE;TZID=America/New_York:20261117T220000\r\n",
        ),
    );
    let td = start(&fx, |_| {});
    let v = get(&td, "inbox");
    let nz = row(&v, "nz");
    assert!(nz.due_today);
    assert_eq!(nz.due.as_deref(), Some("Today 11:30"));
    let ny = row(&v, "ny");
    assert!(!ny.due_today);
    assert_eq!(ny.due.as_deref(), Some("Tomorrow 03:00"));
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
        .call::<_, ViewsGetResult>("views.get", json!({ "view": "someday" }))
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

#[test]
fn nested_subtask_carries_depth_and_parent() {
    let fx = fx();
    fx.write(
        "inbox/list.ics",
        &ics(
            "chk",
            "Packing",
            "DUE:20261117T170000\r\nX-2DOAPP-METADATA:@tasktype:checklist\r\n",
        ),
    );
    fx.write(
        "inbox/item.ics",
        &ics("item", "Socks", "RELATED-TO;RELTYPE=PARENT:chk\r\n"),
    );
    let td = start(&fx, |_| {});
    let v = get(&td, "today");
    let child = row(&v, "item");
    assert_eq!(child.depth, 1);
    let parent = child.parent.as_ref().expect("parent hint");
    assert_eq!(parent.uid, "chk");
    assert_eq!(parent.title, "Packing");
    assert_eq!(row(&v, "chk").depth, 0);
}

#[test]
fn sync_absent_without_sync_config() {
    let fx = fx();
    let td = start(&fx, |_| {});
    assert_eq!(get(&td, "today").sync, None);
}

#[test]
fn sync_summary_reports_failure() {
    let fx = fx();
    let cfg = fx.with_sync("exit 0");
    let status = twydo_core::sync::status::SyncStatus {
        last_error: Some("Cannot reach the server".into()),
        failures: 1,
        // Far ahead of the test clock, so no background run replaces it.
        next_attempt: Some(Utc.with_ymd_and_hms(2026, 11, 18, 0, 0, 0).unwrap()),
        ..Default::default()
    };
    status.save(&fx.state()).unwrap();
    let td = TestDaemon::start_cfg(&fx, cfg, |o| o.clock = morning());
    let sync = get(&td, "inbox").sync.expect("sync summary");
    assert!(sync.attention, "{sync:?}");
    assert_eq!(sync.headline, "Sync failing");
}

#[test]
fn reminders_absent_without_notifier_error() {
    let fx = fx();
    let td = start(&fx, |_| {});
    assert_eq!(get(&td, "today").reminders, None);
}

#[test]
fn reminders_carry_the_notifier_error() {
    let fx = Fixture::new(&[]);
    fx.write_reminder("inbox/r1.ics", "r1", "Ping", "20261117T090000");
    let sink = std::sync::Arc::new(common::RecordingSink::default());
    sink.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    let mut cfg = fx.config();
    cfg.timezone = Some("UTC".into());
    cfg.morning_time = chrono::NaiveTime::from_hms_opt(23, 0, 0).unwrap();
    let s2 = std::sync::Arc::clone(&sink);
    let td = TestDaemon::start_cfg(&fx, cfg, move |o| {
        o.clock = morning();
        o.notify = Some(s2 as std::sync::Arc<dyn twydo_daemon::server::notifier::Sink>);
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let reminders = loop {
        if let Some(r) = get(&td, "inbox").reminders {
            break r;
        }
        assert!(std::time::Instant::now() < deadline, "no reminders summary");
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert!(!reminders.error.is_empty());
    assert!(!reminders.error.contains("Ping"), "never names a title");
    let status: twydo_daemon::protocol::methods::StatusView =
        td.client().call("status.get", empty()).unwrap();
    assert_eq!(Some(reminders.error), status.notifier_error);
}

fn get_ref(td: &TestDaemon, params: serde_json::Value) -> ViewsGetResult {
    td.client().call("views.get", params).unwrap()
}

#[test]
fn every_view_matches_build_view() {
    let fx = fx();
    fx.write(
        "work/fin.ics",
        &ics(
            "fin",
            "Finished",
            "STATUS:COMPLETED\r\nCOMPLETED:20261116T080000Z\r\n",
        ),
    );
    let td = start(&fx, |_| {});
    let snap = fx.snapshot();
    let now = Utc
        .with_ymd_and_hms(2026, 11, 17, 9, 0, 0)
        .unwrap()
        .with_timezone(&snap.tz());
    let cases = [
        (json!({"view": "all"}), View::All),
        (json!({"view": "scheduled"}), View::Scheduled),
        (json!({"view": "done"}), View::Done),
        (json!({"view": "next_3_days"}), View::Next3Days),
        (
            json!({"view": "list", "list": "work"}),
            View::List("work".into()),
        ),
    ];
    for (params, view) in cases {
        let v = get_ref(&td, params.clone());
        let want = build_view(&snap, &view, now, &Expansion::default());
        let want_uids: Vec<_> = want
            .groups
            .iter()
            .flat_map(|(_, rows)| rows.iter().map(|r| snap.tasks[r.task].uid.clone()))
            .collect();
        assert_eq!(uids(&v), want_uids, "{params}");
        let got: Vec<_> = v
            .sections
            .iter()
            .map(|s| (s.group, s.label.clone()))
            .collect();
        let exp: Vec<_> = want
            .groups
            .iter()
            .map(|(g, _)| (SectionKey::from(*g), group_label(*g, now.date_naive())))
            .collect();
        assert_eq!(got, exp, "{params}");
        assert_eq!(v.title, view_name(&snap, &view), "{params}");
        assert_eq!(v.open_count, counts(&snap, &view, now).total, "{params}");
    }
    assert!(!uids(&get_ref(&td, json!({"view": "all"}))).is_empty());
}

#[test]
fn capture_for_new_views() {
    let td = start(&fx(), |_| {});
    let c = get_ref(&td, json!({"view": "list", "list": "work"}))
        .capture
        .unwrap();
    assert_eq!((c.collection.as_str(), c.due), ("work", None));
    for view in ["all", "scheduled", "done", "next_3_days"] {
        let c = get_ref(&td, json!({ "view": view })).capture.unwrap();
        assert_eq!((c.collection.as_str(), c.due), ("inbox", None), "{view}");
    }
}

#[test]
fn list_param_must_match_view() {
    let td = start(&fx(), |_| {});
    for params in [
        json!({"view": "list"}),
        json!({"view": "today", "list": "work"}),
        json!({"view": "list", "list": "nope"}),
    ] {
        let e = td
            .client()
            .call::<_, ViewsGetResult>("views.get", params.clone())
            .unwrap_err();
        assert_eq!(kind(&e), ErrorKind::Invalid, "{params}");
    }
}

#[test]
fn hidden_list_is_served_by_id() {
    let fx = fx();
    let mut cfg = fx.config();
    cfg.timezone = Some("UTC".into());
    cfg.lists.insert(
        "work".into(),
        twydo_core::config::ListPrefs {
            hidden: true,
            ..Default::default()
        },
    );
    let td = TestDaemon::start_cfg(&fx, cfg, |o| o.clock = morning());
    let v = get_ref(&td, json!({"view": "list", "list": "work"}));
    let mut got = uids(&v);
    got.sort();
    assert_eq!(got, ["loose", "next"]);
}

fn list_entries(td: &TestDaemon) -> ViewsListResult {
    td.client().call("views.list", json!({})).unwrap()
}

fn items(r: &ViewsListResult) -> Vec<&ViewsListItem> {
    r.entries
        .iter()
        .filter_map(|e| match e {
            ViewsEntry::View(i) => Some(i),
            ViewsEntry::Heading { .. } => None,
        })
        .collect()
}

#[test]
fn views_list_matches_sidebar() {
    let fx = fx();
    let td = start(&fx, |c| {
        c.groups = vec![twydo_core::config::Group {
            name: "G".into(),
            lists: vec!["work".into()],
        }];
    });
    let r = list_entries(&td);
    let mut snap = fx.snapshot();
    snap.config.groups = vec![twydo_core::config::Group {
        name: "G".into(),
        lists: vec!["work".into()],
    }];
    let side = sidebar(&snap);
    assert_eq!(r.entries.len(), side.len());
    let now = morning().now().with_timezone(&snap.tz());
    for (got, want) in r.entries.iter().zip(&side) {
        match (got, want) {
            (ViewsEntry::Heading { heading }, SidebarEntry::Heading(h)) => assert_eq!(heading, h),
            (ViewsEntry::View(i), SidebarEntry::Item(v)) => {
                assert_eq!(i.name, view_name(&snap, v));
                let c = counts(&snap, v, now);
                assert_eq!(i.open_count, c.total);
                assert_eq!(i.due_count, c.due_or_overdue);
            }
            other => panic!("mismatch: {other:?}"),
        }
    }
    assert_eq!(
        items(&r)[0].view,
        ViewsGetParams {
            view: ViewName::Inbox,
            list: None
        }
    );
    assert!(
        r.entries
            .iter()
            .any(|e| matches!(e, ViewsEntry::Heading { heading } if heading == "G"))
    );
}

#[test]
fn views_list_omits_hidden_inbox() {
    let fx = fx();
    let td = start(&fx, |c| {
        c.lists.insert(
            "inbox".into(),
            twydo_core::config::ListPrefs {
                hidden: true,
                ..Default::default()
            },
        );
    });
    let r = list_entries(&td);
    let it = items(&r);
    assert_eq!(it[0].view.view, ViewName::Today);
    assert!(it.iter().all(|i| i.view.view != ViewName::Inbox));
}

#[test]
fn views_list_item_round_trips() {
    let fx = fx();
    let td = start(&fx, |_| {});
    let r = list_entries(&td);
    assert!(items(&r).len() >= 6);
    for i in items(&r) {
        let v = get_ref(&td, serde_json::to_value(&i.view).unwrap());
        assert_eq!(v.title, i.name);
    }
}

#[test]
fn views_list_colors() {
    let fx = fx();
    fx.write("work/color", "#336699\n");
    let td = start(&fx, |_| {});
    let r = list_entries(&td);
    let it = items(&r);
    let work = it
        .iter()
        .find(|i| i.view.list.as_deref() == Some("work"))
        .unwrap();
    assert_eq!(work.color.as_deref(), Some("#336699"));
    let today = it.iter().find(|i| i.view.view == ViewName::Today).unwrap();
    assert_eq!(today.color, None);
}

/// A start later today is still ahead: the row says when, with its time.
#[test]
fn row_shows_a_start_later_today_with_its_time() {
    let fx = fx();
    fx.write(
        "inbox/tyre.ics",
        &ics("tyre", "Change tyres", "DTSTART:20261117T150000\r\n"),
    );
    let td = start(&fx, |_| {});
    let v = get(&td, "inbox");
    assert_eq!(row(&v, "tyre").start.as_deref(), Some("Today 15:00"));
}

fn settings(td: &TestDaemon) -> SettingsGetResult {
    td.client().call("settings.get", json!({})).unwrap()
}

#[test]
fn settings_without_sync() {
    let fx = fx();
    let td = start(&fx, |_| {});
    let s = settings(&td);
    let info: DaemonInfo = td.client().call("daemon.info", json!({})).unwrap();
    assert_eq!(s.version, info.version);
    assert_eq!(s.protocol, 4);
    assert_eq!(s.server, None);
    assert_eq!(s.sync, None);
    assert_eq!(s.reminders, None);
}

#[test]
fn settings_with_sync() {
    let fx = fx();
    let mut cfg = fx.with_sync("exit 0");
    cfg.sync.as_mut().unwrap().url = "https://user@dav.example.net/dav/".into();
    let status = twydo_core::sync::status::SyncStatus {
        // Far ahead of the test clock, so no background run replaces it.
        next_attempt: Some(Utc.with_ymd_and_hms(2026, 11, 18, 0, 0, 0).unwrap()),
        ..Default::default()
    };
    status.save(&fx.state()).unwrap();
    let td = TestDaemon::start_cfg(&fx, cfg, |o| o.clock = morning());
    let s = settings(&td);
    assert_eq!(s.server.as_deref(), Some("dav.example.net"));
    assert!(s.sync.is_some());
    assert_eq!(s.sync, get(&td, "inbox").sync);
}

#[test]
fn settings_carry_the_notifier_error() {
    let fx = Fixture::new(&[]);
    fx.write_reminder("inbox/r1.ics", "r1", "Ping", "20261117T090000");
    let sink = std::sync::Arc::new(common::RecordingSink::default());
    sink.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    let mut cfg = fx.config();
    cfg.timezone = Some("UTC".into());
    cfg.morning_time = chrono::NaiveTime::from_hms_opt(23, 0, 0).unwrap();
    let s2 = std::sync::Arc::clone(&sink);
    let td = TestDaemon::start_cfg(&fx, cfg, move |o| {
        o.clock = morning();
        o.notify = Some(s2 as std::sync::Arc<dyn twydo_daemon::server::notifier::Sink>);
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let reminders = loop {
        if let Some(r) = settings(&td).reminders {
            break r;
        }
        assert!(std::time::Instant::now() < deadline, "no reminders summary");
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert!(!reminders.error.is_empty());
}

#[test]
fn settings_never_carry_sync_credentials() {
    let fx = fx();
    let mut cfg = fx.with_sync("exit 0");
    cfg.sync.as_mut().unwrap().url = "https://alice:s3cret@dav.example.net/private/dav/".into();
    let status = twydo_core::sync::status::SyncStatus {
        last_error: Some(
            "Unexpected status 401 for https://alice:s3(cret@dav.example.net/private(alice)/dav/"
                .into(),
        ),
        failures: 1,
        // Far ahead of the test clock, so no background run replaces it.
        next_attempt: Some(Utc.with_ymd_and_hms(2026, 11, 18, 0, 0, 0).unwrap()),
        ..Default::default()
    };
    status.save(&fx.state()).unwrap();
    let td = TestDaemon::start_cfg(&fx, cfg, |o| o.clock = morning());
    let raw: serde_json::Value = td.client().call("settings.get", json!({})).unwrap();
    let text = raw.to_string();
    for secret in ["alice", "s3cret", "s3(cret", "/private", "/dav"] {
        assert!(!text.contains(secret), "{secret} in {text}");
    }
    assert!(text.contains("dav.example.net"), "{text}");
}
