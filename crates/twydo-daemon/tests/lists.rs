//! List actions as jobs: create, update, archive, delete, export, a resent request, and edits
//! saved during a list job (queued, fenced at drain). The list adapter is the core tests'
//! offline helper backend (`twydo-core/tests/common/list_backend.py`): the bundled helper
//! against a synthetic in-memory server. Synthetic fixtures; no network, no real vdirsyncer.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{Fixture, TestDaemon, edit_title, empty, fake_vdirsyncer, submit, wait_event};
use serde_json::{Value, json};
use twydo_core::compat::{Matrix, Verdict};
use twydo_core::config::Config;
use twydo_daemon::client::{Client, Notification};
use twydo_daemon::protocol::events::{Cause, Event};
use twydo_daemon::protocol::jobs::{JobResult, JobStarted, JobState};
use twydo_daemon::protocol::lists::{
    BackupPreviewParams, Backups, ForgetPreview, ListArchiveParams, ListCreateParams,
    ListDeleteParams, ListExportParams, ListForgetParams, ListPreviewDeleteParams,
    ListPreviewForgetParams, ListRecoverParams, ListUpdateParams, WireDeletePreview, WireListState,
};
use twydo_daemon::protocol::methods::{OpState, WriteOutcome};
use twydo_daemon::protocol::rpc::ErrorKind;
use twydo_daemon::protocol::sync::{SyncReason, SyncRunParams};
use twydo_daemon::protocol::wire::WireSnapshot;
use twydo_daemon::server::DaemonOptions;

const BOUND: Duration = Duration::from_secs(20);

/// A daemon serving a vdir with lists `c1` and `c2` (one task each) that the synthetic server
/// also holds, configured from its own `config.toml`.
struct Env {
    fx: Fixture,
    td: TestDaemon,
    c: Client,
}

impl Env {
    fn root(&self) -> PathBuf {
        self.fx.vdir().parent().unwrap().to_path_buf()
    }

    /// Sets a flag of the synthetic server (`ignore_write`: it answers a write without
    /// applying it).
    fn flag(&self, name: &str) {
        let path = self.root().join("server.json");
        let mut server: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        server["flags"][name] = true.into();
        fs::write(path, server.to_string()).unwrap();
    }

    /// From now on the helper writes `started` and sleeps 1 s before it runs.
    fn slow(&self) {
        let _ = fs::remove_file(self.root().join("started"));
        fs::write(self.root().join("slow"), "").unwrap();
    }

    /// Waits until a slow helper has started (the job holds the lane, its operation is on).
    fn wait_helper_started(&self) {
        let deadline = Instant::now() + BOUND;
        while !self.root().join("started").exists() {
            assert!(Instant::now() < deadline, "the helper never started");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

const COLOR: &str = "#3289d2";

/// The helper wrapper: runs the offline backend, first writing `started` and sleeping 1 s
/// while `slow` exists. Answers `__probe` at once (see [`fake_vdirsyncer`]).
fn helper(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let backend = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../twydo-core/tests/common/list_backend.py")
        .canonicalize()
        .unwrap();
    let path = root.join("list-python");
    fs::write(
        &path,
        format!(
            "#!/usr/bin/env python3\nimport os,runpy,sys,time\n\
             if sys.argv[1:] == ['__probe']: sys.exit(0)\n\
             if os.path.exists({slow:?}):\n open({started:?}, 'w').close()\n time.sleep(1)\n\
             runpy.run_path({backend:?}, run_name='__main__')\n",
            slow = root.join("slow"),
            started = root.join("started"),
        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    // A script written while another test thread forks can briefly be busy (ETXTBSY).
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match std::process::Command::new(&path).arg("__probe").status() {
            Ok(st) => {
                assert!(st.success(), "probe of {} failed", path.display());
                return path;
            }
            Err(e) if e.raw_os_error() == Some(26) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("cannot run {}: {e}", path.display()),
        }
    }
}

/// The embedded matrix with list metadata edits admitted, as the core's prototype tests load
/// it (only against the offline server).
fn matrix() -> Matrix {
    let mut matrix = Matrix::embedded().clone();
    let row = matrix
        .contract
        .iter_mut()
        .find(|r| r.id == twydo_core::compat::ContractId::ListMetadata)
        .unwrap();
    row.iphone_baseline = Verdict::Pass;
    row.twydo_to_iphone = Verdict::Pass;
    row.iphone_to_twydo = Verdict::Pass;
    row.atomicity = Some(twydo_core::compat::Verdict::Pass);
    matrix
}

fn env_with(f: impl FnOnce(&mut Config)) -> Env {
    env_opts(f, |_| {})
}

/// [`env_with`], and daemon options set by `o`.
fn env_opts(f: impl FnOnce(&mut Config), o: impl FnOnce(&mut DaemonOptions)) -> Env {
    let fx = Fixture::new(&[("c1", "u1", "One", None), ("c2", "u2", "Two", None)]);
    let root = fx.vdir().parent().unwrap().to_path_buf();
    let mut collections = serde_json::Map::new();
    for (id, name) in [("c1", "Synthetic one"), ("c2", "Synthetic two")] {
        fx.write(&format!("{id}/displayname"), name);
        fx.write(&format!("{id}/color"), COLOR);
        collections.insert(id.into(), json!({"name": name, "color": COLOR}));
    }
    fs::write(
        root.join("server.json"),
        json!({"calls": [], "flags": {}, "collections": collections}).to_string(),
    )
    .unwrap();
    let connection = root.join("connection.json");
    fs::write(&connection, json!({"vdir": fx.vdir()}).to_string()).unwrap();
    let vdirsyncer = fake_vdirsyncer(&root, "exit 0");
    let path = fx.instance().config;
    fs::write(
        &path,
        format!(
            "vdir = {:?}\nstate_dir = {:?}\n[writes]\nmode = \"test\"\n[sync]\n\
             pair = \"synthetic\"\nlocal_storage = \"local\"\ncommand = {:?}\n\
             list_python = {:?}\nvdirsyncer_config = {:?}\nmetasync = false\n",
            fx.vdir(),
            fx.config().state_dir(),
            vdirsyncer,
            helper(&root),
            connection,
        ),
    )
    .unwrap();
    let mut cfg = Config::load(&path).unwrap();
    f(&mut cfg);
    let td = TestDaemon::start_cfg(&fx, cfg, |opts| {
        opts.matrix = matrix();
        o(opts);
    });
    let c = td.client();
    let _: Value = c.call("events.subscribe", empty()).unwrap();
    Env { fx, td, c }
}

fn env() -> Env {
    env_with(|_| {})
}

fn state(id: &str, name: &str, archived: bool) -> WireListState {
    WireListState {
        id: id.into(),
        name: name.into(),
        color: Some(COLOR.into()),
        archived,
    }
}

fn call(c: &Client, method: &str, params: impl serde::Serialize) -> String {
    c.call::<_, JobStarted>(method, params).unwrap().job_id
}

/// The events up to and including `job.finished` of `job_id`, and its state.
fn until_finished(c: &Client, job_id: &str) -> (JobState, Vec<Event>) {
    let deadline = Instant::now() + BOUND;
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match c.notifications().recv_timeout(left) {
            Ok(Notification::Event(ev)) => match ev.event {
                Event::JobFinished {
                    job_id: id, state, ..
                } if id == job_id => return (state, seen),
                other => seen.push(other),
            },
            Ok(_) => {}
            Err(e) => panic!("job {job_id} did not finish within {BOUND:?}: {e}"),
        }
    }
}

fn finished(c: &Client, job_id: &str) -> JobState {
    until_finished(c, job_id).0
}

fn saved(state: JobState) -> WireListState {
    match state {
        JobState::Finished {
            result: JobResult::ListSaved { state, .. },
        } => state,
        other => panic!("expected list_saved: {other:?}"),
    }
}

fn failed(state: JobState) -> (ErrorKind, String) {
    match state {
        JobState::Failed { kind, message } => (kind, message),
        other => panic!("expected failed: {other:?}"),
    }
}

fn preview(e: &Env, id: &str) -> WireDeletePreview {
    let job = call(
        &e.c,
        "lists.preview_delete",
        ListPreviewDeleteParams {
            request_id: e.c.next_request_id(),
            id: id.into(),
        },
    );
    match finished(&e.c, &job) {
        JobState::Finished {
            result: JobResult::DeletePreview(p),
        } => p,
        other => panic!("expected delete_preview: {other:?}"),
    }
}

#[test]
fn create_list_job_finishes_with_the_state() {
    let e = env();
    let job = call(
        &e.c,
        "lists.create",
        ListCreateParams {
            request_id: e.c.next_request_id(),
            id: "c3".into(),
            name: "Synthetic three".into(),
            color: "#379417".into(),
        },
    );
    let (state, before) = until_finished(&e.c, &job);
    let state = saved(state);
    assert_eq!(state.id, "c3");
    assert_eq!(state.name, "Synthetic three");
    assert_eq!(state.color.as_deref(), Some("#379417"));
    assert!(!state.archived);
    assert!(
        before.iter().any(|ev| matches!(
            ev,
            Event::DataChanged {
                cause: Cause::List,
                ..
            }
        )),
        "{before:?}"
    );
    let snap = common::snapshot(&e.c);
    let list = snap.collections.iter().find(|l| l.id == "c3");
    assert_eq!(
        list.and_then(|l| l.display_name.as_deref()),
        Some("Synthetic three"),
        "{:?}",
        snap.collections
    );
}

#[test]
fn update_with_a_stale_expected_state_fails() {
    let e = env();
    let job = call(
        &e.c,
        "lists.update",
        ListUpdateParams {
            request_id: e.c.next_request_id(),
            id: "c1".into(),
            name: "Renamed".into(),
            color: Some(COLOR.into()),
            expected: state("c1", "Another name", false),
        },
    );
    assert_eq!(
        failed(finished(&e.c, &job)),
        (
            ErrorKind::Blocked,
            "list changed; reload and try again".to_string()
        )
    );
    assert_eq!(e.fx.read("c1/displayname"), "Synthetic one");
}

#[test]
fn archive_and_unarchive_write_the_config() {
    let e = env();
    let job = call(
        &e.c,
        "lists.archive",
        ListArchiveParams {
            request_id: e.c.next_request_id(),
            expected: state("c1", "Synthetic one", false),
        },
    );
    assert!(saved(finished(&e.c, &job)).archived);
    wait_event(&e.c, BOUND, |ev| {
        matches!(
            ev,
            Event::DataChanged {
                cause: Cause::Config,
                ..
            }
        )
    });
    assert!(common::snapshot(&e.c).config.lists["c1"].archived);
    let text = fs::read_to_string(e.fx.instance().config).unwrap();
    assert!(text.contains("archived = true"), "{text}");

    let job = call(
        &e.c,
        "lists.unarchive",
        ListArchiveParams {
            request_id: e.c.next_request_id(),
            expected: state("c1", "Synthetic one", true),
        },
    );
    assert!(!saved(finished(&e.c, &job)).archived);
    wait_event(&e.c, BOUND, |ev| {
        matches!(
            ev,
            Event::DataChanged {
                cause: Cause::Config,
                ..
            }
        )
    });
    assert!(
        !common::snapshot(&e.c)
            .config
            .lists
            .get("c1")
            .is_some_and(|p| p.archived)
    );
}

/// The daemon runs the service's best-effort deletion. Its result is still the generic
/// forgotten-list one (the daemon protocol gets a proper deletion result later).
#[test]
fn delete_retires_the_list_through_the_daemon() {
    let e = env();
    let p = preview(&e, "c1");
    assert_eq!(p.collection, "c1");
    assert_eq!(p.name, "Synthetic one");
    let job = call(
        &e.c,
        "lists.delete",
        ListDeleteParams {
            request_id: e.c.next_request_id(),
            preview: p,
        },
    );
    let finished = finished(&e.c, &job);
    assert!(
        matches!(&finished, JobState::Finished { result: JobResult::Forgot { name, .. } } if name == "Synthetic one"),
        "{finished:?}"
    );
    assert!(!e.fx.vdir().join("c1").exists());
}

/// Edits of a task in the list a job is changing, saved while the job runs, are queued. When
/// the job ends they drain: applied when the job finished its operation, refused with the
/// fence's message when it left the operation unfinished.
#[test]
fn edit_during_a_list_job_is_queued_and_fenced_at_drain() {
    let e = env();
    let update = |id: &str, name: &str, old: &str| {
        call(
            &e.c,
            "lists.update",
            ListUpdateParams {
                request_id: e.c.next_request_id(),
                id: id.into(),
                name: name.into(),
                color: Some(COLOR.into()),
                expected: state(id, old, false),
            },
        )
    };
    let op_finished = |events: &[Event], op: &str| {
        events
            .iter()
            .find_map(|ev| match ev {
                Event::OpFinished { op_id, state, .. } if op_id == op => Some(state.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no op.finished for {op} before job.finished: {events:?}"))
    };

    // The job finishes its operation: the edit applies after it.
    e.slow();
    let job = update("c1", "Renamed one", "Synthetic one");
    e.wait_helper_started();
    let r = submit(&e.c, edit_title("u1", "Edited one")).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Queued, "{r:?}");
    let (state, events) = until_finished(&e.c, &job);
    assert_eq!(saved(state).name, "Renamed one");
    assert!(
        matches!(op_finished(&events, &r.op_id), OpState::Applied { .. }),
        "{events:?}"
    );
    assert!(e.fx.read("c1/u1.ics").contains("SUMMARY:Edited one"));

    // The server ignores the write: the operation is left for a reconcile, and the edit is
    // refused at drain with the fence's message.
    e.flag("ignore_write");
    e.slow();
    let job = update("c2", "Renamed two", "Synthetic two");
    e.wait_helper_started();
    let r = submit(&e.c, edit_title("u2", "Edited two")).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Queued, "{r:?}");
    let (state, events) = until_finished(&e.c, &job);
    let (kind, message) = failed(state);
    assert_eq!(kind, ErrorKind::Blocked);
    assert!(message.contains("needs reconciliation"), "{message}");
    match op_finished(&events, &r.op_id) {
        OpState::Refused { kind, message } => {
            assert_eq!(kind, ErrorKind::Blocked);
            assert!(
                message.contains("is unfinished; reconcile it before writing or syncing"),
                "{message}"
            );
        }
        other => panic!("expected refused: {other:?}"),
    }
    assert!(e.fx.read("c2/u2.ics").contains("SUMMARY:Two"));
}

/// An undo saved during a list job is queued too (its fence waits for the drain), and applies
/// after the job.
#[test]
fn undo_during_a_list_job_is_queued() {
    let e = env();
    let r = submit(&e.c, edit_title("u2", "Edited two")).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Applied, "{r:?}");
    e.slow();
    let job = call(
        &e.c,
        "lists.update",
        ListUpdateParams {
            request_id: e.c.next_request_id(),
            id: "c1".into(),
            name: "Renamed one".into(),
            color: Some(COLOR.into()),
            expected: state("c1", "Synthetic one", false),
        },
    );
    e.wait_helper_started();
    let u = common::undo(&e.c).unwrap();
    assert_eq!(u.outcome, WriteOutcome::Queued, "{u:?}");
    let (state, events) = until_finished(&e.c, &job);
    saved(state);
    assert!(
        events.iter().any(|ev| matches!(
            ev,
            Event::OpFinished { op_id, state: OpState::Applied { .. }, .. } if *op_id == u.op_id
        )),
        "{events:?}"
    );
    assert!(e.fx.read("c2/u2.ics").contains("SUMMARY:Two"));
}

#[test]
fn export_writes_the_backup_file() {
    let e = env();
    let p = preview(&e, "c1");
    let destination = e.root().join("export");
    let job = call(
        &e.c,
        "lists.export",
        ListExportParams {
            request_id: e.c.next_request_id(),
            backup: p.backup.clone(),
            destination: destination.clone(),
        },
    );
    match finished(&e.c, &job) {
        JobState::Finished {
            result:
                JobResult::Exported {
                    backup,
                    destination: d,
                },
        } => {
            assert_eq!(backup, p.backup);
            assert_eq!(d, destination);
        }
        other => panic!("expected exported: {other:?}"),
    }
    let remote = fs::read_dir(destination.join("remote")).unwrap().count();
    assert_eq!(remote, p.resources);
}

#[test]
fn resent_list_job_returns_the_same_job() {
    let e = env();
    let params = ListCreateParams {
        request_id: e.c.next_request_id(),
        id: "c3".into(),
        name: "Synthetic three".into(),
        color: "#379417".into(),
    };
    let first = call(&e.c, "lists.create", params.clone());
    let again = call(&e.c, "lists.create", params.clone());
    assert_eq!(first, again);
    saved(finished(&e.c, &first));
    // After the job ended too: no second create (which would refuse the existing list).
    assert_eq!(call(&e.c, "lists.create", params), first);
    let server: Value =
        serde_json::from_slice(&fs::read(e.root().join("server.json")).unwrap()).unwrap();
    let creates = server["calls"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| *v == "MKCALENDAR")
        .count();
    assert_eq!(creates, 1);
}

#[test]
fn invalid_colour_is_refused_before_a_job_starts() {
    let e = env();
    let err =
        e.c.call::<_, JobStarted>(
            "lists.create",
            ListCreateParams {
                request_id: e.c.next_request_id(),
                id: "c3".into(),
                name: "Synthetic three".into(),
                color: "green".into(),
            },
        )
        .unwrap_err();
    assert_eq!(common::kind(&err), ErrorKind::Invalid);
    let _ = &e.td;
}

/// Forgets `c1` (as `Synthetic one`); returns the job's state and the events before it ended.
fn forget(e: &Env) -> (JobState, Vec<Event>) {
    let p: ForgetPreview =
        e.c.call(
            "lists.preview_forget",
            ListPreviewForgetParams { name: "c1".into() },
        )
        .unwrap();
    let job = call(
        &e.c,
        "lists.forget",
        ListForgetParams {
            request_id: e.c.next_request_id(),
            name: "c1".into(),
            collection: p.collection,
        },
    );
    until_finished(&e.c, &job)
}

fn forgot(state: JobState) -> (String, String, bool) {
    match state {
        JobState::Finished {
            result:
                JobResult::Forgot {
                    backup,
                    message,
                    sync,
                    ..
                },
        } => (backup, message, sync.is_some()),
        other => panic!("expected forgot: {other:?}"),
    }
}

fn recover_params(e: &Env, backup: &str, to: &str) -> ListRecoverParams {
    ListRecoverParams {
        session: e.c.session().expect("hello"),
        request_id: e.c.next_request_id(),
        backup: backup.into(),
        to: to.into(),
    }
}

fn recovered(state: JobState) -> (Option<String>, bool, String) {
    match state {
        JobState::Finished {
            result:
                JobResult::Recovered {
                    op_id,
                    applied,
                    message,
                },
        } => (op_id, applied, message),
        other => panic!("expected recovered: {other:?}"),
    }
}

#[test]
fn forget_preserves_and_syncs() {
    let e = env();
    let p: ForgetPreview =
        e.c.call(
            "lists.preview_forget",
            ListPreviewForgetParams { name: "c1".into() },
        )
        .unwrap();
    assert_eq!(
        (p.collection.as_str(), p.name.as_str(), p.tasks, p.pending),
        ("c1", "Synthetic one", 1, 0)
    );
    let (state, events) = forget(&e);
    let (backup, message, synced) = forgot(state);
    assert_eq!(
        message,
        format!(
            "preserved \"Synthetic one\" as {backup}; recover with :recover {backup} <destination-list>"
        )
    );
    assert!(synced);
    let started = events
        .iter()
        .filter(|ev| {
            matches!(
                ev,
                Event::SyncStarted {
                    reason: SyncReason::List,
                    ..
                }
            )
        })
        .count();
    assert_eq!(started, 1);
    let snap: WireSnapshot = e.c.call("snapshot.get", empty()).unwrap();
    assert!(snap.collections.iter().all(|c| c.id != "c1"));
}

#[test]
fn forget_without_a_sync_table_runs_no_sync() {
    let e = env_with(|cfg| cfg.sync = None);
    let (state, events) = forget(&e);
    let (_, _, synced) = forgot(state);
    assert!(!synced);
    assert!(
        events
            .iter()
            .all(|ev| !matches!(ev, Event::SyncStarted { .. } | Event::SyncFinished { .. })),
        "{events:?}"
    );
}

#[test]
fn forget_sends_sync_finished_before_job_finished() {
    let e = env();
    let p: ForgetPreview =
        e.c.call(
            "lists.preview_forget",
            ListPreviewForgetParams { name: "c1".into() },
        )
        .unwrap();
    let job = call(
        &e.c,
        "lists.forget",
        ListForgetParams {
            request_id: e.c.next_request_id(),
            name: "c1".into(),
            collection: p.collection,
        },
    );
    let (_, mut events) = until_finished(&e.c, &job);
    // `until_finished` stops at `job.finished`: it comes after every event it returned.
    events.push(Event::JobFinished {
        job_id: job.clone(),
        request_id: None,
        state: JobState::Interrupted,
    });
    let at = |f: &dyn Fn(&Event) -> bool| events.iter().position(f);
    let sync = at(&|ev| {
        matches!(
            ev,
            Event::SyncFinished {
                reason: SyncReason::List,
                ..
            }
        )
    });
    let end = at(&|ev| matches!(ev, Event::JobFinished { .. }));
    assert!(sync.is_some(), "{events:?}");
    assert!(sync < end, "{events:?}");
}

#[test]
fn forget_with_a_changed_list_fails() {
    let e = env();
    let job = call(
        &e.c,
        "lists.forget",
        ListForgetParams {
            request_id: e.c.next_request_id(),
            name: "c1".into(),
            collection: "c2".into(),
        },
    );
    let (kind, message) = failed(finished(&e.c, &job));
    assert_eq!(kind, ErrorKind::Blocked);
    assert_eq!(message, "list changed; reload and try again");
    let snap: WireSnapshot = e.c.call("snapshot.get", empty()).unwrap();
    assert!(snap.collections.iter().any(|c| c.id == "c1"));
}

#[test]
fn recover_into_a_list_applies_once() {
    let e = env();
    let (backup, _, _) = forgot(forget(&e).0);
    let params = recover_params(&e, &backup, "c2");
    let job = call(&e.c, "lists.recover", params.clone());
    assert_eq!(call(&e.c, "lists.recover", params.clone()), job);
    let (op_id, applied, message) = recovered(finished(&e.c, &job));
    assert!(applied && op_id.is_some());
    assert_eq!(message, "recovered local snapshot into c2; backup retained");
    let snap: WireSnapshot = e.c.call("snapshot.get", empty()).unwrap();
    assert_eq!(
        snap.tasks
            .iter()
            .filter(|t| t.task.path.starts_with("c2/"))
            .count(),
        2
    );

    let again = call(&e.c, "lists.recover", recover_params(&e, &backup, "c2"));
    let (_, applied, message) = recovered(finished(&e.c, &again));
    assert!(!applied);
    assert_eq!(message, "already recovered; backup retained");
}

#[test]
fn recover_waits_behind_a_sync_instead_of_failing() {
    let e = env();
    let (backup, _, _) = forgot(forget(&e).0);
    fake_vdirsyncer(&e.root(), "sleep 1");
    let sync: JobStarted =
        e.c.call(
            "sync.run",
            SyncRunParams {
                request_id: e.c.next_request_id(),
                force_delete: Vec::new(),
                discover: false,
            },
        )
        .unwrap();
    wait_event(&e.c, BOUND, |ev| matches!(ev, Event::SyncStarted { .. }));
    let job = call(&e.c, "lists.recover", recover_params(&e, &backup, "c2"));
    let (state, events) = until_finished(&e.c, &job);
    let (_, applied, _) = recovered(state);
    assert!(applied);
    assert!(
        events.iter().any(|ev| matches!(
            ev,
            Event::SyncFinished { job_id: Some(id), .. } if *id == sync.job_id
        )),
        "sync.finished must come before the recover finishes: {events:?}"
    );
}

#[test]
fn backups_list_both_kinds() {
    let e = env();
    let (backup, _, _) = forgot(forget(&e).0);
    let deleted = preview(&e, "c2");
    let b: Backups = e.c.call("lists.backups", empty()).unwrap();
    assert_eq!(b.retired.len(), 1);
    let r = &b.retired[0];
    assert_eq!(
        (
            r.id.as_str(),
            r.collection.as_str(),
            r.name.as_str(),
            r.tasks,
            r.pending,
            r.retired
        ),
        (backup.as_str(), "c1", "Synthetic one", 1, 0, true)
    );
    assert_eq!(b.deleted.len(), 1);
    assert_eq!(b.deleted[0].id, deleted.backup);
    assert_eq!(b.deleted[0].collection, "c2");
}

#[test]
fn backup_preview_shows_the_tasks() {
    let e = env();
    let (backup, _, _) = forgot(forget(&e).0);
    let snap: WireSnapshot =
        e.c.call("lists.backup_preview", BackupPreviewParams { id: backup })
            .unwrap();
    assert_eq!(snap.revision, 0);
    assert_eq!(snap.tasks.len(), 1);
    assert_eq!(snap.collections.len(), 1);
    assert_eq!(snap.collections[0].id, "c1");
}

#[test]
fn recover_can_be_undone() {
    let e = env();
    let (backup, _, _) = forgot(forget(&e).0);
    let job = call(&e.c, "lists.recover", recover_params(&e, &backup, "c2"));
    assert!(recovered(finished(&e.c, &job)).1);
    let count = |e: &Env| {
        let snap: WireSnapshot = e.c.call("snapshot.get", empty()).unwrap();
        snap.tasks
            .iter()
            .filter(|t| t.task.path.starts_with("c2/"))
            .count()
    };
    assert_eq!(count(&e), 2);

    // Another session's undo does not touch it.
    let other = e.td.client();
    let _ = common::undo(&other);
    assert_eq!(count(&e), 2);

    let r = common::undo(&e.c).unwrap();
    assert_eq!(r.message, "undone: recovered list");
    assert_eq!(count(&e), 1);
}

#[test]
fn recover_with_an_unknown_session_is_refused() {
    let e = env();
    let (backup, _, _) = forgot(forget(&e).0);
    let mut p = recover_params(&e, &backup, "c2");
    p.session = "s-nope".into();
    let err = e.c.call::<_, JobStarted>("lists.recover", p).unwrap_err();
    assert_eq!(common::kind(&err), ErrorKind::NotFound);
}

/// Final review: a panic in the recovery (holding the session) fails the job and leaves the
/// session usable: its writes and undo still work.
#[test]
fn a_panic_in_recover_leaves_the_session_usable() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let armed = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&armed);
    let e = env_opts(
        |_| {},
        move |o| {
            o.test_point = Some(Arc::new(move |p: &str| {
                if p == "recover" && flag.swap(false, Ordering::SeqCst) {
                    panic!("test panic in recover");
                }
            }));
        },
    );
    let (backup, _, _) = forgot(forget(&e).0);
    armed.store(true, Ordering::SeqCst);
    let job = call(&e.c, "lists.recover", recover_params(&e, &backup, "c2"));
    let (kind, _) = failed(finished(&e.c, &job));
    assert_eq!(kind, ErrorKind::Internal);
    let w = submit(&e.c, edit_title("u2", "After")).unwrap();
    assert_eq!(w.outcome, WriteOutcome::Applied);
    let u = common::undo(&e.c).unwrap();
    assert_eq!(u.message, "undone: Two");
    // The recovery itself runs on a retry.
    let again = call(&e.c, "lists.recover", recover_params(&e, &backup, "c2"));
    assert!(recovered(finished(&e.c, &again)).1);
}

/// Final review: the unknown-session refusal is not recorded (as for task writes), so the same
/// request resent under a new session runs.
#[test]
fn recover_resent_under_a_new_session_runs() {
    let e = env();
    let (backup, _, _) = forgot(forget(&e).0);
    let mut p = recover_params(&e, &backup, "c2");
    p.session = "s-expired".into();
    let err =
        e.c.call::<_, JobStarted>("lists.recover", p.clone())
            .unwrap_err();
    assert_eq!(common::kind(&err), ErrorKind::NotFound);
    // Hello again: a new session; the same request id.
    let fresh = e.td.client();
    p.session = fresh.session().expect("hello");
    let job = call(&e.c, "lists.recover", p);
    assert!(recovered(finished(&e.c, &job)).1);
}

/// Writes an unfinished list operation record and marks it active, as an interrupted save
/// leaves it.
fn seed_pending(fx: &Fixture, observed: Option<Value>) -> String {
    use twydo_core::lists::helper::HelperRequest;
    use twydo_core::lists::operation::{ListOperation, Phase};
    let cfg = fx.config();
    let state = twydo_core::store::bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let id = "op-synthetic-1".to_string();
    let request = HelperRequest {
        version: 1,
        action: "update".into(),
        pair: "p".into(),
        local_storage: "s".into(),
        vdir: fx.vdir(),
        config: None,
        collection: Some("inbox".into()),
        operation: None,
        backup: None,
        state: None,
        name: Some("Renamed".into()),
        color: Some("#112233".into()),
        expected: None,
        mode: "execute".into(),
        keep_server: false,
        timeout_secs: 5,
    };
    let record = ListOperation {
        format: 1,
        id: id.clone(),
        created: chrono::Utc::now(),
        vdir: fs::canonicalize(fx.vdir()).unwrap(),
        phase: Phase::Uncertain,
        request,
        holds: None,
        receipt: observed.map(|o| json!({"observed": o})),
        remote_target: None,
        connection_digest: None,
        local_base: None,
        local_after: None,
    };
    twydo_core::lists::operation::save(&state, &record).unwrap();
    fs::create_dir_all(state.join("lists")).unwrap();
    fs::write(state.join("lists/active"), &id).unwrap();
    id
}

#[test]
fn lists_pending_reports_an_unfinished_list_save() {
    use twydo_daemon::protocol::lists::PendingListOp;
    let fx = Fixture::new(&[("inbox", "u1", "One", None)]);
    let id = seed_pending(
        &fx,
        Some(json!({"name": "Server name", "color": "#445566"})),
    );
    let td = TestDaemon::start(&fx);
    let got: Option<PendingListOp> = td.client().call("lists.pending", empty()).unwrap();
    assert_eq!(
        got,
        Some(PendingListOp {
            operation: id,
            action: "update".into(),
            collection: Some("inbox".into()),
            name: Some("Renamed".into()),
            color: Some("#112233".into()),
            observed_name: Some("Server name".into()),
            observed_color: Some("#445566".into()),
        })
    );
}

#[test]
fn lists_pending_is_none_when_idle() {
    use twydo_daemon::protocol::lists::PendingListOp;
    let fx = Fixture::new(&[("inbox", "u1", "One", None)]);
    let td = TestDaemon::start(&fx);
    let got: Option<PendingListOp> = td.client().call("lists.pending", empty()).unwrap();
    assert_eq!(got, None);
}

/// A fixture with a journal (so startup recovery really runs) and an unfinished list
/// operation on `inbox` (a preview, which reconciling clears without the server).
fn fenced_fixture() -> (Fixture, Config, String) {
    let fx = Fixture::new(&[("inbox", "u1", "One", None)]);
    let mut cfg = fx.config();
    cfg.sync = Some(twydo_core::config::SyncConfig::default());
    fs::create_dir_all(fx.state().join("journal")).unwrap();
    let mut request = twydo_core::lists::helper::HelperRequest::probe(&cfg).unwrap();
    request.action = "preview-delete".into();
    request.collection = Some("inbox".into());
    let record = twydo_core::lists::operation::prepare(&cfg, request, chrono::Utc::now()).unwrap();
    (fx, cfg, record.id)
}

#[test]
fn a_daemon_starts_over_an_unfinished_list_operation() {
    use twydo_daemon::protocol::lists::PendingListOp;
    use twydo_daemon::protocol::methods::StatusView;
    let (fx, cfg, id) = fenced_fixture();
    let td = TestDaemon::start_cfg(&fx, cfg, |_| {});
    let c = td.client();
    let pending: Option<PendingListOp> = c.call("lists.pending", empty()).unwrap();
    assert_eq!(pending.expect("the operation is pending").operation, id);
    assert_eq!(common::snapshot(&c).tasks.len(), 1);
    let st: StatusView = c.call("status.get", empty()).unwrap();
    assert!(
        st.drain_error
            .as_deref()
            .is_some_and(|m| m.contains("waits for the unfinished operation")),
        "{:?}",
        st.drain_error
    );
    // A task write into the fenced list is not applied.
    let _ = submit(&c, edit_title("u1", "Changed"));
    assert!(!fx.read("inbox/u1.ics").contains("Changed"));
}

#[test]
fn reconciling_the_operation_lets_recovery_and_writes_go_on() {
    use twydo_daemon::protocol::lists::{ListReconcileParams, PendingListOp};
    use twydo_daemon::protocol::methods::StatusView;
    let (fx, cfg, id) = fenced_fixture();
    // Startup can finish over the fence, including after a daemon restart.
    let first = TestDaemon::start_cfg(&fx, cfg.clone(), |_| {});
    let pending: Option<PendingListOp> = first.client().call("lists.pending", empty()).unwrap();
    assert_eq!(pending.expect("the operation is pending").operation, id);
    first.handle().shutdown();
    drop(first);
    let td = TestDaemon::start_cfg(&fx, cfg, |_| {});
    let c = td.client();
    let _: Value = c.call("events.subscribe", empty()).unwrap();
    let job = call(
        &c,
        "lists.reconcile",
        ListReconcileParams {
            request_id: c.next_request_id(),
            operation: id,
            keep_server: false,
        },
    );
    // The preview needs no server: the job may report the helper's refusal, the fence is gone.
    let _ = finished(&c, &job);
    let pending: Option<PendingListOp> = c.call("lists.pending", empty()).unwrap();
    assert_eq!(pending, None);
    let st: StatusView = c.call("status.get", empty()).unwrap();
    assert_eq!(st.drain_error, None);
    let original = fx.read("inbox/u1.ics");
    let written = submit(&c, edit_title("u1", "Changed")).unwrap();
    assert_eq!(written.outcome, WriteOutcome::Applied);
    assert!(fx.read("inbox/u1.ics").contains("Changed"));
    let undone = common::undo(&c).unwrap();
    assert_eq!(undone.outcome, WriteOutcome::Applied);
    assert_eq!(undone.message, "undone: One");
    assert_eq!(fx.read("inbox/u1.ics"), original);
}
