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
        let mut c = Command::new(env!("CARGO_BIN_EXE_tdo"));
        c.env_remove("TWYDO_SOCKET")
            .env("TWYDO_SOCKET", self.socket())
            .env("XDG_CONFIG_HOME", self.root.path().join("config"))
            .env("XDG_STATE_HOME", self.xdg_state())
            .env("XDG_RUNTIME_DIR", self.sock_dir.path().join("run"))
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
        let Ok(raw) = Client::connect_raw(&self.socket()) else {
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
        while Client::connect_raw(&self.socket()).is_ok() {
            if Instant::now() > deadline {
                if let Some(pid) = pid {
                    // SAFETY: signals the daemon this environment started, by pid.
                    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
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
