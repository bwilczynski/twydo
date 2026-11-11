mod common;

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use twydo_daemon::client::Client;
use twydo_daemon::paths;
use twydo_daemon::protocol::methods::{DaemonInfo, HelloParams, HelloResult, Instance};
use twydo_daemon::protocol::rpc::{ErrorKind, MAX_LINE};
use twydo_daemon::server::conn::peer_allowed;
use twydo_daemon::server::{Daemon, DaemonOptions, StartError};

fn fx() -> Fixture {
    Fixture::new(&[("inbox", "a", "Alpha", None)])
}

#[test]
fn hello_returns_epoch_session_and_vdir() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let (_c, r) = Client::connect(td.socket(), td.hello_params()).unwrap();
    assert_eq!(r.protocol, 1);
    assert!(r.session.starts_with("s-"), "{}", r.session);
    assert_eq!(r.session.len(), 2 + 12);
    assert!(r.epoch.starts_with("d-"));
    assert_eq!(r.epoch, td.handle().epoch());
    assert_eq!(r.vdir, std::fs::canonicalize(fx.vdir()).unwrap());
}

#[test]
fn protocol_mismatch_is_refused_with_both_versions() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.raw();
    let p = HelloParams {
        protocol: 2,
        ..td.hello_params()
    };
    let err = c.call::<_, HelloResult>("hello", p).unwrap_err();
    assert_eq!(kind(&err), ErrorKind::Protocol);
    assert_eq!(err_data(&err).protocol, Some(1));
    assert!(err_data(&err).daemon.is_some());
}

#[test]
fn a_client_for_another_instance_gets_wrong_instance() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let other = tempfile::tempdir().unwrap();
    let p = HelloParams {
        instance: Instance {
            config: other.path().join("config.toml"),
            vdir: Some(other.path().to_path_buf()),
        },
        ..td.hello_params()
    };
    let err = td.raw().call::<_, HelloResult>("hello", p).unwrap_err();
    assert_eq!(kind(&err), ErrorKind::WrongInstance);
}

#[test]
fn hello_for_another_instance_and_protocol_is_wrong_instance() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let other = tempfile::tempdir().unwrap();
    let p = HelloParams {
        protocol: 2,
        instance: Instance {
            config: other.path().join("config.toml"),
            vdir: Some(other.path().to_path_buf()),
        },
        ..td.hello_params()
    };
    let err = td.raw().call::<_, HelloResult>("hello", p).unwrap_err();
    assert_eq!(kind(&err), ErrorKind::WrongInstance);
}

#[test]
fn a_refused_instance_leaves_the_daemon_running() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let p = HelloParams {
        instance: Instance {
            config: PathBuf::from("/nowhere/config.toml"),
            vdir: None,
        },
        ..td.hello_params()
    };
    assert!(td.raw().call::<_, HelloResult>("hello", p).is_err());
    let info: DaemonInfo = td.client().call("daemon.info", empty()).unwrap();
    assert_eq!(info.protocol, 1);
}

#[test]
fn socket_flag_wins_over_env_which_wins_over_the_default() {
    let none = |_: &str| -> Option<OsString> { None };
    let flag = Path::new("/flag/x.sock");
    let env = |k: &str| -> Option<OsString> {
        match k {
            "TWYDO_SOCKET" => Some("/env/x.sock".into()),
            "XDG_RUNTIME_DIR" => Some("/run/user/1000".into()),
            _ => None,
        }
    };
    assert_eq!(paths::socket_path_from(Some(flag), &env, 1000), flag);
    assert_eq!(
        paths::socket_path_from(None, &env, 1000),
        PathBuf::from("/env/x.sock")
    );
    if cfg!(target_os = "linux") {
        let xdg = |k: &str| -> Option<OsString> {
            (k == "XDG_RUNTIME_DIR").then(|| "/run/user/1000".into())
        };
        assert_eq!(
            paths::socket_path_from(None, &xdg, 1000),
            PathBuf::from("/run/user/1000/twydo/tdo.sock")
        );
        assert_eq!(
            paths::socket_path_from(None, &none, 1000),
            PathBuf::from("/tmp/twydo-1000/tdo.sock")
        );
    }
}

#[test]
fn instance_reads_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.toml");
    std::fs::write(&cfg, "x").unwrap();
    std::fs::set_permissions(&cfg, std::fs::Permissions::from_mode(0o000)).unwrap();
    let i = paths::instance_for(&cfg, Some(Path::new("relative/vdir"))).unwrap();
    assert_eq!(i.config, cfg);
    assert!(i.vdir.unwrap().is_absolute());
    // The real entry point needs no file either.
    assert!(paths::instance(None).unwrap().config.is_absolute());
}

#[test]
fn lifecycle_methods_answer_without_hello() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.raw();
    let info: DaemonInfo = c.call("daemon.info", json!({})).unwrap();
    assert_eq!(info.protocol, 1);
    let doc: Value = c.call("rpc.discover", json!({})).unwrap();
    assert!(doc.get("openrpc").is_some());
    assert_eq!(
        kind(&c.call::<_, Value>("snapshot.get", json!({})).unwrap_err()),
        ErrorKind::Invalid
    );
    assert_eq!(
        code(&c.call::<_, Value>("no.such.method", json!({})).unwrap_err()),
        -32601
    );
}

/// A state directory that belongs to another vdir keeps a daemon from starting at all.
#[test]
fn daemon_refuses_a_state_dir_of_another_vdir() {
    let fx = fx();
    let cfg = fx.config();
    let state = twydo_core::store::vdir_state_dir(&cfg.state_dir(), &cfg.vdir).unwrap();
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(
        state.join(twydo_core::store::VDIR_ID_FILE),
        "/some/other/vdir\n",
    )
    .unwrap();
    let dir = common::short_socket_dir();
    let opts = DaemonOptions::new(dir.path().join("s").join("tdo.sock"));
    let err = match Daemon::start(cfg, fx.instance(), opts) {
        Ok(_) => panic!("the daemon started"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("belongs to vdir /some/other/vdir"), "{err}");
}

#[test]
fn a_second_daemon_on_the_same_vdir_does_not_start() {
    let fx = fx();
    let _td = TestDaemon::start(&fx);
    let dir = short_socket_dir();
    let opts = DaemonOptions::new(dir.path().join("other.sock"));
    assert!(matches!(
        Daemon::start(fx.config(), fx.instance(), opts),
        Err(StartError::AlreadyRunning)
    ));
}

#[test]
fn a_stale_socket_file_is_replaced() {
    let fx = fx();
    let dir = short_socket_dir();
    let sock = dir.path().join("tdo.sock");
    drop(UnixListener::bind(&sock).unwrap());
    assert!(sock.exists());
    let h = Daemon::start(fx.config(), fx.instance(), DaemonOptions::new(sock.clone())).unwrap();
    assert!(h.wait_ready(Duration::from_secs(10)));
    assert!(Client::connect_raw(&sock).is_ok());
}

#[test]
fn requests_wait_for_recovery_as_unavailable() {
    let fx = fx();
    let (tx, rx) = mpsc::channel::<()>();
    let td = TestDaemon::start_unready(&fx, |o| {
        o.before_recovery = Some(Box::new(move || {
            let _ = rx.recv();
        }));
    });
    let c = td.client();
    let err = c.call::<_, Value>("snapshot.get", empty()).unwrap_err();
    assert_eq!(kind(&err), ErrorKind::Unavailable);
    assert!(err.to_string().contains("starting"), "{err}");
    tx.send(()).unwrap();
    assert!(td.handle().wait_ready(Duration::from_secs(10)));
    let snap: Value = c.call("snapshot.get", empty()).unwrap();
    assert!(snap["revision"].as_u64().unwrap() >= 1);
}

#[test]
fn pipelined_requests_answer_by_id() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let mut s = UnixStream::connect(td.socket()).unwrap();
    let mut batch = String::new();
    for id in 1..=3 {
        batch.push_str(&format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"daemon.info\",\"params\":{{}}}}\n"
        ));
    }
    s.write_all(batch.as_bytes()).unwrap();
    let mut r = BufReader::new(s);
    let mut ids = Vec::new();
    for _ in 0..3 {
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert!(v["result"].is_object(), "{line}");
        ids.push(v["id"].as_u64().unwrap());
    }
    ids.sort();
    assert_eq!(ids, vec![1, 2, 3]);
}

#[test]
fn malformed_input_does_not_hurt_other_clients() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let mut a = UnixStream::connect(td.socket()).unwrap();
    let mut ar = BufReader::new(a.try_clone().unwrap());
    a.write_all(b"not json\n").unwrap();
    let mut line = String::new();
    ar.read_line(&mut line).unwrap();
    let v: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["error"]["code"], -32700);
    assert!(v["id"].is_null());
    // Still open: valid JSON that is not a request.
    a.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":7}\n").unwrap();
    line.clear();
    ar.read_line(&mut line).unwrap();
    let v: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["error"]["code"], -32600);
    // Invalid UTF-8 is a parse error too.
    a.write_all(b"\xff\xfe\n").unwrap();
    line.clear();
    ar.read_line(&mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["error"]["code"],
        -32700
    );
    // An overlong line closes the connection.
    let mut big = a.try_clone().unwrap();
    std::thread::spawn(move || {
        let chunk = vec![b'x'; 1 << 20];
        for _ in 0..(MAX_LINE >> 20) + 2 {
            if big.write_all(&chunk).is_err() {
                break;
            }
        }
    });
    line.clear();
    assert_eq!(ar.read_line(&mut line).unwrap_or(0), 0, "closed");
    let b = td.client();
    assert!(b.call::<_, DaemonInfo>("daemon.info", empty()).is_ok());
}

#[test]
fn shutdown_removes_the_socket_and_releases_the_lock() {
    let fx = fx();
    let dir = short_socket_dir();
    let sock = dir.path().join("tdo.sock");
    let h = Daemon::start(fx.config(), fx.instance(), DaemonOptions::new(sock.clone())).unwrap();
    assert!(h.wait_ready(Duration::from_secs(10)));
    h.shutdown();
    assert!(!sock.exists());
    let h2 = Daemon::start(fx.config(), fx.instance(), DaemonOptions::new(sock.clone())).unwrap();
    assert!(h2.wait_ready(Duration::from_secs(10)));
}

#[test]
fn daemon_shutdown_replies_then_stops() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let r: Value = c
        .call("daemon.shutdown", json!({"when_idle": false}))
        .unwrap();
    assert_eq!(r, json!({}));
    td.handle().wait_stopped();
    assert!(!td.socket().exists());
    assert!(c.call::<_, Value>("daemon.info", empty()).is_err());
}

#[test]
fn peer_check_refuses_other_users() {
    assert!(peer_allowed(1000, 1000));
    assert!(!peer_allowed(0, 1000));
}

#[test]
fn socket_dir_is_private() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(td.socket().parent().unwrap()), 0o700);
    assert_eq!(mode(td.socket()), 0o600);
}

#[test]
fn one_client_serves_several_threads() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    std::thread::scope(|s| {
        for _ in 0..8 {
            let caller = c.caller();
            s.spawn(move || {
                for _ in 0..20 {
                    let i: DaemonInfo = caller.call("daemon.info", empty()).unwrap();
                    assert_eq!(i.protocol, 1);
                }
            });
        }
    });
    assert!(c.next_request_id().starts_with("s-"));
    assert_ne!(c.next_request_id(), c.next_request_id());
}

#[test]
fn pending_calls_fail_when_the_daemon_goes_away() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    td.handle().shutdown();
    let e = c.call::<_, Value>("daemon.info", empty()).unwrap_err();
    assert!(
        matches!(
            e,
            twydo_daemon::client::ClientError::Closed | twydo_daemon::client::ClientError::Io(_)
        ),
        "{e:?}"
    );
}

#[test]
fn shutdown_returns_even_if_the_socket_file_is_gone() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    std::fs::remove_file(td.socket()).unwrap();
    let (tx, rx) = mpsc::channel();
    let td = std::sync::Arc::new(td);
    let t = std::sync::Arc::clone(&td);
    std::thread::spawn(move || {
        t.handle().shutdown();
        let _ = tx.send(());
    });
    rx.recv_timeout(Duration::from_secs(5))
        .expect("shutdown hung");
}

#[test]
fn shutdown_during_recovery_keeps_the_lock_and_never_becomes_ready() {
    let fx = fx();
    let (release, hold) = mpsc::channel::<()>();
    let (entered_tx, entered) = mpsc::channel::<()>();
    let td = std::sync::Arc::new(TestDaemon::start_unready(&fx, |o| {
        o.before_recovery = Some(Box::new(move || {
            let _ = entered_tx.send(());
            let _ = hold.recv();
        }));
    }));
    // Stop only once recovery has really begun.
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    let t = std::sync::Arc::clone(&td);
    let stopper = std::thread::spawn(move || t.handle().shutdown());
    std::thread::sleep(Duration::from_millis(300));
    let dir = short_socket_dir();
    let opts = DaemonOptions::new(dir.path().join("o.sock"));
    let second = Daemon::start(fx.config(), fx.instance(), opts);
    assert!(
        matches!(second, Err(StartError::AlreadyRunning)),
        "{:?}",
        second.err()
    );
    assert!(!stopper.is_finished());
    release.send(()).unwrap();
    stopper.join().unwrap();
    assert!(!td.handle().wait_ready(Duration::from_millis(100)));
    let opts = DaemonOptions::new(dir.path().join("o.sock"));
    assert!(Daemon::start(fx.config(), fx.instance(), opts).is_ok());
}

#[test]
fn an_existing_socket_dir_must_be_private_and_ours() {
    let fx = fx();
    let dir = short_socket_dir();
    let sock = dir.path().join("tdo.sock");
    let set = |m| std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(m)).unwrap();
    set(0o755);
    match Daemon::start(fx.config(), fx.instance(), DaemonOptions::new(sock.clone())) {
        Err(StartError::Io(e)) => {
            assert!(e.to_string().contains(&dir.path().display().to_string()))
        }
        other => panic!("expected Io error, got ok={}", other.is_ok()),
    }
    set(0o700);
    let h = Daemon::start(fx.config(), fx.instance(), DaemonOptions::new(sock)).unwrap();
    assert!(h.wait_ready(Duration::from_secs(10)));
}

#[test]
fn a_regular_file_at_the_socket_path_is_never_deleted() {
    let fx = fx();
    let dir = short_socket_dir();
    let path = dir.path().join("tdo.sock");
    std::fs::write(&path, "not a socket").unwrap();
    match Daemon::start(fx.config(), fx.instance(), DaemonOptions::new(path.clone())) {
        Err(StartError::Io(e)) => {
            assert!(e.to_string().contains(&path.display().to_string()), "{e}")
        }
        Err(e) => panic!("wrong error: {e}"),
        Ok(_) => panic!("started over a regular file"),
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "not a socket");
}

// ---- Task 6: an orphaned vdirsyncer at startup, and stopping during a job ----

mod during_a_job {
    use std::os::unix::process::CommandExt;
    use std::time::{Duration, Instant};

    use super::common::*;
    use serde_json::{Value, json};
    use twydo_core::store::fsio::VdirLock;
    use twydo_core::store::queue::Queue;
    use twydo_core::sync::lock::LockFile;
    use twydo_core::sync::running::{Running, process_start};
    use twydo_daemon::client::Client;
    use twydo_daemon::protocol::events::Event;
    use twydo_daemon::protocol::jobs::{JobStarted, JobState, JobView};
    use twydo_daemon::protocol::methods::WriteOutcome;
    use twydo_daemon::protocol::rpc::ErrorKind;
    use twydo_daemon::protocol::sync::{SyncOutcome, SyncResult, SyncRunParams};

    const BOUND: Duration = Duration::from_secs(10);

    fn fx() -> Fixture {
        Fixture::new(&[("c1", "u1", "One", None)])
    }

    /// A daemon whose fake vdirsyncer runs `body`, stopping syncs after `grace`, and a
    /// subscribed client.
    fn start(fx: &Fixture, body: &str, grace: Duration) -> (TestDaemon, Client) {
        let td = TestDaemon::start_cfg(fx, fx.with_sync(body), |o| o.shutdown_grace = grace);
        let c = td.client();
        let _: Value = c.call("events.subscribe", empty()).unwrap();
        (td, c)
    }

    fn sync_run(c: &Client) -> Result<JobStarted, twydo_daemon::client::ClientError> {
        c.call(
            "sync.run",
            SyncRunParams {
                request_id: c.next_request_id(),
                force_delete: Vec::new(),
            },
        )
    }

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

    fn started(c: &Client, id: &str) {
        wait_event(
            c,
            BOUND,
            |e| matches!(e, Event::SyncStarted { job_id: Some(j), .. } if j == id),
        );
    }

    /// A finished process: its pid names no live runner.
    fn dead_pid() -> u32 {
        let mut gone = std::process::Command::new("true").spawn().unwrap();
        gone.wait().unwrap();
        gone.id()
    }

    /// Waits up to `limit` for the daemon to have stopped.
    fn stopped_within(td: &TestDaemon, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while !td.handle().is_stopped() {
            if Instant::now() > deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        true
    }

    #[test]
    fn startup_reaps_an_orphaned_vdirsyncer() {
        let fx = fx();
        let mut orphan = std::process::Command::new("sh")
            .args(["-c", "sleep 30"])
            .process_group(0)
            .spawn()
            .unwrap();
        let sync_dir = fx.state().join("sync");
        std::fs::create_dir_all(&sync_dir).unwrap();
        let rec = Running {
            runner_pid: dead_pid(),
            runner_start: None,
            child_pgid: Some(orphan.id()),
            child_start: process_start(orphan.id()),
            started: chrono::Utc::now(),
        };
        std::fs::write(
            sync_dir.join("running.json"),
            serde_json::to_vec(&rec).unwrap(),
        )
        .unwrap();
        let t = Instant::now();
        let td = TestDaemon::start_unready_cfg(&fx, fx.with_sync("sleep 30"), |_| {});
        assert!(td.handle().wait_ready(Duration::from_secs(5)), "not ready");
        assert!(t.elapsed() < Duration::from_secs(5));
        let deadline = Instant::now() + Duration::from_secs(3);
        while orphan.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "the orphan survived startup");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!sync_dir.join("running.json").exists());
        assert!(!sync_dir.join("stop").exists());
        VdirLock::exclusive(&fx.state()).expect("the vdir lock is free");
    }

    #[test]
    fn sigterm_during_a_sync_loses_no_edit() {
        let fx = fx();
        let (td, c) = start(&fx, "sleep 30", Duration::from_millis(200));
        let id = sync_run(&c).unwrap().job_id;
        started(&c, &id);
        let r = submit(&c, edit_title("u1", "Renamed")).unwrap();
        assert_eq!(r.outcome, WriteOutcome::Queued, "{r:?}");
        let t = Instant::now();
        td.handle().shutdown();
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
        assert_eq!(sync_finished(&c, &id).outcome, SyncOutcome::Stopped);
        let queue = Queue::existing(&fx.state()).expect("a queue");
        assert!(!queue.is_empty().unwrap(), "the queued edit is on disk");
        assert!(fx.read("c1/u1.ics").contains("SUMMARY:One"));
        drop(td);
        // The next daemon applies it, once.
        let td = TestDaemon::start_cfg(&fx, fx.with_sync("sleep 30"), |_| {});
        let deadline = Instant::now() + BOUND;
        while !Queue::existing(&fx.state()).unwrap().is_empty().unwrap() {
            assert!(Instant::now() < deadline, "never drained");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(count_titled(&td, "Renamed"), 1);
        assert_eq!(count_titled(&td, "One"), 0);
    }

    #[test]
    fn queued_jobs_are_interrupted_at_shutdown() {
        let fx = fx();
        let (td, c) = start(&fx, "sleep 30", Duration::from_millis(200));
        let a = sync_run(&c).unwrap().job_id;
        started(&c, &a);
        let b = sync_run(&c).unwrap().job_id;
        td.handle().shutdown();
        match wait_event(
            &c,
            BOUND,
            |e| matches!(e, Event::JobFinished { job_id, .. } if *job_id == b),
        ) {
            Event::JobFinished { state, .. } => assert_eq!(state, JobState::Interrupted),
            _ => unreachable!(),
        }
        let record = fx.state().join("daemon/jobs").join(format!("{b}.json"));
        let view: JobView = serde_json::from_slice(&std::fs::read(record).unwrap()).unwrap();
        assert_eq!(view.state, JobState::Interrupted);
        assert!(view.ended.is_some());
    }

    #[test]
    fn shutdown_when_idle_waits_for_jobs() {
        let fx = fx();
        let (td, c) = start(
            &fx,
            "sleep 1\necho 'Syncing twydo/c1' >&2\nexit 0",
            Duration::from_secs(10),
        );
        let id = sync_run(&c).unwrap().job_id;
        started(&c, &id);
        let t = Instant::now();
        let r: Value = c
            .call("daemon.shutdown", json!({"when_idle": true}))
            .unwrap();
        assert_eq!(r, json!({}));
        assert!(
            t.elapsed() < Duration::from_millis(500),
            "{:?}",
            t.elapsed()
        );
        let e = sync_run(&c).unwrap_err();
        assert_eq!(kind(&e), ErrorKind::Unavailable);
        assert!(e.to_string().contains("the daemon is shutting down"), "{e}");
        assert!(!td.handle().is_stopped());
        assert_eq!(sync_finished(&c, &id).outcome, SyncOutcome::Ok);
        assert!(stopped_within(&td, BOUND), "the daemon never stopped");
        assert!(!td.socket().exists());
    }

    #[test]
    fn shutdown_when_idle_races_job_creation() {
        for round in 0..50 {
            let fx = fx();
            let (td, c) = start(&fx, "exit 0", Duration::from_secs(5));
            let c2 = td.client();
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
            let b = std::sync::Arc::clone(&barrier);
            let shutter = std::thread::spawn(move || {
                b.wait();
                let r: Result<Value, _> = c2.call("daemon.shutdown", json!({"when_idle": true}));
                r.unwrap();
            });
            barrier.wait();
            let started = sync_run(&c);
            shutter.join().unwrap();
            // `Closed`: teardown shut the connection's read side before the request was read,
            // so it never reached dispatch (nothing was recorded either). A broken pipe is the
            // same race lost earlier: the connection closed while the request was being sent.
            if let Err(e) = &started
                && !matches!(e, twydo_daemon::client::ClientError::Closed)
                && !matches!(e, twydo_daemon::client::ClientError::Io(io)
                    if io.kind() == std::io::ErrorKind::BrokenPipe)
            {
                assert_eq!(kind(e), ErrorKind::Unavailable, "round {round}: {e}");
            }
            assert!(stopped_within(&td, BOUND), "round {round}: never stopped");
            // Whatever was recorded ended: none waits `queued` (or `running`) for a runner
            // that is gone.
            let dir = fx.state().join("daemon/jobs");
            let mut seen = 0;
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let view: JobView =
                    serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap();
                assert!(view.state.is_final(), "round {round}: {:?}", view.state);
                seen += 1;
            }
            assert_eq!(seen, usize::from(started.is_ok()), "round {round}");
        }
    }

    /// Is the daemon lock of `fx`'s vdir still held (a second daemon cannot start)?
    fn vdir_owned(fx: &Fixture) -> bool {
        use twydo_daemon::server::{Daemon, DaemonOptions, StartError};
        let dir = short_socket_dir();
        let opts = DaemonOptions::new(dir.path().join("other.sock"));
        match Daemon::start(fx.config(), fx.instance(), opts) {
            Err(StartError::AlreadyRunning) => true,
            Ok(h) => {
                h.shutdown();
                false
            }
            Err(e) => panic!("{e}"),
        }
    }

    /// Codex review (was task 3's give-up): a manual sync waiting for a run lock another
    /// process holds cannot be stopped. Shutdown keeps the vdir until it ends; once the lock
    /// frees, the sync ends writing nothing, and shutdown completes.
    #[test]
    fn shutdown_waits_for_a_sync_waiting_for_the_run_lock() {
        let fx = fx();
        let ran = fx.state().parent().unwrap().join("ran");
        let td = TestDaemon::start_cfg(
            &fx,
            fx.with_sync(&format!("echo run >> '{}'", ran.display())),
            |o| {
                o.shutdown_grace = Duration::from_millis(100);
                o.stop_wait = Duration::from_millis(100);
            },
        );
        let c = td.client();
        let _: Value = c.call("events.subscribe", empty()).unwrap();
        let sync_dir = fx.state().join("sync");
        std::fs::create_dir_all(&sync_dir).unwrap();
        let lock = LockFile::take(&sync_dir.join("run.lock"), true)
            .unwrap()
            .expect("run lock free");
        let id = sync_run(&c).unwrap().job_id;
        started(&c, &id);
        let status = std::fs::read(sync_dir.join("status.json")).ok();
        let ended = std::fs::read(sync_dir.join("ended")).ok();
        let task = fx.read("c1/u1.ics");
        std::thread::scope(|s| {
            s.spawn(|| td.handle().shutdown());
            // Well past the grace and the stop wait: still waiting, still the vdir's owner.
            std::thread::sleep(Duration::from_millis(700));
            assert!(!td.handle().is_stopped(), "shutdown gave up on the sync");
            assert!(
                vdir_owned(&fx),
                "the vdir was let go while the sync could still write"
            );
            drop(lock);
            assert!(stopped_within(&td, BOUND), "the daemon never stopped");
        });
        assert!(!td.socket().exists());
        assert!(!ran.exists(), "vdirsyncer ran during shutdown");
        assert_eq!(std::fs::read(sync_dir.join("status.json")).ok(), status);
        assert_eq!(std::fs::read(sync_dir.join("ended")).ok(), ended);
        assert_eq!(fx.read("c1/u1.ics"), task);
    }

    /// Codex review (was task 6's give-up): a list job cannot be stopped, and its helper
    /// writes the server and the vdir. Shutdown keeps the vdir until the job ends, however
    /// long past the grace and the stop wait, then completes.
    #[test]
    fn shutdown_waits_for_a_long_list_job() {
        let fx = fx();
        let helper_dir = fx.state().parent().unwrap().join("helper");
        std::fs::create_dir_all(&helper_dir).unwrap();
        let done = helper_dir.join("done");
        let mut cfg = fx.with_sync("exit 0");
        cfg.sync.as_mut().unwrap().list_python = fake_vdirsyncer(
            &helper_dir,
            &format!("sleep 1.5\ntouch '{}'", done.display()),
        )
        .to_str()
        .unwrap()
        .to_string();
        let td = TestDaemon::start_cfg(&fx, cfg, |o| {
            o.shutdown_grace = Duration::from_millis(100);
            o.stop_wait = Duration::from_millis(100);
        });
        let c = td.client();
        let id = c
            .call::<_, JobStarted>(
                "lists.preview_delete",
                json!({"request_id": c.next_request_id(), "id": "c1"}),
            )
            .unwrap()
            .job_id;
        let deadline = Instant::now() + BOUND;
        loop {
            let view: JobView = c.call("job.get", json!({"job_id": id})).unwrap();
            if view.state == JobState::Running {
                break;
            }
            assert!(!view.state.is_final(), "the job ended: {view:?}");
            assert!(Instant::now() < deadline, "the job never ran");
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::scope(|s| {
            s.spawn(|| td.handle().shutdown());
            std::thread::sleep(Duration::from_millis(500));
            assert!(
                !td.handle().is_stopped(),
                "shutdown gave up on the list job"
            );
            assert!(
                vdir_owned(&fx),
                "the vdir was let go while the job could still write"
            );
            assert!(stopped_within(&td, BOUND), "the daemon never stopped");
            assert!(done.exists(), "shutdown returned before the list job ended");
        });
        assert!(!td.socket().exists());
        assert!(!vdir_owned(&fx));
    }

    /// Final review: the sync `lists.forget` runs (under a list job's lane) is stopped at
    /// shutdown like any sync, instead of being waited for.
    #[test]
    fn shutdown_stops_the_sync_of_a_forget() {
        use twydo_daemon::protocol::jobs::JobResult;
        use twydo_daemon::protocol::lists::{ForgetPreview, ListPreviewForgetParams};
        let fx = fx();
        let (td, c) = start(&fx, "sleep 30", Duration::from_millis(200));
        let p: ForgetPreview = c
            .call(
                "lists.preview_forget",
                ListPreviewForgetParams { name: "c1".into() },
            )
            .unwrap();
        let id = c
            .call::<_, JobStarted>(
                "lists.forget",
                json!({"request_id": c.next_request_id(), "name": "c1", "collection": p.collection}),
            )
            .unwrap()
            .job_id;
        started(&c, &id);
        let t = Instant::now();
        td.handle().shutdown();
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
        assert_eq!(sync_finished(&c, &id).outcome, SyncOutcome::Stopped);
        match wait_event(
            &c,
            BOUND,
            |e| matches!(e, Event::JobFinished { job_id, .. } if *job_id == id),
        ) {
            Event::JobFinished {
                state:
                    JobState::Finished {
                        result: JobResult::Forgot { sync: Some(r), .. },
                    },
                ..
            } => assert_eq!(r.outcome, SyncOutcome::Stopped),
            other => panic!("expected forgot with a stopped sync: {other:?}"),
        }
    }
}

#[test]
fn call_timeout_reports_timed_out() {
    use twydo_daemon::client::ClientError;
    let fx = fx();
    let td = TestDaemon::start_with(&fx, |o| {
        o.test_point = Some(std::sync::Arc::new(|p: &str| {
            if p == "request:status.get" {
                std::thread::sleep(Duration::from_secs(2));
            }
        }));
    });
    let (c, _) = Client::connect(td.socket(), td.hello_params()).unwrap();
    let err = c
        .caller()
        .call_timeout::<_, Value>("status.get", json!({}), Duration::from_millis(200))
        .unwrap_err();
    assert!(matches!(err, ClientError::TimedOut(_)), "{err:?}");
    assert_eq!(err.to_string(), "the daemon did not answer within 0.2 s");
    // The connection is still good, and a later call within its limit is answered.
    let info: Value = c
        .caller()
        .call_timeout("daemon.info", json!({}), Duration::from_secs(5))
        .unwrap();
    assert!(info["pid"].is_u64());
}

#[test]
fn a_live_daemon_on_the_socket_blocks_another_vdir() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let other = Fixture::new(&[("inbox", "b", "Beta", None)]);
    let mut opts = DaemonOptions::new(td.socket().to_path_buf());
    opts.schedule = false;
    let r = Daemon::start(other.config(), other.instance(), opts);
    let Err(StartError::Io(e)) = r else {
        panic!("expected a refusal naming the other vdir");
    };
    let text = e.to_string();
    assert!(text.contains("is served by a daemon for"), "{text}");
    assert!(text.contains("use another --socket"), "{text}");
    // Still answering.
    assert!(Client::connect(td.socket(), td.hello_params()).is_ok());
}

#[test]
fn a_request_held_across_shutdown_is_unavailable() {
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    let fx = fx();
    let (reached_tx, reached_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let reached_tx = Mutex::new(reached_tx);
    let release_rx = Mutex::new(release_rx);
    let td = TestDaemon::start_with(&fx, |o| {
        o.test_point = Some(Arc::new(move |p: &str| {
            if p == "dispatch:after-stopping-check" {
                let _ = reached_tx.lock().unwrap().send(());
                let _ = release_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10));
            }
        }));
    });
    let (c, _) = Client::connect(td.socket(), td.hello_params()).unwrap();
    let caller = c.caller();
    std::thread::scope(|s| {
        let call = s.spawn(|| caller.call::<_, Value>("status.get", json!({})));
        reached_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the request never reached dispatch");
        // The request has passed the stopping check; now shut down, which takes the session.
        let stopper = s.spawn(|| td.handle().shutdown());
        std::thread::sleep(Duration::from_millis(500));
        let _ = release_tx.send(());
        let r = call.join().unwrap();
        stopper.join().unwrap();
        match r {
            Err(e) => assert_eq!(kind(&e), ErrorKind::Unavailable, "{e}"),
            Ok(v) => panic!("expected unavailable, got {v}"),
        }
    });
}
