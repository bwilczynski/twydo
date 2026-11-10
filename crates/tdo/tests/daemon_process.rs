//! The hidden `tdo daemon` command against the built binary: socket selection, the single
//! instance, signals, and what survives a crash. Synthetic content; the user's real config,
//! state and socket are never touched (every path is inside a tempdir).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use twydo_core::edit::{NewTask, Op};
use twydo_core::store::bind_vdir_state;
use twydo_core::store::fsio::VdirLock;
use twydo_core::sync::running::{Running, process_start};
use twydo_daemon::client::{Client, ClientError, Notification};
use twydo_daemon::paths;
use twydo_daemon::protocol::events::Event;
use twydo_daemon::protocol::jobs::{JobGetParams, JobStarted, JobState, JobView};
use twydo_daemon::protocol::methods::{
    BasedOn, HelloParams, OpGetParams, OpState, SubmitParams, WriteOutcome, WriteResult,
};
use twydo_daemon::protocol::rpc::ErrorKind;
use twydo_daemon::protocol::sync::SyncRunParams;

const WAIT: Duration = Duration::from_secs(15);

struct Env {
    root: tempfile::TempDir,
    /// A short directory (Unix socket paths are limited to ~104 bytes), mode 0700.
    sock_dir: tempfile::TempDir,
}

/// A spawned daemon, killed and reaped on drop.
struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Env {
    fn new() -> Env {
        Env::with_config_tail(|_| String::new())
    }

    /// An environment whose `[sync]` runs a fake vdirsyncer script with `body` (`$1` is
    /// `sync` or `metasync`), with automatic sync switched off: only `sync.run` runs it.
    fn with_sync(body: &str) -> Env {
        let env = Env::with_config_tail(|root| {
            let fake = root.join("fake-vdirsyncer");
            write_script(&fake, body);
            format!(
                "[sync]\ncommand = \"{}\"\nmetasync = false\n",
                fake.display()
            )
        });
        fs::create_dir_all(env.state().join("sync")).unwrap();
        fs::write(env.state().join("sync/off"), "").unwrap();
        env
    }

    /// `tail` gives the end of `config.toml`, for the environment's root directory.
    fn with_config_tail(tail: impl FnOnce(&Path) -> String) -> Env {
        let root = tempfile::tempdir().unwrap();
        let sock_dir = tempfile::Builder::new()
            .prefix("td")
            .tempdir_in("/tmp")
            .unwrap();
        fs::set_permissions(sock_dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir_all(root.path().join("vdir/inbox")).unwrap();
        fs::create_dir_all(root.path().join("config/twydo")).unwrap();
        let tail = tail(root.path());
        fs::write(
            root.path().join("config/twydo/config.toml"),
            format!(
                "vdir = \"{}\"\nstate_dir = \"{}\"\n[writes]\nmode = \"test\"\n{tail}",
                root.path().join("vdir").display(),
                root.path().join("state").display()
            ),
        )
        .unwrap();
        Env { root, sock_dir }
    }

    fn vdir(&self) -> PathBuf {
        self.root.path().join("vdir")
    }

    fn socket(&self) -> PathBuf {
        self.sock_dir.path().join("tdo.sock")
    }

    fn state(&self) -> PathBuf {
        bind_vdir_state(&self.root.path().join("state"), &self.vdir()).unwrap()
    }

    fn command(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_tdo"));
        c.env("TWYDO_SOCKET", self.socket())
            .env("XDG_CONFIG_HOME", self.root.path().join("config"))
            .env("XDG_STATE_HOME", self.root.path().join("xdg-state"))
            .env("XDG_RUNTIME_DIR", self.sock_dir.path().join("run"))
            .env("HOME", self.root.path())
            .arg("--vdir")
            .arg(self.vdir())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        c
    }

    fn spawn(&self, extra: &[&str]) -> Proc {
        let mut c = self.command();
        c.arg("daemon").args(extra);
        Proc(c.spawn().unwrap())
    }

    fn hello(&self) -> HelloParams {
        let cfg = self.root.path().join("config/twydo/config.toml");
        HelloParams {
            client: "test".into(),
            version: "0".into(),
            protocol: twydo_daemon::PROTOCOL,
            session: None,
            instance: paths::instance_for(&cfg, Some(&self.vdir())).unwrap(),
        }
    }

    /// Connects once the daemon answers `hello`, within [`WAIT`].
    fn connect(&self, socket: &Path) -> Client {
        let deadline = Instant::now() + WAIT;
        loop {
            match Client::connect(socket, self.hello()) {
                Ok((c, _)) => return c,
                Err(e) if Instant::now() > deadline => panic!("daemon never answered: {e}"),
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }
}

/// Writes an executable `sh` script and waits until it can be executed: a script written
/// while another test thread forks can briefly fail with "Text file busy" (ETXTBSY). The
/// script answers `__probe` by exiting at once.
fn write_script(path: &Path, body: &str) {
    fs::write(
        path,
        format!("#!/bin/sh\n[ \"$1\" = __probe ] && exit 0\n{body}\n"),
    )
    .unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match Command::new(path).arg("__probe").status() {
            Ok(st) => {
                assert!(st.success(), "probe of {} failed", path.display());
                return;
            }
            Err(e) if e.raw_os_error() == Some(26) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("cannot run {}: {e}", path.display()),
        }
    }
}

fn wait_exit(p: &mut Proc) -> std::process::ExitStatus {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(s) = p.0.try_wait().unwrap() {
            return s;
        }
        assert!(Instant::now() < deadline, "process did not exit");
        std::thread::sleep(Duration::from_millis(25));
    }
}

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
    let env = Env::new();
    let _d = env.spawn(&[]);
    env.connect(&env.socket());
    assert!(env.socket().exists());
}

#[test]
fn the_socket_flag_wins_over_twydo_socket() {
    let env = Env::new();
    let other = env.sock_dir.path().join("flag.sock");
    let mut c = env.command();
    c.arg("--socket").arg(&other).arg("daemon");
    let _d = Proc(c.spawn().unwrap());
    env.connect(&other);
    assert!(!env.socket().exists());
}

#[test]
fn a_second_daemon_exits_with_the_already_running_code() {
    let env = Env::new();
    let _first = env.spawn(&[]);
    env.connect(&env.socket());
    let mut second = env.spawn(&[]);
    assert_eq!(wait_exit(&mut second).code(), Some(10));
}

#[test]
fn sigterm_shuts_down_and_removes_the_socket() {
    let env = Env::new();
    let mut d = env.spawn(&[]);
    env.connect(&env.socket());
    // SAFETY: signals our own child by pid.
    unsafe { libc::kill(d.0.id() as i32, libc::SIGTERM) };
    assert_eq!(wait_exit(&mut d).code(), Some(0));
    assert!(!env.socket().exists());
}

#[test]
fn a_missing_config_exits_with_the_error_code() {
    let env = Env::new();
    let mut c = Command::new(env!("CARGO_BIN_EXE_tdo"));
    c.env("TWYDO_SOCKET", env.socket())
        .env("XDG_CONFIG_HOME", env.root.path().join("none"))
        .env("HOME", env.root.path())
        .arg("daemon")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut p = Proc(c.spawn().unwrap());
    assert_eq!(wait_exit(&mut p).code(), Some(2));
}

#[test]
fn a_resend_after_a_crash_between_commit_and_reply_returns_the_first_outcome() {
    let env = Env::new();
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
    let env = Env::new();
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
    let env = Env::new();
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
    let env = Env::new();
    let shared = tempfile::Builder::new()
        .prefix("td")
        .tempdir_in("/tmp")
        .unwrap();
    fs::set_permissions(shared.path(), fs::Permissions::from_mode(0o777)).unwrap();
    let mut c = env.command();
    c.current_dir(shared.path())
        .arg("daemon")
        .args(["--socket", "rel.sock"]);
    let mut p = Proc(c.spawn().unwrap());
    assert!(!wait_exit(&mut p).success());
    assert!(!shared.path().join("rel.sock").exists());
    // The same relative path in a private directory is fine.
    fs::set_permissions(shared.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let mut c = env.command();
    c.current_dir(shared.path())
        .arg("daemon")
        .args(["--socket", "rel.sock"]);
    let _p = Proc(c.spawn().unwrap());
    env.connect(&shared.path().join("rel.sock"));
}

#[test]
fn second_sigint_exits_at_once() {
    let env = Env::new();
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
    let env = Env::with_sync("sleep 30");
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
