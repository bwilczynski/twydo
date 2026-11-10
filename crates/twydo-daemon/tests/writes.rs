//! `op.submit`, `op.undo`, `op.redo`, `op.get` and `op.check`: sessions, replay and
//! `based_on`. Synthetic fixture content.

mod common;

use std::sync::{Arc, Barrier};
use std::time::Duration;

use common::{
    Fixture, TestDaemon, based, count_titled, create, edit_title, empty, err_data, kind, project,
    redo, snapshot, submit, submit_at, task, todo_ics, undo, undo_with,
};
use serde_json::json;
use twydo_core::compat::Matrix;
use twydo_core::config::WriteMode;
use twydo_core::edit::{NewTask, Op};
use twydo_core::model::Status;
use twydo_daemon::protocol::events::{Cause, Event};
use twydo_daemon::protocol::methods::{
    BasedOn, CheckParams, CheckResult, OpGetParams, OpState, SubmitParams, WriteOutcome,
    WriteResult,
};
use twydo_daemon::protocol::rpc::ErrorKind;
use twydo_daemon::protocol::wire::WireSnapshot;

fn fx() -> Fixture {
    Fixture::new(&[("inbox", "u1", "One", None), ("inbox", "u2", "Two", None)])
}

#[test]
fn a_create_is_applied_and_published() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let r = submit(&c, Op::Create(NewTask::plain("inbox", "Synthetic new"))).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Applied);
    let created = r.created.clone().expect("created");
    let snap: WireSnapshot = c.call("snapshot.get", json!({})).unwrap();
    assert_eq!(snap.revision, r.revision);
    assert!(snap.tasks.iter().any(|t| t.task.summary == "Synthetic new"));
    let t = task(&snap, &created.uid);
    assert_eq!(t.task.summary, "Synthetic new");
    assert_eq!(t.rev, created.rev);
    assert_eq!(r.message, "added");
}

#[test]
fn the_answer_carries_todays_message() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let r = submit(&c, Op::Complete { uid: "u1".into() }).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Applied);
    assert_eq!(r.message, "done");
    let u = undo(&c).unwrap();
    assert_eq!(u.message, "undone: One");
    let d = redo(&c).unwrap();
    assert_eq!(d.message, "redone: One");
}

#[test]
fn a_resent_request_returns_the_first_result_and_applies_once() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let based = based(&c);
    let id = c.next_request_id();
    let a = submit_at(&c, based.clone(), &id, create("X")).unwrap();
    let b = submit_at(&c, based, &id, create("X")).unwrap();
    assert_eq!(a.op_id, b.op_id);
    assert_eq!(count_titled(&td, "X"), 1);
}

#[test]
fn replay_comes_before_the_session_check() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let based = based(&c);
    let id = c.next_request_id();
    let a = submit_at(&c, based.clone(), &id, create("X")).unwrap();
    let b: WriteResult = c
        .call(
            "op.submit",
            SubmitParams {
                session: "s-unknown".into(),
                request_id: id.clone(),
                based_on: BasedOn {
                    epoch: "d-old".into(),
                    revision: 1,
                },
                op: create("X"),
            },
        )
        .unwrap();
    assert_eq!(a.op_id, b.op_id);
    assert_eq!(count_titled(&td, "X"), 1);
}

#[test]
fn an_unknown_session_is_not_found() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let e = c
        .call::<_, WriteResult>(
            "op.submit",
            SubmitParams {
                session: "s-unknown".into(),
                request_id: c.next_request_id(),
                based_on: based(&c),
                op: create("X"),
            },
        )
        .unwrap_err();
    assert_eq!(kind(&e), ErrorKind::NotFound);
    assert_eq!(e.to_string(), "unknown session; reconnect");
}

#[test]
fn a_missing_request_id_is_invalid() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let e = c
        .call::<_, WriteResult>(
            "op.submit",
            SubmitParams {
                session: c.session().unwrap(),
                request_id: String::new(),
                based_on: based(&c),
                op: create("X"),
            },
        )
        .unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Invalid);
    assert_eq!(count_titled(&td, "X"), 0);
}

#[test]
fn concurrent_resend_applies_once() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let a = td.client();
    let b = td.client();
    let based = based(&a);
    let id = a.next_request_id();
    let session = a.session().unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = [a.caller(), b.caller()]
        .into_iter()
        .map(|caller| {
            let barrier = Arc::clone(&barrier);
            let params = SubmitParams {
                session: session.clone(),
                request_id: id.clone(),
                based_on: based.clone(),
                op: create("X"),
            };
            std::thread::spawn(move || {
                barrier.wait();
                caller.call::<_, WriteResult>("op.submit", params).unwrap()
            })
        })
        .collect();
    let results: Vec<WriteResult> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results[0].op_id, results[1].op_id);
    assert_eq!(count_titled(&td, "X"), 1);
}

#[test]
fn a_write_from_another_epoch_is_stale() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let mut b = based(&c);
    b.epoch = "d-old".into();
    let e = submit_at(&c, b, &c.next_request_id(), create("X")).unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Stale);
    assert!(e.to_string().contains("reload and try again"), "{e}");
    assert_eq!(count_titled(&td, "X"), 0);
}

#[test]
fn a_revision_no_longer_kept_is_stale() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let first = based(&c);
    for i in 0..33 {
        td.touch_outside("inbox/u2.ics", &todo_ics("u2", &format!("Two {i}"), None));
    }
    let e = submit_at(&c, first, &c.next_request_id(), create("X")).unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Stale);
    assert!(e.to_string().contains("reload and try again"), "{e}");
    assert_eq!(count_titled(&td, "X"), 0);
}

#[test]
fn an_edit_of_a_task_changed_since_based_on_is_stale_with_its_current_rev() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let r = based(&c);
    let next = td.touch_outside("inbox/u1.ics", &todo_ics("u1", "One elsewhere", None));
    assert_eq!(next, r.revision + 1);
    let now = snapshot(&c);
    let e = submit_at(&c, r, &c.next_request_id(), edit_title("u1", "One here")).unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Stale);
    let data = err_data(&e);
    assert_eq!(data.uid.as_deref(), Some("u1"));
    assert_eq!(
        data.current_rev.as_deref(),
        Some(task(&now, "u1").rev.as_str())
    );
    assert_eq!(
        e.to_string(),
        "\"One\" changed on disk; reload and try again"
    );
    assert_eq!(count_titled(&td, "One here"), 0);
}

#[test]
fn a_family_operation_is_refused_when_a_member_was_deleted_since_based_on() {
    let fx = project::fixture();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let r = based(&c);
    std::fs::remove_file(
        fx.vdir()
            .join(format!("{}/{}.ics", project::LIST, project::P2)),
    )
    .unwrap();
    td.handle().reload_for_test(Cause::Outside).unwrap();
    let complete = Op::Complete {
        uid: project::P.into(),
    };
    let e = submit_at(&c, r, &c.next_request_id(), complete.clone()).unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Stale, "{e}");
    let snap = snapshot(&c);
    assert_eq!(task(&snap, project::P).task.status, Status::Open);
    // Planning on the newer snapshot succeeds: the refusal came from the old membership.
    let ok = submit(&c, complete).unwrap();
    assert_eq!(ok.outcome, WriteOutcome::Applied);
}

#[test]
fn a_family_write_changes_rows_the_client_did_not_name() {
    let fx = project::fixture();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let _: serde_json::Value = c.call("events.subscribe", empty()).unwrap();
    let r = submit(
        &c,
        Op::Complete {
            uid: project::P.into(),
        },
    )
    .unwrap();
    assert_eq!(r.outcome, WriteOutcome::Applied);
    let snap = snapshot(&c);
    for uid in [project::P, project::P1, project::P2, project::P3] {
        assert_eq!(task(&snap, uid).task.status, Status::Completed, "{uid}");
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let uids = loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match c.notifications().recv_timeout(left) {
            Ok(twydo_daemon::client::Notification::Event(ev)) => {
                if let Event::DataChanged {
                    cause: Cause::Op,
                    uids,
                    revision,
                } = ev.event
                {
                    assert_eq!(revision, r.revision);
                    break uids;
                }
            }
            Ok(_) => {}
            Err(e) => panic!("no data.changed: {e}"),
        }
    };
    for uid in [project::P1, project::P2, project::P3] {
        assert!(uids.iter().any(|u| u == uid), "{uid} not in {uids:?}");
    }
}

#[test]
fn undo_is_per_session() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let a = td.client();
    let b = td.client();
    assert_ne!(a.session(), b.session());
    submit(&a, edit_title("u1", "One edited")).unwrap();
    submit(&b, edit_title("u2", "Two edited")).unwrap();
    let u = undo(&a).unwrap();
    assert_eq!(u.undid.as_ref().unwrap().title, "One");
    let snap = snapshot(&a);
    assert_eq!(task(&snap, "u1").task.summary, "One");
    assert_eq!(task(&snap, "u2").task.summary, "Two edited");
    undo(&b).unwrap();
    let snap = snapshot(&a);
    assert_eq!(task(&snap, "u2").task.summary, "Two");
}

#[test]
fn a_resent_undo_does_not_undo_a_second_edit() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let a = td.client();
    submit(&a, edit_title("u1", "One edited")).unwrap();
    submit(&a, edit_title("u2", "Two edited")).unwrap();
    let id = a.next_request_id();
    let first = undo_with(&a, &id).unwrap();
    let snap = snapshot(&a);
    assert_eq!(task(&snap, "u2").task.summary, "Two");
    let again = undo_with(&a, &id).unwrap();
    assert_eq!(first.op_id, again.op_id);
    let snap = snapshot(&a);
    assert_eq!(task(&snap, "u1").task.summary, "One edited");
}

#[test]
fn a_session_survives_a_reconnect() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let a = td.client();
    let session = a.session().unwrap();
    submit(&a, edit_title("u1", "One edited")).unwrap();
    drop(a);
    let a = td.client_resuming(&session);
    assert_eq!(a.session().as_deref(), Some(session.as_str()));
    undo(&a).unwrap();
    assert_eq!(task(&snapshot(&a), "u1").task.summary, "One");
}

#[test]
fn a_session_expires_after_its_ttl() {
    let fx = fx();
    let td = TestDaemon::start_with(&fx, |o| o.session_ttl = Duration::from_millis(100));
    let a = td.client();
    let session = a.session().unwrap();
    submit(&a, edit_title("u1", "One edited")).unwrap();
    drop(a);
    // The timeout is the assertion: the session must be gone after its ttl.
    std::thread::sleep(Duration::from_millis(300));
    let a = td.client_resuming(&session);
    assert_ne!(a.session().as_deref(), Some(session.as_str()));
    let e = undo(&a).unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Invalid);
    assert_eq!(e.to_string(), "nothing to undo");
}

/// Every operation on a plain task is unverified for Twydo.
fn unverified_matrix() -> Matrix {
    let mut s = String::from("matrix_version = 1\n");
    for case in ["one-time"] {
        for op in ["create", "edit", "complete", "reopen", "move", "delete"] {
            s.push_str(&format!(
                "[[operation]]\ncase = \"{case}\"\nop = \"{op}\"\niphone_baseline = \"pass\"\n\
                 mac_to_iphone = \"unverified\"\niphone_to_mac = \"unverified\"\n\
                 twydo_to_iphone = \"unverified\"\niphone_to_twydo = \"unverified\"\n"
            ));
        }
    }
    Matrix::from_toml_str(&s).unwrap()
}

#[test]
fn the_gate_refusal_is_blocked_with_the_gates_reason() {
    let fx = fx();
    let mut cfg = fx.config();
    cfg.writes.mode = WriteMode::Verified;
    let td = TestDaemon::start_cfg(&fx, cfg.clone(), |o| o.matrix = unverified_matrix());
    let c = td.client();
    let op = Op::Delete { uid: "u1".into() };
    let snap = fx.snapshot();
    let today = chrono::Utc::now().with_timezone(&cfg.tz()).date_naive();
    let reason = twydo_core::gate::check(&snap, &unverified_matrix(), cfg.writes.mode, &op, today)
        .unwrap_err()
        .to_string();
    let e = submit(&c, op).unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Blocked);
    assert_eq!(e.to_string(), reason);
    assert!(fx.vdir().join("inbox/u1.ics").exists());
}

#[test]
fn op_get_reports_an_applied_request() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let id = c.next_request_id();
    let r = submit_at(&c, based(&c), &id, create("X")).unwrap();
    let by_request: OpState = c
        .call(
            "op.get",
            OpGetParams {
                op_id: None,
                request_id: Some(id),
            },
        )
        .unwrap();
    assert_eq!(
        by_request,
        OpState::Applied {
            op_id: r.op_id.clone()
        }
    );
    let by_op: OpState = c
        .call(
            "op.get",
            OpGetParams {
                op_id: Some(r.op_id.clone()),
                request_id: None,
            },
        )
        .unwrap();
    assert_eq!(by_op, OpState::Applied { op_id: r.op_id });
    let e = c
        .call::<_, OpState>(
            "op.get",
            OpGetParams {
                op_id: Some("20260101T000000Z-deadbeef".into()),
                request_id: None,
            },
        )
        .unwrap_err();
    assert_eq!(kind(&e), ErrorKind::NotFound);
}

#[test]
fn a_request_applied_before_a_restart_is_replayed_from_the_journal() {
    let fx = fx();
    let id;
    let first;
    {
        let td = TestDaemon::start(&fx);
        let c = td.client();
        id = c.next_request_id();
        first = submit_at(&c, based(&c), &id, create("X")).unwrap();
    }
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let again = submit_at(&c, based(&c), &id, create("X")).unwrap();
    assert_eq!(again.outcome, WriteOutcome::Applied);
    assert_eq!(again.op_id, first.op_id);
    assert_eq!(again.message, "already applied");
    assert_eq!(count_titled(&td, "X"), 1);
}

#[test]
fn op_check_never_writes() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let r: CheckResult = c
        .call(
            "op.check",
            CheckParams {
                based_on: based(&c),
                op: Op::Delete { uid: "u1".into() },
            },
        )
        .unwrap();
    assert!(r.allowed, "{r:?}");
    assert!(fx.vdir().join("inbox/u1.ics").exists());
}

#[test]
fn a_write_while_a_long_job_holds_the_lane_is_queued_and_its_undo_cancels_it() {
    use twydo_daemon::server::lane::LongKind;
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let long = td.handle().lane_for_test().long(LongKind::Test);
    let r = submit(&c, edit_title("u1", "One queued")).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Queued);
    assert_eq!(r.message, "saved · queued (a sync is running)");
    let pending: OpState = c
        .call(
            "op.get",
            OpGetParams {
                op_id: Some(r.op_id.clone()),
                request_id: None,
            },
        )
        .unwrap();
    assert_eq!(pending, OpState::Pending);
    let u = undo(&c).unwrap();
    assert_eq!(u.outcome, WriteOutcome::Applied);
    assert_eq!(u.op_id, r.op_id);
    assert_eq!(u.message, "undone: One");
    let gone: OpState = c
        .call(
            "op.get",
            OpGetParams {
                op_id: Some(r.op_id),
                request_id: None,
            },
        )
        .unwrap();
    assert_eq!(gone, OpState::Cancelled);
    drop(long);
    assert_eq!(count_titled(&td, "One queued"), 0);
}

#[test]
fn a_queued_reply_does_not_wait_for_a_reload_blocked_by_the_vdir_lock() {
    use std::time::Instant;
    use twydo_core::store::bind_vdir_state;
    use twydo_core::store::fsio::VdirLock;
    use twydo_daemon::protocol::methods::StatusView;
    use twydo_daemon::server::lane::LongKind;

    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let b = based(&c);
    let id = c.next_request_id();
    let cfg = fx.config();
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    // A sync holds the vdir, and a reload (holding the publisher's reload lock) waits for it.
    let vdir_lock = VdirLock::exclusive(&state).unwrap();
    std::thread::scope(|scope| {
        let reload = scope.spawn(|| td.handle().reload_for_test(Cause::Outside).unwrap());
        // Give the reload time to reach the vdir lock. If it has not, the test only gets
        // weaker, never flaky.
        std::thread::sleep(Duration::from_millis(100));
        let long = td.handle().lane_for_test().long(LongKind::Test);
        let (tx, rx) = std::sync::mpsc::channel();
        let caller = c.caller();
        let session = c.session().unwrap();
        scope.spawn(move || {
            let r = caller.call::<_, WriteResult>(
                "op.submit",
                SubmitParams {
                    session,
                    request_id: id,
                    based_on: b,
                    op: edit_title("u1", "One queued"),
                },
            );
            let _ = tx.send(r);
        });
        // Bounded: the old behaviour waited for the vdir lock, which is released only below.
        let got = rx.recv_timeout(Duration::from_millis(500));
        let reload_was_blocked = !reload.is_finished();
        drop(vdir_lock);
        reload.join().unwrap();
        let r = got.expect("the queued reply waited on the reload").unwrap();
        assert_eq!(r.outcome, WriteOutcome::Queued);
        assert!(reload_was_blocked, "the reload was not blocked");
        // Still inside the long job: the end of the job drains the edit.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let st: StatusView = c.call("status.get", empty()).unwrap();
            if st.pending_edits == 1 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "status never showed the queued edit: {st:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        drop(long);
    });
    assert_eq!(count_titled(&td, "One queued"), 1);
}

#[test]
fn a_resent_undo_refused_earlier_does_not_undo_a_later_edit() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let x = c.next_request_id();
    let first = undo_with(&c, &x).unwrap_err();
    assert_eq!(kind(&first), ErrorKind::Invalid);
    submit(&c, edit_title("u1", "One edited")).unwrap();
    let again = undo_with(&c, &x).unwrap_err();
    assert_eq!(kind(&again), ErrorKind::Invalid);
    assert_eq!(again.to_string(), first.to_string());
    assert_eq!(task(&snapshot(&c), "u1").task.summary, "One edited");
}

#[test]
fn an_unknown_session_refusal_is_not_replayed() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let x = c.next_request_id();
    let b = based(&c);
    let params = |session: String| SubmitParams {
        session,
        request_id: x.clone(),
        based_on: b.clone(),
        op: create("X"),
    };
    let e = c
        .call::<_, WriteResult>("op.submit", params("s-unknown".into()))
        .unwrap_err();
    assert_eq!(kind(&e), ErrorKind::NotFound);
    let r: WriteResult = c.call("op.submit", params(c.session().unwrap())).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Applied);
    assert_eq!(count_titled(&td, "X"), 1);
}

#[test]
fn shutdown_is_bounded_and_no_status_refresh_runs_after_it() {
    use std::time::Instant;
    use twydo_core::store::bind_vdir_state;
    use twydo_core::store::fsio::VdirLock;
    use twydo_daemon::server::lane::LongKind;

    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let b = based(&c);
    let id = c.next_request_id();
    let cfg = fx.config();
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let publisher = td.handle().publisher_for_test();
    let vdir_lock = VdirLock::exclusive(&state).unwrap();
    let (status_before, revision_before) = std::thread::scope(|scope| {
        // A reload waits for the vdir lock (it takes it before the publisher's reload lock).
        // The queued write's refresher publishes the pending overlay without the vdir lock,
        // so it finishes before shutdown starts; after `close()` nothing publishes.
        let reload = scope.spawn(|| td.handle().reload_for_test(Cause::Outside).unwrap());
        std::thread::sleep(Duration::from_millis(100));
        let long = td.handle().lane_for_test().long(LongKind::Test);
        let r = submit_at(&c, b, &id, edit_title("u1", "One queued")).unwrap();
        assert_eq!(r.outcome, WriteOutcome::Queued);
        assert!(
            td.handle().refresher_idle_for_test(Duration::from_secs(5)),
            "the overlay was never published"
        );
        let status_before = publisher.status().unwrap();
        let revision_before = publisher.current().0;

        let (tx, rx) = std::sync::mpsc::channel();
        let handle = td.handle();
        scope.spawn(move || {
            let t0 = Instant::now();
            handle.shutdown();
            let _ = tx.send(t0.elapsed());
        });
        let took = rx.recv_timeout(Duration::from_secs(5));
        drop(vdir_lock);
        reload.join().unwrap();
        drop(long);
        let took = took.expect("shutdown hung behind the blocked refresher");
        assert!(took < Duration::from_secs(3), "shutdown took {took:?}");
        (status_before, revision_before)
    });
    assert!(
        td.handle().refresher_idle_for_test(Duration::from_secs(5)),
        "the refresher never finished"
    );
    // Neither the blocked reload nor the refresher published after the stop.
    assert_eq!(publisher.status().unwrap(), status_before);
    assert_eq!(publisher.current().0, revision_before);
}

#[test]
fn shutdown_waits_for_a_commit_in_flight_and_refuses_writes_after_it() {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use twydo_daemon::server::{Daemon, DaemonOptions, StartError};

    let fx = fx();
    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let (entered_tx, go_rx) = (Mutex::new(entered_tx), Mutex::new(go_rx));
    let armed = AtomicBool::new(true);
    let td = TestDaemon::start_with(&fx, move |o| {
        o.in_commit = Some(Arc::new(move || {
            if armed.swap(false, Ordering::SeqCst) {
                entered_tx.lock().unwrap().send(()).unwrap();
                // Err: the test failed and dropped the sender; carry on.
                let _ = go_rx.lock().unwrap().recv();
            }
        }));
    });
    let (c1, c2) = (td.client(), td.client());
    let (b1, b2) = (based(&c1), based(&c2));
    let second_daemon = || {
        let dir = common::short_socket_dir();
        let r = Daemon::start(
            fx.config(),
            fx.instance(),
            DaemonOptions::new(dir.path().join("other.sock")),
        );
        (r, dir)
    };
    std::thread::scope(|scope| {
        // Owned here, so a failed assertion drops it and the held write is let go.
        let go_tx = go_tx;
        let held = scope.spawn(move || submit_at(&c1, b1, "held", create("Held")));
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // Waits for the lane behind the held write, past the stopping check.
        let late = scope.spawn(move || submit_at(&c2, b2, "late", create("Late")));
        std::thread::sleep(Duration::from_millis(150));
        let (tx, rx) = mpsc::channel();
        let handle = td.handle();
        scope.spawn(move || {
            handle.shutdown();
            let _ = tx.send(());
        });
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            rx.try_recv().is_err(),
            "shutdown returned with a commit in flight"
        );
        assert!(
            matches!(second_daemon().0, Err(StartError::AlreadyRunning)),
            "the lock was released with a commit in flight"
        );
        go_tx.send(()).unwrap();
        rx.recv_timeout(Duration::from_secs(5))
            .expect("shutdown never finished");
        let held = held.join().unwrap().unwrap();
        assert_eq!(held.outcome, WriteOutcome::Applied);
        let late = late.join().unwrap().unwrap_err();
        assert_eq!(kind(&late), ErrorKind::Unavailable, "{late}");
    });
    assert_eq!(count_titled(&td, "Held"), 1);
    assert_eq!(count_titled(&td, "Late"), 0);
    let (again, _dir) = second_daemon();
    assert!(again.is_ok(), "the lock was not released");
}

/// Commits a create under request `id`, stops the daemon, then sets the entry's stage with
/// `set` (which may also touch the vdir) and starts a fresh daemon.
fn restart_with_entry(
    fx: &Fixture,
    id: &str,
    set: impl FnOnce(&twydo_core::store::journal::Journal, &str),
) -> (TestDaemon, String) {
    use twydo_core::store::bind_vdir_state;
    use twydo_core::store::journal::Journal;
    let op_id = {
        let td = TestDaemon::start(fx);
        let c = td.client();
        submit_at(&c, based(&c), id, create("X")).unwrap().op_id
    };
    let cfg = fx.config();
    let journal = Journal::open(&bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap()).unwrap();
    set(&journal, &op_id);
    (TestDaemon::start(fx), op_id)
}

fn op_get(
    c: &twydo_daemon::client::Client,
    op_id: Option<&str>,
    request_id: Option<&str>,
) -> Result<OpState, twydo_daemon::client::ClientError> {
    c.call(
        "op.get",
        OpGetParams {
            op_id: op_id.map(str::to_string),
            request_id: request_id.map(str::to_string),
        },
    )
}

#[test]
fn a_resend_of_a_stuck_entry_is_not_reported_applied() {
    use twydo_core::store::journal::Stage;
    let fx = fx();
    let vdir = fx.vdir();
    let (td, op_id) = restart_with_entry(&fx, "R", |j, op_id| {
        // Prepared, and the file it created changed since: recovery cannot finish it.
        let e = j.get(op_id).unwrap().unwrap();
        j.set_stage(op_id, Stage::Prepared).unwrap();
        let created = e.writes.iter().find(|w| w.before.is_none()).unwrap();
        std::fs::write(vdir.join(&created.path), todo_ics("zz", "Changed", None)).unwrap();
    });
    let c = td.client();
    for _ in 0..2 {
        let e = submit_at(&c, based(&c), "R", create("X")).unwrap_err();
        assert_eq!(kind(&e), ErrorKind::Stuck, "{e}");
    }
    let e = op_get(&c, None, Some("R")).unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Stuck, "{e}");
    let e = op_get(&c, Some(&op_id), None).unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Stuck, "{e}");
}

#[test]
fn a_resend_of_an_abandoned_entry_is_refused() {
    use twydo_core::store::journal::Stage;
    let fx = fx();
    let (td, op_id) = restart_with_entry(&fx, "R", |j, op_id| {
        j.set_stage(op_id, Stage::Abandoned).unwrap();
    });
    let c = td.client();
    let e = submit_at(&c, based(&c), "R", create("X")).unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Blocked, "{e}");
    assert!(e.to_string().contains("abandoned"), "{e}");
    let refused = OpState::Refused {
        kind: ErrorKind::Blocked,
        message: "the operation was abandoned; it was not applied".into(),
    };
    assert_eq!(op_get(&c, None, Some("R")).unwrap(), refused);
    assert_eq!(op_get(&c, Some(&op_id), None).unwrap(), refused);
    assert_eq!(count_titled(&td, "X"), 1);
}

#[test]
fn replayed_create_returns_the_created_uid() {
    let fx = fx();
    let first;
    {
        let td = TestDaemon::start(&fx);
        let c = td.client();
        first = submit_at(&c, based(&c), "C1", create("Made")).unwrap();
        assert!(first.created.is_some());
        // In the same epoch, too.
        let same = submit_at(&c, based(&c), "C1", create("Made")).unwrap();
        assert_eq!(same.created, first.created);
    }
    // After a restart the answer comes from the journal.
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let again = submit_at(&c, based(&c), "C1", create("Made")).unwrap();
    assert_eq!(again.message, "already applied");
    let created = again.created.expect("created");
    let want = first.created.unwrap();
    assert_eq!(created.uid, want.uid);
    assert_eq!(created.rev, want.rev);
    assert_eq!(count_titled(&td, "Made"), 1);
    // An edit's replay names no created task.
    let e = submit_at(&c, based(&c), "E1", edit_title("u1", "One!")).unwrap();
    assert!(e.created.is_none());
}

#[test]
fn stale_outside_the_view_names_the_uid() {
    let fx = project::fixture();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let r = based(&c);
    // A new item joins the project from outside, after the client's view.
    td.touch_outside(
        &format!("{}/joined.ics", project::LIST),
        &todo_ics("joined-uid", "Joined", Some(project::P)),
    );
    let e = submit_at(
        &c,
        r,
        &c.next_request_id(),
        Op::Complete {
            uid: project::P.into(),
        },
    )
    .unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Stale, "{e}");
    assert_eq!(err_data(&e).uid.as_deref(), Some("joined-uid"), "{e}");
}

#[test]
fn op_get_by_request_of_a_cancelling_undo_is_cancelled() {
    use twydo_daemon::server::lane::LongKind;
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let long = td.handle().hold_long(LongKind::Test);
    let edit_id = c.next_request_id();
    let r = submit_at(&c, based(&c), &edit_id, edit_title("u1", "One queued")).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Queued);
    let undo_id = c.next_request_id();
    let u = undo_with(&c, &undo_id).unwrap();
    assert_eq!(u.outcome, WriteOutcome::Applied);
    assert_eq!(u.op_id, r.op_id);
    for id in [&undo_id, &edit_id] {
        assert_eq!(
            op_get(&c, None, Some(id)).unwrap(),
            OpState::Cancelled,
            "{id}"
        );
    }
    // After a restart the durable records answer the same.
    drop(long);
    drop(c);
    drop(td);
    let td = TestDaemon::start(&fx);
    let c = td.client();
    assert_eq!(
        op_get(&c, None, Some(&undo_id)).unwrap(),
        OpState::Cancelled
    );
}

#[test]
fn entry_mid_commit_is_pending_not_stuck() {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use twydo_core::store::bind_vdir_state;
    use twydo_core::store::journal::{Entry, Journal, Stage, Write};

    let fx = fx();
    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let (entered_tx, go_rx) = (Mutex::new(entered_tx), Mutex::new(go_rx));
    let armed = AtomicBool::new(true);
    let td = TestDaemon::start_with(&fx, move |o| {
        o.in_commit = Some(Arc::new(move || {
            if armed.swap(false, Ordering::SeqCst) {
                entered_tx.lock().unwrap().send(()).unwrap();
                let _ = go_rx.lock().unwrap().recv();
            }
        }));
    });
    let (c1, c2) = (td.client(), td.client());
    let b1 = based(&c1);
    std::thread::scope(|scope| {
        let go_tx = go_tx;
        let held = scope.spawn(move || submit_at(&c1, b1, "held", create("Held")));
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // The request is in flight, seen from another client.
        assert_eq!(op_get(&c2, None, Some("held")).unwrap(), OpState::Pending);
        // An entry the daemon is committing (Prepared, not on the stuck list): pending.
        let cfg = fx.config();
        let journal =
            Journal::open(&bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap()).unwrap();
        journal
            .prepare(&Entry {
                format: 1,
                id: "20261101T120000Z-0000000a".into(),
                created: chrono::Utc::now(),
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
                    path: "inbox/zz.ics".into(),
                    before: None,
                    after: Some(todo_ics("zz", "Mid commit", None)),
                }],
            })
            .unwrap();
        assert_eq!(
            op_get(&c2, Some("20261101T120000Z-0000000a"), None).unwrap(),
            OpState::Pending
        );
        go_tx.send(()).unwrap();
        assert_eq!(held.join().unwrap().unwrap().outcome, WriteOutcome::Applied);
    });
}

#[test]
fn replay_lookup_does_not_scan_the_journal() {
    use chrono::Utc;
    use std::time::Instant;
    use twydo_core::store::journal::{Entry, Journal, RequestRef, Stage, Write};

    let fx = fx();
    let journal = Journal::open(&fx.state()).unwrap();
    let filler = "X".repeat(8 * 1024);
    for n in 0..2_000 {
        journal
            .prepare(&Entry {
                format: 1,
                id: format!("20261101T120000Z-{n:08x}"),
                created: Utc::now(),
                kind: "edit".into(),
                uids: vec!["u1".into()],
                stage: Stage::Applied,
                inverse_of: None,
                from_queue: None,
                request: Some(RequestRef {
                    session: "s-old".into(),
                    request_id: format!("req-{n}"),
                }),
                synced: Some(Utc::now()),
                holds: Vec::new(),
                family: None,
                families: Vec::new(),
                writes: vec![Write {
                    path: "inbox/u1.ics".into(),
                    before: Some(filler.clone()),
                    after: Some(filler.clone()),
                }],
            })
            .unwrap();
    }
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let started = Instant::now();
    let again = submit_at(&c, based(&c), "req-1999", create("X")).unwrap();
    let took = started.elapsed();
    assert_eq!(again.message, "already applied");
    assert_eq!(again.op_id, "20261101T120000Z-000007cf");
    assert!(took < Duration::from_millis(50), "took {took:?}");
}

#[test]
fn resent_queued_revert_replays() {
    use twydo_core::store::bind_vdir_state;
    use twydo_core::store::journal::Journal;
    use twydo_daemon::protocol::admin::JournalRevertParams;
    use twydo_daemon::server::lane::LongKind;

    let fx = fx();
    let revert = |c: &twydo_daemon::client::Client, op_id: &str| {
        c.call::<_, WriteResult>(
            "journal.revert",
            JournalRevertParams {
                session: c.session().expect("hello"),
                request_id: "rv1".into(),
                op_id: op_id.into(),
            },
        )
    };
    let cfg = fx.config();
    let entries = || {
        Journal::open(&bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap())
            .unwrap()
            .list()
            .unwrap()
            .len()
    };
    let first;
    {
        let td = TestDaemon::start(&fx);
        let c = td.client();
        let edit = submit(&c, edit_title("u1", "One!")).unwrap();
        let long = td.handle().hold_long(LongKind::Test);
        first = revert(&c, &edit.op_id).unwrap();
        assert_eq!(first.outcome, WriteOutcome::Queued);
        // Resent while it is still queued.
        let again = revert(&c, &edit.op_id).unwrap();
        assert_eq!(again.outcome, WriteOutcome::Queued);
        assert_eq!(again.op_id, first.op_id);
        drop(long);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while task(&snapshot(&c), "u1").task.summary != "One" {
            assert!(std::time::Instant::now() < deadline, "never drained");
            std::thread::sleep(Duration::from_millis(20));
        }
        // Resent after it drained: the first answer, no second revert.
        let after = revert(&c, &edit.op_id).unwrap();
        assert_eq!(after.op_id, first.op_id);
        assert_eq!(entries(), 2);
        // And after a restart: answered from the journal, still no second revert.
        drop(c);
        drop(td);
        let td = TestDaemon::start(&fx);
        let c = td.client();
        let replay = revert(&c, &edit.op_id).unwrap();
        assert_eq!(replay.outcome, WriteOutcome::Applied);
        assert_eq!(entries(), 2);
        assert_eq!(task(&snapshot(&c), "u1").task.summary, "One");
    }
}

#[test]
fn a_resend_of_an_entry_mid_commit_is_unavailable_and_pending() {
    use twydo_core::store::bind_vdir_state;
    use twydo_core::store::journal::{Entry, Journal, RequestRef, Stage, Write};

    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let cfg = fx.config();
    Journal::open(&bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap())
        .unwrap()
        .prepare(&Entry {
            format: 1,
            id: "20261101T120000Z-0000000a".into(),
            created: chrono::Utc::now(),
            kind: "create".into(),
            uids: vec!["zz".into()],
            stage: Stage::Prepared,
            inverse_of: None,
            from_queue: None,
            request: Some(RequestRef {
                session: "s-other".into(),
                request_id: "mid1".into(),
            }),
            synced: None,
            holds: Vec::new(),
            family: None,
            families: Vec::new(),
            writes: vec![Write {
                path: "inbox/zz.ics".into(),
                before: None,
                after: Some(todo_ics("zz", "Mid commit", None)),
            }],
        })
        .unwrap();
    for _ in 0..2 {
        let e = submit_at(&c, based(&c), "mid1", create("Mid commit")).unwrap_err();
        assert_eq!(kind(&e), ErrorKind::Unavailable, "{e}");
    }
    assert_eq!(op_get(&c, None, Some("mid1")).unwrap(), OpState::Pending);
    assert_eq!(count_titled(&td, "Mid commit"), 0);
}
