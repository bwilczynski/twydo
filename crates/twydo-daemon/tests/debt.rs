//! Plan-1 review debt: behaviour of the write queue and the status refresher that had no
//! test. Synthetic fixture content.

mod common;

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use common::{
    Fixture, TestDaemon, create, edit_title, empty, queue_op, submit, submit_at, todo_ics,
};
use twydo_core::compat::Matrix;
use twydo_core::store::fsio::VdirLock;
use twydo_core::store::journal::{Entry, FromQueue, Journal, RequestRef, Stage, Write};
use twydo_core::store::queue::Queue;
use twydo_core::store::{Writer, bind_vdir_state};
use twydo_daemon::client::Client;
use twydo_daemon::protocol::methods::{
    AckParams, AckResult, OpGetParams, OpState, StatusView, WriteOutcome,
};
use twydo_daemon::server::lane::LongKind;

const BOUND: Duration = Duration::from_secs(5);

fn fx() -> Fixture {
    Fixture::new(&[("inbox", "u1", "One", None), ("inbox", "u2", "Two", None)])
}

fn state_dir(fx: &Fixture) -> std::path::PathBuf {
    let cfg = fx.config();
    bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap()
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

/// Waits until `fx`'s `rel` contains `text`, within [`BOUND`].
fn file_contains_until(fx: &Fixture, rel: &str, text: &str) {
    let deadline = Instant::now() + BOUND;
    while !fx.read(rel).contains(text) {
        assert!(Instant::now() < deadline, "{rel} never held {text}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A queued item left on disk by an earlier owner (the vdir was held while it was saved).
fn leave_queued_edit(fx: &Fixture, request_id: &str) {
    let _lock = VdirLock::shared(&state_dir(fx)).unwrap();
    queue_op(
        &fx.config(),
        &fx.snapshot(),
        &edit_title("u1", "Renamed"),
        "One",
        Some(RequestRef {
            session: "s-earlier".into(),
            request_id: request_id.into(),
        }),
        Utc::now(),
    );
}

#[test]
fn drain_error_is_reported_and_cleared() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let job = td.handle().hold_long(LongKind::Test);
    let q = submit(&c, edit_title("u1", "Renamed")).unwrap();
    assert_eq!(q.outcome, WriteOutcome::Queued);
    // An unfinished journal entry for an unrelated task, which recovery cannot finish (its
    // file holds neither the before nor the after text), blocks every drain.
    let other = |title: &str| todo_ics("other", title, None);
    fx.write("inbox/other.ics", &other("Outside"));
    let entry = Entry {
        format: 1,
        id: "unfinished-1".into(),
        created: Utc::now(),
        kind: "edit".into(),
        uids: vec!["other".into()],
        stage: Stage::Prepared,
        inverse_of: None,
        from_queue: None,
        request: None,
        synced: None,
        holds: Vec::new(),
        family: None,
        families: Vec::new(),
        writes: vec![Write {
            path: "inbox/other.ics".into(),
            before: Some(other("Before")),
            after: Some(other("After")),
        }],
    };
    Journal::open(&state_dir(&fx))
        .unwrap()
        .prepare(&entry)
        .unwrap();
    drop(job);
    let st = status_until(&c, "no drain error", |s| s.drain_error.is_some());
    assert!(
        st.drain_error.as_deref().unwrap().contains("unfinished"),
        "{st:?}"
    );
    assert!(!fx.read("inbox/u1.ics").contains("Renamed"));

    let writer = Writer::open(&fx.config(), Matrix::embedded().clone()).unwrap();
    writer
        .lock_waiting()
        .unwrap()
        .abandon("unfinished-1")
        .unwrap();
    drop(td.handle().hold_long(LongKind::Test));
    status_until(&c, "the drain error stays", |s| s.drain_error.is_none());
    file_contains_until(&fx, "inbox/u1.ics", "SUMMARY:Renamed");
}

#[test]
fn refusal_at_drain_with_the_session_gone_lands_in_status() {
    let fx = fx();
    let td = TestDaemon::start_with(&fx, |o| o.session_ttl = Duration::from_millis(50));
    let a = td.client();
    let observer = td.client();
    let job = td.handle().hold_long(LongKind::Test);
    let rid = a.next_request_id();
    let r = submit_at(&a, common::based(&a), &rid, edit_title("u1", "Renamed")).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Queued);
    fx.write("inbox/u1.ics", &todo_ics("u1", "One from outside", None));
    drop(a);
    // Long past the ttl: the next hello would find the session expired.
    std::thread::sleep(Duration::from_millis(300));
    drop(job);
    let st = status_until(&observer, "no refusal", |s| !s.refusals.is_empty());
    assert_eq!(st.refusals.len(), 1, "{st:?}");
    assert_eq!(st.refusals[0].request_id.as_deref(), Some(rid.as_str()));
    assert_eq!(st.refusals[0].id, r.op_id);
}

/// An item whose journal entry exists (the daemon died after the commit, before removing the
/// item) drains as applied, once: nothing is written again.
///
/// The brief's `already_applied_item_drains_as_satisfied` (an outside write of the same title
/// drains as `satisfied`) describes write-queue design §6.4 steps 3-4, which the store does not
/// implement: such an item is refused as stale. This pins the drain's "already applied" branch,
/// which had no daemon test.
#[test]
fn already_applied_item_drains_as_applied_without_a_second_write() {
    let fx = fx();
    leave_queued_edit(&fx, "s-earlier-1-1");
    let queue = Queue::existing(&state_dir(&fx)).unwrap();
    let item = queue.list().unwrap().remove(0);
    // The drain had claimed the item before it committed.
    assert!(queue.claim(&item.id).unwrap());
    let entry = Entry {
        format: 1,
        id: "applied-before-the-crash".into(),
        created: Utc::now(),
        kind: "edit".into(),
        uids: vec!["u1".into()],
        stage: Stage::Applied,
        inverse_of: None,
        from_queue: Some(FromQueue {
            id: item.id.clone(),
            queued: item.created,
        }),
        request: item.request.clone(),
        synced: None,
        holds: Vec::new(),
        family: None,
        families: Vec::new(),
        writes: Vec::new(),
    };
    Journal::open(&state_dir(&fx))
        .unwrap()
        .prepare(&entry)
        .unwrap();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let st: OpState = c
        .call(
            "op.get",
            OpGetParams {
                op_id: None,
                request_id: Some("s-earlier-1-1".into()),
            },
        )
        .unwrap();
    assert_eq!(
        st,
        OpState::Applied {
            op_id: entry.id.clone()
        }
    );
    assert_eq!(queue.len().unwrap(), 0);
    assert!(
        !fx.read("inbox/u1.ics").contains("Renamed"),
        "no second write"
    );
}

#[test]
fn queued_reply_without_a_job_triggers_a_drain() {
    let fx = fx();
    // The hook parks the submit after its item is on disk and before the reply is built.
    let (pushed_tx, pushed_rx) = mpsc::channel::<()>();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let chan = Mutex::new(Some((pushed_tx, go_rx)));
    let td = TestDaemon::start_with(&fx, |o| {
        o.before_queued_reply = Some(Arc::new(move || {
            if let Some((tx, rx)) = chan.lock().unwrap().take() {
                tx.send(()).unwrap();
                let _ = rx.recv_timeout(BOUND);
            }
        }));
    });
    let c = td.client();
    let job = td.handle().hold_long(LongKind::Test);
    let params = twydo_daemon::protocol::methods::SubmitParams {
        session: c.session().unwrap(),
        request_id: c.next_request_id(),
        based_on: common::based(&c),
        op: edit_title("u1", "Renamed"),
    };
    let caller = c.caller();
    let reply = std::thread::spawn(move || {
        caller.call::<_, twydo_daemon::protocol::methods::WriteResult>("op.submit", params)
    });
    pushed_rx
        .recv_timeout(BOUND)
        .expect("the item was never queued");

    // Swap the job's drain and release hooks for ones that do nothing, so ending the job
    // cannot apply the item: only the reply's own check can.
    let lane = td.handle().lane_for_test();
    let ran = Arc::new(Mutex::new(false));
    let flag = Arc::clone(&ran);
    lane.set_drain_hook(Box::new(move || *flag.lock().unwrap() = true));
    lane.set_release_hook(Box::new(|| {}));
    drop(job);
    assert!(*ran.lock().unwrap(), "the job's drain hook did not run");
    assert!(!fx.read("inbox/u1.ics").contains("Renamed"));
    go_tx.send(()).unwrap();

    let r = reply.join().unwrap().unwrap();
    assert_eq!(r.outcome, WriteOutcome::Queued);
    file_contains_until(&fx, "inbox/u1.ics", "SUMMARY:Renamed");
}

#[test]
fn startup_drain_skips_a_held_lane() {
    let fx = fx();
    leave_queued_edit(&fx, "s-earlier-1-1");
    assert_eq!(Queue::existing(&state_dir(&fx)).unwrap().len().unwrap(), 1);

    let (at_hook_tx, at_hook_rx) = mpsc::channel::<()>();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let td = TestDaemon::start_unready(&fx, |o| {
        o.before_recovery = Some(Box::new(move || {
            at_hook_tx.send(()).unwrap();
            let _ = go_rx.recv_timeout(BOUND);
        }));
    });
    at_hook_rx.recv_timeout(BOUND).expect("startup never began");
    std::thread::scope(|s| {
        let (held_tx, held_rx) = mpsc::channel::<()>();
        let (td, fx) = (&td, &fx);
        s.spawn(move || {
            let guard = td.handle().lane_for_test().long(LongKind::Test);
            held_tx.send(()).unwrap();
            // Held until startup has finished without draining.
            assert!(td.handle().wait_ready(BOUND), "never became ready");
            std::thread::sleep(Duration::from_millis(100));
            assert!(!fx.read("inbox/u1.ics").contains("Renamed"));
            drop(guard);
        });
        held_rx.recv_timeout(BOUND).unwrap();
        go_tx.send(()).unwrap();
    });
    file_contains_until(&fx, "inbox/u1.ics", "SUMMARY:Renamed");
}

#[test]
fn refresher_panic_does_not_stop_later_refreshes() {
    let fx = fx();
    let td = TestDaemon::start_with(&fx, |o| o.status_panic_once = true);
    let c = td.client();
    let job = td.handle().hold_long(LongKind::Test);
    let q = submit(&c, edit_title("u1", "Renamed")).unwrap();
    fx.write("inbox/u1.ics", &todo_ics("u1", "One from outside", None));
    drop(job);
    let st = status_until(&c, "no refusal", |s| !s.refusals.is_empty());
    let a: AckResult = c
        .call(
            "refusals.ack",
            AckParams {
                request_id: c.next_request_id(),
                ids: vec![q.op_id.clone()],
            },
        )
        .unwrap();
    assert_eq!(a.acknowledged, 1);
    let after = status_until(&c, "the ack never reached the status", |s| {
        s.refusals.is_empty()
    });
    assert!(after.status_rev > st.status_rev, "{st:?} {after:?}");
}

#[test]
fn an_unreadable_queue_item_is_named_in_the_status() {
    let fx = fx();
    let queue = Queue::open(&state_dir(&fx)).unwrap();
    let path = state_dir(&fx).join("queue/20261106T120000000000Z-00000001-0000000001.json");
    std::fs::write(&path, "{").unwrap();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let st = status_until(&c, "not reported", |s| !s.queue_unreadable.is_empty());
    assert_eq!(st.queue_unreadable, vec![path.display().to_string()]);
    assert_eq!(queue.unreadable().unwrap(), vec![path]);
}

/// Runs `f` on a thread and waits at most `limit` for its result.
fn within<T: Send + 'static>(limit: Duration, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(limit).expect("the call never answered")
}

#[test]
fn a_panicking_handler_answers_internal() {
    let fx = fx();
    let td = TestDaemon::start_with(&fx, |o| {
        o.test_point = Some(Arc::new(|p: &str| {
            if p == "request:status.get" {
                panic!("synthetic handler panic");
            }
        }));
    });
    let c = td.client();
    let caller = c.caller();
    let r = within(Duration::from_secs(5), move || {
        caller.call::<_, StatusView>("status.get", empty())
    });
    let e = r.unwrap_err();
    assert_eq!(
        common::kind(&e),
        twydo_daemon::protocol::rpc::ErrorKind::Internal
    );
    assert_eq!(
        common::err_data(&e).kind,
        twydo_daemon::protocol::rpc::ErrorKind::Internal
    );
    assert!(
        e.to_string()
            .contains("internal error: synthetic handler panic"),
        "{e}"
    );
    // The connection is still usable.
    let caller = c.caller();
    within(Duration::from_secs(5), move || {
        caller
            .call::<_, serde_json::Value>("daemon.info", empty())
            .unwrap()
    });
}

#[test]
fn a_poisoned_session_table_keeps_serving() {
    let fx = fx();
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hook = Arc::clone(&armed);
    let td = TestDaemon::start_with(&fx, |o| {
        o.test_point = Some(Arc::new(move |p: &str| {
            if p == "sessions_locked" && hook.swap(false, std::sync::atomic::Ordering::SeqCst) {
                panic!("synthetic panic holding the session table");
            }
        }));
    });
    armed.store(true, std::sync::atomic::Ordering::SeqCst);
    let socket = td.socket().to_path_buf();
    let p = td.hello_params();
    let first = within(Duration::from_secs(5), move || {
        Client::connect(&socket, p).map(|_| ())
    });
    assert!(first.is_err());
    let socket = td.socket().to_path_buf();
    let p = td.hello_params();
    let c = within(Duration::from_secs(5), move || {
        Client::connect(&socket, p)
            .expect("hello after poisoning")
            .0
    });
    let r = within(Duration::from_secs(5), move || {
        submit(&c, create("After poisoning")).unwrap()
    });
    assert_eq!(r.outcome, WriteOutcome::Applied);
}

#[test]
fn a_client_that_stops_reading_does_not_pin_the_daemon() {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let mut raw = UnixStream::connect(td.socket()).unwrap();
    let hello = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "hello",
        "params": td.hello_params()});
    let sub = serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "events.subscribe",
        "params": {}});
    writeln!(raw, "{hello}").unwrap();
    writeln!(raw, "{sub}").unwrap();
    // Each touch changes a title, so each reload publishes an event.
    for i in 0..10_000 {
        td.touch_outside("inbox/u1.ics", &todo_ics("u1", &format!("Title {i}"), None));
    }
    let started = Instant::now();
    let other = td.client();
    let t = Instant::now();
    let caller = other.caller();
    within(Duration::from_secs(5), move || {
        caller
            .call::<_, serde_json::Value>("daemon.info", empty())
            .unwrap()
    });
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    // Not reading for longer than the write timeout (5 s), the client is dropped; reading now
    // drains what was sent and then meets the close, within 10 s of the burst.
    std::thread::sleep(Duration::from_secs(6));
    raw.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let mut buf = [0u8; 65536];
    loop {
        match raw.read(&mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => assert!(
                e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut,
                "{e}"
            ),
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the daemon never closed the connection"
        );
    }
}

#[test]
fn session_recovery_io_runs_outside_the_table_lock() {
    let fx = fx();
    let slow = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hook = Arc::clone(&slow);
    let td = TestDaemon::start_with(&fx, |o| {
        o.test_point = Some(Arc::new(move |p: &str| {
            if p == "without_recovery" && hook.swap(false, std::sync::atomic::Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(2));
            }
        }));
    });
    // The first open is slow; the second, on another session, must not wait for it.
    slow.store(true, std::sync::atomic::Ordering::SeqCst);
    let (socket, p) = (td.socket().to_path_buf(), td.hello_params());
    let first = std::thread::spawn(move || Client::connect(&socket, p).map(|_| ()));
    std::thread::sleep(Duration::from_millis(300));
    let (socket, p) = (td.socket().to_path_buf(), td.hello_params());
    let t = Instant::now();
    within(Duration::from_secs(5), move || {
        Client::connect(&socket, p).expect("second hello").0
    });
    assert!(
        t.elapsed() < Duration::from_millis(500),
        "{:?}",
        t.elapsed()
    );
    first.join().unwrap().unwrap();
}
