//! `conflicts.list` and `conflicts.resolve`: the unresolved conflicts with their titles, a
//! resolution that publishes (or records a choice and starts a sync), the replay of a resent
//! resolution even after a restart, and a resolution waiting for a running sync. Synthetic
//! fixture content; the vdirsyncer is a fake script.

mod common;

use std::fs;
use std::time::Duration;

use chrono::Utc;
use common::{FakeClock, Fixture, TestDaemon, empty, kind, todo_ics, wait_event};
use twydo_core::conflicts::{ConflictKind, ConflictStore, NewConflict, Side};
use twydo_daemon::client::Client;
use twydo_daemon::protocol::admin::{
    ConflictList, ConflictsResolveParams, ResolveResult, WireConflictKind, WireConflictState,
    WireKeep, WireResolution, WireSide,
};
use twydo_daemon::protocol::events::{Cause, Event};
use twydo_daemon::protocol::jobs::JobStarted;
use twydo_daemon::protocol::rpc::ErrorKind;
use twydo_daemon::protocol::sync::{SyncReason, SyncRunParams};

const BOUND: Duration = Duration::from_secs(10);

fn fx() -> Fixture {
    Fixture::new(&[("c1", "u1", "One", None)])
}

/// Records a conflict for `c1/u1.ics`, which holds the remote version (so keeping local
/// writes). Returns its id.
fn seed(fx: &Fixture, held: bool) -> String {
    let local = todo_ics("u1", "Local title", None);
    let remote = todo_ics("u1", "Remote title", None);
    fx.write("c1/u1.ics", &remote);
    ConflictStore::open(&fx.state())
        .unwrap()
        .record(
            NewConflict {
                uid: "u1".into(),
                collection: "c1".into(),
                path: "c1/u1.ics".into(),
                kind: ConflictKind::BothChanged,
                held,
                vdir_holds: Side::Remote,
                local: Some(local),
                remote: Some(remote),
            },
            Utc::now(),
        )
        .unwrap()
        .id
}

fn list(c: &Client) -> ConflictList {
    c.call("conflicts.list", empty()).unwrap()
}

fn resolve(
    c: &Client,
    request_id: &str,
    id: &str,
    keep: WireKeep,
) -> Result<ResolveResult, twydo_daemon::client::ClientError> {
    c.call(
        "conflicts.resolve",
        ConflictsResolveParams {
            request_id: request_id.into(),
            id: id.into(),
            keep,
        },
    )
}

fn subscribe(c: &Client) {
    let _: serde_json::Value = c.call("events.subscribe", empty()).unwrap();
}

#[test]
fn conflicts_list_shows_unresolved_with_titles() {
    let fx = fx();
    let id = seed(&fx, false);
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let l = list(&c);
    assert_eq!(l.conflicts.len(), 1);
    let v = &l.conflicts[0];
    assert_eq!(v.id, id);
    assert_eq!(v.uid, "u1");
    assert_eq!(v.collection, "c1");
    assert_eq!(v.path, "c1/u1.ics");
    assert_eq!(v.title.as_deref(), Some("Local title"));
    assert_eq!(v.kind, WireConflictKind::BothChanged);
    assert!(!v.held);
    assert_eq!(v.state, WireConflictState::Open);
    assert_eq!(v.choice, None);
}

#[test]
fn resolve_written_publishes_and_records() {
    let fx = fx();
    let id = seed(&fx, false);
    let td = TestDaemon::start(&fx);
    let c = td.client();
    subscribe(&c);
    let r = resolve(&c, "r1", &id, WireKeep::Local).unwrap();
    assert_eq!(r.resolution, WireResolution::Written);
    assert!(r.op_id.is_some());
    assert_eq!(r.message, "written; conflict resolved");
    let ev = wait_event(
        &c,
        BOUND,
        |e| matches!(e, Event::DataChanged { cause: Cause::Op, uids, .. } if uids == &["u1".to_string()]),
    );
    assert!(matches!(ev, Event::DataChanged { .. }));
    assert!(list(&c).conflicts.is_empty());
    assert!(fx.read("c1/u1.ics").contains("Local title"));
}

#[test]
fn resolve_recorded_starts_a_sync() {
    let fx = fx();
    let id = seed(&fx, true);
    let cfg = fx.with_sync("exit 0");
    let clock = FakeClock::new();
    let td = TestDaemon::start_scheduled(&fx, cfg, clock);
    let c = td.client();
    subscribe(&c);
    // Let the startup sync end first, so the next one is ours.
    wait_event(&c, BOUND, |e| {
        matches!(
            e,
            Event::SyncFinished {
                reason: SyncReason::Startup,
                ..
            }
        )
    });
    let r = resolve(&c, "r1", &id, WireKeep::Remote).unwrap();
    assert_eq!(r.resolution, WireResolution::Recorded);
    assert_eq!(r.op_id, None);
    assert_eq!(r.message, "recorded; the next sync applies it");
    wait_event(&c, BOUND, |e| {
        matches!(
            e,
            Event::SyncStarted {
                reason: SyncReason::Resolve,
                ..
            }
        )
    });
}

#[test]
fn resend_after_restart_replays_the_resolution() {
    let fx = fx();
    let id = seed(&fx, false);
    let first = {
        let td = TestDaemon::start(&fx);
        let c = td.client();
        resolve(&c, "r1", &id, WireKeep::Local).unwrap()
    };
    assert_eq!(first.resolution, WireResolution::Written);
    let entries = || {
        fs::read_dir(fx.state().join("journal"))
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|x| x == "json")
            })
            .count()
    };
    let before = entries();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let again = resolve(&c, "r1", &id, WireKeep::Local).unwrap();
    assert_eq!(again, first);
    assert_eq!(entries(), before);
    // The same id for another conflict still answers the first: it is the same request.
    assert_eq!(resolve(&c, "r1", "other", WireKeep::Local).unwrap(), first);
}

/// Codex review: A chooses local, B remote on a held conflict; after a restart A's resend is
/// answered from A's record and the choice stays remote.
#[test]
fn resend_of_an_earlier_choice_after_restart_keeps_the_later_one() {
    let fx = fx();
    let id = seed(&fx, true);
    let first = {
        let td = TestDaemon::start(&fx);
        let c = td.client();
        let a = resolve(&c, "a", &id, WireKeep::Local).unwrap();
        resolve(&c, "b", &id, WireKeep::Remote).unwrap();
        a
    };
    assert_eq!(first.resolution, WireResolution::Recorded);
    let td = TestDaemon::start(&fx);
    let c = td.client();
    assert_eq!(resolve(&c, "a", &id, WireKeep::Local).unwrap(), first);
    let l = list(&c);
    assert_eq!(l.conflicts.len(), 1);
    assert_eq!(l.conflicts[0].state, WireConflictState::Chosen);
    assert_eq!(l.conflicts[0].choice, Some(WireSide::Remote));
}

#[test]
fn resolve_during_a_sync_waits_for_it() {
    let fx = fx();
    let id = seed(&fx, false);
    let cfg = fx.with_sync("[ \"$1\" = sync ] && sleep 1\nexit 0");
    let td = TestDaemon::start_cfg(&fx, cfg, |_| {});
    let c = td.client();
    subscribe(&c);
    let job: JobStarted = c
        .call(
            "sync.run",
            SyncRunParams {
                request_id: "s1".into(),
                force_delete: Vec::new(),
            },
        )
        .unwrap();
    wait_event(&c, BOUND, |e| matches!(e, Event::SyncStarted { .. }));
    let c2 = td.client();
    let (tx, rx) = std::sync::mpsc::channel();
    let h = std::thread::spawn(move || {
        tx.send(resolve(&c2, "r1", &id, WireKeep::Local)).unwrap();
    });
    // The sync takes a second: the resolution does not answer before it ends.
    assert!(rx.recv_timeout(Duration::from_millis(400)).is_err());
    let done = wait_event(&c, BOUND, |e| matches!(e, Event::SyncFinished { .. }));
    assert!(matches!(&done, Event::SyncFinished { job_id: Some(j), .. } if *j == job.job_id));
    let r = rx.recv_timeout(BOUND).unwrap().unwrap();
    assert_eq!(r.resolution, WireResolution::Written);
    h.join().unwrap();
}

#[test]
fn resolve_unknown_conflict_is_not_found() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let e = resolve(&c, "r1", "nope", WireKeep::Local).unwrap_err();
    assert_eq!(kind(&e), ErrorKind::NotFound);
    // A deterministic refusal is kept: the resend answers the same.
    let e = resolve(&c, "r1", "nope", WireKeep::Local).unwrap_err();
    assert_eq!(kind(&e), ErrorKind::NotFound);
}

#[test]
fn keep_current_on_a_held_conflict_is_invalid() {
    let fx = fx();
    let id = seed(&fx, true);
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let e = resolve(&c, "r1", &id, WireKeep::Current).unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Invalid);
    let e = resolve(&c, "", &id, WireKeep::Local).unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Invalid);
}
