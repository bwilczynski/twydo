//! The journal methods (`journal.list`, `journal.recover`, `journal.revert`, `journal.abandon`)
//! and `conflicts.list` and `conflicts.resolve`: the unresolved conflicts with their titles, a
//! resolution that publishes (or records a choice and starts a sync), the replay of a resent
//! resolution even after a restart, and a resolution waiting for a running sync. Synthetic
//! fixture content; the vdirsyncer is a fake script.

mod common;

use std::fs;
use std::time::Duration;

use chrono::Utc;
use common::{
    FakeClock, Fixture, TestDaemon, based, edit_title, empty, kind, snapshot, submit, task,
    todo_ics, undo, wait_event,
};
use twydo_core::conflicts::{ConflictKind, ConflictStore, NewConflict, Side};
use twydo_core::store::bind_vdir_state;
use twydo_core::store::journal::{Entry, Journal, Stage, Write};
use twydo_daemon::client::Client;
use twydo_daemon::protocol::admin::{
    AbandonResult, ConflictDetail, ConflictList, ConflictsResolveParams, ConflictsShowParams,
    InputField, InputParseParams, InputParsed, JournalAbandonParams, JournalEntryView,
    JournalRevertParams, RecoverResult, ResolveResult, WireConflictKind, WireConflictState,
    WireKeep, WireResolution, WireSide, WireStage,
};
use twydo_daemon::protocol::events::{Cause, Event};
use twydo_daemon::protocol::jobs::JobStarted;
use twydo_daemon::protocol::methods::{StatusView, WriteOutcome, WriteResult};
use twydo_daemon::protocol::rpc::ErrorKind;
use twydo_daemon::protocol::sync::{SyncReason, SyncRunParams};
use twydo_daemon::server::lane::LongKind;

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

/// A chooses local, B asks for remote on a held conflict: the later choice wins.
#[test]
fn a_differing_rechoice_from_a_new_request_wins() {
    let fx = fx();
    let id = seed(&fx, true);
    let td = TestDaemon::start(&fx);
    let c = td.client();
    resolve(&c, "a", &id, WireKeep::Local).unwrap();
    let b = resolve(&c, "b", &id, WireKeep::Remote).unwrap();
    assert_eq!(b.resolution, WireResolution::Recorded);
    let l = list(&c);
    assert_eq!(l.conflicts[0].choice, Some(WireSide::Remote));
}

/// A local, B remote, restart, A resent: A's own answer, nothing re-runs, the choice stays
/// remote.
#[test]
fn resending_an_earlier_choice_after_a_newer_one_does_not_rerun_it() {
    let fx = fx();
    let id = seed(&fx, true);
    let a = {
        let td = TestDaemon::start(&fx);
        let c = td.client();
        let a = resolve(&c, "a", &id, WireKeep::Local).unwrap();
        resolve(&c, "b", &id, WireKeep::Remote).unwrap();
        assert_eq!(resolve(&c, "a", &id, WireKeep::Local).unwrap(), a);
        a
    };
    assert_eq!(a.resolution, WireResolution::Recorded);
    let td = TestDaemon::start(&fx);
    let c = td.client();
    assert_eq!(resolve(&c, "a", &id, WireKeep::Local).unwrap(), a);
    let l = list(&c);
    assert_eq!(l.conflicts.len(), 1);
    assert_eq!(l.conflicts[0].state, WireConflictState::Chosen);
    assert_eq!(l.conflicts[0].choice, Some(WireSide::Remote));
}

/// A crash between the restore and recording the resolution: the resend finds the journal
/// entry its request wrote, answers `written` with its id and closes the conflict.
#[test]
fn resend_after_a_crash_between_restore_and_the_record_answers_written() {
    use twydo_core::store::Store;
    use twydo_core::store::journal::RequestRef;
    let fx = fx();
    let id = seed(&fx, false);
    let remote = todo_ics("u1", "Remote title", None);
    let local = todo_ics("u1", "Local title", None);
    let (mut store, _) = Store::open(&fx.config()).unwrap();
    store.set_request(Some(RequestRef {
        session: "s".into(),
        request_id: "r-crash".into(),
    }));
    let op = store
        .restore(
            "c1/u1.ics",
            Some(&remote),
            Some(&local),
            vec!["u1".into()],
            Utc::now(),
        )
        .unwrap();
    drop(store);
    assert_eq!(list(&TestDaemon::start(&fx).client()).conflicts.len(), 1);
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let r = resolve(&c, "r-crash", &id, WireKeep::Local).unwrap();
    assert_eq!(r.resolution, WireResolution::Written);
    assert_eq!(r.op_id.as_deref(), Some(op.as_str()));
    assert!(list(&c).conflicts.is_empty());
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
                discover: false,
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
fn conflicts_show_reports_both_sides_and_current() {
    let fx = fx();
    let local = todo_ics("u1", "Local title", None);
    let remote =
        todo_ics("u1", "Remote title", None).replace("END:VTODO", "STATUS:COMPLETED\r\nEND:VTODO");
    fx.write("c1/u1.ics", &remote);
    let id = ConflictStore::open(&fx.state())
        .unwrap()
        .record(
            NewConflict {
                uid: "u1".into(),
                collection: "c1".into(),
                path: "c1/u1.ics".into(),
                kind: ConflictKind::RemoteDeleteLocalEdit,
                held: false,
                vdir_holds: Side::Remote,
                local: Some(local),
                remote: None,
            },
            Utc::now(),
        )
        .unwrap()
        .id;
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let d: ConflictDetail = c
        .call("conflicts.show", ConflictsShowParams { id: id.clone() })
        .unwrap();
    assert_eq!(d.conflict.id, id);
    let l = d.local.unwrap();
    assert_eq!(l.summary.as_deref(), Some("Local title"));
    assert_eq!(
        l.file,
        Some(fx.state().join("conflicts").join(&id).join("local.ics"))
    );
    assert!(l.file.unwrap().is_absolute());
    assert!(d.remote.is_none());
    let cur = d.current.unwrap();
    assert_eq!(cur.summary.as_deref(), Some("Remote title"));
    assert_eq!(cur.status.as_deref(), Some("COMPLETED"));
    assert_eq!(cur.file, None);
    assert!(d.vdir_file.ends_with("c1/u1.ics"));
}

#[test]
fn conflicts_show_unknown_id_is_not_found() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let e = c
        .call::<_, ConflictDetail>("conflicts.show", ConflictsShowParams { id: "nope".into() })
        .unwrap_err();
    assert_eq!(kind(&e), ErrorKind::NotFound);
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

// ---- journal methods ----

fn journal_list(c: &Client) -> Vec<JournalEntryView> {
    c.call("journal.list", empty()).unwrap()
}

fn status(c: &Client) -> StatusView {
    c.call("status.get", empty()).unwrap()
}

fn revert(
    c: &Client,
    request_id: &str,
    op_id: &str,
) -> Result<WriteResult, twydo_daemon::client::ClientError> {
    c.call(
        "journal.revert",
        JournalRevertParams {
            session: c.session().expect("hello"),
            request_id: request_id.into(),
            op_id: op_id.into(),
        },
    )
}

fn abandon(
    c: &Client,
    request_id: &str,
    op_id: &str,
) -> Result<AbandonResult, twydo_daemon::client::ClientError> {
    c.call(
        "journal.abandon",
        JournalAbandonParams {
            request_id: request_id.into(),
            op_id: op_id.into(),
        },
    )
}

fn jfx() -> Fixture {
    Fixture::new(&[("c1", "u1", "One", None), ("c1", "u2", "Two", None)])
}

/// Leaves a `Prepared` entry in the journal that creates `rel` (so recovery can finish it
/// unless `rel` already holds other content).
fn prepare(fx: &Fixture, id: &str, rel: &str) {
    let cfg = fx.config();
    let journal = Journal::open(&bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap()).unwrap();
    journal
        .prepare(&Entry {
            format: 1,
            id: id.into(),
            created: Utc::now(),
            kind: "edit".into(),
            uids: vec!["zz".into()],
            stage: Stage::Prepared,
            inverse_of: None,
            from_queue: None,
            request: None,
            synced: None,
            holds: Vec::new(),
            family: None,
            families: Vec::new(),
            writes: vec![Write {
                path: rel.into(),
                before: None,
                after: Some(todo_ics("zz", "Made by recovery", None)),
            }],
        })
        .unwrap();
}

#[test]
fn journal_list_matches_the_store() {
    let fx = jfx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    assert!(journal_list(&c).is_empty());
    let a = submit(&c, edit_title("u1", "One!")).unwrap();
    let b = submit(&c, edit_title("u2", "Two!")).unwrap();
    let l = journal_list(&c);
    assert_eq!(l.len(), 2);
    // Ids end in random hex, so two entries of one second may list in either order: the
    // daemon's list is the store's list, whatever its order.
    let cfg = fx.config();
    let journal = Journal::open(&bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap()).unwrap();
    let ids: Vec<_> = journal.list().unwrap().into_iter().map(|e| e.id).collect();
    assert_eq!(l.iter().map(|e| e.id.clone()).collect::<Vec<_>>(), ids);
    let find = |id: &str| l.iter().find(|e| e.id == id).unwrap();
    assert!(
        l.iter()
            .all(|e| e.stage == WireStage::Applied && e.synced.is_none())
    );
    assert_eq!(find(&a.op_id).uids, vec!["u1".to_string()]);
    assert_eq!(find(&b.op_id).uids, vec!["u2".to_string()]);
    assert!(find(&a.op_id).request_id.is_some());
    assert_ne!(find(&a.op_id).request_id, find(&b.op_id).request_id);
    assert_eq!(find(&a.op_id).kind, "edit");
}

#[test]
fn journal_recover_resumes_and_reports_stuck() {
    let fx = jfx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    subscribe(&c);
    // One entry recovery can finish, one it cannot (its file holds other content).
    prepare(&fx, "20261101T120000Z-0000000a", "c1/made.ics");
    fx.write("c1/blocked.ics", &todo_ics("other", "Other", None));
    prepare(&fx, "20261101T120001Z-0000000b", "c1/blocked.ics");
    let r: RecoverResult = c.call("journal.recover", empty()).unwrap();
    assert_eq!(r.resumed, vec!["20261101T120000Z-0000000a".to_string()]);
    assert_eq!(r.stuck.len(), 1);
    assert_eq!(r.stuck[0].id, "20261101T120001Z-0000000b");
    assert!(!r.stuck[0].reason.is_empty());
    assert_eq!(
        status(&c).journal.stuck,
        vec!["20261101T120001Z-0000000b".to_string()]
    );
    assert!(fx.read("c1/made.ics").contains("Made by recovery"));
    wait_event(&c, BOUND, |e| {
        matches!(
            e,
            Event::DataChanged {
                cause: Cause::Recovery,
                ..
            }
        )
    });
    // Nothing left to resume: the same stuck entry, and no resumed ones.
    let again: RecoverResult = c.call("journal.recover", empty()).unwrap();
    assert!(again.resumed.is_empty());
    assert_eq!(again.stuck, r.stuck);
}

#[test]
fn journal_recover_with_a_busy_vdir_is_unavailable_and_keeps_stuck() {
    use twydo_core::store::fsio::VdirLock;
    let fx = jfx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    fx.write("c1/blocked.ics", &todo_ics("other", "Other", None));
    prepare(&fx, "20261101T120001Z-0000000b", "c1/blocked.ics");
    let r: RecoverResult = c.call("journal.recover", empty()).unwrap();
    assert_eq!(r.stuck.len(), 1);
    let lock = VdirLock::shared(&fx.state()).unwrap();
    let e = c
        .call::<_, RecoverResult>("journal.recover", empty())
        .unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Unavailable, "{e}");
    assert_eq!(
        status(&c).journal.stuck,
        vec!["20261101T120001Z-0000000b".to_string()]
    );
    drop(lock);
    let again: RecoverResult = c.call("journal.recover", empty()).unwrap();
    assert_eq!(again.stuck, r.stuck);
}

#[test]
fn revert_by_id_from_another_session_keeps_stacks() {
    let fx = jfx();
    let td = TestDaemon::start(&fx);
    let a = td.client();
    let b = td.client();
    let first = submit(&a, edit_title("u1", "One!")).unwrap();
    submit(&a, edit_title("u2", "Two!")).unwrap();
    let r = revert(&b, "rv1", &first.op_id).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Applied);
    assert_eq!(r.message, "undone: One");
    assert!(r.undid.is_none());
    assert_eq!(task(&snapshot(&b), "u1").task.summary, "One");
    // A resend replays; the revert is not on B's stack, so B has nothing to undo.
    assert_eq!(revert(&b, "rv1", &first.op_id).unwrap().op_id, r.op_id);
    assert_eq!(journal_list(&b).len(), 3);
    let e = undo(&b).unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Invalid, "{e}");
    // A's undo still takes back A's own last edit, not the revert.
    let u = undo(&a).unwrap();
    assert_eq!(u.outcome, WriteOutcome::Applied);
    let snap = snapshot(&a);
    assert_eq!(task(&snap, "u2").task.summary, "Two");
    assert_eq!(task(&snap, "u1").task.summary, "One");
}

#[test]
fn revert_of_an_unknown_entry_is_not_found() {
    let fx = jfx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let e = revert(&c, "rv1", "nope").unwrap_err();
    assert_eq!(kind(&e), ErrorKind::NotFound, "{e}");
}

#[test]
fn revert_during_a_sync_is_queued() {
    let fx = jfx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let a = submit(&c, edit_title("u1", "One!")).unwrap();
    let _ = based(&c);
    let job = td.handle().hold_long(LongKind::Test);
    let r = revert(&c, "rv1", &a.op_id).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Queued);
    assert!(fx.read("c1/u1.ics").contains("One!"));
    drop(job);
    // The drain after the job applies it.
    let deadline = std::time::Instant::now() + BOUND;
    while !fx.read("c1/u1.ics").contains("SUMMARY:One\r") && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(fx.read("c1/u1.ics").contains("SUMMARY:One\r\n"));
}

#[test]
fn abandon_is_idempotent_and_clears_stuck() {
    let fx = jfx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    fx.write("c1/blocked.ics", &todo_ics("other", "Other", None));
    prepare(&fx, "20261101T120001Z-0000000b", "c1/blocked.ics");
    let r: RecoverResult = c.call("journal.recover", empty()).unwrap();
    assert_eq!(r.stuck.len(), 1);
    assert_eq!(status(&c).journal.stuck.len(), 1);
    assert_eq!(
        abandon(&c, "ab1", "20261101T120001Z-0000000b").unwrap(),
        AbandonResult { abandoned: true }
    );
    assert!(status(&c).journal.stuck.is_empty());
    let l = journal_list(&c);
    assert_eq!(l[0].stage, WireStage::Abandoned);
    // A resend replays; a new request finds it already abandoned.
    assert!(
        abandon(&c, "ab1", "20261101T120001Z-0000000b")
            .unwrap()
            .abandoned
    );
    assert!(
        !abandon(&c, "ab2", "20261101T120001Z-0000000b")
            .unwrap()
            .abandoned
    );
    // Writes continue.
    submit(&c, edit_title("u1", "One!")).unwrap();
    // An applied entry cannot be abandoned.
    let applied = journal_list(&c).last().unwrap().id.clone();
    let e = abandon(&c, "ab3", &applied).unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Invalid, "{e}");
    let e = abandon(&c, "ab4", "missing").unwrap_err();
    assert_eq!(kind(&e), ErrorKind::NotFound, "{e}");
}

/// Final review: writes a stuck entry held back drain once `journal.recover` resumes it (as
/// after `journal.abandon`), with no other trigger.
#[test]
fn journal_recover_drains_what_the_stuck_entry_held_back() {
    let fx = jfx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let job = td.handle().hold_long(LongKind::Test);
    let q = submit(&c, edit_title("u1", "One!")).unwrap();
    assert_eq!(q.outcome, WriteOutcome::Queued);
    // An unfinished entry recovery cannot finish yet: the drains after the job stop on it.
    fx.write("c1/blocked.ics", &todo_ics("other", "Other", None));
    prepare(&fx, "20261101T120001Z-0000000b", "c1/blocked.ics");
    drop(job);
    let deadline = std::time::Instant::now() + BOUND;
    while status(&c).drain_error.is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "the drain never stopped"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // Let the drainer started after the job finish its (failing) try.
    std::thread::sleep(Duration::from_millis(300));
    assert!(fx.read("c1/u1.ics").contains("SUMMARY:One\r\n"));
    // The blocking file goes; recovery can now finish the entry.
    fx.remove("c1/blocked.ics");
    let r: RecoverResult = c.call("journal.recover", empty()).unwrap();
    assert_eq!(r.resumed, vec!["20261101T120001Z-0000000b".to_string()]);
    while !fx.read("c1/u1.ics").contains("SUMMARY:One!") {
        assert!(
            std::time::Instant::now() < deadline + BOUND,
            "the held-back write never drained"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn journal_recover_finishes_the_queued_items_it_frees() {
    let fx = jfx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    subscribe(&c);
    let job = td.handle().hold_long(LongKind::Test);
    let q = submit(&c, edit_title("u1", "One!")).unwrap();
    assert_eq!(q.outcome, WriteOutcome::Queued);
    fx.write("c1/blocked.ics", &todo_ics("other", "Other", None));
    prepare(&fx, "20261101T120001Z-0000000b", "c1/blocked.ics");
    drop(job);
    let deadline = std::time::Instant::now() + BOUND;
    while status(&c).drain_error.is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "the drain never stopped"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    fx.remove("c1/blocked.ics");
    let _: RecoverResult = c.call("journal.recover", empty()).unwrap();
    let ev = wait_event(
        &c,
        BOUND,
        |e| matches!(e, Event::OpFinished { op_id, .. } if *op_id == q.op_id),
    );
    assert!(matches!(
        ev,
        Event::OpFinished {
            state: twydo_daemon::protocol::methods::OpState::Applied { .. },
            ..
        }
    ));
}

// ---- input.parse ----

fn parse_input(
    c: &Client,
    field: InputField,
    text: &str,
) -> Result<InputParsed, twydo_daemon::client::ClientError> {
    c.call(
        "input.parse",
        InputParseParams {
            field,
            text: text.into(),
            today: Some(chrono::NaiveDate::from_ymd_opt(2026, 11, 10).unwrap()),
            anchor: None,
        },
    )
}

#[test]
fn input_parse_due_and_repeat() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();

    let due = parse_input(&c, InputField::Due, "fri 5pm").unwrap();
    let want = chrono::NaiveDate::from_ymd_opt(2026, 11, 13)
        .unwrap()
        .and_hms_opt(17, 0, 0)
        .unwrap();
    assert_eq!(due.due, Some(twydo_core::edit::Due::Time(want)));
    let parsed = twydo_core::dateinput::parse("fri 5pm", today)
        .unwrap()
        .unwrap();
    assert_eq!(
        due.description,
        twydo_core::dateinput::describe(&parsed, today)
    );
    assert!(!due.cleared && due.repeat.is_none());

    let start = parse_input(&c, InputField::Start, "tomorrow").unwrap();
    assert_eq!(
        start.due,
        Some(twydo_core::edit::Due::Date(today.succ_opt().unwrap()))
    );

    let rep = parse_input(&c, InputField::Repeat, "2nd tue").unwrap();
    assert_eq!(rep.repeat.unwrap().rrule, "FREQ=MONTHLY;BYDAY=2TU");
    assert!(rep.due.is_none() && !rep.description.is_empty());

    let cleared = parse_input(&c, InputField::Due, "").unwrap();
    assert!(cleared.cleared && cleared.due.is_none());
    let cleared = parse_input(&c, InputField::Repeat, "none").unwrap();
    assert!(cleared.cleared && cleared.repeat.is_none());

    let e = parse_input(&c, InputField::Due, "9").unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Invalid);
}

#[test]
fn input_parse_describes_a_monthly_repeat_from_the_anchor() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let call = |anchor| -> InputParsed {
        c.call(
            "input.parse",
            InputParseParams {
                field: InputField::Repeat,
                text: "monthly".into(),
                today: Some(chrono::NaiveDate::from_ymd_opt(2026, 11, 10).unwrap()),
                anchor,
            },
        )
        .unwrap()
    };
    let anchored = call(Some(chrono::NaiveDate::from_ymd_opt(2026, 11, 14).unwrap()));
    assert!(anchored.description.contains("14"), "{anchored:?}");
    let plain = call(None);
    assert!(!plain.description.contains("14"), "{plain:?}");
}

#[test]
fn input_parse_defaults_to_today_in_the_configured_zone() {
    use chrono::Weekday;
    let fx = fx();
    let mut cfg = fx.config();
    cfg.timezone = Some("Pacific/Kiritimati".into());
    cfg.work_days = vec![Weekday::Mon, Weekday::Wed];
    let td_tz = cfg.tz();
    let td = TestDaemon::start_cfg(&fx, cfg, |_| {});
    let c = td.client();
    let tz = td_tz;
    let today = || Utc::now().with_timezone(&tz).date_naive();
    let before = today();
    let r: InputParsed = c
        .call(
            "input.parse",
            InputParseParams {
                field: InputField::Due,
                text: "tomorrow".into(),
                today: None,
                anchor: None,
            },
        )
        .unwrap();
    let after = today();
    let got = match r.due {
        Some(twydo_core::edit::Due::Date(d)) => d,
        other => panic!("{other:?}"),
    };
    assert!(
        got == before.succ_opt().unwrap() || got == after.succ_opt().unwrap(),
        "{got} is not tomorrow in the configured zone ({before})"
    );
    let r: InputParsed = c
        .call(
            "input.parse",
            InputParseParams {
                field: InputField::Repeat,
                text: "workdays".into(),
                today: None,
                anchor: None,
            },
        )
        .unwrap();
    assert_eq!(r.repeat.unwrap().rrule, "FREQ=WEEKLY;BYDAY=MO,WE");
}
