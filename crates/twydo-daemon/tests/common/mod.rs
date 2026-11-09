//! Shared fixtures: a vdir of synthetic tasks in a tempdir.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use twydo_core::edit::{Change, NewTask, Op};
use twydo_daemon::client::{Client, ClientError};
use twydo_daemon::protocol::methods::{
    BasedOn, HelloParams, Instance, SubmitParams, UndoParams, WriteResult,
};
use twydo_daemon::protocol::rpc::{ErrorData, ErrorKind};
use twydo_daemon::protocol::wire::{WireSnapshot, WireTask};
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

    /// The text at `rel` (relative to the vdir).
    pub fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.vdir().join(rel)).unwrap()
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

    /// Starts with configuration `cfg` (for `fx`'s vdir) and waits until ready.
    pub fn start_cfg(fx: &Fixture, cfg: Config, f: impl FnOnce(&mut DaemonOptions)) -> TestDaemon {
        let td = Self::start_unready_cfg(fx, cfg, f);
        assert!(td.handle().wait_ready(Duration::from_secs(10)), "not ready");
        td
    }

    /// Starts without waiting for startup to finish.
    pub fn start_unready(fx: &Fixture, f: impl FnOnce(&mut DaemonOptions)) -> TestDaemon {
        Self::start_unready_cfg(fx, fx.config(), f)
    }

    fn start_unready_cfg(
        fx: &Fixture,
        cfg: Config,
        f: impl FnOnce(&mut DaemonOptions),
    ) -> TestDaemon {
        let dir = short_socket_dir();
        let socket = dir.path().join("s").join("tdo.sock");
        let mut opts = DaemonOptions::new(socket.clone());
        f(&mut opts);
        let instance = fx.instance();
        let handle = Daemon::start(cfg, instance.clone(), opts).expect("start");
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

    /// A client that has said hello asking to resume `session`.
    pub fn client_resuming(&self, session: &str) -> Client {
        let mut p = self.hello_params();
        p.session = Some(session.to_string());
        Client::connect(&self.socket, p).expect("hello").0
    }

    pub fn vdir(&self) -> &Path {
        &self.vdir
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

// ---- writes ----

/// The latest snapshot.
pub fn snapshot(c: &Client) -> WireSnapshot {
    c.call("snapshot.get", empty()).unwrap()
}

/// The task `uid` in `snap`.
pub fn task<'a>(snap: &'a WireSnapshot, uid: &str) -> &'a WireTask {
    snap.tasks
        .iter()
        .find(|t| t.task.uid == uid)
        .unwrap_or_else(|| panic!("no task {uid}"))
}

/// The snapshot `c` sees now, as `based_on`.
pub fn based(c: &Client) -> BasedOn {
    BasedOn {
        epoch: c.epoch().expect("hello"),
        revision: snapshot(c).revision,
    }
}

/// Submits `op` based on the current revision, with a fresh request id.
pub fn submit(c: &Client, op: Op) -> Result<WriteResult, ClientError> {
    let b = based(c);
    submit_at(c, b, &c.next_request_id(), op)
}

pub fn submit_at(
    c: &Client,
    based_on: BasedOn,
    request_id: &str,
    op: Op,
) -> Result<WriteResult, ClientError> {
    c.call(
        "op.submit",
        SubmitParams {
            session: c.session().expect("hello"),
            request_id: request_id.to_string(),
            based_on,
            op,
        },
    )
}

pub fn undo(c: &Client) -> Result<WriteResult, ClientError> {
    undo_with(c, &c.next_request_id())
}

pub fn undo_with(c: &Client, request_id: &str) -> Result<WriteResult, ClientError> {
    c.call(
        "op.undo",
        UndoParams {
            session: c.session().expect("hello"),
            request_id: request_id.to_string(),
        },
    )
}

pub fn redo(c: &Client) -> Result<WriteResult, ClientError> {
    c.call(
        "op.redo",
        UndoParams {
            session: c.session().expect("hello"),
            request_id: c.next_request_id(),
        },
    )
}

pub fn create(title: &str) -> Op {
    Op::Create(NewTask::plain("inbox", title))
}

pub fn edit_title(uid: &str, title: &str) -> Op {
    Op::Edit {
        uid: uid.into(),
        changes: vec![Change::Summary(title.into())],
    }
}

/// How many `.ics` files in the vdir have SUMMARY `title`.
pub fn count_titled(td: &TestDaemon, title: &str) -> usize {
    let want = format!("SUMMARY:{title}");
    let mut n = 0;
    for col in fs::read_dir(td.vdir()).unwrap() {
        let col = col.unwrap().path();
        if !col.is_dir() {
            continue;
        }
        for f in fs::read_dir(&col).unwrap() {
            let f = f.unwrap().path();
            if f.extension().is_some_and(|e| e == "ics")
                && fs::read_to_string(&f)
                    .unwrap()
                    .lines()
                    .any(|l| l.trim_end() == want)
            {
                n += 1;
            }
        }
    }
    n
}

/// The project family of the phase-0 capture `plan6-list1/04-reopen-child` (synthetic test
/// account): project `P` with open items `p1`, `p2`, `p3` and a completed history copy, in
/// list `list1`.
pub mod project {
    pub const P: &str = "09c49c2cd76f483498fc2ff3a0f66160";
    pub const P1: &str = "e668818024cc48f4a404fb9ced368789";
    pub const P2: &str = "e0483d712d7a487480c8aaa7e6cd4b8d";
    pub const P3: &str = "6a41bc3350d04ca2abffeebf69893007";
    pub const P3_COPY: &str = "905ce14bad6e4f849d697df5da10ad48";
    pub const LIST: &str = "list1";

    pub fn fixture() -> super::Fixture {
        let fx = super::Fixture::new(&[]);
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/phase0/v1/plan6-list1/04-reopen-child");
        for uid in [P, P1, P2, P3, P3_COPY] {
            let text = std::fs::read_to_string(src.join(format!("{uid}.ics"))).unwrap();
            fx.write(&format!("{LIST}/{uid}.ics"), &text);
        }
        fx.write(
            &format!("{LIST}/displayname"),
            &std::fs::read_to_string(src.join("displayname")).unwrap(),
        );
        fx
    }
}
