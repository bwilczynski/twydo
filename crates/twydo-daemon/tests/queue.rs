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
use twydo_daemon::protocol::events::Event;
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
