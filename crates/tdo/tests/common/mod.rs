//! Shared helpers for the integration tests that run the built `tdo` binary against a daemon.
//! Every path lives inside tempdirs; the user's real config, state and socket are never
//! touched, and no spawned `tdo` inherits `TWYDO_SOCKET`, `XDG_*` or `HOME`.
#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use twydo_core::compat::Matrix;
use twydo_core::config::Config;
use twydo_core::store::bind_vdir_state;
use twydo_daemon::client::Client;
use twydo_daemon::paths;
use twydo_daemon::protocol::methods::{DaemonInfo, HelloParams, Instance, ShutdownParams};
use twydo_daemon::server::{Daemon, DaemonHandle, DaemonOptions};

pub const WAIT: Duration = Duration::from_secs(15);

/// A directory for sockets: Unix socket paths are limited to about 104 bytes, so it lives
/// directly under `/tmp`, and it is mode 0700 because the daemon refuses any other.
pub fn short_socket_dir() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("td")
        .tempdir_in("/tmp")
        .unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

/// The one place that builds a `tdo` command: it always names its own socket, config and state
/// directories, and never inherits `TWYDO_SOCKET`. Callers add their own variables (`HOME`,
/// `XDG_RUNTIME_DIR`, ...); a test that never reaches a daemon passes a socket path that
/// does not exist.
pub fn tdo_bin(socket: &Path, xdg_config: &Path, xdg_state: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_tdo"));
    c.env_remove("TWYDO_SOCKET")
        .env("TWYDO_SOCKET", socket)
        .env("XDG_CONFIG_HOME", xdg_config)
        .env("XDG_STATE_HOME", xdg_state);
    c
}

/// A spawned process, killed and reaped on drop.
pub struct Proc(pub Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A configuration, a vdir with one list, a state directory and a socket, all in tempdirs.
/// Dropping it stops whatever daemon still answers on its socket, detached ones included.
pub struct TestEnv {
    pub root: tempfile::TempDir,
    pub sock_dir: tempfile::TempDir,
}

impl TestEnv {
    /// `config_toml` is added to the end of the generated `config.toml` (which sets `vdir`,
    /// `state_dir` and test write mode); it may be empty.
    pub fn new(config_toml: &str) -> TestEnv {
        TestEnv::with_config_tail(|_| config_toml.to_string())
    }

    /// An environment whose `[sync]` runs a fake vdirsyncer script with `body` (`$1` is
    /// `sync` or `metasync`), with automatic sync switched off: only `sync.run` runs it.
    pub fn with_sync(body: &str) -> TestEnv {
        let env = TestEnv::with_config_tail(|root| {
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
    pub fn with_config_tail(tail: impl FnOnce(&Path) -> String) -> TestEnv {
        let root = tempfile::tempdir().unwrap();
        let sock_dir = short_socket_dir();
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
        TestEnv { root, sock_dir }
    }

    pub fn vdir(&self) -> PathBuf {
        self.root.path().join("vdir")
    }

    pub fn socket(&self) -> PathBuf {
        self.sock_dir.path().join("tdo.sock")
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.path().join("config/twydo/config.toml")
    }

    pub fn xdg_state(&self) -> PathBuf {
        self.root.path().join("xdg-state")
    }

    /// The per-vdir state directory.
    pub fn state(&self) -> PathBuf {
        bind_vdir_state(&self.root.path().join("state"), &self.vdir()).unwrap()
    }

    /// The identity a client of this environment presents (no `--vdir` flag).
    pub fn instance(&self) -> Instance {
        paths::instance_for(&self.config_path(), None).unwrap()
    }

    pub fn hello(&self) -> HelloParams {
        HelloParams {
            client: "test".into(),
            version: "0".into(),
            protocol: twydo_daemon::PROTOCOL,
            session: None,
            instance: self.instance(),
        }
    }

    /// A `tdo` command in this environment: its own config, state and socket, and no
    /// `--vdir` (the configuration names the vdir). Output is not redirected.
    pub fn tdo_cmd(&self) -> Command {
        let mut c = tdo_bin(
            &self.socket(),
            &self.root.path().join("config"),
            &self.xdg_state(),
        );
        c.env("XDG_RUNTIME_DIR", self.sock_dir.path().join("run"))
            .env("HOME", self.root.path());
        c
    }

    /// [`TestEnv::tdo_cmd`] with output discarded.
    pub fn quiet_cmd(&self) -> Command {
        let mut c = self.tdo_cmd();
        c.stdout(Stdio::null()).stderr(Stdio::null());
        c
    }

    /// Runs `tdo daemon <extra>` in the foreground as a child, killed on drop of the
    /// returned [`Proc`] by [`TestEnv::spawn`]; this returns the bare [`Child`].
    pub fn start_daemon(&self, extra: &[&str]) -> Child {
        let mut c = self.quiet_cmd();
        c.arg("daemon").args(extra);
        c.spawn().unwrap()
    }

    pub fn spawn(&self, extra: &[&str]) -> Proc {
        Proc(self.start_daemon(extra))
    }

    /// Stops the daemon answering on this environment's socket (however it was started) and
    /// waits until the socket is gone; kills it if it does not go.
    pub fn stop_daemon(&self) {
        stop_daemon_at(&self.socket());
    }

    /// `daemon.info` of the daemon on this socket.
    pub fn info(&self) -> DaemonInfo {
        let c = Client::connect_raw(&self.socket()).expect("a daemon answers");
        c.caller()
            .call_timeout("daemon.info", serde_json::json!({}), WAIT)
            .unwrap()
    }

    /// Connects once the daemon answers `hello`, within [`WAIT`].
    pub fn connect(&self, socket: &Path) -> Client {
        let deadline = Instant::now() + WAIT;
        loop {
            match Client::connect(socket, self.hello()) {
                Ok((c, _)) => {
                    // Hello is answered while startup (recovery, first snapshot) goes on.
                    while matches!(
                        c.call::<_, serde_json::Value>("status.get", serde_json::json!({})),
                        Err(twydo_daemon::client::ClientError::Rpc(ref e))
                            if e.message == "the daemon is starting"
                    ) {
                        assert!(Instant::now() < deadline, "daemon never became ready");
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    return c;
                }
                Err(e) if Instant::now() > deadline => panic!("daemon never answered: {e}"),
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    /// A daemon inside this test process for this environment's configuration and socket;
    /// `version` replaces the version it reports. The scheduler is off.
    pub fn in_process_daemon(&self, version: Option<&str>) -> DaemonHandle {
        let cfg = Config::load(&self.config_path()).unwrap();
        let mut opts = DaemonOptions::new(self.socket());
        opts.schedule = false;
        opts.version_for_test = version.map(String::from);
        let h = Daemon::start(cfg, self.instance(), opts).expect("start");
        assert!(h.wait_ready(Duration::from_secs(10)), "not ready");
        h
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        self.stop_daemon();
    }
}

/// Stops the daemon answering on `socket` and waits until the socket is gone; kills it if it
/// does not go. Nothing happens when none answers.
pub fn stop_daemon_at(socket: &Path) {
    let Ok(raw) = Client::connect_raw(socket) else {
        return;
    };
    let pid = raw
        .caller()
        .call_timeout::<_, DaemonInfo>("daemon.info", serde_json::json!({}), WAIT)
        .map(|i| i.pid)
        .ok();
    let _ = raw.caller().call_timeout::<_, serde_json::Value>(
        "daemon.shutdown",
        ShutdownParams { when_idle: false },
        WAIT,
    );
    drop(raw);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Client::connect_raw(socket).is_ok() {
        if Instant::now() > deadline {
            if let Some(pid) = pid {
                // SAFETY: signals the daemon this test started, by pid.
                unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            }
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// A daemon inside this test process for `cfg`, on a socket of its own (the returned
/// directory holds it). The instance it serves has a configuration path inside that
/// directory and `cfg.vdir` as its `--vdir`.
pub fn in_process_daemon(cfg: Config, matrix: Matrix) -> (DaemonHandle, tempfile::TempDir) {
    let dir = short_socket_dir();
    let mut opts = DaemonOptions::new(dir.path().join("s").join("tdo.sock"));
    opts.schedule = false;
    opts.matrix = matrix;
    let instance = Instance {
        config: dir.path().join("config.toml"),
        vdir: Some(cfg.vdir.clone()),
    };
    let h = Daemon::start(cfg, instance, opts).expect("start");
    assert!(h.wait_ready(Duration::from_secs(10)), "not ready");
    (h, dir)
}

/// Writes an executable `sh` script and waits until it can be executed: a script written
/// while another test thread forks can briefly fail with "Text file busy" (ETXTBSY). The
/// script answers `__probe` by exiting at once.
pub fn write_script(path: &Path, body: &str) {
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

pub fn wait_exit(p: &mut Proc) -> std::process::ExitStatus {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(s) = p.0.try_wait().unwrap() {
            return s;
        }
        assert!(Instant::now() < deadline, "process did not exit");
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// A daemon in this test process and a client of it, for tests of the write path. It plans
/// every write on the snapshot it last handed out, as the TUI plans on the view it shows.
pub struct WriteDaemon {
    pub handle: DaemonHandle,
    pub client: Client,
    pub epoch: String,
    _dir: tempfile::TempDir,
    revision: std::cell::Cell<u64>,
    view: std::cell::RefCell<twydo_core::views::Snapshot>,
}

impl WriteDaemon {
    pub fn start(cfg: Config, matrix: Matrix) -> WriteDaemon {
        let (handle, dir) = in_process_daemon(cfg.clone(), matrix);
        let hello = HelloParams {
            client: "test".into(),
            version: "0".into(),
            protocol: twydo_daemon::PROTOCOL,
            session: None,
            instance: Instance {
                config: dir.path().join("config.toml"),
                vdir: Some(cfg.vdir.clone()),
            },
        };
        let (client, result) = Client::connect(handle.socket(), hello).expect("connect");
        let d = WriteDaemon {
            handle,
            client,
            epoch: result.epoch,
            _dir: dir,
            revision: std::cell::Cell::new(0),
            view: std::cell::RefCell::new(twydo_core::views::Snapshot::new(
                twydo_core::vdir::load(&cfg.vdir).unwrap(),
                cfg,
            )),
        };
        d.fetch();
        d
    }

    /// Fetches the daemon's current snapshot and plans the next writes on it.
    pub fn fetch(&self) -> twydo_core::views::Snapshot {
        let wire: twydo_daemon::protocol::wire::WireSnapshot = self
            .client
            .call("snapshot.get", serde_json::json!({}))
            .expect("snapshot.get");
        self.revision.set(wire.revision);
        let snap = twydo_daemon::protocol::wire::from_wire(wire);
        *self.view.borrow_mut() = snap.clone();
        snap
    }

    /// Reads the files again (as the watcher will after an outside change), then
    /// [`WriteDaemon::fetch`].
    pub fn snapshot(&self) -> twydo_core::views::Snapshot {
        self.handle
            .reload_for_test(twydo_daemon::protocol::events::Cause::Unknown)
            .expect("reload");
        self.fetch()
    }

    /// The revision of the snapshot writes are planned on.
    pub fn revision(&self) -> u64 {
        self.revision.get()
    }

    /// A write planned on the snapshot last fetched.
    pub fn run(&self, req: &tdo::write::Request) -> Result<String, String> {
        self.run_at(self.revision.get(), req)
    }

    /// A write planned on `revision` of this daemon.
    pub fn run_at(&self, revision: u64, req: &tdo::write::Request) -> Result<String, String> {
        let caller = self.client.caller();
        let session = self.client.session().expect("session");
        let based_on = twydo_daemon::protocol::methods::BasedOn {
            epoch: self.epoch.clone(),
            revision,
        };
        let done = tdo::write::execute(
            &caller,
            &session,
            &based_on,
            &self.view.borrow(),
            req,
            &self.client.next_request_id(),
        );
        if done.is_ok() {
            // The daemon published the change before it answered; the TUI takes that snapshot.
            self.fetch();
        }
        done
    }

    /// Reads the files again, then writes on the new snapshot.
    pub fn run_fresh(&self, req: &tdo::write::Request) -> Result<String, String> {
        self.snapshot();
        self.run(req)
    }
}

/// A `ClientSync` as the status line and the conflict overlay read it, built directly from
/// what a sync summary holds (the daemon sends the same facts in `status.get` and
/// `conflicts.list`).
pub fn client_sync(s: &twydo_core::sync::SyncSummary) -> tdo::app::ClientSync {
    use twydo_daemon::protocol::admin::ConflictView;
    use twydo_daemon::protocol::methods::{ConfigStatus, SyncState, SyncView};
    tdo::app::ClientSync {
        sync: Some(SyncView {
            state: SyncState::Idle,
            auto: !s.off,
            last_success: s.status.last_success,
            last_attempt: s.status.last_attempt,
            next_attempt: s.status.next_attempt,
            failures: s.status.failures,
            last_error: s.status.last_error.clone(),
            job_id: None,
            collections: s
                .status
                .collections
                .iter()
                .map(|(k, v)| (k.clone(), v.into()))
                .collect(),
            metasync_error: s.status.metasync_error.clone(),
        }),
        unsynced: s.pending,
        queued: s.queued,
        conflicts: s.conflicts.iter().map(ConflictView::of).collect(),
        refusals: Vec::new(),
        config: ConfigStatus::default(),
        drain_error: None,
    }
}

/// A daemon with lists `c1` and `c2` that a synthetic server also holds (the core tests'
/// offline list backend), and the way to reach it.
pub struct ListEnv {
    pub root: tempfile::TempDir,
    pub _sockets: tempfile::TempDir,
    pub daemon: DaemonHandle,
    pub socket: PathBuf,
    pub instance: Instance,
}

pub const COLOR: &str = "#3289d2";

pub fn list_env() -> ListEnv {
    let root = tempfile::tempdir().unwrap();
    let sockets = short_socket_dir();
    let vdir = root.path().join("vdir");
    let mut collections = serde_json::Map::new();
    for (id, name) in [("c1", "Synthetic one"), ("c2", "Synthetic two")] {
        fs::create_dir_all(vdir.join(id)).unwrap();
        fs::write(
            vdir.join(id).join(format!("u-{id}.ics")),
            format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\nUID:u-{id}\r\nSUMMARY:One\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"),
        )
        .unwrap();
        fs::write(vdir.join(id).join("displayname"), name).unwrap();
        fs::write(vdir.join(id).join("color"), COLOR).unwrap();
        collections.insert(id.into(), serde_json::json!({"name": name, "color": COLOR}));
    }
    fs::write(
        root.path().join("server.json"),
        serde_json::json!({"calls": [], "flags": {}, "collections": collections}).to_string(),
    )
    .unwrap();
    let connection = root.path().join("connection.json");
    fs::write(&connection, serde_json::json!({"vdir": vdir}).to_string()).unwrap();
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
    row.iphone_baseline = twydo_core::compat::Verdict::Pass;
    row.twydo_to_iphone = twydo_core::compat::Verdict::Pass;
    row.iphone_to_twydo = twydo_core::compat::Verdict::Pass;
    matrix
        .edge_case
        .iter_mut()
        .find(|e| e.id == "list-metadata-atomicity")
        .unwrap()
        .verdict = twydo_core::compat::Verdict::Pass;
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
        root,
        _sockets: sockets,
        daemon,
        socket,
        instance,
    }
}

/// A link that connects to `socket` as `instance` (no daemon is started for it).
pub fn link_to(socket: PathBuf, instance: Instance) -> tdo::term::link::Link {
    let connect: tdo::term::link::Connector = Box::new(move |session| {
        let hello = twydo_daemon::protocol::methods::HelloParams {
            client: "link-test".into(),
            version: "0".into(),
            protocol: twydo_daemon::PROTOCOL,
            session,
            instance: instance.clone(),
        };
        twydo_daemon::client::Client::connect(&socket, hello)
            .map_err(|e| tdo::client::ConnectError::Unavailable(e.to_string()))
    });
    tdo::term::link::Link::start_with(connect)
}

/// Which answers to `op.submit` a [`lossy_proxy`] swallows.
#[derive(Clone, Copy, PartialEq)]
pub enum Lose {
    /// The first one, closing the connection (a connection lost after the daemon committed).
    First,
    /// Every one, leaving the connection open (a daemon that never answers).
    All,
}

/// A proxy to `target` that swallows answers to `op.submit` as `lose` says; it records the
/// request id of every `op.submit` that passes.
pub fn lossy_proxy(
    target: PathBuf,
    dir: &Path,
    lose: Lose,
) -> (PathBuf, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    lossy_proxy_for(target, dir, lose, "op.submit")
}

/// [`lossy_proxy`] for another changing `method`.
pub fn lossy_proxy_for(
    target: PathBuf,
    dir: &Path,
    lose: Lose,
    method: &'static str,
) -> (PathBuf, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use std::collections::HashSet;
    use std::io::{BufRead, BufReader, Write};
    use std::net::Shutdown;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    let path = dir.join("proxy.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let dropped = Arc::new(AtomicBool::new(false));
    let record = Arc::clone(&seen);
    std::thread::spawn(move || {
        for client in listener.incoming() {
            let Ok(client) = client else { return };
            let Ok(daemon) = UnixStream::connect(&target) else {
                continue;
            };
            let (mut to_daemon, from_client) =
                (daemon.try_clone().unwrap(), client.try_clone().unwrap());
            let record = Arc::clone(&record);
            // JSON-RPC ids of the `op.submit` requests on this connection.
            let submits = Arc::new(Mutex::new(HashSet::<u64>::new()));
            let submit_ids = Arc::clone(&submits);
            std::thread::spawn(move || {
                for line in BufReader::new(from_client).lines() {
                    let Ok(line) = line else { break };
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line)
                        && v["method"] == method
                    {
                        record
                            .lock()
                            .unwrap()
                            .push(v["params"]["request_id"].as_str().unwrap().to_string());
                        submit_ids.lock().unwrap().insert(v["id"].as_u64().unwrap());
                    }
                    if writeln!(to_daemon, "{line}").is_err() {
                        break;
                    }
                }
            });
            let dropped = Arc::clone(&dropped);
            std::thread::spawn(move || {
                let mut to_client = client.try_clone().unwrap();
                for line in BufReader::new(daemon.try_clone().unwrap()).lines() {
                    let Ok(line) = line else { break };
                    let is_submit = serde_json::from_str::<serde_json::Value>(&line)
                        .ok()
                        .and_then(|v| v["id"].as_u64())
                        .is_some_and(|id| submits.lock().unwrap().contains(&id));
                    if is_submit {
                        match lose {
                            Lose::First if !dropped.swap(true, Ordering::SeqCst) => break,
                            Lose::All => continue,
                            Lose::First => {}
                        }
                    }
                    if writeln!(to_client, "{line}").is_err() {
                        break;
                    }
                }
                let _ = client.shutdown(Shutdown::Both);
                let _ = daemon.shutdown(Shutdown::Both);
            });
        }
    });
    (path, seen)
}

/// A proxy to `target` that drops the first `op.submit` before forwarding it, closes the
/// connection and then refuses every connection until the returned gate is set (the daemon
/// can be restarted meanwhile). Records the request id of every `op.submit` it sees.
pub fn dropping_proxy(
    target: PathBuf,
    dir: &Path,
) -> (
    PathBuf,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use std::io::{BufRead, BufReader, Write};
    use std::net::Shutdown;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    let path = dir.join("drop.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let open = Arc::new(AtomicBool::new(true));
    let (record, gate) = (Arc::clone(&seen), Arc::clone(&open));
    std::thread::spawn(move || {
        for client in listener.incoming() {
            let Ok(client) = client else { return };
            if !gate.load(Ordering::SeqCst) {
                continue;
            }
            let Ok(daemon) = UnixStream::connect(&target) else {
                continue;
            };
            let (mut to_daemon, from_client) =
                (daemon.try_clone().unwrap(), client.try_clone().unwrap());
            let (record, gate) = (Arc::clone(&record), Arc::clone(&gate));
            std::thread::spawn(move || {
                for line in BufReader::new(from_client).lines() {
                    let Ok(line) = line else { break };
                    let v = serde_json::from_str::<serde_json::Value>(&line).ok();
                    if let Some(v) = v
                        && v["method"] == "op.submit"
                    {
                        let first = {
                            let mut r = record.lock().unwrap();
                            r.push(v["params"]["request_id"].as_str().unwrap().to_string());
                            r.len() == 1
                        };
                        if first {
                            gate.store(false, Ordering::SeqCst);
                            break;
                        }
                    }
                    if writeln!(to_daemon, "{line}").is_err() {
                        break;
                    }
                }
                let _ = to_daemon.shutdown(Shutdown::Both);
            });
            std::thread::spawn(move || {
                let mut to_client = client.try_clone().unwrap();
                for line in BufReader::new(daemon.try_clone().unwrap()).lines() {
                    let Ok(line) = line else { break };
                    if writeln!(to_client, "{line}").is_err() {
                        break;
                    }
                }
                let _ = client.shutdown(Shutdown::Both);
            });
        }
    });
    (path, seen, open)
}
