//! Job records: replay of a resent job request, `job.get` by id and by request id (also after
//! a restart), and records a stopped daemon left unfinished. Synthetic fixture content.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chrono::Utc;
use common::{Fixture, TestDaemon, edit_title, empty, kind, submit, wait_event};
use twydo_daemon::client::{Client, Notification};
use twydo_daemon::protocol::events::Event;
use twydo_daemon::protocol::jobs::{
    JobGetParams, JobKind, JobResult, JobStarted, JobState, JobView,
};
use twydo_daemon::protocol::methods::{StatusView, SyncState, WriteOutcome};
use twydo_daemon::protocol::rpc::ErrorKind;
use twydo_daemon::protocol::sync::{SyncOutcome, SyncReason, SyncRunParams};
use twydo_daemon::server::DaemonOptions;

const BOUND: Duration = Duration::from_secs(10);
const BODY: &str = "echo 'Syncing twydo/c1' >&2\nexit 0";

fn fx() -> Fixture {
    Fixture::new(&[("c1", "u1", "One", None)])
}

fn start(fx: &Fixture) -> (TestDaemon, Client) {
    let td = TestDaemon::start_cfg(fx, fx.with_sync(BODY), |_| {});
    let c = td.client();
    let _: serde_json::Value = c.call("events.subscribe", empty()).unwrap();
    (td, c)
}

fn sync_run(c: &Client, request_id: &str) -> JobStarted {
    c.call(
        "sync.run",
        SyncRunParams {
            request_id: request_id.into(),
            force_delete: Vec::new(),
        },
    )
    .unwrap()
}

fn job_get(c: &Client, job_id: Option<&str>, request_id: Option<&str>) -> JobView {
    c.call(
        "job.get",
        JobGetParams {
            job_id: job_id.map(Into::into),
            request_id: request_id.map(Into::into),
        },
    )
    .unwrap()
}

/// How many `sync.started` arrive within `limit`.
fn count_sync_started(c: &Client, limit: Duration) -> usize {
    let deadline = Instant::now() + limit;
    let mut n = 0;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match c.notifications().recv_timeout(left) {
            Ok(Notification::Event(ev)) if matches!(ev.event, Event::SyncStarted { .. }) => n += 1,
            Ok(_) => {}
            Err(_) => return n,
        }
    }
}

fn job_finished(c: &Client, id: &str) -> JobState {
    match wait_event(
        c,
        BOUND,
        |e| matches!(e, Event::JobFinished { job_id, .. } if job_id == id),
    ) {
        Event::JobFinished { state, .. } => state,
        _ => unreachable!(),
    }
}

#[test]
fn resent_sync_run_returns_the_same_job() {
    let fx = fx();
    let (_td, c) = start(&fx);
    let a = sync_run(&c, "r-1");
    let b = sync_run(&c, "r-1");
    assert_eq!(a, b);
    // One sync.started, then the job's end, then nothing more.
    let started = wait_event(&c, BOUND, |e| matches!(e, Event::SyncStarted { .. }));
    assert!(
        matches!(&started, Event::SyncStarted { job_id: Some(j), .. } if *j == a.job_id),
        "{started:?}"
    );
    job_finished(&c, &a.job_id);
    assert_eq!(count_sync_started(&c, Duration::from_millis(500)), 0);
    // Resent after the end: still the same job.
    assert_eq!(sync_run(&c, "r-1"), a);
    assert_eq!(count_sync_started(&c, Duration::from_millis(300)), 0);
}

#[test]
fn job_get_by_request_id_after_a_restart() {
    let fx = fx();
    let id = {
        let (_td, c) = start(&fx);
        let id = sync_run(&c, "r-restart").job_id;
        let state = job_finished(&c, &id);
        assert!(
            matches!(
                &state,
                JobState::Finished { result: JobResult::Sync(r) } if r.outcome == SyncOutcome::Ok
            ),
            "{state:?}"
        );
        id
    };
    let (_td, c) = start(&fx);
    let v = job_get(&c, None, Some("r-restart"));
    assert_eq!(v.job_id, id);
    assert_eq!(v.kind, JobKind::Sync);
    assert_eq!(v.request_id.as_deref(), Some("r-restart"));
    assert!(
        matches!(&v.state, JobState::Finished { result: JobResult::Sync(r) } if r.outcome == SyncOutcome::Ok),
        "{v:?}"
    );
}

#[test]
fn unfinished_records_become_interrupted_at_startup() {
    let fx = fx();
    // What a daemon killed during a sync leaves behind.
    let jobs = fx.state().join("daemon").join("jobs");
    std::fs::create_dir_all(&jobs).unwrap();
    let id = "job-20261110T090000-0badcafe";
    let record = JobView {
        job_id: id.into(),
        request_id: Some("r-killed".into()),
        kind: JobKind::Sync,
        state: JobState::Running,
        created: Utc::now() - chrono::Duration::minutes(1),
        ended: None,
    };
    std::fs::write(
        jobs.join(format!("{id}.json")),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();

    let (_td, c) = start(&fx);
    let v = job_get(&c, Some(id), None);
    assert_eq!(v.state, JobState::Interrupted);
    assert!(v.ended.is_some());
    assert_eq!(job_get(&c, None, Some("r-killed")), v);
    // Resending its request answers with that job and starts no sync.
    assert_eq!(sync_run(&c, "r-killed").job_id, id);
    assert_eq!(count_sync_started(&c, Duration::from_millis(500)), 0);
}

#[test]
fn job_get_unknown_is_not_found() {
    let fx = fx();
    let (_td, c) = start(&fx);
    let e = c
        .call::<_, JobView>(
            "job.get",
            JobGetParams {
                job_id: Some("job-x".into()),
                request_id: None,
            },
        )
        .unwrap_err();
    assert_eq!(kind(&e), ErrorKind::NotFound);
    let e = c
        .call::<_, JobView>(
            "job.get",
            JobGetParams {
                job_id: None,
                request_id: Some("r-never".into()),
            },
        )
        .unwrap_err();
    assert_eq!(kind(&e), ErrorKind::NotFound);
    for (job_id, request_id) in [(None, None), (Some("job-x"), Some("r-x"))] {
        let e = c
            .call::<_, JobView>(
                "job.get",
                JobGetParams {
                    job_id: job_id.map(Into::into),
                    request_id: request_id.map(Into::into),
                },
            )
            .unwrap_err();
        assert_eq!(kind(&e), ErrorKind::Invalid);
        assert!(
            e.to_string()
                .contains("give exactly one of job_id or request_id"),
            "{e}"
        );
    }
}

// ---- a panic on the runner thread (final review) ----

/// A daemon (as [`start`]) whose options `f` sets, given a one-shot flag: a hook that panics
/// takes it, so it panics once.
fn start_armed(
    fx: &Fixture,
    f: impl FnOnce(&mut DaemonOptions, Arc<AtomicBool>),
) -> (TestDaemon, Client, Arc<AtomicBool>) {
    let armed = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&armed);
    let td = TestDaemon::start_cfg(fx, fx.with_sync(BODY), |o| f(o, flag));
    let c = td.client();
    let _: serde_json::Value = c.call("events.subscribe", empty()).unwrap();
    (td, c, armed)
}

/// Panics at test point `point` once `armed` is set (and clears it).
fn panic_at(o: &mut DaemonOptions, point: &'static str, armed: Arc<AtomicBool>) {
    o.test_point = Some(Arc::new(move |p: &str| {
        if p == point && armed.swap(false, Ordering::SeqCst) {
            panic!("test panic at {p}");
        }
    }));
}

/// Waits until the status no longer says a sync runs.
fn sync_idle(c: &Client) {
    let deadline = Instant::now() + BOUND;
    loop {
        let st: StatusView = c.call("status.get", empty()).unwrap();
        if st.sync.as_ref().is_some_and(|s| s.state == SyncState::Idle) {
            return;
        }
        assert!(Instant::now() < deadline, "the sync state is stuck: {st:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The runner still works: a new `sync.run` runs and finishes ok, and a shutdown when idle
/// stops the daemon.
fn runner_still_works(td: &TestDaemon, c: &Client) {
    sync_idle(c);
    let id = sync_run(c, &c.next_request_id()).job_id;
    let state = job_finished(c, &id);
    assert!(
        matches!(&state, JobState::Finished { result: JobResult::Sync(r) } if r.outcome == SyncOutcome::Ok),
        "{state:?}"
    );
    sync_idle(c);
    let w = submit(c, edit_title("u1", "After")).unwrap();
    assert_eq!(w.outcome, WriteOutcome::Applied);
    let _: serde_json::Value = c
        .call("daemon.shutdown", serde_json::json!({"when_idle": true}))
        .unwrap();
    let deadline = Instant::now() + BOUND;
    while !td.handle().is_stopped() {
        assert!(
            Instant::now() < deadline,
            "shutdown when idle never stopped"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_panic_in_a_background_sync_keeps_the_runner() {
    let fx = fx();
    let (td, c, armed) = start_armed(&fx, |o, a| panic_at(o, "sync", a));
    armed.store(true, Ordering::SeqCst);
    td.handle().post_sync_for_test(SyncReason::Periodic);
    wait_event(&c, BOUND, |e| {
        matches!(e, Event::SyncStarted { job_id: None, .. })
    });
    runner_still_works(&td, &c);
}

#[test]
fn a_panic_in_a_sync_job_fails_it_and_clears_the_sync_state() {
    let fx = fx();
    let (td, c, armed) = start_armed(&fx, |o, a| panic_at(o, "sync", a));
    armed.store(true, Ordering::SeqCst);
    let id = sync_run(&c, "r-panic").job_id;
    match job_finished(&c, &id) {
        JobState::Failed { kind, .. } => assert_eq!(kind, ErrorKind::Internal),
        other => panic!("expected failed: {other:?}"),
    }
    assert!(job_get(&c, Some(&id), None).state.is_final());
    runner_still_works(&td, &c);
}

#[test]
fn a_panic_in_the_drain_after_a_job_keeps_the_runner() {
    let fx = fx();
    let (td, c, armed) = start_armed(&fx, |o, a| {
        o.before_drain_passes = Some(Arc::new(move || {
            if a.swap(false, Ordering::SeqCst) {
                panic!("test panic in the drain hook");
            }
        }));
    });
    armed.store(true, Ordering::SeqCst);
    let id = sync_run(&c, "r-drain-panic").job_id;
    // The job's own work was done and recorded before its drain hook panicked.
    let state = job_finished(&c, &id);
    assert!(state.is_final(), "{state:?}");
    assert_eq!(job_get(&c, Some(&id), None).state, state);
    assert!(!armed.load(Ordering::SeqCst), "the hook never ran");
    runner_still_works(&td, &c);
}

#[test]
fn a_panicking_sync_job_ends_failed_and_clears_running() {
    let fx = fx();
    let (_td, c, armed) = start_armed(&fx, |o, a| panic_at(o, "sync", a));
    armed.store(true, Ordering::SeqCst);
    let id = sync_run(&c, "r-panic-2").job_id;
    let finished = wait_event(
        &c,
        BOUND,
        |e| matches!(e, Event::SyncFinished { job_id: Some(j), .. } if *j == id),
    );
    let Event::SyncFinished { result, .. } = finished else {
        unreachable!()
    };
    assert_eq!(result.outcome, SyncOutcome::Failed);
    match job_finished(&c, &id) {
        JobState::Failed { kind, .. } => assert_eq!(kind, ErrorKind::Internal),
        other => panic!("expected failed: {other:?}"),
    }
    sync_idle(&c);
}
