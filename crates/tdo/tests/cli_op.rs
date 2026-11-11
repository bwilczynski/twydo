//! `tdo op` and startup recovery, driven through the built binary. Content is synthetic
//! (phase-0 fixtures from the isolated test account).

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use common::Proc;

const TASK_E: &str = "44123b09407c4ea1adae8f2ecdb64d49";
const TASK_A: &str = "0a04f22aeb294b719633c978fe1975e5";

struct Env {
    dir: tempfile::TempDir,
    sock_dir: tempfile::TempDir,
    /// The daemon `op` started with this env's matrix, if it did.
    daemon: std::cell::RefCell<Option<Proc>>,
}

impl Env {
    fn new(mode: &str) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let vdir = root.join("vdir");
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/phase0/v1/plan4-list1/02-edit");
        fs::create_dir_all(vdir.join("list1")).unwrap();
        fs::create_dir_all(vdir.join("list2")).unwrap();
        for e in fs::read_dir(&src).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "ics") {
                fs::copy(&p, vdir.join("list1").join(p.file_name().unwrap())).unwrap();
            }
        }
        fs::create_dir_all(root.join("config/twydo")).unwrap();
        fs::write(
            root.join("config/twydo/config.toml"),
            format!(
                "vdir = \"{}\"\ntimezone = \"America/New_York\"\n[writes]\nmode = \"{mode}\"\n",
                vdir.display()
            ),
        )
        .unwrap();
        let mut matrix = String::from("matrix_version = 1\n");
        // every case the tests touch passes the iPhone baseline; the other columns stay
        // unverified, which test mode accepts
        for (case, op) in [
            ("checklist", "complete"),
            ("one-time", "complete"),
            ("monthly", "complete"),
            ("one-time", "create"),
            ("one-time", "edit"),
            ("monthly", "create"),
            ("monthly", "edit"),
            ("date-only", "create"),
            ("date-only", "edit"),
            ("checklist", "create"),
            ("checklist", "edit"),
            ("project", "edit"),
            ("recurring-project", "edit"),
            ("daily", "create"),
            ("task-to-project", "convert"),
        ] {
            matrix.push_str(&format!(
                "[[operation]]\ncase = \"{case}\"\nop = \"{op}\"\niphone_baseline = \"pass\"\n\
                 mac_to_iphone = \"unverified\"\niphone_to_mac = \"unverified\"\n\
                 twydo_to_iphone = \"unverified\"\niphone_to_twydo = \"unverified\"\n"
            ));
        }
        matrix.push_str(
            "[[rule]]\nclass = \"months-day-1-28\"\npreserve = \"pass\"\nexecute = \"pass\"\nedit = \"pass\"\n",
        );
        fs::write(root.join("matrix.toml"), matrix).unwrap();
        Env {
            dir,
            sock_dir: common::short_socket_dir(),
            daemon: Default::default(),
        }
    }

    fn vdir(&self) -> PathBuf {
        self.dir.path().join("vdir")
    }

    fn state(&self) -> PathBuf {
        self.dir.path().join("state")
    }

    /// The per-vdir state dir `tdo` uses for this env's vdir (journal and lock).
    fn vdir_state(&self) -> PathBuf {
        twydo_core::store::vdir_state_dir(&self.state().join("twydo"), &self.vdir()).unwrap()
    }

    fn socket(&self) -> PathBuf {
        self.sock_dir.path().join("tdo.sock")
    }

    /// A `tdo` command in this env: its own config, state and socket (never the user's).
    fn cmd(&self) -> Command {
        let mut c = common::tdo_bin(
            &self.socket(),
            &self.dir.path().join("config"),
            &self.state(),
        );
        c.env("XDG_RUNTIME_DIR", self.sock_dir.path().join("run"))
            .env("HOME", self.dir.path());
        c
    }

    fn tdo(&self, args: &[&str]) -> Output {
        self.cmd().args(args).output().expect("run tdo")
    }

    /// Starts `tdo daemon --matrix <env matrix>` unless one answers already; waits for its
    /// socket.
    fn ensure_daemon(&self) {
        self.ensure_daemon_with(true);
    }

    fn ensure_daemon_with(&self, matrix: bool) {
        if std::os::unix::net::UnixStream::connect(self.socket()).is_ok() {
            return;
        }
        let m = self.dir.path().join("matrix.toml");
        let mut c = self.cmd();
        c.arg("daemon");
        if matrix {
            c.args(["--matrix", m.to_str().unwrap()]);
        }
        c.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        *self.daemon.borrow_mut() = Some(Proc(c.spawn().unwrap()));
        let deadline = Instant::now() + common::WAIT;
        while std::os::unix::net::UnixStream::connect(self.socket()).is_err() {
            assert!(Instant::now() < deadline, "daemon never listened");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// `tdo op <args>` against a daemon running this env's matrix.
    fn op(&self, args: &[&str]) -> Output {
        let mut a = vec!["op"];
        a.extend_from_slice(args);
        self.ensure_daemon();
        self.tdo(&a)
    }

    /// Spawns `tdo op <args>` with piped output.
    fn spawn_op(&self, args: &[&str]) -> std::process::Child {
        self.ensure_daemon();
        self.cmd()
            .arg("op")
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    }

    /// An in-process daemon with this env's matrix, so a test can hold its lane.
    fn in_process_daemon(&self) -> twydo_daemon::server::DaemonHandle {
        use twydo_daemon::server::{Daemon, DaemonOptions};
        let mut cfg =
            twydo_core::config::Config::load(&self.dir.path().join("config/twydo/config.toml"))
                .unwrap();
        cfg.state_dir = Some(self.state().join("twydo"));
        let mut opts = DaemonOptions::new(self.socket());
        opts.schedule = false;
        opts.matrix =
            twydo_core::compat::Matrix::load(&self.dir.path().join("matrix.toml")).unwrap();
        let instance = twydo_daemon::paths::instance_for(
            &self.dir.path().join("config/twydo/config.toml"),
            None,
        )
        .unwrap();
        let h = Daemon::start(cfg, instance, opts).expect("start");
        assert!(h.wait_ready(Duration::from_secs(10)), "not ready");
        h
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        self.stop_daemon();
    }
}

impl Env {
    /// Stops whatever daemon answers on this env's socket and waits for it to go.
    fn stop_daemon(&self) {
        use twydo_daemon::client::Client;
        // Stops a daemon any `tdo` started on demand; the matrix one is killed by its Proc.
        if let Ok(raw) = Client::connect_raw(&self.socket()) {
            let _ = raw.caller().call_timeout::<_, serde_json::Value>(
                "daemon.shutdown",
                twydo_daemon::protocol::methods::ShutdownParams { when_idle: false },
                Duration::from_secs(5),
            );
            drop(raw);
            let deadline = Instant::now() + Duration::from_secs(5);
            while Client::connect_raw(&self.socket()).is_ok() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn synthetic_task(uid: &str, title: &str, extra: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\n\
         UID:{uid}\r\nSUMMARY:{title}\r\n{extra}END:VTODO\r\nEND:VCALENDAR\r\n"
    )
}

fn list_files(env: &Env) -> std::collections::BTreeMap<String, Vec<u8>> {
    fs::read_dir(env.vdir().join("list1"))
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            (
                p.file_name().unwrap().to_str().unwrap().to_owned(),
                fs::read(p).unwrap(),
            )
        })
        .collect()
}

fn op_id_ok(s: &str) -> bool {
    let b = s.trim().as_bytes();
    b.len() == 25
        && b[..8].iter().all(u8::is_ascii_digit)
        && b[8] == b'T'
        && b[9..15].iter().all(u8::is_ascii_digit)
        && b[15] == b'Z'
        && b[16] == b'-'
        && b[17..]
            .iter()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

#[test]
fn op_blocked_in_verified_mode_exits_1() {
    let env = Env::new("verified");
    let file = env.vdir().join(format!("list1/{TASK_E}.ics"));
    let before = fs::read(&file).unwrap();
    let o = env.op(&["complete", TASK_E]);
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    assert!(err(&o).contains("not verified yet"), "{}", err(&o));
    assert!(err(&o).starts_with("tdo: "));
    assert_eq!(fs::read(&file).unwrap(), before);
}

#[test]
fn op_test_mode_warns() {
    let env = Env::new("test");
    let o = env.op(&["journal"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        err(&o).contains("test write mode: unverified operations are enabled"),
        "{}",
        err(&o)
    );
    let o = Env::new("verified").op(&["journal"]);
    assert!(!err(&o).contains("test write mode"));
}

#[test]
fn op_complete_in_test_mode_prints_op_id() {
    let env = Env::new("test");
    let before = fs::read_dir(env.vdir().join("list1")).unwrap().count();
    let o = env.op(&["complete", TASK_E]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(op_id_ok(&out(&o)), "stdout was {:?}", out(&o));
    let after = fs::read_dir(env.vdir().join("list1")).unwrap().count();
    assert_eq!(after, before + 1, "history copy missing");

    let j = env.op(&["journal"]);
    assert!(j.status.success(), "{}", err(&j));
    let line = out(&j);
    assert!(line.contains(out(&o).trim()), "{line}");
    assert!(line.contains("complete  applied"), "{line}");
    assert!(line.contains(TASK_E), "{line}");
}

#[test]
fn op_check_lists_every_kind() {
    let env = Env::new("test");
    let o = env.op(&["check", TASK_E]);
    assert!(o.status.success(), "{}", err(&o));
    let text = out(&o);
    for action in [
        "edit title",
        "edit notes",
        "complete",
        "reopen",
        "delete",
        "create",
        "edit dates",
        "edit repeat",
        "convert",
        "move",
        "reorder",
        "insert",
    ] {
        assert!(
            text.lines().any(|l| l.starts_with(&format!("{action}: "))),
            "{text}"
        );
    }
    assert!(text.contains("complete: allowed"), "{text}");
    assert!(text.contains("reopen: inapplicable"), "{text}");
    assert!(
        text.contains("move: needs input: destination list"),
        "{text}"
    );
    assert!(!text.contains("edit: ok"), "{text}");
}

#[test]
fn op_journal_and_recover_run_on_empty_state() {
    let env = Env::new("verified");
    let j = env.op(&["journal"]);
    assert!(j.status.success(), "{}", err(&j));
    assert_eq!(out(&j), "");
    let r = env.op(&["recover"]);
    assert!(r.status.success(), "{}", err(&r));
    assert!(out(&r).contains("resumed: 0"), "{}", out(&r));
    assert!(out(&r).contains("stuck: 0"), "{}", out(&r));
}

#[test]
fn op_edit_without_changes_is_usage_error() {
    let env = Env::new("test");
    let o = env.op(&["edit", TASK_A]);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
}

#[test]
fn recover_runs_before_export() {
    let env = Env::new("verified");
    let rel = format!("list1/{TASK_A}.ics");
    let before = fs::read_to_string(env.vdir().join(&rel)).unwrap();
    let title = before
        .lines()
        .find_map(|l| l.strip_prefix("SUMMARY:"))
        .unwrap()
        .to_string();
    let after = before.replacen(&format!("SUMMARY:{title}"), "SUMMARY:Recovered title", 1);
    assert_ne!(before, after);
    let id = "20261101T120000Z-abcdef12";
    let entry = serde_json::json!({
        "format": 1,
        "id": id,
        "created": "2026-11-01T12:00:00Z",
        "kind": "edit",
        "uids": [TASK_A],
        "stage": "prepared",
        "inverse_of": null,
        "writes": [{"path": rel, "before": before, "after": after}],
    });
    let jdir = env.vdir_state().join("journal");
    fs::create_dir_all(&jdir).unwrap();
    fs::write(jdir.join(format!("{id}.json")), entry.to_string()).unwrap();

    let o = env.tdo(&["export-json"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("Recovered title"), "{}", out(&o));
    assert_eq!(fs::read_to_string(env.vdir().join(&rel)).unwrap(), after);
}

/// Readers take the shared vdir lock (spec §6): `export-json` waits while a writer holds the
/// exclusive lock.
#[test]
fn export_waits_for_writer_lock() {
    use std::time::{Duration, Instant};
    let env = Env::new("verified");
    let sdir = env.vdir_state();
    fs::create_dir_all(&sdir).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(sdir.join("vdir.lock"))
        .unwrap();
    lock.lock().unwrap();
    let mut child = env
        .cmd()
        .arg("export-json")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(700) {
        assert!(
            child.try_wait().unwrap().is_none(),
            "export-json finished while the exclusive lock was held"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    lock.unlock().unwrap();
    let status = child.wait().unwrap();
    assert!(status.success());
}

/// A Prepared entry whose `before` no longer matches the disk is stuck: readers warn and
/// still read, writes exit 1 until the entry is abandoned.
#[test]
fn stuck_entry_blocks_writes_until_abandoned() {
    let env = Env::new("test");
    let rel = format!("list1/{TASK_A}.ics");
    let disk = fs::read_to_string(env.vdir().join(&rel)).unwrap();
    let id = "20261101T120000Z-0badf00d";
    let entry = serde_json::json!({
        "format": 1,
        "id": id,
        "created": "2026-11-01T12:00:00Z",
        "kind": "edit",
        "uids": [TASK_A],
        "stage": "prepared",
        "inverse_of": null,
        "writes": [{"path": rel, "before": "neither before", "after": "nor after"}],
    });
    let jdir = env.vdir_state().join("journal");
    fs::create_dir_all(&jdir).unwrap();
    fs::write(jdir.join(format!("{id}.json")), entry.to_string()).unwrap();

    let o = env.tdo(&["export-json"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains(TASK_A), "export missing tasks");
    assert!(
        err(&o).contains("tdo: journal has unfinished operations; run 'tdo op journal'"),
        "{}",
        err(&o)
    );
    assert_eq!(fs::read_to_string(env.vdir().join(&rel)).unwrap(), disk);

    let e_file = env.vdir().join(format!("list1/{TASK_E}.ics"));
    let e_before = fs::read(&e_file).unwrap();
    let c = env.op(&["complete", TASK_E]);
    assert_eq!(c.status.code(), Some(1), "{}", err(&c));
    assert!(
        err(&c).contains("journal has unfinished operations"),
        "{}",
        err(&c)
    );
    assert_eq!(fs::read_to_string(env.vdir().join(&rel)).unwrap(), disk);
    assert_eq!(fs::read(&e_file).unwrap(), e_before);

    let a = env.op(&["abandon", id]);
    assert!(a.status.success(), "{}", err(&a));
    assert_eq!(out(&a).trim(), format!("abandoned {id}"));
    let c = env.op(&["complete", TASK_E]);
    assert!(c.status.success(), "{}", err(&c));
    assert!(op_id_ok(&out(&c)), "stdout was {:?}", out(&c));
    assert_ne!(fs::read(&e_file).unwrap(), e_before);
    // The abandoned entry's file is left as it was.
    assert_eq!(fs::read_to_string(env.vdir().join(&rel)).unwrap(), disk);
}

/// `tdo op` loads its snapshot under the shared vdir lock: `op check` waits while a writer
/// holds the exclusive lock, so it never judges a partly applied operation.
#[test]
fn op_snapshot_waits_for_writer_lock() {
    use std::time::{Duration, Instant};
    use twydo_core::store::fsio::VdirLock;
    let env = Env::new("verified");
    let lock = VdirLock::exclusive(&env.vdir_state()).unwrap();
    let m = env.dir.path().join("matrix.toml");
    let mut child = env
        .cmd()
        .args(["op", "check", TASK_E, "--matrix", m.to_str().unwrap()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(300) {
        assert!(
            child.try_wait().unwrap().is_none(),
            "op check finished while the exclusive lock was held"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(lock);
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(0));
}

/// `tdo op check <uid>` until the daemon has seen the task's file (it notices changes made
/// behind its back through its watcher), within 10 s.
fn op_check_when_seen(env: &Env, uid: &str) -> Output {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let o = env.op(&["check", uid]);
        if o.status.success() || Instant::now() > deadline {
            return o;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A missing vdir is reported as unreadable, not as a journal failure.
#[test]
fn missing_vdir_reports_only_read_error() {
    let env = Env::new("verified");
    let missing = env.dir.path().join("no-such-vdir");
    let o = env.tdo(&["--vdir", missing.to_str().unwrap(), "check"]);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    assert!(err(&o).contains("no-such-vdir"), "{}", err(&o));
    assert!(!err(&o).contains("journal recovery failed"), "{}", err(&o));
}

#[test]
fn op_journal_shows_synced() {
    let env = Env::new("test");
    let id = out(&env.op(&["complete", TASK_E])).trim().to_string();
    let j = out(&env.op(&["journal"]));
    assert!(j.lines().next().unwrap().ends_with("pending"), "{j}");

    let journal = twydo_core::store::journal::Journal::open(&env.vdir_state()).unwrap();
    let at = chrono::DateTime::parse_from_rfc3339("2026-11-02T08:30:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    journal.mark_synced(&id, at).unwrap();
    let j = out(&env.op(&["journal"]));
    assert!(
        j.lines()
            .next()
            .unwrap()
            .ends_with("synced 2026-11-02T08:30:00Z"),
        "{j}"
    );
}

/// Adds a `[sync]` table whose command is a fake script touching `marker`; returns the marker.
fn with_fake_sync(env: &Env) -> PathBuf {
    use std::io::Write;
    let marker = env.dir.path().join("marker");
    let fake = env.dir.path().join("fake.sh");
    common::write_script(&fake, &format!("touch '{}'", marker.display()));
    let mut cfg = fs::OpenOptions::new()
        .append(true)
        .open(env.dir.path().join("config/twydo/config.toml"))
        .unwrap();
    write!(
        cfg,
        "[sync]\ncommand = \"{}\"\nmetasync = false\n",
        fake.display()
    )
    .unwrap();
    marker
}

/// True if `marker` stays absent for a grace period.
fn marker_stays_absent(marker: &Path) -> bool {
    std::thread::sleep(std::time::Duration::from_millis(1500));
    !marker.exists()
}

#[test]
fn complete_parent_needs_all_flag() {
    let env = Env::new("test");
    let task = |uid: &str, title: &str, extra: &str| {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\n\
             UID:{uid}\r\nSUMMARY:{title}\r\n{extra}STATUS:NEEDS-ACTION\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
        )
    };
    let dir = env.vdir().join("list1");
    fs::write(
        dir.join("fam-p.ics"),
        task(
            "fam-p",
            "Synthetic parent",
            "X-2DOAPP-METADATA:@tasktype:checklist\r\n",
        ),
    )
    .unwrap();
    for n in ["fam-1", "fam-2"] {
        fs::write(
            dir.join(format!("{n}.ics")),
            task(n, "Synthetic item", "RELATED-TO:fam-p\r\n"),
        )
        .unwrap();
    }
    let o = env.op(&["complete", "fam-p"]);
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    assert!(
        err(&o).contains("\"Synthetic parent\" has 2 open items; pass --all to complete them too"),
        "{}",
        err(&o)
    );
    let o = env.op(&["complete", "fam-p", "--all"]);
    assert!(o.status.success(), "{}", err(&o));
}

#[test]
fn complete_parent_with_one_item_uses_the_singular() {
    let env = Env::new("test");
    let dir = env.vdir().join("list1");
    let task = |uid: &str, extra: &str| {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\n\
             UID:{uid}\r\nSUMMARY:Synthetic {uid}\r\n{extra}STATUS:NEEDS-ACTION\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
        )
    };
    fs::write(
        dir.join("one-p.ics"),
        task("one-p", "X-2DOAPP-METADATA:@tasktype:checklist\r\n"),
    )
    .unwrap();
    fs::write(dir.join("one-1.ics"), task("one-1", "RELATED-TO:one-p\r\n")).unwrap();
    let o = env.op(&["complete", "one-p"]);
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    assert!(
        err(&o).contains("has 1 open item; pass --all"),
        "{}",
        err(&o)
    );
}

/// The text of the one vdir file whose summary is `title`.
fn file_titled(env: &Env, title: &str) -> String {
    for e in fs::read_dir(env.vdir().join("list1")).unwrap() {
        let s = fs::read_to_string(e.unwrap().path()).unwrap();
        if s.contains(&format!("SUMMARY:{title}\r\n")) || s.contains(&format!("SUMMARY:{title}\n"))
        {
            return s;
        }
    }
    panic!("no file titled {title}");
}

fn write_checklist(env: &Env) {
    fs::write(
        env.vdir().join("list1/cl-p.ics"),
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\n\
         UID:cl-p\r\nSUMMARY:Synthetic list\r\nX-2DOAPP-METADATA:@tasktype:checklist\r\n\
         STATUS:NEEDS-ACTION\r\nEND:VTODO\r\nEND:VCALENDAR\r\n",
    )
    .unwrap();
}

#[test]
fn add_item_under_checklist() {
    let env = Env::new("test");
    write_checklist(&env);
    let o = env.op(&[
        "add",
        "--list",
        "list1",
        "--title",
        "Synthetic item",
        "--parent",
        "cl-p",
    ]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(file_titled(&env, "Synthetic item").contains("RELATED-TO:cl-p"));
}

#[test]
fn add_with_repeat() {
    let env = Env::new("test");
    let o = env.op(&[
        "add",
        "--list",
        "list1",
        "--title",
        "Synthetic rep",
        "--due",
        "2026-12-05",
        "--repeat",
        "monthly",
    ]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(file_titled(&env, "Synthetic rep").contains("RRULE:FREQ=MONTHLY"));
    // without a due date the task becomes due today, as 2Do does
    let o = env.op(&[
        "add",
        "--list",
        "list1",
        "--title",
        "Synthetic undated",
        "--repeat",
        "monthly",
    ]);
    assert!(o.status.success(), "{}", err(&o));
    let f = file_titled(&env, "Synthetic undated");
    assert!(f.contains("RRULE:FREQ=MONTHLY") && f.contains("DUE"), "{f}");
}

#[test]
fn edit_kind() {
    let env = Env::new("test");
    let o = env.op(&["edit", TASK_E, "--kind", "project"]);
    assert!(o.status.success(), "{}", err(&o));
    let f = fs::read_to_string(env.vdir().join(format!("list1/{TASK_E}.ics")))
        .unwrap()
        .replace("\r\n ", "")
        .replace("\n ", "");
    assert!(f.contains("TaskType%22%3A2"), "{f}");
}

#[test]
fn edit_repeat_bad_text_exits_2() {
    let env = Env::new("test");
    let o = env.op(&["edit", TASK_E, "--repeat", "every blue moon"]);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    assert!(err(&o).starts_with("tdo: "), "{}", err(&o));
    assert!(err(&o).len() > 10, "{}", err(&o));
}

/// Setting or removing a repeat needs the `repeat` rows: this matrix has none, and the shipped
/// one enables plain daily/weekly/monthly/yearly and scoped completion-anchor changes.
#[test]
fn edit_setting_a_repeat_needs_the_repeat_rows() {
    let file_of = |env: &Env| env.vdir().join(format!("list1/{TASK_A}.ics"));
    let set = ["edit", TASK_A, "--due", "2026-12-05", "--repeat", "monthly"];
    let env = Env::new("test");
    let before = fs::read(file_of(&env)).unwrap();
    let o = env.op(&set);
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    assert!(
        err(&o).contains(
            "changing the repeat rule of one-time tasks is not verified yet (iphone_baseline)"
        ),
        "{}",
        err(&o)
    );
    assert_eq!(fs::read(file_of(&env)).unwrap(), before);
    // A due change mixed with establishing completion anchoring is outside RF/RA scope.
    let env = Env::new("verified");
    let o = env.tdo(&[
        "op",
        "edit",
        TASK_A,
        "--due",
        "2026-12-05",
        "--repeat",
        "monthly after done",
    ]);
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    assert!(
        err(&o).contains("from-completion") && err(&o).contains("captured"),
        "{}",
        err(&o)
    );
    assert_eq!(fs::read(file_of(&env)).unwrap(), before);
    // Setting a monthly repeat on a one-time task and removing E's monthly repeat are verified.
    let mut args = vec!["op"];
    args.extend_from_slice(&set);
    let o = env.tdo(&args);
    assert!(o.status.success(), "{}", err(&o));
    assert_ne!(fs::read(file_of(&env)).unwrap(), before);
    let o = env.tdo(&["op", "edit", TASK_A, "--repeat", "yearly"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        String::from_utf8(fs::read(file_of(&env)).unwrap())
            .unwrap()
            .contains("RRULE:FREQ=YEARLY")
    );
    let o = env.tdo(&["op", "edit", TASK_E, "--no-repeat"]);
    assert!(o.status.success(), "{}", err(&o));
}

/// `--kind` naming the type the file already declares changes nothing, so nothing is
/// written: not even new stamps.
#[test]
fn edit_kind_same_as_declared_writes_nothing() {
    let env = Env::new("test");
    let file = env.vdir().join(format!("list1/{TASK_A}.ics"));
    let before = fs::read(&file).unwrap();
    let o = env.op(&["edit", TASK_A, "--kind", "task"]);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    assert!(err(&o).contains("already a task"), "{}", err(&o));
    assert_eq!(fs::read(&file).unwrap(), before);
    // With another change, only that change is written.
    let o = env.op(&["edit", TASK_A, "--kind", "task", "--title", "Synthetic A"]);
    assert!(o.status.success(), "{}", err(&o));
    let after = fs::read_to_string(&file).unwrap();
    assert!(after.contains("SUMMARY:Synthetic A"), "{after}");
}

#[test]
fn verified_from_completion_edits_and_interval_card_vocabulary_respect_scoped_permissions() {
    let env = Env::new("verified");
    let uid = "twydo-p8-rfra-fields-p";
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/phase0/v1/plan8-rfra/202-seeded-after-upload")
        .join(format!("{uid}.ics"));
    let target = env.vdir().join(format!("list1/{uid}.ics"));
    fs::copy(source, &target).unwrap();
    for args in [
        vec!["op", "edit", uid, "--notes", "CLI verified"],
        vec!["op", "edit", uid, "--repeat", "every 2 months after done"],
    ] {
        let o = env.tdo(&args);
        assert!(o.status.success(), "{}", err(&o));
    }
    let before = fs::read(&target).unwrap();
    assert!(String::from_utf8_lossy(&before).contains("RRULE:FREQ=MONTHLY;INTERVAL=2"));
    let o = env.tdo(&["op", "edit", uid, "--notes", "text after interval change"]);
    assert!(o.status.success(), "{}", err(&o));
    let after = fs::read(&target).unwrap();
    assert_ne!(after, before);
    assert!(String::from_utf8_lossy(&after).contains("DESCRIPTION:text after interval change"));
    let o = env.tdo(&["op", "edit", uid, "--due", "2026-12-12"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(fs::read(&target).unwrap(), after);
    let single = env.tdo(&["op", "check", uid]);
    assert_eq!(single.status.code(), Some(0), "{}", err(&single));
    assert!(
        out(&single).contains("edit notes: allowed"),
        "{}",
        out(&single)
    );
    let aggregate = env.tdo(&["check"]);
    assert_eq!(aggregate.status.code(), Some(0), "{}", err(&aggregate));
    assert!(
        out(&aggregate).contains("edit notes: allowed="),
        "{}",
        out(&aggregate)
    );
}

/// Changes a task's file by a byte-visible property, as another device's edit would.
fn touch_task_file(env: &Env, uid: &str) {
    let file = env.vdir().join(format!("list1/{uid}.ics"));
    let text = fs::read_to_string(&file).unwrap();
    let changed = text.replacen("END:VTODO", "X-TEST-OTHER-DEVICE:1\r\nEND:VTODO", 1);
    assert_ne!(text, changed);
    fs::write(&file, changed).unwrap();
}

/// Waits until the daemon has recorded `n` refusals nobody has seen.
fn wait_unshown(env: &Env, n: usize) {
    let deadline = Instant::now() + common::WAIT;
    loop {
        if let Some(q) = twydo_core::store::queue::Queue::existing(&env.vdir_state())
            && q.unshown().unwrap().len() == n
        {
            return;
        }
        assert!(Instant::now() < deadline, "no {n} unshown refusals");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn op_during_a_long_job_is_queued() {
    use twydo_daemon::server::lane::LongKind;
    let env = Env::new("test");
    let d = env.in_process_daemon();
    let lock = d.hold_long(LongKind::Test);
    let start = Instant::now();
    let o = env.op(&["complete", TASK_E]);
    assert!(start.elapsed() < Duration::from_secs(5));
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert!(
        out(&o).starts_with("queued (a sync is running): "),
        "{}",
        out(&o)
    );
    let queued = fs::read_dir(env.vdir_state().join("queue"))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "json")
        })
        .count();
    assert_eq!(queued, 1);
    drop(lock);
}

#[test]
fn op_wait_reports_the_outcome() {
    use twydo_daemon::server::lane::LongKind;
    let env = Env::new("test");
    let d = env.in_process_daemon();
    let lock = d.hold_long(LongKind::Test);
    let child = env.spawn_op(&["complete", TASK_E, "--wait"]);
    std::thread::sleep(Duration::from_millis(1000));
    drop(lock);
    let o = child.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert!(
        err(&o).contains("waiting for the sync to finish (queued "),
        "{}",
        err(&o)
    );
    assert!(op_id_ok(&out(&o)), "stdout was {:?}", out(&o));
    let j = env.op(&["journal"]);
    assert!(out(&j).contains(out(&o).trim()), "{}", out(&j));
}

#[test]
fn op_wait_reports_a_refusal() {
    use twydo_daemon::server::lane::LongKind;
    let env = Env::new("test");
    let d = env.in_process_daemon();
    let lock = d.hold_long(LongKind::Test);
    let child = env.spawn_op(&["complete", TASK_E, "--wait"]);
    std::thread::sleep(Duration::from_millis(1000));
    touch_task_file(&env, TASK_E);
    drop(lock);
    let o = child.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    assert!(err(&o).contains("Not saved"), "{}", err(&o));
    assert_eq!(out(&o), "");
    // the waiter showed it: nothing is repeated later
    let j = env.op(&["journal"]);
    assert!(!err(&j).contains("Not saved"), "{}", err(&j));
}

#[test]
fn op_prints_unshown_refusals_once() {
    use twydo_daemon::server::lane::LongKind;
    let env = Env::new("test");
    let d = env.in_process_daemon();
    let lock = d.hold_long(LongKind::Test);
    let o = env.op(&["complete", TASK_E]);
    assert!(out(&o).starts_with("queued"), "{}", out(&o));
    touch_task_file(&env, TASK_E);
    drop(lock);
    wait_unshown(&env, 1);
    let queue = twydo_core::store::queue::Queue::existing(&env.vdir_state()).unwrap();
    let pending = queue.unshown().unwrap();
    let before = serde_json::to_value(&pending).unwrap();
    let private_label = &pending[0].item.label;
    // Permission reports must not disclose task labels or consume the notification.
    for report in [env.op(&["check", TASK_E]), env.tdo(&["check"])] {
        assert_eq!(report.status.code(), Some(0), "{}", err(&report));
        assert!(!err(&report).contains("Not saved"), "{}", err(&report));
        assert!(!err(&report).contains(private_label), "{}", err(&report));
        assert!(!out(&report).contains(private_label), "{}", out(&report));
        assert_eq!(
            serde_json::to_value(queue.unshown().unwrap()).unwrap(),
            before
        );
    }
    // The next ordinary operation reports it once.
    let first = env.op(&["journal"]);
    assert!(err(&first).contains("Not saved"), "{}", err(&first));
    let second = env.op(&["journal"]);
    assert!(!err(&second).contains("Not saved"), "{}", err(&second));
}

#[test]
fn op_prints_refusals_of_ended_cli_sessions_once() {
    use twydo_daemon::server::lane::LongKind;
    let env = Env::new("test");
    let d = env.in_process_daemon();
    let lock = d.hold_long(LongKind::Test);
    // This session ends when `tdo` exits; its queued write is refused afterwards.
    let o = env.op(&["edit", TASK_A, "--title", "Renamed"]);
    assert!(out(&o).starts_with("queued"), "{}", out(&o));
    touch_task_file(&env, TASK_A);
    drop(lock);
    wait_unshown(&env, 1);
    let first = env.op(&["edit", TASK_E, "--title", "Other"]);
    assert!(first.status.success(), "{}", err(&first));
    assert_eq!(
        err(&first).matches("Not saved").count(),
        1,
        "{}",
        err(&first)
    );
    let second = env.op(&["journal"]);
    assert!(!err(&second).contains("Not saved"), "{}", err(&second));
}

/// The daemon is killed while a queued write waits behind a sleeping sync; the daemon that
/// replaces it applies the write and the waiting `tdo op --wait` prints the journal id.
#[test]
fn op_wait_survives_a_daemon_restart() {
    use twydo_daemon::client::Client;
    use twydo_daemon::protocol::sync::SyncRunParams;
    let env = Env::new("test");
    let release = env.dir.path().join("release");
    let fake = env.dir.path().join("sleeper.sh");
    common::write_script(
        &fake,
        &format!("while [ ! -f '{}' ]; do sleep 0.1; done", release.display()),
    );
    fs::OpenOptions::new()
        .append(true)
        .open(env.dir.path().join("config/twydo/config.toml"))
        .and_then(|mut f| {
            use std::io::Write;
            write!(
                f,
                "[sync]\ncommand = \"{}\"\nmetasync = false\n",
                fake.display()
            )
        })
        .unwrap();
    // Automatic sync off: only the `sync.run` below runs the script.
    let sdir = twydo_core::store::bind_vdir_state(&env.state().join("twydo"), &env.vdir()).unwrap();
    fs::create_dir_all(sdir.join("sync")).unwrap();
    fs::write(sdir.join("sync/off"), "").unwrap();

    env.ensure_daemon_with(false);
    let cfg_path = env.dir.path().join("config/twydo/config.toml");
    let hello = twydo_daemon::protocol::methods::HelloParams {
        client: "test".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        protocol: twydo_daemon::PROTOCOL,
        session: None,
        instance: twydo_daemon::paths::instance_for(&cfg_path, None).unwrap(),
    };
    let deadline = Instant::now() + common::WAIT;
    let raw = loop {
        match Client::connect(&env.socket(), hello.clone()) {
            Ok((c, _)) => break c,
            Err(e) => {
                assert!(Instant::now() < deadline, "no daemon: {e}");
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    };
    let started = loop {
        let r: Result<serde_json::Value, _> = raw.call(
            "sync.run",
            SyncRunParams {
                request_id: "t-sync".into(),
                force_delete: vec![],
            },
        );
        match r {
            Ok(v) => break v,
            Err(e) if e.to_string() == "the daemon is starting" => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(e) => panic!("sync.run: {e}"),
        }
    };
    let _ = started;

    let child = env.spawn_op(&["edit", TASK_A, "--title", "Queued title", "--wait"]);
    let qdir = env.vdir_state().join("queue");
    let deadline = Instant::now() + common::WAIT;
    while !fs::read_dir(&qdir).is_ok_and(|d| {
        d.filter_map(Result::ok)
            .any(|e| e.path().extension().is_some_and(|x| x == "json"))
    }) {
        assert!(Instant::now() < deadline, "the write was never queued");
        std::thread::sleep(Duration::from_millis(20));
    }
    // kill -9 the daemon, let the sync script finish, and bring a daemon back.
    {
        let mut d = env.daemon.borrow_mut();
        let p = d.as_mut().unwrap();
        p.0.kill().unwrap();
        p.0.wait().unwrap();
    }
    drop(raw);
    fs::write(&release, "").unwrap();
    env.ensure_daemon_with(false);
    let o = child.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert!(op_id_ok(&out(&o)), "stdout was {:?}", out(&o));
    let after = fs::read_to_string(env.vdir().join(format!("list1/{TASK_A}.ics"))).unwrap();
    assert!(after.contains("SUMMARY:Queued title"), "{after}");
}

/// `tdo op` never opens `config.toml` itself: with the file unreadable the daemon still
/// serves it.
#[test]
fn op_never_opens_the_config_file() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new("test");
    env.ensure_daemon_with(false);
    // Wait until the daemon is ready, then take the file away.
    let ready = env.tdo(&["op", "journal"]);
    assert!(ready.status.success(), "{}", err(&ready));
    let cfg = env.dir.path().join("config/twydo/config.toml");
    fs::set_permissions(&cfg, fs::Permissions::from_mode(0o000)).unwrap();
    let o = env.tdo(&["op", "edit", TASK_A, "--title", "Without config"]);
    fs::set_permissions(&cfg, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert!(op_id_ok(&out(&o)), "stdout was {:?}", out(&o));
}

/// With `config.toml` unreadable the daemon still serves every read-only command.
#[test]
fn read_only_commands_never_open_the_config_file() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new("test");
    env.ensure_daemon();
    let ready = env.tdo(&["op", "journal"]);
    assert!(ready.status.success(), "{}", err(&ready));
    let cfg = env.dir.path().join("config/twydo/config.toml");
    fs::set_permissions(&cfg, fs::Permissions::from_mode(0o000)).unwrap();
    let outs: Vec<(&str, Output)> = [
        vec!["check"],
        vec!["export-json"],
        vec!["conflicts"],
        vec!["op", "check", TASK_E],
    ]
    .into_iter()
    .map(|a| (a[0], env.tdo(&a)))
    .collect();
    fs::set_permissions(&cfg, fs::Permissions::from_mode(0o644)).unwrap();
    for (name, o) in &outs {
        assert!(o.status.success(), "{name}: {}", err(o));
    }
}

/// `status` needs `[sync]`; its own test checks that it too works without the file.
#[test]
fn status_never_opens_the_config_file() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new("test");
    let cfg = env.dir.path().join("config/twydo/config.toml");
    let mut text = fs::read_to_string(&cfg).unwrap();
    text.push_str("\n[sync]\ncommand = \"/nonexistent/vdirsyncer\"\n");
    fs::write(&cfg, text).unwrap();
    env.ensure_daemon();
    let ready = env.tdo(&["op", "journal"]);
    assert!(ready.status.success(), "{}", err(&ready));
    fs::set_permissions(&cfg, fs::Permissions::from_mode(0o000)).unwrap();
    let o = env.tdo(&["status"]);
    fs::set_permissions(&cfg, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("last sync: never"), "{}", out(&o));
}

/// `check`, `export-json` and `status` warn on stderr when journal recovery left stuck entries.
#[test]
fn read_only_commands_warn_about_a_stuck_journal() {
    let env = Env::new("test");
    let cfg = env.dir.path().join("config/twydo/config.toml");
    let mut text = fs::read_to_string(&cfg).unwrap();
    text.push_str("\n[sync]\ncommand = \"/nonexistent/vdirsyncer\"\n");
    fs::write(&cfg, text).unwrap();
    let rel = format!("list1/{TASK_A}.ics");
    let id = "20261101T120000Z-0badf00d";
    let entry = serde_json::json!({
        "format": 1, "id": id, "created": "2026-11-01T12:00:00Z", "kind": "edit",
        "uids": [TASK_A], "stage": "prepared", "inverse_of": null,
        "writes": [{"path": rel, "before": "neither before", "after": "nor after"}],
    });
    let jdir = env.vdir_state().join("journal");
    fs::create_dir_all(&jdir).unwrap();
    fs::write(jdir.join(format!("{id}.json")), entry.to_string()).unwrap();
    env.ensure_daemon();
    let m = env.dir.path().join("matrix.toml");
    for args in [
        vec!["check", "--matrix", m.to_str().unwrap()],
        vec!["export-json"],
        vec!["status"],
    ] {
        let o = env.tdo(&args);
        assert!(o.status.success(), "{args:?}: {}", err(&o));
        assert!(
            err(&o).contains("tdo: journal has unfinished operations; run 'tdo op journal'"),
            "{args:?}: {}",
            err(&o)
        );
    }
    let o = env.op(&["check", TASK_E]);
    assert!(!err(&o).contains("journal has unfinished"), "{}", err(&o));
}

#[test]
fn both_reports_use_the_configured_mode_and_real_gate() {
    use chrono::Utc;
    use twydo_core::{
        compat::Matrix,
        config::Config,
        edit::{Change, Op},
        gate,
        views::Snapshot,
    };
    for mode in ["verified", "test"] {
        let env = Env::new(mode);
        let cfg = Config::load(&env.dir.path().join("config/twydo/config.toml")).unwrap();
        let matrix = Matrix::load(&env.dir.path().join("matrix.toml")).unwrap();
        let snap = Snapshot::new(twydo_core::vdir::load(&env.vdir()).unwrap(), cfg.clone());
        let task = &snap.tasks[snap.find(TASK_E).unwrap()];
        let single = env.op(&["check", TASK_E]);
        let m = env.dir.path().join("matrix.toml");
        let aggregate = env.tdo(&["check", "--matrix", m.to_str().unwrap()]);
        assert_eq!(single.status.code(), Some(0));
        assert_eq!(aggregate.status.code(), Some(0));
        for (action, op) in [
            (
                "edit title",
                Op::Edit {
                    uid: TASK_E.into(),
                    changes: vec![Change::Summary(task.summary.clone())],
                },
            ),
            (
                "edit notes",
                Op::Edit {
                    uid: TASK_E.into(),
                    changes: vec![Change::Description(task.description.clone())],
                },
            ),
            ("delete", Op::Delete { uid: TASK_E.into() }),
        ] {
            match gate::check(
                &snap,
                &matrix,
                cfg.writes.mode,
                &op,
                Utc::now().with_timezone(&cfg.tz()).date_naive(),
            ) {
                Ok(()) => assert!(
                    out(&single).contains(&format!("{action}: allowed")),
                    "{}",
                    out(&single)
                ),
                Err(b) => {
                    assert!(
                        out(&single).contains(&format!("{action}: refused: {b}")),
                        "{}",
                        out(&single)
                    );
                    assert!(out(&aggregate).contains(&b.0), "{}", out(&aggregate));
                }
            }
        }
        assert!(out(&aggregate).contains(&format!("mode={mode}")));
        assert_eq!(
            env.op(&["check", "synthetic-missing-uid"]).status.code(),
            Some(1)
        );
    }
}

#[test]
fn both_reports_see_a_queued_creation_without_draining_or_syncing() {
    use twydo_core::{config::Config, store::queue};
    use twydo_daemon::server::lane::LongKind;
    let env = Env::new("test");
    let cfg = {
        let mut cfg = Config::load(&env.dir.path().join("config/twydo/config.toml")).unwrap();
        cfg.state_dir = Some(env.state().join("twydo"));
        cfg
    };
    // A long job holds the daemon's lane, so the creation waits in the queue.
    let daemon = env.in_process_daemon();
    let job = daemon.hold_long(LongKind::Test);
    let count = twydo_core::vdir::load(&env.vdir()).unwrap().tasks.len();
    let added = env.op(&[
        "add",
        "--list",
        "list1",
        "--title",
        "Synthetic queued report target",
    ]);
    assert!(
        out(&added).starts_with("queued (a sync is running): "),
        "{} {}",
        out(&added),
        err(&added)
    );
    let queued = queue::view(twydo_core::vdir::load(&env.vdir()).unwrap(), cfg);
    let uid = queued
        .tasks
        .iter()
        .find(|t| t.summary == "Synthetic queued report target")
        .unwrap()
        .uid
        .clone();
    let queue_dir = env.vdir_state().join("queue");
    let queue_path = fs::read_dir(&queue_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "json"))
        .expect("a queued item");
    let queue_bytes = fs::read(&queue_path).unwrap();
    // Configure a marker only after enqueueing, so the preparation is not a sync trigger.
    let marker = with_fake_sync(&env);
    let files: Vec<_> = fs::read_dir(env.vdir().join("list1"))
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            let b = fs::read(&p).unwrap();
            (p, b)
        })
        .collect();
    let single = env.op(&["check", &uid]);
    assert_eq!(single.status.code(), Some(0), "{}", err(&single));
    assert!(
        out(&single).contains("edit title: allowed"),
        "{}",
        out(&single)
    );
    let m = env.dir.path().join("matrix.toml");
    let aggregate = env.tdo(&["check", "--matrix", m.to_str().unwrap(), "--verbose"]);
    assert_eq!(aggregate.status.code(), Some(0), "{}", err(&aggregate));
    assert!(
        out(&aggregate).contains(&format!("tasks: {}", count + 1)),
        "{}",
        out(&aggregate)
    );
    assert!(
        out(&aggregate).contains("Synthetic queued report target"),
        "{}",
        out(&aggregate)
    );
    assert_eq!(fs::read(queue_path).unwrap(), queue_bytes);
    assert_eq!(
        twydo_core::vdir::load(&env.vdir()).unwrap().tasks.len(),
        count
    );
    for (p, before) in files {
        assert_eq!(fs::read(p).unwrap(), before);
    }
    assert!(marker_stays_absent(&marker), "reporting started sync");
    drop(job);
}

#[test]
fn checklist_due_capability_cli_preserves_items_and_refuses_interval_candidate() {
    let env = Env::new("verified");
    let list = env.vdir().join("list1");
    fs::remove_dir_all(&list).unwrap();
    fs::create_dir(&list).unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/phase0/v1/plan8-tui-a/010-first-phone-actions");
    for p in fs::read_dir(src).unwrap().map(|e| e.unwrap().path()) {
        if p.extension().is_some_and(|x| x == "ics") {
            fs::copy(&p, list.join(p.file_name().unwrap())).unwrap();
        }
    }
    let uid = "61ed74be3ba34212ab7476b698de1263";
    let output = env.tdo(&["op", "edit", uid, "--due", "2026-12-11"]);
    assert!(output.status.success(), "{}", err(&output));
    let bytes = fs::read_to_string(list.join(format!("{uid}.ics"))).unwrap();
    assert!(bytes.contains("DUE;VALUE=DATE:20261211"));
    assert!(bytes.contains("RRULE:FREQ=MONTHLY"));
    for path in fs::read_dir(&list).unwrap().map(|e| e.unwrap().path()) {
        let text = fs::read_to_string(path).unwrap();
        if text.contains(&format!("RELATED-TO:{uid}")) {
            assert!(!text.contains("DUE;"));
            assert!(!text.contains("DTSTART;"));
        }
    }
    let output = env.tdo(&["op", "edit", uid, "--interval", "2"]);
    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(list.join(format!("{uid}.ics"))).unwrap(),
        bytes
    );
}

#[test]
fn check_and_op_check_reflect_exact_resource_deletion_without_titles() {
    let env = Env::new("verified");
    let e_file = env.vdir().join(format!("list1/{TASK_E}.ics"));
    let text = fs::read_to_string(&e_file)
        .unwrap()
        .replace("RRULE:FREQ=MONTHLY", "RRULE:FREQ=WEEKLY");
    fs::write(&e_file, text).unwrap();

    let o = env.tdo(&["op", "check", TASK_E]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert!(out(&o).contains("delete: allowed"), "{}", out(&o));

    let parent = "synthetic-cap2-parent";
    let child = "synthetic-cap2-child";
    fs::write(
        env.vdir().join(format!("list1/{parent}.ics")),
        synthetic_task(
            parent,
            "Synthetic cap2 parent",
            "STATUS:NEEDS-ACTION\r\nX-2DOAPP-METADATA:@tasktype:checklist\r\n",
        ),
    )
    .unwrap();
    fs::write(
        env.vdir().join(format!("list1/{child}.ics")),
        synthetic_task(
            child,
            "Synthetic cap2 child",
            &format!("RELATED-TO:{parent}\r\nSTATUS:NEEDS-ACTION\r\n"),
        ),
    )
    .unwrap();

    let o = op_check_when_seen(&env, parent);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert!(out(&o).contains("delete: allowed"), "{}", out(&o));
    assert!(op_check_when_seen(&env, child).status.success());
    let o = env.tdo(&["check"]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert!(out(&o).contains("delete: allowed="), "{}", out(&o));
    assert!(!out(&o).contains("Synthetic cap2 parent"), "{}", out(&o));
    assert!(!out(&o).contains("Synthetic cap2 child"), "{}", out(&o));
    let v = env.tdo(&["check", "--verbose"]);
    assert!(out(&v).contains("Synthetic cap2 parent"), "{}", out(&v));
    assert!(out(&v).contains("Synthetic cap2 child"), "{}", out(&v));

    // A plain task with a child remains refused and writes no bytes.
    let plain = "synthetic-cap2-plain";
    let plain_child = "synthetic-cap2-plain-child";
    fs::write(
        env.vdir().join(format!("list1/{plain}.ics")),
        synthetic_task(plain, "Synthetic cap2 plain", "STATUS:NEEDS-ACTION\r\n"),
    )
    .unwrap();
    fs::write(
        env.vdir().join(format!("list1/{plain_child}.ics")),
        synthetic_task(
            plain_child,
            "Synthetic cap2 plain child",
            &format!("RELATED-TO:{plain}\r\nSTATUS:NEEDS-ACTION\r\n"),
        ),
    )
    .unwrap();
    assert!(op_check_when_seen(&env, plain_child).status.success());
    assert!(op_check_when_seen(&env, plain).status.success());
    let before = list_files(&env);
    let o = env.tdo(&["op", "delete", plain]);
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    assert!(err(&o).contains("subtasks of a plain task"), "{}", err(&o));
    assert_eq!(list_files(&env), before);
}

#[test]
fn history_reopen_reports_match_real_gate_without_task_content() {
    let env = Env::new("verified");
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/phase0/v1/plan6-list1/05-complete-parent");
    fs::remove_dir_all(env.vdir().join("list1")).unwrap();
    fs::create_dir(env.vdir().join("list1")).unwrap();
    for p in fs::read_dir(src).unwrap().map(|p| p.unwrap().path()) {
        if p.extension().is_some_and(|e| e == "ics") {
            fs::copy(&p, env.vdir().join("list1").join(p.file_name().unwrap())).unwrap();
        }
    }
    fs::write(
        env.dir.path().join("matrix.toml"),
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../compat/matrix.toml")).unwrap(),
    )
    .unwrap();
    let mut cfg = twydo_core::config::Config::with_vdir(env.vdir());
    cfg.timezone = Some("America/New_York".into());
    let snap = twydo_core::views::Snapshot::new(twydo_core::vdir::load(&env.vdir()).unwrap(), cfg);
    let today = chrono::Utc::now().with_timezone(&snap.tz()).date_naive();
    let mut allowed = 0;
    let mut refused = 0;
    for t in snap
        .tasks
        .iter()
        .filter(|t| t.status == twydo_core::model::Status::Completed)
    {
        let permits = twydo_core::gate::check(
            &snap,
            twydo_core::compat::Matrix::embedded(),
            twydo_core::config::WriteMode::Verified,
            &twydo_core::edit::Op::Reopen { uid: t.uid.clone() },
            today,
        )
        .is_ok();
        if permits {
            allowed += 1;
        } else {
            refused += 1;
        }
        let output = env.tdo(&["op", "check", &t.uid]);
        assert!(output.status.success(), "{}", err(&output));
        assert!(out(&output).contains(if permits {
            "reopen: allowed"
        } else {
            "reopen: refused"
        }));
        assert!(!out(&output).contains(&format!("SUMMARY:{}", t.summary)));
    }
    let report = env.tdo(&["check"]);
    assert!(report.status.success(), "{}", err(&report));
    assert!(
        out(&report).contains(&format!("reopen: allowed={allowed} refused={refused}")),
        "{}",
        out(&report)
    );
    assert!(!out(&report).contains("r1"));
}

#[test]
fn capability_move_cli_uses_shipping_matrix_and_requires_destination_probe() {
    let env = Env::new("verified");
    let report = env.tdo(&["op", "check", TASK_A]);
    assert!(report.status.success(), "{}", err(&report));
    assert!(out(&report).contains("move: needs input: destination list"));
    // The existing yearly fixture has no broad shipping move verdict. Its
    // scheduling bytes remain intact under the relocation-specific contract.
    let yearly = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../twydo-core/tests/data/gate/inbox/gate-yearly.ics");
    fs::copy(yearly, env.vdir().join("list1/gate-yearly.ics")).unwrap();
    assert!(op_check_when_seen(&env, "gate-yearly").status.success());
    let relocated = env.tdo(&["op", "move", "gate-yearly", "--to", "list2"]);
    assert!(relocated.status.success(), "{}", err(&relocated));
    assert!(!env.vdir().join("list1/gate-yearly.ics").exists());
    let moved = env.tdo(&["op", "move", TASK_A, "--to", "list2"]);
    assert!(moved.status.success(), "{}", err(&moved));
    assert!(!env.vdir().join(format!("list1/{TASK_A}.ics")).exists());
    let files: Vec<_> = fs::read_dir(env.vdir().join("list2")).unwrap().collect();
    assert_eq!(files.len(), 2);
}

#[test]
fn reparent_cli_reports_context_and_shipping_refusal() {
    let e = Env::new("verified");
    let before = list_files(&e);
    let report = e.tdo(&["op", "check", TASK_A]);
    assert!(report.status.success());
    assert!(out(&report).contains("reparent: needs input: destination list and parent/context"));
    let result = e.tdo(&["op", "reparent", TASK_A, "--list", "list2"]);
    assert_eq!(result.status.code(), Some(1), "{}", err(&result));
    assert!(err(&result).contains("item-outdent"));
    assert_eq!(list_files(&e), before);
    let help = e.tdo(&["op", "reparent", "--help"]);
    assert!(help.status.success());
    assert!(out(&help).contains("--wait"));
    assert!(out(&help).contains("--parent"));
    // Only this isolated test's custom matrix admits the candidate. Shipping stays refused.
    let text = "matrix_version=1\n[[operation]]\ncase=\"item-outdent\"\nop=\"reparent\"\niphone_baseline=\"pass\"\nmac_to_iphone=\"unverified\"\niphone_to_mac=\"unverified\"\ntwydo_to_iphone=\"pass\"\niphone_to_twydo=\"pass\"\n";
    fs::write(e.dir.path().join("matrix.toml"), text).unwrap();
    e.stop_daemon();
    let moved = e.op(&["reparent", TASK_A, "--list", "list2"]);
    assert!(moved.status.success(), "{}", err(&moved));
    assert!(!e.vdir().join(format!("list1/{TASK_A}.ics")).exists());
    let id = out(&moved).trim().to_owned();
    let undone = e.op(&["undo", &id]);
    assert!(undone.status.success(), "{}", err(&undone));
    assert_eq!(list_files(&e), before);
}

#[test]
fn text_capability_reports_and_cli_edit_closed_recurring_checklists() {
    let env = Env::new("verified");
    let list = env.vdir().join("list1");
    fs::remove_dir_all(&list).unwrap();
    fs::create_dir(&list).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/phase0/v1/plan8-checklist-b/011-phone-items-and-text");
    for path in fs::read_dir(source).unwrap().map(|e| e.unwrap().path()) {
        if path.extension().is_some_and(|e| e == "ics") {
            fs::copy(&path, list.join(path.file_name().unwrap())).unwrap();
        }
    }
    let uid = "twydo-p8-c29-b";
    let path = list.join(format!("{uid}.ics"));
    let before = fs::read_to_string(&path).unwrap().replace(
        "STATUS:NEEDS-ACTION",
        "STATUS:COMPLETED\nCOMPLETED:20261210T120000Z",
    );
    fs::write(&path, &before).unwrap();
    for report in [env.tdo(&["check"]), env.tdo(&["op", "check", uid])] {
        assert!(report.status.success(), "{}", err(&report));
        let output = out(&report);
        assert!(
            !output.contains("checklist creation and text editing"),
            "{output}"
        );
        assert!(output.contains("edit title: allowed"), "{output}");
        assert!(output.contains("edit notes: allowed"), "{output}");
        assert!(!output.contains(uid), "{output}");
    }
    let bad = env.tdo(&["op", "edit", uid, "--title", "   "]);
    assert!(!bad.status.success());
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
    let edited = env.tdo(&[
        "op",
        "edit",
        uid,
        "--title",
        "New title",
        "--notes",
        "New notes",
    ]);
    assert!(edited.status.success(), "{}", err(&edited));
    let after = fs::read_to_string(&path).unwrap();
    assert!(after.contains("SUMMARY:New title"));
    assert!(after.contains("DESCRIPTION:New notes"));
    assert!(after.contains("STATUS:COMPLETED"));
    assert!(after.contains("RRULE:FREQ=MONTHLY"));
    let held = |raw: &str| {
        let mut cal = twydo_core::ical::parse(raw.as_bytes()).unwrap();
        let todo = cal.root.children_named_mut("VTODO").next().unwrap();
        for prop in ["SUMMARY", "DESCRIPTION", "DTSTAMP", "LAST-MODIFIED"] {
            todo.remove_props(prop);
        }
        twydo_core::ical::serialize(&cal)
    };
    assert_eq!(held(&before), held(&after));
}

/// Entries of `op journal`, one line each.
fn journal_lines(env: &Env) -> Vec<String> {
    let j = env.op(&["journal"]);
    assert!(j.status.success(), "{}", err(&j));
    out(&j).lines().map(str::to_string).collect()
}

#[test]
fn op_add_resent_after_a_lost_answer_applies_once() {
    let env = Env::new("test");
    env.ensure_daemon();
    let (proxy, submitted) =
        common::lossy_proxy(env.socket(), env.sock_dir.path(), common::Lose::First);
    let o = env
        .cmd()
        .env("TWYDO_SOCKET", &proxy)
        .args(["op", "add", "--list", "list1", "--title", "Synthetic lost"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", err(&o));
    assert!(op_id_ok(&out(&o)), "stdout was {:?}", out(&o));
    let ids = submitted.lock().unwrap().clone();
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert_eq!(ids[0], ids[1], "the resend must reuse the request id");
    let n = fs::read_dir(env.vdir().join("list1"))
        .unwrap()
        .filter_map(|e| fs::read_to_string(e.unwrap().path()).ok())
        .filter(|t| t.contains("SUMMARY:Synthetic lost"))
        .count();
    assert_eq!(n, 1, "the task was created once");
    let journal = journal_lines(&env);
    assert_eq!(
        journal.iter().filter(|l| l.contains("create")).count(),
        1,
        "{journal:?}"
    );
}

#[test]
fn op_add_with_no_answer_reports_an_unknown_outcome() {
    let env = Env::new("test");
    env.ensure_daemon();
    let (proxy, submitted) =
        common::lossy_proxy(env.socket(), env.sock_dir.path(), common::Lose::All);
    let o = env
        .cmd()
        .env("TWYDO_SOCKET", &proxy)
        .env("TDO_RECONNECT_FOR_MS", "2000")
        .args([
            "op",
            "add",
            "--list",
            "list1",
            "--title",
            "Synthetic silent",
        ])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    let e = err(&o);
    assert!(e.contains("may have been applied"), "{e}");
    assert!(e.contains("request "), "{e}");
    assert!(!e.contains("try again"), "{e}");
    assert!(!submitted.lock().unwrap().is_empty());
}

#[test]
fn op_add_lost_before_receipt_then_daemon_restart() {
    let env = Env::new("test");
    env.ensure_daemon();
    let (proxy, submitted, gate) = common::dropping_proxy(env.socket(), env.sock_dir.path());
    let child = env
        .cmd()
        .env("TWYDO_SOCKET", &proxy)
        .env("TDO_RECONNECT_FOR_MS", "20000")
        .args([
            "op",
            "add",
            "--list",
            "list1",
            "--title",
            "Synthetic restart",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while submitted.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "no submit seen");
        std::thread::sleep(Duration::from_millis(20));
    }
    // The first submit never reached the daemon; restart it (a new epoch and session).
    env.stop_daemon();
    *env.daemon.borrow_mut() = None;
    env.ensure_daemon();
    gate.store(true, std::sync::atomic::Ordering::SeqCst);
    let o = child.wait_with_output().unwrap();
    let e = err(&o);
    assert!(!e.contains("unknown session"), "{e}");
    assert!(!e.contains("reconnect"), "{e}");
    let ids = submitted.lock().unwrap().clone();
    assert!(ids.len() >= 2, "{ids:?}");
    assert!(ids.iter().all(|i| *i == ids[0]), "same request id: {ids:?}");
    let n = fs::read_dir(env.vdir().join("list1"))
        .unwrap()
        .filter_map(|e| fs::read_to_string(e.unwrap().path()).ok())
        .filter(|t| t.contains("SUMMARY:Synthetic restart"))
        .count();
    match o.status.code() {
        Some(0) => assert_eq!(n, 1, "applied once"),
        Some(1) => {
            assert_eq!(n, 0, "a refusal creates nothing");
            assert!(e.contains("reload and try again"), "{e}");
        }
        c => panic!("exit {c:?}: {e}"),
    }
}

#[test]
fn op_status_reports_an_applied_request() {
    let env = Env::new("test");
    env.ensure_daemon();
    let (proxy, submitted) =
        common::lossy_proxy(env.socket(), env.sock_dir.path(), common::Lose::First);
    let o = env
        .cmd()
        .env("TWYDO_SOCKET", &proxy)
        .args([
            "op",
            "add",
            "--list",
            "list1",
            "--title",
            "Synthetic status",
        ])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", err(&o));
    let rid = submitted.lock().unwrap()[0].clone();
    let s = env.tdo(&["op", "status", &rid]);
    assert_eq!(s.status.code(), Some(0), "{}", err(&s));
    assert_eq!(out(&s).trim(), format!("applied {}", out(&o).trim()));
}

#[test]
fn op_status_reports_a_queued_request() {
    use twydo_daemon::server::lane::LongKind;
    let env = Env::new("test");
    let d = env.in_process_daemon();
    let lock = d.hold_long(LongKind::Test);
    let (proxy, submitted) =
        common::lossy_proxy(env.socket(), env.sock_dir.path(), common::Lose::First);
    let o = env
        .cmd()
        .env("TWYDO_SOCKET", &proxy)
        .args(["op", "complete", TASK_E])
        .output()
        .unwrap();
    assert!(out(&o).starts_with("queued"), "{}", out(&o));
    let rid = submitted.lock().unwrap()[0].clone();
    let s = env.tdo(&["op", "status", &rid]);
    assert_eq!(s.status.code(), Some(0), "{}", err(&s));
    assert_eq!(out(&s).trim(), "pending");
    drop(lock);
}

#[test]
fn op_status_reports_a_sync_job() {
    use twydo_daemon::client::Client;
    use twydo_daemon::protocol::sync::SyncRunParams;
    let env = Env::new("test");
    let fake = env.dir.path().join("quick.sh");
    common::write_script(&fake, "exit 0");
    let cfg_path = env.dir.path().join("config/twydo/config.toml");
    fs::OpenOptions::new()
        .append(true)
        .open(&cfg_path)
        .and_then(|mut f| {
            use std::io::Write;
            write!(
                f,
                "[sync]\ncommand = \"{}\"\nmetasync = false\n",
                fake.display()
            )
        })
        .unwrap();
    let sdir = twydo_core::store::bind_vdir_state(&env.state().join("twydo"), &env.vdir()).unwrap();
    fs::create_dir_all(sdir.join("sync")).unwrap();
    fs::write(sdir.join("sync/off"), "").unwrap();
    env.ensure_daemon_with(false);
    let hello = twydo_daemon::protocol::methods::HelloParams {
        client: "test".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        protocol: twydo_daemon::PROTOCOL,
        session: None,
        instance: twydo_daemon::paths::instance_for(&cfg_path, None).unwrap(),
    };
    let (raw, _) = Client::connect(&env.socket(), hello).unwrap();
    let deadline = Instant::now() + common::WAIT;
    loop {
        let r: Result<serde_json::Value, _> = raw.call(
            "sync.run",
            SyncRunParams {
                request_id: "t-status-sync".into(),
                force_delete: vec![],
            },
        );
        match r {
            Ok(_) => break,
            Err(e) if e.to_string() == "the daemon is starting" => {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(e) => panic!("sync.run: {e}"),
        }
    }
    let s = env.tdo(&["op", "status", "t-status-sync"]);
    assert_eq!(s.status.code(), Some(0), "{}", err(&s));
    let line = out(&s);
    assert!(
        ["queued", "running", "finished", "failed: ", "interrupted"]
            .iter()
            .any(|p| line.starts_with(p)),
        "{line}"
    );
}

#[test]
fn op_status_of_an_unknown_request_exits_1() {
    let env = Env::new("test");
    let o = env.op(&["status", "no-such-request"]);
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    assert!(
        err(&o).contains(
            "no record of request no-such-request: only task writes and syncs are looked up; it may not have reached the daemon, or may still be applied"
        ),
        "{}",
        err(&o)
    );
    assert!(out(&o).is_empty());
}

/// Runs `args` through a proxy that swallows every answer to `method`; returns stderr.
fn silent_stderr(method: &'static str, args: &[&str]) -> String {
    let env = Env::new("test");
    env.ensure_daemon();
    let (proxy, submitted) =
        common::lossy_proxy_for(env.socket(), env.sock_dir.path(), common::Lose::All, method);
    let o = env
        .cmd()
        .env("TWYDO_SOCKET", &proxy)
        .env("TDO_RECONNECT_FOR_MS", "1500")
        .args(args)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    assert!(!submitted.lock().unwrap().is_empty());
    err(&o)
}

#[test]
fn set_auto_with_no_answer_names_tdo_status() {
    let e = silent_stderr("sync.set_auto", &["sync", "off"]);
    assert!(
        e.contains("automatic sync may already be off (request "),
        "{e}"
    );
    assert!(
        e.contains("check with tdo status; running it again is safe"),
        "{e}"
    );
    assert!(!e.contains("tdo op status"), "{e}");
}

#[test]
fn resolve_with_no_answer_names_tdo_conflicts() {
    let e = silent_stderr(
        "conflicts.resolve",
        &["conflicts", "resolve", "c-1", "--keep", "local"],
    );
    assert!(
        e.contains("the choice may have been recorded (request "),
        "{e}"
    );
    assert!(
        e.contains("check with tdo conflicts; choosing again is safe"),
        "{e}"
    );
    assert!(!e.contains("tdo op status"), "{e}");
}

#[test]
fn abandon_with_no_answer_names_op_journal() {
    let e = silent_stderr("journal.abandon", &["op", "abandon", "op-1"]);
    assert!(e.contains("op-1 may already be abandoned (request "), "{e}");
    assert!(
        e.contains("check with tdo op journal; running it again is safe"),
        "{e}"
    );
    assert!(!e.contains("tdo op status"), "{e}");
}
