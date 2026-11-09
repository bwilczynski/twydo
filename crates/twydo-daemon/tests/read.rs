//! `snapshot.get` and `status.get`. Synthetic fixture content.

mod common;

use chrono::{TimeZone, Utc};
use common::{Fixture, TestDaemon, empty};
use twydo_core::compat::Matrix;
use twydo_core::edit::{Change, Op};
use twydo_core::store::fsio::VdirLock;
use twydo_core::store::queue::{Queue, refusal_message};
use twydo_core::store::{Store, Submitted, bind_vdir_state};
use twydo_daemon::protocol::methods::StatusView;
use twydo_daemon::protocol::wire::WireSnapshot;

fn fx() -> Fixture {
    Fixture::new(&[
        ("inbox", "uid-a", "Alpha", None),
        ("work", "uid-b", "Beta", None),
    ])
}

/// Queues an edit of `uid`'s summary while an outside holder keeps the vdir lock.
fn queue_edit(fx: &Fixture, uid: &str, text: &str) -> twydo_core::store::queue::Item {
    let cfg = fx.config();
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let _lock = VdirLock::exclusive(&state).unwrap();
    let (mut store, _) = Store::open_with(&cfg, Matrix::embedded().clone()).unwrap();
    let op = Op::Edit {
        uid: uid.into(),
        changes: vec![Change::Summary(text.into())],
    };
    let now = Utc.with_ymd_and_hms(2026, 11, 9, 12, 0, 0).unwrap();
    match store
        .submit(&fx.snapshot(), &op, "edit", "tui", now)
        .unwrap()
    {
        Submitted::Queued(item) => item,
        _ => panic!("expected the edit to be queued"),
    }
}

/// An outside sync's shared hold on the vdir: the daemon's startup drain finds it busy and
/// leaves queued items in place.
fn outside_sync(fx: &Fixture) -> VdirLock {
    let cfg = fx.config();
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    VdirLock::shared(&state).unwrap()
}

#[test]
fn snapshot_has_tasks_collections_and_load_errors() {
    let fx = fx();
    fx.write("inbox/bad.ics", "this is not an ics file");
    let td = TestDaemon::start(&fx);
    let snap: WireSnapshot = td.client().call("snapshot.get", empty()).unwrap();
    assert_eq!(snap.revision, 1);
    let mut uids: Vec<_> = snap.tasks.iter().map(|t| t.task.uid.as_str()).collect();
    uids.sort();
    assert_eq!(uids, ["uid-a", "uid-b"]);
    let mut cols: Vec<_> = snap.collections.iter().map(|c| c.id.as_str()).collect();
    cols.sort();
    assert_eq!(cols, ["inbox", "work"]);
    assert_eq!(snap.errors.len(), 1);
    assert_eq!(snap.errors[0].path, "inbox/bad.ics");
    assert!(snap.tasks.iter().all(|t| !t.pending && t.rev.len() == 16));
}

#[test]
fn snapshot_marks_queued_rows_pending() {
    let fx = fx();
    queue_edit(&fx, "uid-a", "Alpha queued");
    let _sync = outside_sync(&fx);
    let td = TestDaemon::start(&fx);
    let snap: WireSnapshot = td.client().call("snapshot.get", empty()).unwrap();
    let a = snap.tasks.iter().find(|t| t.task.uid == "uid-a").unwrap();
    let b = snap.tasks.iter().find(|t| t.task.uid == "uid-b").unwrap();
    assert!(a.pending);
    assert_eq!(a.task.summary, "Alpha queued");
    assert!(!b.pending);
}

#[test]
fn status_reports_pending_edits_and_unacknowledged_refusals() {
    let fx = fx();
    let item = queue_edit(&fx, "uid-a", "Alpha queued");
    let refused = {
        let cfg = fx.config();
        let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
        let q = Queue::open(&state).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 11, 9, 12, 5, 0).unwrap();
        assert!(q.claim(&item.id).unwrap());
        q.refuse(&item, "the file changed", false, now).unwrap();
        q.unshown().unwrap().remove(0)
    };
    let td = TestDaemon::start(&fx);
    let st: StatusView = td.client().call("status.get", empty()).unwrap();
    assert_eq!(st.refusals.len(), 1);
    assert_eq!(st.refusals[0].id, item.id);
    assert_eq!(st.refusals[0].message, refusal_message(&refused));
    assert_eq!(st.pending_edits, 0);
    assert!(st.status_rev >= 1);
}

#[test]
fn status_counts_queued_edits() {
    let fx = fx();
    queue_edit(&fx, "uid-a", "Alpha queued");
    let _sync = outside_sync(&fx);
    let td = TestDaemon::start(&fx);
    let st: StatusView = td.client().call("status.get", empty()).unwrap();
    assert_eq!(st.pending_edits, 1);
    assert!(st.refusals.is_empty());
}

#[test]
fn status_has_no_sync_without_a_sync_table() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let st: StatusView = td.client().call("status.get", empty()).unwrap();
    assert!(st.sync.is_none());
    assert_eq!(st.journal.unfinished, 0);
    assert!(st.journal.stuck.is_empty());
}

#[test]
fn reads_answer_while_a_reload_waits_for_the_vdir_lock() {
    use std::time::{Duration, Instant};
    use twydo_daemon::protocol::events::Cause;
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let before: WireSnapshot = c.call("snapshot.get", empty()).unwrap();
    let cfg = fx.config();
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let lock = VdirLock::exclusive(&state).unwrap();
    fx.write("inbox/uid-c.ics", &common::todo_ics("uid-c", "Gamma", None));
    std::thread::scope(|s| {
        let reload = s.spawn(|| td.handle().reload_for_test(Cause::Outside).unwrap());
        // Give the reload time to reach the lock wait.
        std::thread::sleep(Duration::from_millis(100));
        let t = Instant::now();
        let snap: WireSnapshot = c.call("snapshot.get", empty()).unwrap();
        let _: StatusView = c.call("status.get", empty()).unwrap();
        assert!(t.elapsed() < Duration::from_millis(500), "reads blocked");
        assert_eq!(snap.revision, before.revision);
        assert!(!reload.is_finished());
        drop(lock);
        assert_eq!(reload.join().unwrap(), before.revision + 1);
    });
    let after: WireSnapshot = c.call("snapshot.get", empty()).unwrap();
    assert_eq!(after.tasks.len(), before.tasks.len() + 1);
}
