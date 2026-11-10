//! The TUI link against an in-process daemon. Every test has its own socket; content is
//! synthetic and no real vdirsyncer runs.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{TestEnv, short_socket_dir, write_script};
use serde_json::json;
use tdo::app::{ListAction, ListCompletion, SYNC_DONE};
use tdo::term::link::{Command, Connector, Link, Update, WriteRequest, sync_message};
use tdo::write::Request;
use twydo_core::compat::{Matrix, Verdict};
use twydo_core::config::Config;
use twydo_core::edit::{Change, Op};
use twydo_core::lists::{ListRequest, ListResult};
use twydo_daemon::client::Client;
use twydo_daemon::protocol::events::Cause;
use twydo_daemon::protocol::methods::{HelloParams, Instance};
use twydo_daemon::protocol::sync::{SyncOutcome, SyncResult};
use twydo_daemon::server::{Daemon, DaemonHandle, DaemonOptions};

const BOUND: Duration = Duration::from_secs(20);

/// A link that connects to `socket` as `instance` (no daemon is started for it).
fn link_to(socket: PathBuf, instance: Instance) -> Link {
    let connect: Connector = Box::new(move |session| {
        let hello = HelloParams {
            client: "link-test".into(),
            version: "0".into(),
            protocol: twydo_daemon::PROTOCOL,
            session,
            instance: instance.clone(),
        };
        Client::connect(&socket, hello)
            .map_err(|e| tdo::client::ConnectError::Unavailable(e.to_string()))
    });
    Link::start_with(connect)
}

fn link_for(env: &TestEnv) -> Link {
    link_to(env.socket(), env.instance())
}

/// Updates until `pick` accepts one, within [`BOUND`].
fn until<T>(link: &Link, mut pick: impl FnMut(Update) -> Option<T>) -> T {
    let end = Instant::now() + BOUND;
    loop {
        let left = end.saturating_duration_since(Instant::now());
        let Some(u) = link.next_update(left) else {
            panic!("no matching update within {BOUND:?}");
        };
        if let Some(t) = pick(u) {
            return t;
        }
    }
}

fn task_ics(uid: &str, summary: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\nUID:{uid}\r\n\
         SUMMARY:{summary}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
    )
}

#[test]
fn link_delivers_the_first_snapshot_and_status() {
    let env = TestEnv::new("");
    fs::write(
        env.vdir().join("inbox/a.ics"),
        task_ics("uid-a", "Synthetic"),
    )
    .unwrap();
    let _daemon = env.in_process_daemon(None);
    let link = link_for(&env);
    let mut seen = Vec::new();
    let (mut snapshot, mut status) = (None, None);
    until(&link, |u| {
        match u {
            Update::Connected { restarted, .. } => {
                assert!(!restarted);
                seen.push("connected");
            }
            Update::Snapshot {
                revision,
                snapshot: s,
                ..
            } => {
                seen.push("snapshot");
                snapshot = Some((revision, s.tasks.len()));
            }
            Update::Status(s) => {
                seen.push("status");
                status = Some(s);
            }
            other => panic!("unexpected {other:?}"),
        }
        (snapshot.is_some() && status.is_some()).then_some(())
    });
    assert_eq!(seen[0], "connected");
    assert_eq!(snapshot.unwrap().1, 1);
    // No [sync]: the status has no sync part.
    assert!(status.unwrap().sync.is_none());
    let based_on = link.based_on().unwrap();
    assert_eq!(based_on.revision, snapshot.unwrap().0);
    assert!(link.caller().is_some());
}

#[test]
fn an_outside_change_arrives_as_a_snapshot_with_cause_outside() {
    let env = TestEnv::new("");
    let daemon = env.in_process_daemon(None);
    let link = link_for(&env);
    until(&link, |u| matches!(u, Update::Status(_)).then_some(()));
    fs::write(
        env.vdir().join("inbox/b.ics"),
        task_ics("uid-b", "From outside"),
    )
    .unwrap();
    daemon
        .reload_for_test_with(Cause::Outside, vec!["uid-b".into()])
        .unwrap();
    let (cause, uids, tasks) = until(&link, |u| match u {
        Update::Snapshot {
            cause,
            uids,
            snapshot,
            ..
        } if cause == Cause::Outside => Some((cause, uids, snapshot.tasks.len())),
        _ => None,
    });
    assert_eq!(cause, Cause::Outside);
    assert_eq!(uids, vec!["uid-b".to_string()]);
    assert_eq!(tasks, 1);
}

#[test]
fn link_reconnects_and_reports_a_restart() {
    let env = TestEnv::new("");
    let first = env.in_process_daemon(None);
    let link = link_for(&env);
    until(&link, |u| matches!(u, Update::Status(_)).then_some(()));
    first.shutdown();
    drop(first);
    let begun = Instant::now();
    let down = until(&link, |u| match u {
        Update::Down(t) => Some(t),
        _ => None,
    });
    assert!(
        begun.elapsed() < Duration::from_secs(2),
        "{:?}",
        begun.elapsed()
    );
    assert!(down.contains("daemon not running"), "{down}");
    assert!(down.ends_with("retrying"), "{down}");
    assert!(link.caller().is_none());
    let _second = env.in_process_daemon(None);
    let restarted = until(&link, |u| match u {
        Update::Connected { restarted, .. } => Some(restarted),
        _ => None,
    });
    assert!(restarted);
    until(&link, |u| {
        matches!(u, Update::Snapshot { .. }).then_some(())
    });
    assert!(link.caller().is_some());
}

#[test]
fn missed_events_refetch_snapshot_and_status() {
    let env = TestEnv::new("");
    let cfg = Config::load(&env.config_path()).unwrap();
    let mut opts = DaemonOptions::new(env.socket());
    opts.schedule = false;
    opts.event_buffer = 1;
    let daemon = Daemon::start(cfg, env.instance(), opts).unwrap();
    assert!(daemon.wait_ready(Duration::from_secs(10)));
    let link = link_for(&env);
    until(&link, |u| matches!(u, Update::Status(_)).then_some(()));
    let mut last = 0;
    for i in 0..12 {
        let uid = format!("uid-{i}");
        fs::write(
            env.vdir().join(format!("inbox/{uid}.ics")),
            task_ics(&uid, "Burst"),
        )
        .unwrap();
        last = daemon.reload_for_test(Cause::Outside).unwrap();
    }
    let tasks = until(&link, |u| match u {
        Update::Snapshot {
            revision, snapshot, ..
        } if revision == last => Some(snapshot.tasks.len()),
        _ => None,
    });
    assert_eq!(tasks, 12);
    assert_eq!(link.based_on().unwrap().revision, last);
}

#[test]
fn a_write_through_the_link_is_answered_and_refetched() {
    let env = TestEnv::new("");
    fs::write(env.vdir().join("inbox/a.ics"), task_ics("uid-a", "Before")).unwrap();
    let _daemon = env.in_process_daemon(None);
    let link = link_for(&env);
    until(&link, |u| matches!(u, Update::Status(_)).then_some(()));
    let based_on = link.based_on().unwrap();
    link.send(Command::Write {
        seq: 7,
        write: WriteRequest {
            request: Request::Apply(Op::Edit {
                uid: "uid-a".into(),
                changes: vec![Change::Summary("After".into())],
            }),
            based_on,
        },
    });
    let (seq, written) = until(&link, |u| match u {
        Update::Written { seq, result } => Some((seq, result)),
        _ => None,
    });
    assert_eq!(seq, 7);
    assert!(written.is_ok(), "{written:?}");
    // The snapshot after an own write says so (cause `op`), with the uids when the daemon's
    // event came first.
    let (title, cause) = until(&link, |u| match u {
        Update::Snapshot {
            snapshot, cause, ..
        } => Some((snapshot.tasks[0].summary.clone(), cause)),
        _ => None,
    });
    assert_eq!(title, "After");
    assert_eq!(cause, Cause::Op);
}

#[test]
fn a_write_resent_after_a_reconnect_applies_once() {
    let env = TestEnv::new("");
    fs::write(env.vdir().join("inbox/a.ics"), task_ics("uid-a", "Before")).unwrap();
    let _daemon = env.in_process_daemon(None);
    let (proxy, submitted) =
        common::lossy_proxy(env.socket(), env.sock_dir.path(), common::Lose::First);
    let link = link_to(proxy, env.instance());
    until(&link, |u| matches!(u, Update::Status(_)).then_some(()));
    let based_on = link.based_on().unwrap();
    link.send(Command::Write {
        seq: 1,
        write: WriteRequest {
            request: Request::Apply(Op::Edit {
                uid: "uid-a".into(),
                changes: vec![Change::Summary("After".into())],
            }),
            based_on,
        },
    });
    // The first answer is lost with the connection; the link reconnects and asks again.
    let (seq, result) = until(&link, |u| match u {
        Update::Written { seq, result } => Some((seq, result)),
        _ => None,
    });
    assert_eq!(seq, 1);
    assert!(result.is_ok(), "{result:?}");
    // Exactly one reply, however long we wait.
    while let Some(u) = link.next_update(Duration::from_millis(500)) {
        assert!(
            !matches!(u, Update::Written { .. }),
            "a second reply: {u:?}"
        );
    }
    let ids = submitted.lock().unwrap().clone();
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert_eq!(ids[0], ids[1], "the resend must reuse the request id");
    // Applied once: one journal entry for it, and the task changed.
    let c = env.connect(&env.socket());
    let entries: Vec<twydo_daemon::protocol::admin::JournalEntryView> =
        c.call("journal.list", json!({})).unwrap();
    let ours: Vec<_> = entries
        .iter()
        .filter(|e| e.request_id.as_deref() == Some(ids[0].as_str()))
        .collect();
    assert_eq!(ours.len(), 1, "{entries:?}");
    assert_eq!(entries.len(), 1, "{entries:?}");
    let text = fs::read_to_string(env.vdir().join("inbox/a.ics")).unwrap();
    assert!(text.contains("SUMMARY:After"), "{text}");
}

#[test]
fn sync_message_matches_tdo_sync_wording() {
    let r = |outcome, problems: &[&str], warnings: &[&str]| SyncResult {
        outcome,
        problems: problems.iter().map(|s| s.to_string()).collect(),
        warnings: warnings.iter().map(|s| s.to_string()).collect(),
        notes: Vec::new(),
        new_conflicts: 0,
        drained: 0,
    };
    assert_eq!(
        sync_message(&r(SyncOutcome::Failed, &["one", "two"], &["careful"])),
        "one · two · warning: careful"
    );
    assert_eq!(sync_message(&r(SyncOutcome::Ok, &[], &[])), SYNC_DONE);
    assert_eq!(
        sync_message(&r(SyncOutcome::Failed, &[], &[])),
        "sync failed"
    );
    assert_eq!(
        sync_message(&r(SyncOutcome::Stopped, &[], &[])),
        "sync stopped"
    );
    assert_eq!(
        sync_message(&r(SyncOutcome::Unknown, &[], &[])),
        "sync stopped"
    );
    assert_eq!(sync_message(&r(SyncOutcome::Ok, &[], &["w"])), "warning: w");
}

#[test]
fn a_sync_through_the_link_reports_its_message() {
    let env = TestEnv::with_sync("exit 0");
    let _daemon = env.in_process_daemon(None);
    let link = link_for(&env);
    until(&link, |u| matches!(u, Update::Status(_)).then_some(()));
    link.send(Command::Sync {
        force_delete: Vec::new(),
    });
    let done = until(&link, |u| match u {
        Update::SyncDone(m) => Some(m),
        _ => None,
    });
    assert_eq!(done, SYNC_DONE);
}

/// A daemon with lists `c1` and `c2` that a synthetic server also holds (the core tests'
/// offline list backend), and the way to reach it.
struct ListEnv {
    _root: tempfile::TempDir,
    _sockets: tempfile::TempDir,
    daemon: DaemonHandle,
    socket: PathBuf,
    instance: Instance,
}

const COLOR: &str = "#3289d2";

fn list_env() -> ListEnv {
    let root = tempfile::tempdir().unwrap();
    let sockets = short_socket_dir();
    let vdir = root.path().join("vdir");
    let mut collections = serde_json::Map::new();
    for (id, name) in [("c1", "Synthetic one"), ("c2", "Synthetic two")] {
        fs::create_dir_all(vdir.join(id)).unwrap();
        fs::write(
            vdir.join(id).join(format!("u-{id}.ics")),
            task_ics(&format!("u-{id}"), "One"),
        )
        .unwrap();
        fs::write(vdir.join(id).join("displayname"), name).unwrap();
        fs::write(vdir.join(id).join("color"), COLOR).unwrap();
        collections.insert(id.into(), json!({"name": name, "color": COLOR}));
    }
    fs::write(
        root.path().join("server.json"),
        json!({"calls": [], "flags": {}, "collections": collections}).to_string(),
    )
    .unwrap();
    let connection = root.path().join("connection.json");
    fs::write(&connection, json!({"vdir": vdir}).to_string()).unwrap();
    let vdirsyncer = root.path().join("fake-vdirsyncer");
    write_script(&vdirsyncer, "exit 0");
    let backend = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../twydo-core/tests/common/list_backend.py")
        .canonicalize()
        .unwrap();
    let helper = root.path().join("list-python");
    fs::write(
        &helper,
        format!(
            "#!/usr/bin/env python3\nimport runpy,sys\n\
             if sys.argv[1:] == ['__probe']: sys.exit(0)\n\
             runpy.run_path({backend:?}, run_name='__main__')\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
    // A script written while another test thread forks can briefly be busy: probe it.
    let end = Instant::now() + Duration::from_secs(5);
    while std::process::Command::new(&helper)
        .arg("__probe")
        .status()
        .map(|s| !s.success())
        .unwrap_or(true)
    {
        assert!(Instant::now() < end, "the list helper never ran");
        std::thread::sleep(Duration::from_millis(10));
    }
    let config = root.path().join("config.toml");
    fs::write(
        &config,
        format!(
            "vdir = {vdir:?}\nstate_dir = {:?}\n[writes]\nmode = \"test\"\n[sync]\n\
             pair = \"synthetic\"\nlocal_storage = \"local\"\ncommand = {vdirsyncer:?}\n\
             list_python = {helper:?}\nvdirsyncer_config = {connection:?}\nmetasync = false\n",
            root.path().join("state"),
        ),
    )
    .unwrap();
    let cfg = Config::load(&config).unwrap();
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
    let socket = sockets.path().join("s").join("tdo.sock");
    let mut opts = DaemonOptions::new(socket.clone());
    opts.schedule = false;
    opts.matrix = matrix;
    let instance = Instance {
        config,
        vdir: Some(vdir),
    };
    let daemon = Daemon::start(cfg, instance.clone(), opts).unwrap();
    assert!(daemon.wait_ready(Duration::from_secs(10)));
    ListEnv {
        _root: root,
        _sockets: sockets,
        daemon,
        socket,
        instance,
    }
}

#[test]
fn a_list_save_through_the_link_reports_its_result() {
    let e = list_env();
    let _ = &e.daemon;
    let link = link_to(e.socket.clone(), e.instance.clone());
    until(&link, |u| matches!(u, Update::Status(_)).then_some(()));
    let action = ListAction::Save(ListRequest::Create {
        id: "c3".into(),
        name: "Synthetic three".into(),
        color: (0x37, 0x94, 0x17),
    });
    link.send(Command::List(action.clone()));
    let done: ListCompletion = until(&link, |u| match u {
        Update::ListDone(c) => Some(c),
        _ => None,
    });
    assert_eq!(done.action, action);
    match done.result {
        Ok(ListResult::Saved { state, .. }) => {
            assert_eq!(state.id, "c3");
            assert_eq!(state.name, "Synthetic three");
            assert_eq!(state.color, Some((0x37, 0x94, 0x17)));
        }
        other => panic!("expected a saved list: {other:?}"),
    }
    assert_eq!(done.pending, None);
}

#[test]
fn a_failed_list_job_reports_its_message() {
    let e = list_env();
    let link = link_to(e.socket.clone(), e.instance.clone());
    until(&link, |u| matches!(u, Update::Status(_)).then_some(()));
    // `c1` exists already: creating it is refused.
    link.send(Command::List(ListAction::Save(ListRequest::Create {
        id: "c1".into(),
        name: "Again".into(),
        color: (1, 2, 3),
    })));
    let done = until(&link, |u| match u {
        Update::ListDone(c) => Some(c),
        _ => None,
    });
    assert!(done.result.is_err(), "{done:?}");
}

#[test]
fn forget_through_the_link_reports_the_backup() {
    let e = list_env();
    let link = link_to(e.socket.clone(), e.instance.clone());
    until(&link, |u| matches!(u, Update::Status(_)).then_some(()));
    link.send(Command::Forget {
        name: "c1".into(),
        collection: "c1".into(),
    });
    let message = until(&link, |u| match u {
        Update::Forgot(m) => Some(m),
        _ => None,
    });
    assert!(message.starts_with("preserved "), "{message}");
    assert!(message.contains("recover with :recover"), "{message}");
}

#[test]
fn a_daemon_that_never_answers_hello_shows_down_and_retries() {
    let env = TestEnv::new("");
    let listener = std::os::unix::net::UnixListener::bind(env.socket()).unwrap();
    let held = std::thread::spawn(move || {
        let mut keep = Vec::new();
        while let Ok((s, _)) = listener.accept() {
            keep.push(s);
        }
    });
    let link = link_for(&env);
    let begun = Instant::now();
    let Some(Update::Down(m)) = link.next_update(Duration::from_secs(10)) else {
        panic!("no Down update within 10 s");
    };
    assert!(m.contains("did not answer"), "{m}");
    assert!(m.ends_with("retrying"), "{m}");
    assert!(begun.elapsed() < Duration::from_secs(10));
    // It tries again (a second connect is another hello the fake never answers), and the
    // thread is not stuck: it takes Quit.
    link.send(Command::Quit);
    // Nothing answers there for the teardown to stop.
    let _ = fs::remove_file(env.socket());
    drop(held);
}
