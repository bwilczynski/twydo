//! Syncs run by the daemon as jobs on the vdir lane: `sync.run`, the `sync.*` and
//! `job.finished` events, edits queued during a sync, and a run lock held by another process.
//! The vdirsyncer is a fake script; synthetic fixture content.

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};
use twydo_core::config::Config;

use common::{Fixture, TestDaemon, edit_title, empty, kind, submit, wait_event};
use twydo_core::sync::lock::LockFile;
use twydo_core::sync::report::switch_message;
use twydo_daemon::client::{Client, Notification};
use twydo_daemon::protocol::events::Event;
use twydo_daemon::protocol::jobs::{JobGetParams, JobResult, JobStarted, JobState, JobView};
use twydo_daemon::protocol::methods::{OpState, StatusView, SyncState, WriteOutcome};
use twydo_daemon::protocol::rpc::ErrorKind;
use twydo_daemon::protocol::sync::{
    SyncOutcome, SyncReason, SyncResult, SyncRunParams, SyncSetAutoParams, SyncSetAutoResult,
    SyncStopResult,
};

const BOUND: Duration = Duration::from_secs(10);

/// A fake vdirsyncer that sleeps `secs`, reports `twydo/c1` and succeeds.
fn sleeps(secs: &str) -> String {
    format!("sleep {secs}\necho 'Syncing twydo/c1' >&2\nexit 0")
}

fn fx() -> Fixture {
    Fixture::new(&[("c1", "u1", "One", None)])
}

/// A daemon whose fake vdirsyncer runs `body`, and a subscribed client.
fn start(fx: &Fixture, body: &str) -> (TestDaemon, Client) {
    let td = TestDaemon::start_cfg(fx, fx.with_sync(body), |_| {});
    let c = td.client();
    let _: serde_json::Value = c.call("events.subscribe", empty()).unwrap();
    (td, c)
}

fn sync_run(c: &Client) -> JobStarted {
    c.call(
        "sync.run",
        SyncRunParams {
            request_id: c.next_request_id(),
            force_delete: Vec::new(),
            discover: false,
        },
    )
    .unwrap()
}

/// The `sync.finished` of job `id`: its result.
fn sync_finished(c: &Client, id: &str) -> SyncResult {
    match wait_event(
        c,
        BOUND,
        |e| matches!(e, Event::SyncFinished { job_id: Some(j), .. } if j == id),
    ) {
        Event::SyncFinished { result, .. } => result,
        _ => unreachable!(),
    }
}

#[test]
fn sync_run_answers_at_once_and_reports_finished() {
    let fx = fx();
    let (_td, c) = start(&fx, &sleeps("0.5"));
    let t = Instant::now();
    let started = sync_run(&c);
    assert!(
        t.elapsed() < Duration::from_millis(200),
        "sync.run took {:?}",
        t.elapsed()
    );
    let id = started.job_id;
    // The order of the three outcome events, and nothing of this job before its start.
    let mut seen = Vec::new();
    let finished_state = loop {
        let left = BOUND.saturating_sub(t.elapsed());
        match c.notifications().recv_timeout(left) {
            Ok(Notification::Event(ev)) => match ev.event {
                Event::SyncStarted { job_id, reason } if job_id.as_deref() == Some(&id) => {
                    assert_eq!(reason, SyncReason::Manual);
                    seen.push("started");
                }
                Event::SyncFinished {
                    job_id,
                    reason,
                    result,
                    ..
                } if job_id.as_deref() == Some(&id) => {
                    assert_eq!(reason, SyncReason::Manual);
                    assert_eq!(result.outcome, SyncOutcome::Ok, "{result:?}");
                    seen.push("finished");
                }
                Event::JobFinished { job_id, state, .. } if job_id == id => {
                    seen.push("job");
                    break state;
                }
                _ => {}
            },
            Ok(_) => {}
            Err(e) => panic!("no job.finished: {e}; saw {seen:?}"),
        }
    };
    assert_eq!(seen, ["started", "finished", "job"]);
    match &finished_state {
        JobState::Finished {
            result: JobResult::Sync(r),
        } => assert_eq!(r.outcome, SyncOutcome::Ok),
        other => panic!("{other:?}"),
    }
    let got: JobView = c
        .call(
            "job.get",
            JobGetParams {
                job_id: Some(id.clone()),
                request_id: None,
            },
        )
        .unwrap();
    assert_eq!(got.state, finished_state);
    assert_eq!(got.job_id, id);
    assert!(got.ended.is_some());
}

#[test]
fn edit_during_a_sync_is_queued_then_applied() {
    let fx = fx();
    let (_td, c) = start(&fx, &sleeps("1"));
    let id = sync_run(&c).job_id;
    wait_event(&c, BOUND, |e| matches!(e, Event::SyncStarted { .. }));
    let r = submit(&c, edit_title("u1", "Renamed")).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Queued, "{r:?}");
    // Every op.finished for the item until the job's end, and for a while after it.
    let mut finished = Vec::new();
    let mut drained = None;
    let mut job_done: Option<Instant> = None;
    let deadline = Instant::now() + BOUND;
    loop {
        let wait = match job_done {
            Some(at) => (at + Duration::from_millis(500)).saturating_duration_since(Instant::now()),
            None => deadline.saturating_duration_since(Instant::now()),
        };
        match c.notifications().recv_timeout(wait) {
            Ok(Notification::Event(ev)) => match ev.event {
                Event::OpFinished { op_id, state, .. } if op_id == r.op_id => finished.push(state),
                Event::SyncFinished {
                    job_id: Some(j),
                    result,
                    ..
                } if j == id => drained = Some(result.drained),
                Event::JobFinished { job_id, .. } if job_id == id => {
                    job_done = Some(Instant::now())
                }
                _ => {}
            },
            Ok(_) => {}
            Err(_) if job_done.is_some() => break,
            Err(e) => panic!("no job.finished: {e}"),
        }
    }
    assert_eq!(finished.len(), 1, "{finished:?}");
    assert!(
        matches!(finished[0], OpState::Applied { .. }),
        "{finished:?}"
    );
    assert_eq!(drained, Some(1), "the runner's own drain applied it");
    assert!(fx.read("c1/u1.ics").contains("SUMMARY:Renamed"));
}

#[test]
fn manual_sync_waits_behind_a_running_one() {
    let fx = fx();
    let (_td, c) = start(&fx, &sleeps("0.3"));
    let a = sync_run(&c).job_id;
    let b = sync_run(&c).job_id;
    assert_ne!(a, b);
    let mut order = Vec::new();
    let t = Instant::now();
    while order.len() < 4 {
        let left = BOUND.saturating_sub(t.elapsed());
        match c.notifications().recv_timeout(left) {
            Ok(Notification::Event(ev)) => match ev.event {
                Event::SyncStarted {
                    job_id: Some(j), ..
                } => order.push(format!("started {j}")),
                Event::SyncFinished {
                    job_id: Some(j),
                    result,
                    ..
                } => {
                    assert_eq!(result.outcome, SyncOutcome::Ok, "{result:?}");
                    order.push(format!("finished {j}"));
                }
                _ => {}
            },
            Ok(_) => {}
            Err(e) => panic!("{e}; saw {order:?}"),
        }
    }
    assert_eq!(
        order,
        [
            format!("started {a}"),
            format!("finished {a}"),
            format!("started {b}"),
            format!("finished {b}"),
        ]
    );
}

#[test]
fn sync_problem_lines_match_tdo_sync() {
    let fx = fx();
    let (_td, c) = start(&fx, "echo 'error: synthetic failure' >&2\nexit 1");
    let id = sync_run(&c).job_id;
    let r = sync_finished(&c, &id);
    assert_eq!(r.outcome, SyncOutcome::Failed);
    // `tdo sync` prints this as `tdo: sync failed: <first error line>` (cli/sync.rs, now
    // `twydo_core::sync::report`).
    assert_eq!(r.problems, ["sync failed: synthetic failure"]);
}

#[test]
fn another_runner_holding_run_lock_is_skipped() {
    let fx = fx();
    let (td, c) = start(&fx, &sleeps("0"));
    let sync_dir = fx.state().join("sync");
    std::fs::create_dir_all(&sync_dir).unwrap();
    let lock = LockFile::take(&sync_dir.join("run.lock"), true)
        .unwrap()
        .expect("run lock free");
    td.handle().post_sync_for_test(SyncReason::Periodic);
    let ev = wait_event(&c, BOUND, |e| {
        matches!(
            e,
            Event::SyncFinished {
                reason: SyncReason::Periodic,
                ..
            }
        )
    });
    let Event::SyncFinished { job_id, result, .. } = ev else {
        unreachable!()
    };
    assert_eq!(job_id, None, "a background sync is not a job");
    assert_eq!(result.outcome, SyncOutcome::Skipped);
    assert!(result.problems.is_empty(), "{result:?}");
    assert!(result.warnings.is_empty(), "{result:?}");

    // A manual sync waits for the lock, then runs.
    let id = sync_run(&c).job_id;
    wait_event(
        &c,
        BOUND,
        |e| matches!(e, Event::SyncStarted { job_id: Some(j), .. } if *j == id),
    );
    std::thread::sleep(Duration::from_millis(300));
    let v: JobView = c
        .call(
            "job.get",
            JobGetParams {
                job_id: Some(id.clone()),
                request_id: None,
            },
        )
        .unwrap();
    assert_eq!(v.state, JobState::Running, "waits for the run lock");
    let st: StatusView = c.call("status.get", empty()).unwrap();
    let sync = st.sync.expect("sync configured");
    assert_eq!(sync.state, SyncState::Running);
    assert_eq!(sync.job_id.as_deref(), Some(id.as_str()));
    drop(lock);
    assert_eq!(sync_finished(&c, &id).outcome, SyncOutcome::Ok);
    let st: StatusView = c.call("status.get", empty()).unwrap();
    let sync = st.sync.expect("sync configured");
    assert_eq!(sync.state, SyncState::Idle);
    assert_eq!(sync.job_id, None);
}

fn sync_run_with(
    c: &Client,
    request_id: &str,
    force_delete: &[&str],
) -> Result<JobStarted, twydo_daemon::client::ClientError> {
    c.call(
        "sync.run",
        SyncRunParams {
            request_id: request_id.into(),
            force_delete: force_delete.iter().map(|s| s.to_string()).collect(),
            discover: false,
        },
    )
}

#[test]
fn sync_run_without_sync_config_is_refused() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let e = sync_run_with(&c, "r-1", &[]).unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Invalid);
    let twydo_daemon::client::ClientError::Rpc(r) = &e else {
        unreachable!()
    };
    assert_eq!(
        r.message,
        "sync is not configured: add a [sync] table (docs/sync.md)"
    );
    // No job was started for it.
    let e = c
        .call::<_, JobView>(
            "job.get",
            JobGetParams {
                job_id: None,
                request_id: Some("r-1".into()),
            },
        )
        .unwrap_err();
    assert_eq!(kind(&e), ErrorKind::NotFound);
}

/// A fake vdirsyncer that records its arguments in `args` next to itself.
const LOGS_ARGS: &str =
    "echo \"$@\" >> \"$(dirname \"$0\")/args\"\necho 'Syncing twydo/c1' >&2\nexit 0";

#[test]
fn force_delete_maps_a_display_name_to_its_collection() {
    let fx = fx();
    fx.write("c1/displayname", "Groceries\n");
    let (_td, c) = start(&fx, LOGS_ARGS);
    let id = sync_run_with(&c, "r-fd", &["Groceries"]).unwrap().job_id;
    sync_finished(&c, &id);
    let args = std::fs::read_to_string(fx.vdir().parent().unwrap().join("args")).unwrap();
    assert!(
        args.lines().any(|l| l == "sync --force-delete twydo/c1"),
        "{args}"
    );
}

#[test]
fn force_delete_of_an_unknown_list_is_invalid() {
    let fx = fx();
    let (_td, c) = start(&fx, LOGS_ARGS);
    let e = sync_run_with(&c, "r-unknown", &["Nope"]).unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Invalid);
}

#[test]
fn a_resend_replays_before_mapping_force_delete() {
    let fx = fx();
    let (_td, c) = start(&fx, LOGS_ARGS);
    let id = sync_run_with(&c, "r-fd", &["c1"]).unwrap().job_id;
    sync_finished(&c, &id);
    // The list is gone since: the resend still answers with its job.
    std::fs::remove_dir_all(fx.vdir().join("c1")).unwrap();
    assert_eq!(sync_run_with(&c, "r-fd", &["c1"]).unwrap().job_id, id);
    assert_eq!(sync_run_with(&c, "r-fd", &["Nope"]).unwrap().job_id, id);
}

fn sync_stop(c: &Client) -> SyncStopResult {
    c.call("sync.stop", empty()).unwrap()
}

fn set_auto(c: &Client, request_id: &str, on: bool) -> SyncSetAutoResult {
    c.call(
        "sync.set_auto",
        SyncSetAutoParams {
            request_id: request_id.to_string(),
            on,
        },
    )
    .unwrap()
}

#[test]
fn sync_stop_ends_a_running_sync() {
    let fx = fx();
    let (_td, c) = start(&fx, "sleep 30");
    let id = sync_run(&c).job_id;
    wait_event(&c, BOUND, |e| matches!(e, Event::SyncStarted { .. }));
    assert!(sync_stop(&c).stopping);
    let r = sync_finished(&c, &id);
    assert_eq!(r.outcome, SyncOutcome::Stopped, "{r:?}");
}

#[test]
fn stop_with_nothing_running_does_not_block_the_next_sync() {
    let fx = fx();
    let (_td, c) = start(&fx, &sleeps("0.1"));
    assert!(!sync_stop(&c).stopping);
    let id = sync_run(&c).job_id;
    let r = sync_finished(&c, &id);
    assert_eq!(r.outcome, SyncOutcome::Ok, "{r:?}");
}

#[test]
fn stop_request_is_withdrawn_after_the_stopped_sync() {
    let fx = fx();
    let (_td, c) = start(&fx, "sleep 30");
    let id = sync_run(&c).job_id;
    wait_event(&c, BOUND, |e| matches!(e, Event::SyncStarted { .. }));
    assert!(sync_stop(&c).stopping);
    assert_eq!(sync_finished(&c, &id).outcome, SyncOutcome::Stopped);
    assert!(!fx.state().join("sync/stop").exists());
}

#[test]
fn the_sync_after_a_stopped_one_finishes_ok() {
    let fx = fx();
    let marker = fx.state().parent().unwrap().join("slow");
    let body = format!(
        "if [ -e {m} ]; then sleep 0.1; else sleep 30; fi\necho 'Syncing twydo/c1' >&2\nexit 0",
        m = marker.display()
    );
    let (_td, c) = start(&fx, &body);
    let id = sync_run(&c).job_id;
    wait_event(&c, BOUND, |e| matches!(e, Event::SyncStarted { .. }));
    assert!(sync_stop(&c).stopping);
    assert_eq!(sync_finished(&c, &id).outcome, SyncOutcome::Stopped);
    std::fs::write(&marker, "").unwrap();
    let id = sync_run(&c).job_id;
    let r = sync_finished(&c, &id);
    assert_eq!(r.outcome, SyncOutcome::Ok, "{r:?}");
}

fn status(c: &Client) -> StatusView {
    c.call("status.get", empty()).unwrap()
}

fn auto_is(c: &Client, want: bool) -> bool {
    let deadline = Instant::now() + BOUND;
    loop {
        if status(c).sync.is_some_and(|s| s.auto == want) {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn set_auto_persists_the_switch() {
    let fx = fx();
    let (_td, c) = start(&fx, &sleeps("0"));
    let off = fx.state().join("sync/off");
    let a = set_auto(&c, "r-off", false);
    assert!(!a.auto);
    assert_eq!(a.message, switch_message(false));
    assert!(off.exists());
    assert!(auto_is(&c, false));
    assert_eq!(set_auto(&c, "r-off", false), a, "a resend replays");
    let b = set_auto(&c, "r-on", true);
    assert!(b.auto);
    assert_eq!(b.message, switch_message(true));
    assert!(!off.exists());
    assert!(auto_is(&c, true));
}

#[test]
fn set_auto_needs_a_request_id() {
    let fx = fx();
    let (_td, c) = start(&fx, &sleeps("0"));
    let e = c
        .call::<_, SyncSetAutoResult>(
            "sync.set_auto",
            SyncSetAutoParams {
                request_id: String::new(),
                on: false,
            },
        )
        .unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Invalid);
}

#[test]
fn writes_during_a_stopped_sync_drain_after_it() {
    let fx = fx();
    let (_td, c) = start(&fx, "sleep 30");
    let id = sync_run(&c).job_id;
    wait_event(&c, BOUND, |e| matches!(e, Event::SyncStarted { .. }));
    let r = submit(&c, edit_title("u1", "Renamed")).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Queued, "{r:?}");
    assert!(sync_stop(&c).stopping);
    let done = sync_finished(&c, &id);
    assert_eq!(done.outcome, SyncOutcome::Stopped, "{done:?}");
    // The runner's own drain (op.finished comes before sync.finished) applied it.
    assert_eq!(done.drained, 1, "{done:?}");
    assert!(fx.read("c1/u1.ics").contains("SUMMARY:Renamed"));
}

/// Every notification already delivered on `c`.
fn pending(c: &Client) -> Vec<Event> {
    c.notifications()
        .try_iter()
        .filter_map(|n| match n {
            Notification::Event(ev) => Some(ev.event),
            _ => None,
        })
        .collect()
}

#[test]
fn a_sync_that_drains_then_fails_finishes_each_drained_item() {
    let fx = Fixture::new(&[("c1", "u1", "One", None), ("c1", "u2", "Two", None)]);
    // After vdirsyncer is done, the status file becomes a directory: the run fails when it
    // records its status, after it drained the queued edits.
    let status = fx.state().join("sync").join("status.json");
    let body = format!(
        "sleep 1\necho 'Syncing twydo/c1' >&2\nrm -rf '{p}'\nmkdir '{p}'\nexit 0",
        p = status.display()
    );
    let (_td, c) = start(&fx, &body);
    let id = sync_run(&c).job_id;
    wait_event(&c, BOUND, |e| matches!(e, Event::SyncStarted { .. }));
    let a = submit(&c, edit_title("u1", "First")).unwrap();
    let b = submit(&c, edit_title("u2", "Second")).unwrap();
    assert_eq!(a.outcome, WriteOutcome::Queued, "{a:?}");
    assert_eq!(b.outcome, WriteOutcome::Queued, "{b:?}");
    let mut finished = Vec::new();
    let mut result = None;
    let deadline = Instant::now() + BOUND;
    loop {
        match c
            .notifications()
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(Notification::Event(ev)) => match ev.event {
                Event::OpFinished { op_id, state, .. } => finished.push((op_id, state)),
                Event::SyncFinished {
                    job_id: Some(j),
                    result: r,
                    ..
                } if j == id => result = Some(r),
                Event::JobFinished { job_id, .. } if job_id == id => break,
                _ => {}
            },
            Ok(_) => {}
            Err(e) => panic!("no job.finished: {e}"),
        }
    }
    let r = result.expect("sync.finished came before job.finished");
    assert_eq!(r.outcome, SyncOutcome::Failed, "{r:?}");
    assert!(!r.problems.is_empty(), "{r:?}");
    assert_eq!(r.drained, 2, "{r:?}");
    assert_eq!(finished.len(), 2, "{finished:?}");
    for (op_id, state) in &finished {
        assert!(
            matches!(state, OpState::Applied { .. }),
            "{op_id}: {state:?}"
        );
    }
    assert!(fx.read("c1/u1.ics").contains("SUMMARY:First"));
    assert!(fx.read("c1/u2.ics").contains("SUMMARY:Second"));
}

#[test]
fn job_get_is_final_only_after_job_finished() {
    let fx = fx();
    let (_td, c) = start(&fx, &sleeps("0.3"));
    let id = sync_run(&c).job_id;
    // One connection: events sent before a reply are delivered before it.
    let deadline = Instant::now() + BOUND;
    let mut seen = Vec::new();
    loop {
        let v: JobView = c
            .call(
                "job.get",
                JobGetParams {
                    job_id: Some(id.clone()),
                    request_id: None,
                },
            )
            .unwrap();
        seen.extend(pending(&c));
        if v.state.is_final() {
            break;
        }
        assert!(Instant::now() < deadline, "never final");
    }
    let names: Vec<&str> = seen
        .iter()
        .filter_map(|e| match e {
            Event::SyncFinished { .. } => Some("sync.finished"),
            Event::JobFinished { .. } => Some("job.finished"),
            _ => None,
        })
        .collect();
    assert_eq!(names, ["sync.finished", "job.finished"], "{seen:?}");
}

#[test]
fn status_reports_running_until_sync_finished() {
    let fx = fx();
    let (_td, c) = start(&fx, &sleeps("0.4"));
    let _ = sync_run(&c);
    let deadline = Instant::now() + BOUND;
    let mut seen = Vec::new();
    let mut running = 0;
    loop {
        seen.extend(pending(&c));
        let st: StatusView = c.call("status.get", empty()).unwrap();
        let started = seen.iter().any(|e| matches!(e, Event::SyncStarted { .. }));
        if started {
            match st.sync.as_ref().map(|s| s.state) {
                Some(SyncState::Running) => running += 1,
                _ => {
                    // The reply came after every event sent before it was readable.
                    seen.extend(pending(&c));
                    assert!(
                        seen.iter().any(|e| matches!(e, Event::SyncFinished { .. })),
                        "idle before sync.finished: {seen:?}"
                    );
                    break;
                }
            }
        }
        assert!(Instant::now() < deadline, "never idle");
    }
    assert!(running > 0, "never saw the sync running");
}

// ---- list discovery ----

/// The fixture's config with the fake vdirsyncer `body` and the list helper routed to the
/// offline backend of twydo-core over this fixture's vdir. `discover` is the backend's
/// `discover` store. Returns the config and the helper directory.
fn with_helper(fx: &Fixture, body: &str, discover: serde_json::Value) -> (Config, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let mut cfg = fx.with_sync(body);
    let keep = fx.vdir().parent().unwrap().join("helper");
    std::fs::create_dir_all(&keep).unwrap();
    let backend = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../twydo-core/tests/common/list_backend.py"
    );
    let wrapper = keep.join("python");
    std::fs::write(
        &wrapper,
        format!("#!/usr/bin/env python3\nimport runpy\nrunpy.run_path({backend:?},run_name='__main__')\n"),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let connection = keep.join("connection.json");
    std::fs::write(
        &connection,
        serde_json::json!({"vdir": fx.vdir()}).to_string(),
    )
    .unwrap();
    std::fs::write(
        keep.join("server.json"),
        serde_json::json!({"calls":[],"collections":{},"flags":{},"discover":discover}).to_string(),
    )
    .unwrap();
    let sync = cfg.sync.as_mut().unwrap();
    sync.pair = "synthetic".into();
    sync.local_storage = "local".into();
    sync.list_python = wrapper.to_str().unwrap().into();
    sync.vdirsyncer_config = Some(connection);
    (cfg, keep)
}

fn discoveries(keep: &std::path::Path) -> usize {
    let store: serde_json::Value =
        serde_json::from_slice(&std::fs::read(keep.join("server.json")).unwrap()).unwrap();
    store["calls"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c.as_str() == Some("DISCOVER"))
        .count()
}

const OK_SYNC: &str = "[ \"$1\" = sync ] && echo 'Syncing synthetic/c1' >&2\nexit 0";

fn finished(c: &Client, reason: SyncReason) -> SyncResult {
    match wait_event(
        c,
        BOUND,
        |e| matches!(e, Event::SyncFinished { reason: r, .. } if *r == reason),
    ) {
        Event::SyncFinished { result, .. } => result,
        _ => unreachable!(),
    }
}

#[test]
fn startup_sync_discovers_new_lists() {
    let fx = fx();
    let (cfg, _keep) = with_helper(
        &fx,
        OK_SYNC,
        serde_json::json!({"remote_only":["phone-list"],"local_only":[]}),
    );
    let td = TestDaemon::start_cfg(&fx, cfg, |_| {});
    let c = td.client();
    let _: serde_json::Value = c.call("events.subscribe", empty()).unwrap();
    td.handle().post_sync_for_test(SyncReason::Startup);
    let r = finished(&c, SyncReason::Startup);
    assert!(
        r.notes.iter().any(|n| n.contains("found new list")),
        "{r:?}"
    );
    assert!(
        common::snapshot(&c)
            .collections
            .iter()
            .any(|col| col.id == "phone-list")
    );
}

#[test]
fn startup_discovery_failure_is_a_warning() {
    let fx = fx();
    let (cfg, _keep) = with_helper(
        &fx,
        OK_SYNC,
        serde_json::json!({"remote_only":[],"local_only":["stale"]}),
    );
    std::fs::create_dir_all(fx.vdir().join("stale")).unwrap();
    let td = TestDaemon::start_cfg(&fx, cfg, |_| {});
    let c = td.client();
    let _: serde_json::Value = c.call("events.subscribe", empty()).unwrap();
    td.handle().post_sync_for_test(SyncReason::Startup);
    let r = finished(&c, SyncReason::Startup);
    assert_eq!(r.outcome, SyncOutcome::Ok, "{r:?}");
    assert!(r.warnings.iter().any(|w| w.contains("stale")), "{r:?}");
    let _: serde_json::Value = c.call("rpc.discover", serde_json::json!({})).unwrap();
}

#[test]
fn periodic_sync_does_not_discover() {
    let fx = fx();
    let (cfg, keep) = with_helper(
        &fx,
        OK_SYNC,
        serde_json::json!({"remote_only":["phone-list"],"local_only":[]}),
    );
    let marker = discover_marker(&cfg);
    let td = TestDaemon::start_cfg(&fx, cfg, |_| {});
    // Startup leaves the marker for the first sync; with it consumed, periodic ones do not
    // discover.
    std::fs::remove_file(&marker).unwrap();
    let c = td.client();
    let _: serde_json::Value = c.call("events.subscribe", empty()).unwrap();
    td.handle().post_sync_for_test(SyncReason::Periodic);
    finished(&c, SyncReason::Periodic);
    assert_eq!(discoveries(&keep), 0);
}

fn discover_marker(cfg: &Config) -> PathBuf {
    twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir)
        .unwrap()
        .join("sync/discover")
}

#[test]
fn startup_leaves_a_discovery_marker_that_the_first_sync_consumes() {
    let fx = fx();
    let (cfg, keep) = with_helper(
        &fx,
        OK_SYNC,
        serde_json::json!({"remote_only":["phone-list"],"local_only":[]}),
    );
    let marker = discover_marker(&cfg);
    let td = TestDaemon::start_cfg(&fx, cfg, |_| {});
    assert!(marker.exists(), "no marker after startup");
    let c = td.client();
    let _: serde_json::Value = c.call("events.subscribe", empty()).unwrap();
    td.handle().post_sync_for_test(SyncReason::Batch);
    let r = finished(&c, SyncReason::Batch);
    assert!(
        r.notes.iter().any(|n| n.contains("found new list")),
        "{r:?}"
    );
    assert!(!marker.exists());
    assert_eq!(discoveries(&keep), 1);
}

#[test]
fn startup_discovery_survives_a_startup_sync_skipped_by_backoff() {
    let fx = fx();
    let (cfg, keep) = with_helper(
        &fx,
        OK_SYNC,
        serde_json::json!({"remote_only":["phone-list"],"local_only":[]}),
    );
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    twydo_core::sync::status::SyncStatus {
        next_attempt: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
        ..Default::default()
    }
    .save(&state)
    .unwrap();
    let marker = discover_marker(&cfg);
    let td = TestDaemon::start_cfg(&fx, cfg, |_| {});
    let c = td.client();
    let _: serde_json::Value = c.call("events.subscribe", empty()).unwrap();
    td.handle().post_sync_for_test(SyncReason::Startup);
    std::thread::sleep(Duration::from_millis(500));
    assert!(marker.exists(), "marker lost with the skipped sync");
    assert_eq!(discoveries(&keep), 0);
    // Backoff over: the next periodic sync discovers.
    twydo_core::sync::status::SyncStatus::default()
        .save(&state)
        .unwrap();
    td.handle().post_sync_for_test(SyncReason::Periodic);
    let r = finished(&c, SyncReason::Periodic);
    assert!(
        r.notes.iter().any(|n| n.contains("found new list")),
        "{r:?}"
    );
    assert!(!marker.exists());
}

#[test]
fn sync_run_with_discover_fails_on_missing_remote() {
    let fx = fx();
    let (cfg, _keep) = with_helper(
        &fx,
        OK_SYNC,
        serde_json::json!({"remote_only":[],"local_only":["stale"]}),
    );
    std::fs::create_dir_all(fx.vdir().join("stale")).unwrap();
    let td = TestDaemon::start_cfg(&fx, cfg, |_| {});
    let c = td.client();
    let _: serde_json::Value = c.call("events.subscribe", empty()).unwrap();
    let started: JobStarted = c
        .call(
            "sync.run",
            SyncRunParams {
                request_id: c.next_request_id(),
                force_delete: Vec::new(),
                discover: true,
            },
        )
        .unwrap();
    let r = sync_finished(&c, &started.job_id);
    assert_eq!(r.outcome, SyncOutcome::Failed, "{r:?}");
    assert!(
        r.problems.iter().any(|p| p.contains(":forget-list stale")),
        "{r:?}"
    );
}
