//! The write queue inside the daemon: writes during a long job are queued at once and shown
//! pending, drained when the job ends (and at startup), refusals stay in the status until
//! acknowledged. Synthetic fixture content.

mod common;

use std::time::{Duration, Instant};

use common::{
    Fixture, TestDaemon, count_titled, edit_title, empty, kind, snapshot, submit, task, todo_ics,
    undo,
};
use twydo_core::compat::Matrix;
use twydo_core::store::fsio::VdirLock;
use twydo_core::store::journal::RequestRef;
use twydo_core::store::queue::Queue;
use twydo_core::store::{Store, Submitted, bind_vdir_state};
use twydo_daemon::client::{Client, Notification};
use twydo_daemon::protocol::events::{Cause, Event};
use twydo_daemon::protocol::methods::{
    AckParams, AckResult, OpGetParams, OpState, StatusView, WriteOutcome,
};
use twydo_daemon::protocol::rpc::ErrorKind;
use twydo_daemon::protocol::wire::WireSnapshot;
use twydo_daemon::server::lane::LongKind;

const BOUND: Duration = Duration::from_secs(5);

fn fx() -> Fixture {
    Fixture::new(&[("inbox", "u1", "One", None), ("inbox", "u2", "Two", None)])
}

/// Polls `snapshot.get` until `ok` holds, within [`BOUND`].
fn snapshot_until(c: &Client, what: &str, ok: impl Fn(&WireSnapshot) -> bool) -> WireSnapshot {
    let deadline = Instant::now() + BOUND;
    loop {
        let s = snapshot(c);
        if ok(&s) {
            return s;
        }
        assert!(Instant::now() < deadline, "{what}: {s:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Polls `status.get` until `ok` holds, within [`BOUND`].
fn status_until(c: &Client, what: &str, ok: impl Fn(&StatusView) -> bool) -> StatusView {
    let deadline = Instant::now() + BOUND;
    loop {
        let s: StatusView = c.call("status.get", empty()).unwrap();
        if ok(&s) {
            return s;
        }
        assert!(Instant::now() < deadline, "{what}: {s:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The next `op.finished` for `op_id`, within [`BOUND`].
fn op_finished(c: &Client, op_id: &str) -> (Option<String>, OpState) {
    let deadline = Instant::now() + BOUND;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match c.notifications().recv_timeout(left) {
            Ok(Notification::Event(ev)) => {
                if let Event::OpFinished {
                    op_id: id,
                    request_id,
                    state,
                } = ev.event
                    && id == op_id
                {
                    return (request_id, state);
                }
            }
            Ok(_) => {}
            Err(e) => panic!("no op.finished for {op_id}: {e}"),
        }
    }
}

fn op_get(c: &Client, op_id: Option<&str>, request_id: Option<&str>) -> OpState {
    c.call(
        "op.get",
        OpGetParams {
            op_id: op_id.map(str::to_string),
            request_id: request_id.map(str::to_string),
        },
    )
    .unwrap()
}

fn state_dir(fx: &Fixture) -> std::path::PathBuf {
    let cfg = fx.config();
    bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap()
}

fn queue_len(fx: &Fixture) -> usize {
    Queue::existing(&state_dir(fx)).map_or(0, |q| q.len().unwrap())
}

#[test]
fn an_edit_during_a_long_job_is_queued_at_once_and_shown_pending() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let job = td.handle().hold_long(LongKind::Test);
    let t = Instant::now();
    let r = submit(&c, edit_title("u1", "Renamed")).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Queued);
    assert!(
        t.elapsed() < Duration::from_millis(200),
        "never waits for the lane"
    );
    let snap = snapshot_until(&c, "the queued row is never shown pending", |s| {
        task(s, "u1").pending
    });
    let u1 = task(&snap, "u1");
    assert_eq!(u1.task.summary, "Renamed");
    assert!(!task(&snap, "u2").pending);
    assert!(!fx.read("inbox/u1.ics").contains("Renamed"));
    drop(job);
}

#[test]
fn queued_row_is_pending_while_the_vdir_lock_is_held() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let _: serde_json::Value = c.call("events.subscribe", empty()).unwrap();
    let job = td.handle().hold_long(LongKind::Test);
    // As a sync's reconcile step does: no reader gets the vdir while this is held.
    let lock = VdirLock::exclusive(&state_dir(&fx)).unwrap();
    let r = submit(&c, edit_title("u1", "Renamed")).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Queued);
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match c.notifications().recv_timeout(left) {
            Ok(Notification::Event(ev)) if matches!(ev.event, Event::DataChanged { .. }) => break,
            Ok(_) => {}
            Err(e) => panic!("no data.changed while the vdir lock is held: {e}"),
        }
    }
    let snap = snapshot(&c);
    assert!(task(&snap, "u1").pending);
    assert_eq!(task(&snap, "u1").task.summary, "Renamed");
    assert!(!task(&snap, "u2").pending);
    drop(lock);
    drop(job);
}

#[test]
fn overlay_is_not_blocked_by_a_waiting_watcher_reload() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let _: serde_json::Value = c.call("events.subscribe", empty()).unwrap();
    let job = td.handle().hold_long(LongKind::Test);
    let lock = VdirLock::exclusive(&state_dir(&fx)).unwrap();
    std::thread::scope(|scope| {
        let publisher = td.handle().publisher_for_test();
        let reload = scope.spawn(move || publisher.reload(Cause::Op, Vec::new()));
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            !reload.is_finished(),
            "the reload must wait for the vdir lock"
        );
        let r = submit(&c, edit_title("u1", "Renamed")).unwrap();
        assert_eq!(r.outcome, WriteOutcome::Queued);
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match c.notifications().recv_timeout(left) {
                Ok(Notification::Event(ev)) if matches!(ev.event, Event::DataChanged { .. }) => {
                    break;
                }
                Ok(_) => {}
                Err(e) => panic!("overlay blocked behind a waiting reload: {e}"),
            }
        }
        assert!(task(&snapshot(&c), "u1").pending);
        drop(lock);
        reload.join().unwrap().unwrap();
    });
    drop(job);
}

#[test]
fn the_drain_after_the_job_applies_and_reports_op_finished() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let _: serde_json::Value = c.call("events.subscribe", empty()).unwrap();
    let job = td.handle().hold_long(LongKind::Test);
    let rid = c.next_request_id();
    let r = common::submit_at(&c, common::based(&c), &rid, edit_title("u1", "Renamed")).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Queued);
    drop(job);
    let (request_id, state) = op_finished(&c, &r.op_id);
    assert_eq!(request_id.as_deref(), Some(rid.as_str()));
    let OpState::Applied { op_id } = state else {
        panic!("not applied: {state:?}");
    };
    assert_ne!(op_id, r.op_id, "applied names the journal entry");
    assert!(fx.read("inbox/u1.ics").contains("SUMMARY:Renamed"));
    // Published before the event went out.
    let u1 = task(&snapshot(&c), "u1").clone();
    assert!(!u1.pending);
    assert_eq!(u1.task.summary, "Renamed");
    assert_eq!(queue_len(&fx), 0);
}

#[test]
fn a_queued_edit_refused_at_drain_stays_in_status_until_acknowledged() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let _: serde_json::Value = c.call("events.subscribe", empty()).unwrap();
    let job = td.handle().hold_long(LongKind::Test);
    let rid = c.next_request_id();
    let r = common::submit_at(&c, common::based(&c), &rid, edit_title("u1", "Renamed")).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Queued);
    fx.write("inbox/u1.ics", &todo_ics("u1", "One from outside", None));
    drop(job);
    let (_, state) = op_finished(&c, &r.op_id);
    let OpState::Refused { kind: k, message } = state else {
        panic!("not refused: {state:?}");
    };
    assert_eq!(k, ErrorKind::Stale);
    assert_eq!(message, "Not saved: \"One\" changed on another device");
    assert!(fx.read("inbox/u1.ics").contains("One from outside"));

    // The status was refreshed before the event: the refusal is there at once.
    let st: StatusView = c.call("status.get", empty()).unwrap();
    assert_eq!(st.refusals.len(), 1, "{st:?}");
    let rv = &st.refusals[0];
    assert_eq!(rv.id, r.op_id);
    assert_eq!(rv.message, message);
    assert_eq!(rv.session, c.session());
    assert_eq!(rv.request_id.as_deref(), Some(rid.as_str()));

    // Acknowledging needs a hello and a request id.
    let e = td
        .raw()
        .call::<_, AckResult>(
            "refusals.ack",
            AckParams {
                request_id: "x-1".into(),
                ids: vec![r.op_id.clone()],
            },
        )
        .unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Invalid);
    assert!(e.to_string().contains("say hello first"), "{e}");
    let e = c
        .call::<_, AckResult>(
            "refusals.ack",
            AckParams {
                request_id: String::new(),
                ids: vec![r.op_id.clone()],
            },
        )
        .unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Invalid);

    let ack_id = c.next_request_id();
    let params = AckParams {
        request_id: ack_id,
        ids: vec![r.op_id.clone(), "unknown".into()],
    };
    let a: AckResult = c.call("refusals.ack", params.clone()).unwrap();
    assert_eq!(a.acknowledged, 1);
    // A resend replays the first answer.
    let again: AckResult = c.call("refusals.ack", params).unwrap();
    assert_eq!(again, a);
    status_until(&c, "the acknowledged refusal stays", |s| {
        s.refusals.is_empty()
    });
    // op.get still reports the refusal.
    assert!(matches!(
        op_get(&c, Some(&r.op_id), None),
        OpState::Refused { .. }
    ));
}

#[test]
fn undo_of_a_still_queued_edit_removes_it() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let job = td.handle().hold_long(LongKind::Test);
    let q = submit(&c, edit_title("u1", "Renamed")).unwrap();
    assert_eq!(q.outcome, WriteOutcome::Queued);
    snapshot_until(&c, "pending", |s| task(s, "u1").pending);
    let u = undo(&c).unwrap();
    assert_eq!(u.outcome, WriteOutcome::Applied);
    assert_eq!(u.op_id, q.op_id);
    assert_eq!(u.message, "undone: One");
    assert_eq!(queue_len(&fx), 0);
    // The overlay without the item is published.
    let snap = snapshot_until(&c, "the cancelled row stays pending", |s| {
        !task(s, "u1").pending
    });
    assert_eq!(task(&snap, "u1").task.summary, "One");
    drop(job);
    assert_eq!(count_titled(&td, "Renamed"), 0);
    assert_eq!(op_get(&c, Some(&q.op_id), None), OpState::Cancelled);
}

#[test]
fn undo_skips_a_refused_queued_edit() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let a = submit(&c, edit_title("u2", "U2")).unwrap();
    assert_eq!(a.outcome, WriteOutcome::Applied);
    let job = td.handle().hold_long(LongKind::Test);
    let q = submit(&c, edit_title("u1", "Renamed")).unwrap();
    assert_eq!(q.outcome, WriteOutcome::Queued);
    fx.write("inbox/u1.ics", &todo_ics("u1", "One from outside", None));
    drop(job);
    assert!(matches!(
        op_get(&c, Some(&q.op_id), None),
        OpState::Refused { .. }
    ));
    let u = undo(&c).unwrap();
    assert_eq!(u.outcome, WriteOutcome::Applied);
    assert_eq!(u.undid.as_ref().map(|t| t.title.as_str()), Some("Two"));
    assert_eq!(
        u.undid.as_ref().and_then(|t| t.op_id.as_deref()),
        Some(a.op_id.as_str())
    );
    assert_eq!(u.message, "undone: Two");
    assert!(fx.read("inbox/u2.ics").contains("SUMMARY:Two"));
    assert!(fx.read("inbox/u1.ics").contains("One from outside"));
}

#[test]
fn op_get_follows_a_queued_op_to_its_outcome() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let job = td.handle().hold_long(LongKind::Test);
    let rid = c.next_request_id();
    let q = common::submit_at(&c, common::based(&c), &rid, edit_title("u1", "Renamed")).unwrap();
    assert_eq!(op_get(&c, Some(&q.op_id), None), OpState::Pending);
    assert_eq!(op_get(&c, None, Some(&rid)), OpState::Pending);
    drop(job);
    let by_id = op_get(&c, Some(&q.op_id), None);
    let OpState::Applied { op_id } = &by_id else {
        panic!("not applied: {by_id:?}");
    };
    assert_ne!(op_id, &q.op_id);
    assert_eq!(op_get(&c, None, Some(&rid)), by_id);
}

#[test]
fn an_outside_lock_at_drain_leaves_items_for_the_next_drain() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let job = td.handle().hold_long(LongKind::Test);
    let q = submit(&c, edit_title("u1", "Renamed")).unwrap();
    let outside = VdirLock::exclusive(&state_dir(&fx)).unwrap();
    drop(job);
    assert_eq!(op_get(&c, Some(&q.op_id), None), OpState::Pending);
    drop(outside);
    // Not an error: the status names no drain problem.
    let st: StatusView = c.call("status.get", empty()).unwrap();
    assert_eq!(st.drain_error, None);
    drop(td.handle().hold_long(LongKind::Test));
    assert!(matches!(
        op_get(&c, Some(&q.op_id), None),
        OpState::Applied { .. }
    ));
    assert!(fx.read("inbox/u1.ics").contains("SUMMARY:Renamed"));
}

#[test]
fn queued_items_left_by_an_earlier_daemon_drain_at_startup() {
    let fx = fx();
    let cfg = fx.config();
    let state = state_dir(&fx);
    {
        // A sync of an earlier owner holds the vdir: the edit is queued.
        let _lock = VdirLock::shared(&state).unwrap();
        let mut store = Store::without_recovery(&cfg, Matrix::embedded().clone()).unwrap();
        store.set_request(Some(RequestRef {
            session: "s-earlier".into(),
            request_id: "s-earlier-1-1".into(),
        }));
        let snap = fx.snapshot();
        let s = store
            .submit(
                &snap,
                &edit_title("u1", "Renamed"),
                "One",
                "daemon",
                chrono::Utc::now(),
            )
            .unwrap();
        assert!(matches!(s, Submitted::Queued(_)), "{s:?}");
    }
    assert_eq!(queue_len(&fx), 1);
    let td = TestDaemon::start(&fx);
    let c = td.client();
    assert!(fx.read("inbox/u1.ics").contains("SUMMARY:Renamed"));
    assert_eq!(queue_len(&fx), 0);
    assert!(matches!(
        op_get(&c, None, Some("s-earlier-1-1")),
        OpState::Applied { .. }
    ));
    let u1 = task(&snapshot(&c), "u1").clone();
    assert_eq!(u1.task.summary, "Renamed");
    assert!(!u1.pending);
}

#[test]
fn an_edit_queued_while_the_drain_runs_is_drained_after_the_job() {
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    let fx = fx();
    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    // The first drain after its last pass waits for `go`: the job still holds the lane.
    let gate = Arc::new(Mutex::new(Some((entered_tx, go_rx))));
    let td = TestDaemon::start_with(&fx, |o| {
        let gate = Arc::clone(&gate);
        o.after_drain_passes = Some(Arc::new(move || {
            let taken = gate.lock().unwrap().take();
            if let Some((tx, rx)) = taken {
                tx.send(()).unwrap();
                let _ = rx.recv_timeout(BOUND);
            }
        }));
    });
    let c = td.client();
    let _: serde_json::Value = c.call("events.subscribe", empty()).unwrap();
    let job = td.handle().hold_long(LongKind::Test);
    let a = submit(&c, edit_title("u1", "Renamed")).unwrap();
    assert_eq!(a.outcome, WriteOutcome::Queued);
    std::thread::scope(|scope| {
        let ending = scope.spawn(move || drop(job));
        entered_rx
            .recv_timeout(BOUND)
            .expect("the drain never reached its end");
        assert_eq!(
            td.handle().lane_for_test().current_long(),
            Some(LongKind::Test)
        );
        let b = submit(&c, edit_title("u2", "Late")).unwrap();
        assert_eq!(b.outcome, WriteOutcome::Queued);
        go_tx.send(()).unwrap();
        ending.join().unwrap();
        let (_, state) = op_finished(&c, &b.op_id);
        assert!(matches!(state, OpState::Applied { .. }), "{state:?}");
    });
    assert!(fx.read("inbox/u1.ics").contains("SUMMARY:Renamed"));
    assert!(fx.read("inbox/u2.ics").contains("SUMMARY:Late"));
    snapshot_until(&c, "the late edit stays pending", |s| {
        !task(s, "u2").pending && task(s, "u2").task.summary == "Late"
    });
    assert_eq!(queue_len(&fx), 0);
}

#[test]
fn an_undo_during_a_background_drain_that_refuses_its_item_does_not_deadlock() {
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    let fx = fx();
    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    // The background drain waits after its passes, still holding the write lane.
    let gate = Arc::new(Mutex::new(Some((entered_tx, go_rx))));
    let td = TestDaemon::start_with(&fx, |o| {
        let gate = Arc::clone(&gate);
        o.after_drain_passes = Some(Arc::new(move || {
            let taken = gate.lock().unwrap().take();
            if let Some((tx, rx)) = taken {
                tx.send(()).unwrap();
                let _ = rx.recv_timeout(BOUND);
            }
        }));
    });
    let c = td.client();
    // An outside lock holder: the edit is queued, then made stale before the drain sees it.
    let lock = VdirLock::exclusive(&state_dir(&fx)).unwrap();
    let r = submit(&c, edit_title("u1", "Renamed")).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Queued);
    fx.write("inbox/u1.ics", &todo_ics("u1", "Outside", None));
    drop(lock);
    entered_rx
        .recv_timeout(BOUND)
        .expect("the background drain never finished its passes");
    let caller = c.caller();
    let params = twydo_daemon::protocol::methods::UndoParams {
        session: c.session().unwrap(),
        request_id: c.next_request_id(),
    };
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let r = caller.call::<_, twydo_daemon::protocol::methods::WriteResult>("op.undo", params);
        let _ = done_tx.send(r);
    });
    // Let the undo take its session and wait for the lane, then let the drain go on.
    std::thread::sleep(Duration::from_millis(200));
    go_tx.send(()).unwrap();
    let undone = done_rx
        .recv_timeout(BOUND)
        .expect("the undo never returned: the drain and the undo deadlocked");
    // The refused edit left the undo stack: nothing is left to undo.
    assert_eq!(kind(&undone.unwrap_err()), ErrorKind::Invalid);
    // Writes still go through.
    let w = submit(&c, edit_title("u2", "After")).unwrap();
    assert_eq!(w.outcome, WriteOutcome::Applied);
}

#[test]
fn a_resent_undo_that_withdrew_a_queued_edit_is_answered_after_a_restart() {
    let fx = fx();
    let (edit_rid, undo_rid);
    {
        let td = TestDaemon::start(&fx);
        let c = td.client();
        let job = td.handle().hold_long(LongKind::Test);
        edit_rid = c.next_request_id();
        let q = common::submit_at(&c, common::based(&c), &edit_rid, edit_title("u1", "Queued"))
            .unwrap();
        assert_eq!(q.outcome, WriteOutcome::Queued);
        undo_rid = c.next_request_id();
        let u = common::undo_with(&c, &undo_rid).unwrap();
        assert_eq!(u.outcome, WriteOutcome::Applied);
        drop(job);
    }
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let later = submit(&c, edit_title("u1", "Later")).unwrap();
    assert_eq!(later.outcome, WriteOutcome::Applied);
    // Resent from the new session: answered from disk, the later edit stays.
    let again = common::undo_with(&c, &undo_rid).unwrap();
    assert_eq!(again.outcome, WriteOutcome::Applied);
    assert_eq!(again.message, "already undone");
    assert_eq!(task(&snapshot(&c), "u1").task.summary, "Later");
    // The withdrawn edit's own request is not run again either.
    let edit =
        common::submit_at(&c, common::based(&c), &edit_rid, edit_title("u1", "Queued")).unwrap();
    assert_eq!(edit.outcome, WriteOutcome::Queued);
    assert_eq!(op_get(&c, Some(&edit.op_id), None), OpState::Cancelled);
    assert_eq!(op_get(&c, None, Some(&edit_rid)), OpState::Cancelled);
    assert_eq!(task(&snapshot(&c), "u1").task.summary, "Later");
    assert_eq!(queue_len(&fx), 0);
}

#[test]
fn a_resent_undo_cut_short_by_a_crash_is_refused_not_rerun() {
    use twydo_core::store::queue::{Cancellation, FORMAT};
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let _: serde_json::Value = c.call("events.subscribe", empty()).unwrap();
    let job = td.handle().hold_long(LongKind::Test);
    let q = submit(&c, edit_title("u1", "Queued")).unwrap();
    assert_eq!(q.outcome, WriteOutcome::Queued);
    // The record an undo writes before removing the item, left alone by a crash.
    let queue = Queue::existing(&state_dir(&fx)).unwrap();
    let item = queue.list().unwrap().remove(0);
    let rec = Cancellation {
        format: FORMAT,
        item: item.clone(),
        by: Some(RequestRef {
            session: "s-gone".into(),
            request_id: "s-gone-1-1".into(),
        }),
        at: chrono::Utc::now(),
    };
    let dir = state_dir(&fx).join("queue/cancelled");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{}.json", item.id)),
        serde_json::to_vec(&rec).unwrap(),
    )
    .unwrap();
    let e = common::undo_with(&c, "s-gone-1-1").unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Blocked);
    // Nothing was undone: the edit is still queued and drains when the job ends.
    assert_eq!(queue_len(&fx), 1);
    drop(job);
    let (_, state) = op_finished(&c, &q.op_id);
    assert!(matches!(state, OpState::Applied { .. }), "{state:?}");
}

#[test]
fn an_undo_names_the_step_it_reverted_when_a_drain_refuses_one_meanwhile() {
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    let fx = fx();
    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    // Armed below: the background drain then waits before its passes, holding the lane.
    type Gate = Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>;
    let gate: Arc<Mutex<Gate>> = Arc::new(Mutex::new(None));
    let td = TestDaemon::start_with(&fx, |o| {
        let gate = Arc::clone(&gate);
        o.before_drain_passes = Some(Arc::new(move || {
            let taken = gate.lock().unwrap().take();
            if let Some((tx, rx)) = taken {
                tx.send(()).unwrap();
                let _ = rx.recv_timeout(BOUND);
            }
        }));
    });
    let c = td.client();
    let first = submit(&c, edit_title("u1", "First")).unwrap();
    assert_eq!(first.outcome, WriteOutcome::Applied);
    *gate.lock().unwrap() = Some((entered_tx, go_rx));
    // An outside lock holder: the second edit is queued; the drainer picks it up.
    let lock = VdirLock::exclusive(&state_dir(&fx)).unwrap();
    let second = submit(&c, edit_title("u2", "Second")).unwrap();
    assert_eq!(second.outcome, WriteOutcome::Queued);
    entered_rx
        .recv_timeout(BOUND)
        .expect("the background drain never started");
    // Made stale before the drain sees it: the drain will refuse it.
    fx.write("inbox/u2.ics", &todo_ics("u2", "Outside", None));
    drop(lock);
    let caller = c.caller();
    let params = twydo_daemon::protocol::methods::UndoParams {
        session: c.session().unwrap(),
        request_id: c.next_request_id(),
    };
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let r = caller.call::<_, twydo_daemon::protocol::methods::WriteResult>("op.undo", params);
        let _ = done_tx.send(r);
    });
    // Let the undo take its session and wait for the lane, then let the drain refuse.
    std::thread::sleep(Duration::from_millis(200));
    go_tx.send(()).unwrap();
    let undone = done_rx.recv_timeout(BOUND).unwrap().unwrap();
    // The store skipped the refused edit and reverted the first one; the reply says so.
    assert_eq!(undone.outcome, WriteOutcome::Applied);
    let undid = undone.undid.expect("names what it undid");
    assert_eq!(undid.op_id, Some(first.op_id.clone()));
    assert!(undone.message.contains("One"), "{}", undone.message);
    assert_eq!(task(&snapshot(&c), "u1").task.summary, "One");
    // Redo names it too.
    let redone = common::redo(&c).unwrap();
    assert_eq!(
        redone.undid.expect("names what it redid").op_id,
        Some(undone.op_id.clone())
    );
    assert_eq!(task(&snapshot(&c), "u1").task.summary, "First");
}

/// Final review: with no long job, an outside holder of the vdir lock leaves the overlay
/// refresher waiting in a reload (`request_overlay_publish`'s reload path). Shutdown still
/// finishes within its bound, and nothing is published after it.
#[test]
fn shutdown_does_not_wait_for_a_reload_stuck_behind_an_outside_lock() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let lock = VdirLock::exclusive(&state_dir(&fx)).unwrap();
    // No long job: the edit is queued (the vdir is busy) and its overlay reloads the vdir,
    // which waits for the lock.
    let r = submit(&c, edit_title("u1", "Renamed")).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Queued);
    assert!(
        !td.handle()
            .refresher_idle_for_test(Duration::from_millis(300)),
        "the refresher must be waiting for the vdir lock"
    );
    let publisher = td.handle().publisher_for_test();
    let before = publisher.current().0;
    let t = Instant::now();
    td.handle().shutdown();
    assert!(t.elapsed() < BOUND, "shutdown took {:?}", t.elapsed());
    drop(lock);
    // The refresher gets the lock now; the closed publisher must publish nothing.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(publisher.current().0, before);
    assert!(fx.read("inbox/u1.ics").contains("SUMMARY:One"));
}
