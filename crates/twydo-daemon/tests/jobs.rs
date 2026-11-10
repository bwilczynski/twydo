//! Job records: replay of a resent job request, `job.get` by id and by request id (also after
//! a restart), and records a stopped daemon left unfinished. Synthetic fixture content.

mod common;

use std::time::{Duration, Instant};

use chrono::Utc;
use common::{Fixture, TestDaemon, empty, kind, wait_event};
use twydo_daemon::client::{Client, Notification};
use twydo_daemon::protocol::events::Event;
use twydo_daemon::protocol::jobs::{
    JobGetParams, JobKind, JobResult, JobStarted, JobState, JobView,
};
use twydo_daemon::protocol::rpc::ErrorKind;
use twydo_daemon::protocol::sync::{SyncOutcome, SyncRunParams};

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
    }
}
