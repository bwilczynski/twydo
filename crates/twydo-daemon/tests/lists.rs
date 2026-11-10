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
    ListArchiveParams, ListCreateParams, ListDeleteParams, ListExportParams,
    ListPreviewDeleteParams, ListUpdateParams, WireDeletePreview, WireListState,
};
use twydo_daemon::protocol::methods::{OpState, WriteOutcome};
use twydo_daemon::protocol::rpc::ErrorKind;

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
        .operation
        .iter_mut()
        .find(|r| r.case == "list-metadata" && r.op == "edit")
        .unwrap();
    row.iphone_baseline = Verdict::Pass;
    row.twydo_to_iphone = Verdict::Pass;
    row.iphone_to_twydo = Verdict::Pass;
    matrix
        .edge_case
        .iter_mut()
        .find(|e| e.id == "list-metadata-atomicity")
        .unwrap()
        .verdict = Verdict::Pass;
    matrix
}

fn env_with(f: impl FnOnce(&mut Config)) -> Env {
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
    let td = TestDaemon::start_cfg(&fx, cfg, |o| o.matrix = matrix());
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

#[test]
fn delete_keeps_todays_refusal() {
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
    let (kind, message) = failed(finished(&e.c, &job));
    assert_eq!(kind, ErrorKind::Blocked);
    assert!(
        message.starts_with("List deletion is unavailable"),
        "{message}"
    );
    assert!(e.fx.vdir().join("c1/u1.ics").exists());
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
