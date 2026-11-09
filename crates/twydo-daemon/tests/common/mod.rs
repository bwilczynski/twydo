//! Shared fixtures: a vdir of synthetic tasks in a tempdir.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use twydo_daemon::client::{Client, ClientError};
use twydo_daemon::protocol::methods::{HelloParams, Instance};
use twydo_daemon::protocol::rpc::{ErrorData, ErrorKind};
use twydo_daemon::server::{Daemon, DaemonHandle, DaemonOptions};

use twydo_core::config::{
    Config, Group, ListPrefs, Sort, SortField, SyncConfig, WriteMode, Writes,
};
use twydo_core::vdir;
use twydo_core::views::Snapshot;

pub struct Fixture {
    tmp: tempfile::TempDir,
}

/// A synthetic VTODO resource.
pub fn todo_ics(uid: &str, summary: &str, parent: Option<&str>) -> String {
    let related = parent
        .map(|p| format!("RELATED-TO;RELTYPE=PARENT:{p}\r\n"))
        .unwrap_or_default();
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\nUID:{uid}\r\n\
         SUMMARY:{summary}\r\n{related}END:VTODO\r\nEND:VCALENDAR\r\n"
    )
}

impl Fixture {
    /// Tasks as `(collection, uid, summary, parent uid)`.
    pub fn new(tasks: &[(&str, &str, &str, Option<&str>)]) -> Fixture {
        let fx = Fixture {
            tmp: tempfile::tempdir().unwrap(),
        };
        fs::create_dir_all(fx.vdir()).unwrap();
        for (collection, uid, summary, parent) in tasks {
            fx.write(
                &format!("{collection}/{uid}.ics"),
                &todo_ics(uid, summary, *parent),
            );
        }
        fx
    }

    pub fn vdir(&self) -> PathBuf {
        self.tmp.path().join("vdir")
    }

    /// The instance identity a client of this fixture's daemon presents.
    pub fn instance(&self) -> Instance {
        Instance {
            config: self.tmp.path().join("config.toml"),
            vdir: Some(self.vdir()),
        }
    }

    pub fn config(&self) -> Config {
        let mut c = Config::with_vdir(self.vdir());
        c.state_dir = Some(self.tmp.path().join("state"));
        c.writes.mode = WriteMode::Test;
        c
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot::new(vdir::load(&self.vdir()).unwrap(), self.config())
    }

    /// Write `text` at `rel` (relative to the vdir), as a change from outside.
    pub fn write(&self, rel: &str, text: &str) {
        let p = self.vdir().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    pub fn remove(&self, rel: &str) {
        fs::remove_file(self.vdir().join(rel)).unwrap();
    }
}

/// A configuration with every table set (synthetic values).
pub fn full_config() -> Config {
    let mut c = Config::with_vdir(PathBuf::from("/synthetic/vdir"));
    c.timezone = Some("Europe/Berlin".into());
    c.week_start = chrono::Weekday::Sun;
    c.work_days = vec![chrono::Weekday::Mon, chrono::Weekday::Wed];
    c.morning_time = chrono::NaiveTime::from_hms_opt(7, 30, 0).unwrap();
    c.default_list = Some("inbox".into());
    c.lists.insert(
        "inbox".into(),
        ListPrefs {
            archived: true,
            hidden: true,
            show_in_all: false,
            show_in_today: false,
            show_in_done: false,
            sort: Sort {
                field: SortField::Title,
                descending: true,
            },
        },
    );
    c.groups = vec![Group {
        name: "Synthetic group".into(),
        lists: vec!["inbox".into()],
    }];
    c.writes = Writes {
        mode: WriteMode::Test,
    };
    c.state_dir = Some(PathBuf::from("/synthetic/state"));
    c.sync = Some(SyncConfig {
        vdirsyncer_config: Some(PathBuf::from("/synthetic/vdirsyncer.conf")),
        after_edit: Some(true),
        ..SyncConfig::default()
    });
    c
}

/// A daemon on a socket of its own. Unix socket paths are limited to about 104 bytes, so the
/// socket lives in a short directory under `/tmp`, not in the (long) fixture tempdir. The
/// socket's parent directory (`s`) is left to the daemon to create.
pub struct TestDaemon {
    handle: Option<DaemonHandle>,
    socket: PathBuf,
    instance: Instance,
    vdir: PathBuf,
    _dir: tempfile::TempDir,
}

/// The daemon refuses a socket directory that is not private, so this one is made 0700.
pub fn short_socket_dir() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::Builder::new()
        .prefix("td")
        .tempdir_in("/tmp")
        .unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

impl TestDaemon {
    /// Starts and waits until the daemon is ready.
    pub fn start(fx: &Fixture) -> TestDaemon {
        Self::start_with(fx, |_| {})
    }

    pub fn start_with(fx: &Fixture, f: impl FnOnce(&mut DaemonOptions)) -> TestDaemon {
        let td = Self::start_unready(fx, f);
        assert!(td.handle().wait_ready(Duration::from_secs(10)), "not ready");
        td
    }

    /// Starts without waiting for startup to finish.
    pub fn start_unready(fx: &Fixture, f: impl FnOnce(&mut DaemonOptions)) -> TestDaemon {
        let dir = short_socket_dir();
        let socket = dir.path().join("s").join("tdo.sock");
        let mut opts = DaemonOptions::new(socket.clone());
        f(&mut opts);
        let instance = fx.instance();
        let handle = Daemon::start(fx.config(), instance.clone(), opts).expect("start");
        TestDaemon {
            handle: Some(handle),
            socket,
            instance,
            vdir: fx.vdir(),
            _dir: dir,
        }
    }

    /// Writes `text` at `rel` in the vdir (a change from outside) and reloads as the watcher
    /// will; returns the revision.
    pub fn touch_outside(&self, rel: &str, text: &str) -> u64 {
        let p = self.vdir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
        self.handle()
            .reload_for_test(twydo_daemon::protocol::events::Cause::Outside)
            .unwrap()
    }

    pub fn handle(&self) -> &DaemonHandle {
        self.handle.as_ref().unwrap()
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    pub fn hello_params(&self) -> HelloParams {
        HelloParams {
            client: "test".into(),
            version: "0".into(),
            protocol: twydo_daemon::PROTOCOL,
            session: None,
            instance: self.instance.clone(),
        }
    }

    /// A client that has said hello.
    pub fn client(&self) -> Client {
        Client::connect(&self.socket, self.hello_params())
            .expect("hello")
            .0
    }

    /// A client that has not said hello.
    pub fn raw(&self) -> Client {
        Client::connect_raw(&self.socket).expect("connect")
    }
}

pub fn kind(e: &ClientError) -> ErrorKind {
    err_data(e).kind
}

pub fn err_data(e: &ClientError) -> ErrorData {
    match e {
        ClientError::Rpc(r) => r.data.clone().expect("error data"),
        other => panic!("not an rpc error: {other:?}"),
    }
}

pub fn code(e: &ClientError) -> i64 {
    match e {
        ClientError::Rpc(r) => r.code,
        other => panic!("not an rpc error: {other:?}"),
    }
}

pub fn empty() -> Value {
    serde_json::json!({})
}
