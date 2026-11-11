//! The hidden `tdo daemon` command against the built binary: socket selection, the single
//! instance, signals, and what survives a crash. Synthetic content; the user's real config,
//! state and socket are never touched (every path is inside a tempdir).

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::*;

use twydo_core::edit::{NewTask, Op};
use twydo_core::store::fsio::VdirLock;
use twydo_core::sync::running::{Running, process_start};
use twydo_daemon::client::{Client, ClientError, Notification};
use twydo_daemon::protocol::events::Event;
use twydo_daemon::protocol::jobs::{JobGetParams, JobStarted, JobState, JobView};
use twydo_daemon::protocol::methods::{
    BasedOn, OpGetParams, OpState, SubmitParams, WriteOutcome, WriteResult,
};
use twydo_daemon::protocol::rpc::ErrorKind;
use twydo_daemon::protocol::sync::SyncRunParams;

type Env = TestEnv;

fn submit(c: &Client, request_id: &str, title: &str) -> Result<WriteResult, ClientError> {
    let snap: serde_json::Value = c.call("snapshot.get", serde_json::json!({})).unwrap();
    c.call(
        "op.submit",
        SubmitParams {
            session: c.session().unwrap(),
            request_id: request_id.into(),
            based_on: BasedOn {
                epoch: c.epoch().unwrap(),
                revision: snap["revision"].as_u64().unwrap(),
            },
            op: Op::Create(NewTask::plain("inbox", title)),
        },
    )
}

fn count_titled(env: &Env, title: &str) -> usize {
    let want = format!("SUMMARY:{title}");
    fs::read_dir(env.vdir().join("inbox"))
        .unwrap()
        .filter(|f| {
            fs::read_to_string(f.as_ref().unwrap().path())
                .unwrap()
                .lines()
                .any(|l| l.trim_end() == want)
        })
        .count()
}

#[test]
fn the_daemon_listens_on_twydo_socket() {
    let env = TestEnv::new("");
    let _d = env.spawn(&[]);
    env.connect(&env.socket());
    assert!(env.socket().exists());
}

#[test]
fn the_socket_flag_wins_over_twydo_socket() {
    let env = TestEnv::new("");
    let other = env.sock_dir.path().join("flag.sock");
    let mut c = env.quiet_cmd();
    c.arg("--socket").arg(&other).arg("daemon");
    let _d = Proc(c.spawn().unwrap());
    env.connect(&other);
    assert!(!env.socket().exists());
}

#[test]
fn a_second_daemon_exits_with_the_already_running_code() {
    let env = TestEnv::new("");
    let _first = env.spawn(&[]);
    env.connect(&env.socket());
    let mut second = env.spawn(&[]);
    assert_eq!(wait_exit(&mut second).code(), Some(10));
}

#[test]
fn sigterm_shuts_down_and_removes_the_socket() {
    let env = TestEnv::new("");
    let mut d = env.spawn(&[]);
    env.connect(&env.socket());
    // SAFETY: signals our own child by pid.
    unsafe { libc::kill(d.0.id() as i32, libc::SIGTERM) };
    assert_eq!(wait_exit(&mut d).code(), Some(0));
    assert!(!env.socket().exists());
}

#[test]
fn a_missing_config_exits_with_the_error_code() {
    let env = TestEnv::new("");
    let mut c = common::tdo_bin(
        &env.socket(),
        &env.root.path().join("none"),
        &env.xdg_state(),
    );
    c.env("HOME", env.root.path())
        .arg("daemon")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut p = Proc(c.spawn().unwrap());
    assert_eq!(wait_exit(&mut p).code(), Some(2));
}

#[test]
fn a_resend_after_a_crash_between_commit_and_reply_returns_the_first_outcome() {
    let env = TestEnv::new("");
    let mut d = env.spawn(&["--test-crash-after-commit"]);
    let c = env.connect(&env.socket());
    let err = submit(&c, "R", "Synthetic crash").unwrap_err();
    assert!(
        matches!(err, ClientError::Closed | ClientError::Io(_)),
        "{err:?}"
    );
    assert!(!wait_exit(&mut d).success(), "the daemon should abort");
    drop(c);
    // Restart without the flag: the old session is unknown to the new daemon.
    let _d2 = env.spawn(&[]);
    let c2 = env.connect(&env.socket());
    let r = submit(&c2, "R", "Synthetic crash").unwrap();
    assert_eq!(r.outcome, WriteOutcome::Applied);
    let state: OpState = c2
        .call(
            "op.get",
            OpGetParams {
                op_id: None,
                request_id: Some("R".into()),
            },
        )
        .unwrap();
    assert_eq!(
        state,
        OpState::Applied {
            op_id: r.op_id.clone()
        }
    );
    assert_eq!(count_titled(&env, "Synthetic crash"), 1);
}

#[test]
fn a_queued_item_survives_kill_and_is_applied_once_after_restart() {
    let env = TestEnv::new("");
    let mut d = env.spawn(&[]);
    let c = env.connect(&env.socket());
    // An external sync holds the vdir: the daemon's 250 ms try fails and the write queues.
    let lock = VdirLock::shared(&env.state()).unwrap();
    let r = submit(&c, "Q", "Synthetic queued").unwrap();
    assert_eq!(r.outcome, WriteOutcome::Queued);
    assert_eq!(count_titled(&env, "Synthetic queued"), 0);
    d.0.kill().unwrap();
    wait_exit(&mut d);
    drop(c);
    drop(lock);
    let _d2 = env.spawn(&[]);
    let c2 = env.connect(&env.socket());
    let deadline = Instant::now() + WAIT;
    loop {
        let s: OpState = c2
            .call(
                "op.get",
                OpGetParams {
                    op_id: None,
                    request_id: Some("Q".into()),
                },
            )
            .unwrap();
        if matches!(s, OpState::Applied { .. }) {
            break;
        }
        assert!(Instant::now() < deadline, "never applied: {s:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(count_titled(&env, "Synthetic queued"), 1);
}

#[test]
fn daemon_shutdown_ends_the_process_with_success() {
    let env = TestEnv::new("");
    let mut d = env.spawn(&[]);
    let c = env.connect(&env.socket());
    let _: serde_json::Value = c
        .call("daemon.shutdown", serde_json::json!({"when_idle": false}))
        .unwrap();
    assert_eq!(wait_exit(&mut d).code(), Some(0));
    assert!(!env.socket().exists());
}

#[test]
fn a_relative_socket_in_a_shared_directory_is_refused() {
    let env = TestEnv::new("");
    let shared = tempfile::Builder::new()
        .prefix("td")
        .tempdir_in("/tmp")
        .unwrap();
    fs::set_permissions(shared.path(), fs::Permissions::from_mode(0o777)).unwrap();
    let mut c = env.quiet_cmd();
    c.current_dir(shared.path())
        .arg("daemon")
        .args(["--socket", "rel.sock"]);
    let mut p = Proc(c.spawn().unwrap());
    assert!(!wait_exit(&mut p).success());
    assert!(!shared.path().join("rel.sock").exists());
    // The same relative path in a private directory is fine.
    fs::set_permissions(shared.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let mut c = env.quiet_cmd();
    c.current_dir(shared.path())
        .arg("daemon")
        .args(["--socket", "rel.sock"]);
    let _p = Proc(c.spawn().unwrap());
    env.connect(&shared.path().join("rel.sock"));
}

#[test]
fn second_sigint_exits_at_once() {
    let env = TestEnv::new("");
    let mut d = env.spawn(&[]);
    let c = env.connect(&env.socket());
    // An outside holder of the vdir lock stalls the daemon's teardown: the queued write's
    // status refresh waits for the lock, and teardown waits (about a second) for it.
    let lock = VdirLock::exclusive(&env.state()).unwrap();
    let r = submit(&c, "S", "Synthetic slow stop").unwrap();
    assert_eq!(r.outcome, WriteOutcome::Queued);
    let pid = d.0.id() as i32;
    // SAFETY: signals our own child by pid.
    unsafe { libc::kill(pid, libc::SIGINT) };
    std::thread::sleep(Duration::from_millis(10));
    // SAFETY: as above.
    unsafe { libc::kill(pid, libc::SIGINT) };
    let started = Instant::now();
    let status = wait_exit(&mut d);
    drop(lock);
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "{:?}",
        started.elapsed()
    );
    // 130 (the second signal); 0 only if teardown beat the second signal, which the held
    // lock rules out.
    assert_eq!(status.code(), Some(130), "{status:?}");
}

fn job_by_request(c: &Client, request_id: &str) -> Result<JobView, ClientError> {
    c.call(
        "job.get",
        JobGetParams {
            job_id: None,
            request_id: Some(request_id.into()),
        },
    )
}

fn sync_run(c: &Client, request_id: &str) -> JobStarted {
    c.call(
        "sync.run",
        SyncRunParams {
            request_id: request_id.into(),
            force_delete: Vec::new(),
            discover: false,
        },
    )
    .unwrap()
}

/// The recorded running sync, once it has a vdirsyncer child.
fn wait_for_child(env: &Env) -> Running {
    let path = env.state().join("sync/running.json");
    let deadline = Instant::now() + WAIT;
    loop {
        if let Ok(b) = fs::read(&path)
            && let Ok(r) = serde_json::from_slice::<Running>(&b)
            && r.child_pgid.is_some()
        {
            return r;
        }
        assert!(Instant::now() < deadline, "no vdirsyncer child recorded");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn job_cut_short_by_kill_is_interrupted() {
    let env = TestEnv::with_sync("sleep 30");
    let mut d = env.spawn(&[]);
    let c = env.connect(&env.socket());
    let _: serde_json::Value = c.call("events.subscribe", serde_json::json!({})).unwrap();
    let id = sync_run(&c, "K").job_id;
    let deadline = Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match c.notifications().recv_timeout(left) {
            Ok(Notification::Event(ev)) if matches!(&ev.event, Event::SyncStarted { job_id: Some(j), .. } if *j == id) =>
            {
                break;
            }
            Ok(_) => {}
            Err(e) => panic!("no sync.started: {e}"),
        }
    }
    let rec = wait_for_child(&env);
    let child = rec.child_pgid.unwrap();
    d.0.kill().unwrap();
    wait_exit(&mut d);
    drop(c);
    assert!(
        process_start(child).is_some(),
        "the vdirsyncer outlives its daemon"
    );

    let t = Instant::now();
    let _d2 = env.spawn(&[]);
    let c2 = env.connect(&env.socket());
    let view = loop {
        match job_by_request(&c2, "K") {
            Ok(v) => break v,
            Err(ClientError::Rpc(r))
                if r.data
                    .as_ref()
                    .is_some_and(|d| d.kind == ErrorKind::Unavailable) =>
            {
                assert!(
                    t.elapsed() < Duration::from_secs(5),
                    "not ready: {}",
                    r.message
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("{e:?}"),
        }
    };
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    assert_eq!(view.job_id, id);
    assert_eq!(view.state, JobState::Interrupted);
    // The orphaned vdirsyncer was reaped.
    let deadline = Instant::now() + Duration::from_secs(3);
    while process_start(child) == rec.child_start {
        assert!(
            Instant::now() < deadline,
            "the orphaned vdirsyncer survived"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // A resend answers with the same job and starts nothing.
    let _: serde_json::Value = c2.call("events.subscribe", serde_json::json!({})).unwrap();
    assert_eq!(sync_run(&c2, "K").job_id, id);
    let quiet = Instant::now() + Duration::from_millis(500);
    loop {
        let left = quiet.saturating_duration_since(Instant::now());
        match c2.notifications().recv_timeout(left) {
            Ok(Notification::Event(ev)) => assert!(
                !matches!(&ev.event, Event::SyncStarted { job_id: Some(j), .. } if *j == id),
                "the interrupted job ran again"
            ),
            Ok(_) => {}
            Err(_) => break,
        }
    }
}

// ---- connecting from a client: on-demand start and the upgrade rule ----

fn log_of(env: &TestEnv) -> std::path::PathBuf {
    env.xdg_state().join("twydo").join(format!(
        "daemon-{}.log",
        twydo_daemon::paths::socket_id(&env.socket())
    ))
}

/// Runs `tdo daemon ping` (a client that starts the daemon on demand); returns the exit
/// code, stdout and stderr.
fn ping(cmd: &mut Command) -> (Option<i32>, String, String) {
    let out = cmd.args(["daemon", "ping"]).output().unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn a_client_starts_the_daemon_on_demand() {
    let env = TestEnv::new("");
    assert!(!env.socket().exists());
    let (code, epoch, err) = ping(&mut env.tdo_cmd());
    assert_eq!(code, Some(0), "{err}");
    let info = env.info();
    assert_eq!(info.epoch, epoch);
    assert_eq!(info.vdir, fs::canonicalize(env.vdir()).unwrap());
    let log = fs::read_to_string(log_of(&env)).unwrap();
    assert!(log.contains("tdo daemon: serving"), "{log}");
    // A second client finds it running and starts nothing.
    let (code, again, _) = ping(&mut env.tdo_cmd());
    assert_eq!((code, again), (Some(0), epoch));
}

#[test]
fn two_clients_starting_at_once_leave_one_daemon() {
    let env = TestEnv::new("");
    let run = |env: &TestEnv| {
        let mut c = env.tdo_cmd();
        std::thread::spawn(move || ping(&mut c))
    };
    let (a, b) = (run(&env), run(&env));
    let (a, b) = (a.join().unwrap(), b.join().unwrap());
    assert_eq!((a.0, b.0), (Some(0), Some(0)), "{} {}", a.2, b.2);
    assert_eq!(a.1, b.1, "both clients reached the same daemon");
    let pids: std::collections::HashSet<u32> = (0..3).map(|_| env.info().pid).collect();
    assert_eq!(pids.len(), 1, "{pids:?}");
    // A second daemon, if one was started, lost the lock and said so in the log instead of
    // serving.
    let log = fs::read_to_string(log_of(&env)).unwrap();
    let starts = log.matches("tdo daemon: serving").count();
    assert_eq!(starts, 1, "{log}");
}

#[test]
fn wrong_instance_never_restarts_the_daemon() {
    let a = TestEnv::new("");
    let _d = a.spawn(&[]);
    a.connect(&a.socket());
    let epoch = a.info().epoch;
    let b = TestEnv::new("");
    let (code, _, err) = ping(b.tdo_cmd().env("TWYDO_SOCKET", a.socket()));
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("not the instance"), "{err}");
    assert_eq!(a.info().epoch, epoch);
}

#[test]
fn no_config_fails_without_starting_anything() {
    let env = TestEnv::new("");
    fs::remove_file(env.config_path()).unwrap();
    let (code, _, err) = ping(&mut env.tdo_cmd());
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("no config at"), "{err}");
    assert!(!env.socket().exists());
    assert!(!log_of(&env).exists());
}

#[test]
fn a_newer_client_restarts_an_older_daemon_when_idle() {
    let env = TestEnv::new("");
    let old = env.in_process_daemon(Some("0.0.1"));
    let before = old.epoch().to_string();
    let (code, epoch, err) = ping(&mut env.tdo_cmd());
    assert_eq!(code, Some(0), "{err}");
    assert_ne!(epoch, before, "the old daemon was replaced");
    let info = env.info();
    assert_eq!(info.epoch, epoch);
    assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
}

#[test]
fn an_older_client_never_restarts_a_newer_daemon() {
    let env = TestEnv::new("");
    // The same protocol: the newer daemon serves the older client as it is.
    let new = env.in_process_daemon(Some("99.0.0"));
    let before = new.epoch().to_string();
    let (code, epoch, err) = ping(&mut env.tdo_cmd());
    assert_eq!((code, epoch), (Some(0), before.clone()), "{err}");
    new.shutdown();
    drop(new);

    // A daemon of another protocol and a higher version is refused, not stopped.
    let fake = FakeProtocolDaemon::start(&env, serde_json::json!("99.0.0"));
    let (code, _, err) = ping(&mut env.tdo_cmd());
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("tdo was upgraded; restart it"), "{err}");
    assert_eq!(fake.finish(), vec!["hello".to_string()]);
}

/// A socket that answers every request with a `protocol` refusal naming `daemon` (JSON null:
/// no version given), and records the methods it was asked.
struct FakeProtocolDaemon {
    seen: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: std::thread::JoinHandle<()>,
    socket: std::path::PathBuf,
}

impl FakeProtocolDaemon {
    fn start(env: &TestEnv, daemon: serde_json::Value) -> FakeProtocolDaemon {
        let mut data = serde_json::json!({"kind": "protocol", "protocol": 2});
        if !daemon.is_null() {
            data["daemon"] = daemon;
        }
        Self::start_refusing(env, data, "protocol 1 is not supported")
    }

    /// A socket that refuses every request with `data` as the error data.
    fn start_refusing(
        env: &TestEnv,
        data: serde_json::Value,
        message: &'static str,
    ) -> FakeProtocolDaemon {
        let listener = std::os::unix::net::UnixListener::bind(env.socket()).unwrap();
        listener.set_nonblocking(true).unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread = {
            let (seen, stop) = (seen.clone(), stop.clone());
            let data = data.clone();
            std::thread::spawn(move || {
                use std::io::{BufRead, BufReader, Write};
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let Ok((s, _)) = listener.accept() else {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    };
                    s.set_nonblocking(false).unwrap();
                    let mut line = String::new();
                    if BufReader::new(&s).read_line(&mut line).unwrap_or(0) == 0 {
                        continue;
                    }
                    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
                    seen.lock()
                        .unwrap()
                        .push(v["method"].as_str().unwrap().to_string());
                    let reply = serde_json::json!({"jsonrpc": "2.0", "id": v["id"], "error": {
                        "code": -32000, "message": message, "data": data}});
                    let _ = writeln!(&s, "{reply}");
                }
            })
        };
        FakeProtocolDaemon {
            seen,
            stop,
            thread,
            socket: env.socket(),
        }
    }

    /// Stops the server and returns the methods it saw.
    #[allow(clippy::let_and_return)]
    fn finish(self) -> Vec<String> {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.thread.join().unwrap();
        let _ = fs::remove_file(&self.socket);
        let seen = self.seen.lock().unwrap().clone();
        seen
    }
}

#[test]
fn an_unknown_daemon_version_is_refused_never_stopped() {
    let env = TestEnv::new("");
    let fake = FakeProtocolDaemon::start(&env, serde_json::Value::Null);
    let (code, _, err) = ping(&mut env.tdo_cmd());
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("tdo was upgraded; restart it"), "{err}");
    assert_eq!(fake.finish(), vec!["hello".to_string()]);
}

#[test]
fn a_second_upgrader_does_not_restart_the_upgraded_daemon() {
    let env = TestEnv::new("");
    // The daemon that answers now is already at this client's version (another client
    // upgraded it); this client still believes it saw an older one.
    let current = env.in_process_daemon(None);
    let epoch = current.epoch().to_string();
    let target = tdo::client::Target {
        socket: env.socket(),
        vdir: None,
    };
    let hello = env.hello();
    let mark = tdo::client::start::LogMark::take(&env.socket());
    let (_c, r) = tdo::client::restart(&target, &hello, &mark, Some("d-stale")).unwrap();
    assert_eq!(r.epoch, epoch);
    // The same holds for an equal version seen under the same epoch, and with no epoch.
    let (_c, r) = tdo::client::restart(&target, &hello, &mark, Some(&epoch)).unwrap();
    assert_eq!(r.epoch, epoch);
    let (_c, r) = tdo::client::restart(&target, &hello, &mark, None).unwrap();
    assert_eq!(r.epoch, epoch);
    assert_eq!(env.info().epoch, epoch);
}

#[test]
fn a_live_socket_of_another_vdir_is_not_taken_over() {
    let a = TestEnv::new("");
    let _d = a.spawn(&[]);
    a.connect(&a.socket());
    let epoch = a.info().epoch;
    let b = TestEnv::new("");
    let mut c = b.quiet_cmd();
    c.stderr(Stdio::piped())
        .args(["--socket"])
        .arg(a.socket())
        .arg("daemon");
    let out = c.output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("is served by a daemon for"), "{err}");
    assert!(err.contains("use another --socket"), "{err}");
    assert_eq!(a.info().epoch, epoch);
}

#[test]
fn daemon_stop_waits_for_the_socket_to_go() {
    let env = TestEnv::new("");
    let out = env.tdo_cmd().args(["daemon", "stop"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "no daemon is running"
    );
    assert!(!env.socket().exists(), "stop must not start a daemon");

    let (code, _, err) = ping(&mut env.tdo_cmd());
    assert_eq!(code, Some(0), "{err}");
    let out = env.tdo_cmd().args(["daemon", "stop"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The socket is gone the moment stop returns: nothing answers and the file is removed.
    assert!(!env.socket().exists());
    assert!(twydo_daemon::client::Client::connect_raw(&env.socket()).is_err());
}

#[test]
fn daemon_stop_refuses_another_instance() {
    let a = TestEnv::new("");
    let _d = a.spawn(&[]);
    a.connect(&a.socket());
    let epoch = a.info().epoch;
    let b = TestEnv::new("");
    let out = b
        .tdo_cmd()
        .env("TWYDO_SOCKET", a.socket())
        .args(["daemon", "stop"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("not the instance"), "{err}");
    assert_eq!(a.info().epoch, epoch, "the daemon was left alone");
}

#[test]
fn a_client_of_another_instance_with_another_protocol_never_stops_it() {
    let env = TestEnv::new("");
    // What a new daemon answers a hello that is wrong twice: wrong_instance, which says
    // nothing of its version.
    let fake = FakeProtocolDaemon::start_refusing(
        &env,
        serde_json::json!({"kind": "wrong_instance"}),
        "this daemon serves another vdir, not the instance you asked for",
    );
    let (code, _, err) = ping(&mut env.tdo_cmd());
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("not the instance"), "{err}");
    let out = env.tdo_cmd().args(["daemon", "stop"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        fake.finish(),
        vec!["hello".to_string(), "hello".to_string()],
        "no shutdown or info was sent"
    );
}

#[test]
fn a_daemon_that_never_answers_hello_does_not_hang_the_cli() {
    let env = TestEnv::new("");
    let listener = std::os::unix::net::UnixListener::bind(env.socket()).unwrap();
    let held = std::thread::spawn(move || {
        let mut keep = Vec::new();
        while let Ok((s, _)) = listener.accept() {
            keep.push(s);
        }
    });
    let begun = std::time::Instant::now();
    let (code, _, err) = ping(&mut env.tdo_cmd());
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("did not answer"), "{err}");
    assert!(
        begun.elapsed() < Duration::from_secs(15),
        "{:?}",
        begun.elapsed()
    );
    // Nothing answers there for the teardown to stop.
    let _ = fs::remove_file(env.socket());
    drop(held);
}

type Seen = std::sync::Arc<std::sync::Mutex<Vec<(usize, String)>>>;

/// A socket of an older daemon of another protocol: it refuses `hello` with `protocol`,
/// answers `daemon.info` (reporting version 0.0.1) and `daemon.shutdown`, and records every
/// method with the number of the connection it arrived on. After `daemon.shutdown` it goes
/// away (removes its socket) the way a stopping daemon does.
fn serve_old_protocol_daemon(env: &TestEnv) -> (Seen, std::thread::JoinHandle<()>) {
    use std::io::{BufRead, BufReader, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    let listener = std::os::unix::net::UnixListener::bind(env.socket()).unwrap();
    let seen = Arc::new(Mutex::new(Vec::<(usize, String)>::new()));
    let gone = Arc::new(AtomicBool::new(false));
    let socket = env.socket();
    let recorded = Arc::clone(&seen);
    let thread = std::thread::spawn(move || {
        let mut conns = 0;
        listener.set_nonblocking(true).unwrap();
        while !gone.load(Ordering::SeqCst) {
            let Ok((s, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            };
            s.set_nonblocking(false).unwrap();
            let n = conns;
            conns += 1;
            let (seen, gone, socket) = (Arc::clone(&recorded), Arc::clone(&gone), socket.clone());
            std::thread::spawn(move || {
                let mut w = &s;
                for line in BufReader::new(&s).lines() {
                    let Ok(line) = line else { break };
                    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
                    let method = v["method"].as_str().unwrap().to_string();
                    seen.lock().unwrap().push((n, method.clone()));
                    let reply = match method.as_str() {
                        "daemon.info" => serde_json::json!({"jsonrpc": "2.0", "id": v["id"],
                            "result": {"version": "0.0.1", "protocol": 1, "pid": 1,
                                "uptime_secs": 1, "vdir": "/synthetic", "epoch": "d-old"}}),
                        "daemon.shutdown" => {
                            let _ = std::fs::remove_file(&socket);
                            gone.store(true, Ordering::SeqCst);
                            serde_json::json!({"jsonrpc": "2.0", "id": v["id"], "result": {}})
                        }
                        _ => serde_json::json!({"jsonrpc": "2.0", "id": v["id"], "error": {
                            "code": -32000, "message": "protocol 1 is not supported",
                            "data": {"kind": "protocol", "protocol": 2, "daemon": "0.0.1"}}}),
                    };
                    let _ = writeln!(w, "{reply}");
                }
            });
        }
    });
    (seen, thread)
}

#[test]
fn an_upgrade_sends_its_lifecycle_calls_on_the_checked_connection() {
    let env = TestEnv::new("");
    let (seen, thread) = serve_old_protocol_daemon(&env);
    let (code, _, err) = ping(&mut env.tdo_cmd());
    assert_eq!(code, Some(0), "{err}");
    thread.join().unwrap();
    let seen = seen.lock().unwrap().clone();
    let first: Vec<_> = seen.iter().take(3).cloned().collect();
    assert_eq!(
        first,
        vec![
            (0, "hello".to_string()),
            (0, "daemon.info".to_string()),
            (0, "daemon.shutdown".to_string())
        ],
        "{seen:?}"
    );
    assert_eq!(seen.len(), 3, "no other connection was made: {seen:?}");
}

#[test]
fn stop_sends_shutdown_on_the_connection_whose_hello_was_refused() {
    let env = TestEnv::new("");
    let (seen, thread) = serve_old_protocol_daemon(&env);
    let out = env.tdo_cmd().args(["daemon", "stop"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    thread.join().unwrap();
    let seen = seen.lock().unwrap().clone();
    assert_eq!(
        seen,
        vec![(0, "hello".to_string()), (0, "daemon.shutdown".to_string())]
    );
}

#[test]
fn upgrade_accepts_a_daemon_restarted_by_the_manager() {
    let env = TestEnv::new("");
    let old = env.in_process_daemon(Some("0.0.1"));
    let before = old.epoch().to_string();
    // The service manager: starts a new daemon on the same socket as soon as the old one
    // exits, before the client sees the socket go.
    let socket = env.socket();
    let mut cmd = env.tdo_cmd();
    cmd.args(["daemon", "--detach"]);
    let watcher = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while twydo_daemon::client::Client::connect_raw(&socket).is_ok() {
            assert!(
                std::time::Instant::now() < deadline,
                "old daemon never left"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(cmd.status().unwrap().success());
    });
    let started = std::time::Instant::now();
    let (code, epoch, err) = ping(&mut env.tdo_cmd());
    watcher.join().unwrap();
    assert_eq!(code, Some(0), "{err}");
    assert!(!err.contains("still busy"), "{err}");
    assert_ne!(epoch, before);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}
