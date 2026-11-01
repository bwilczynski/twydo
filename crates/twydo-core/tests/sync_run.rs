//! The sync runner against fake vdirsyncer commands (small `sh` scripts).
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};
use twydo_core::config::{Config, SyncConfig};
use twydo_core::conflicts::{ConflictKind, ConflictStore, Side};
use twydo_core::store::bind_vdir_state;
use twydo_core::store::fsio::VdirLock;
use twydo_core::store::journal::{Entry, Journal, Stage, Write};
use twydo_core::sync::run::{RunOptions, Skip, run};
use twydo_core::sync::status::{CollState, SyncStatus, backoff};

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 1, 10, 0, 0).unwrap()
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/vdirsyncer")
        .join(name)
}

fn task(uid: &str, summary: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\nBEGIN:VTODO\r\n\
         UID:{uid}\r\nSUMMARY:{summary}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
    )
}

/// A vdir with collection `c1`, a state dir, and `[sync]` whose command is a fake script.
struct Fx {
    dir: tempfile::TempDir,
    cfg: Config,
}

impl Fx {
    /// `body` is the fake's shell script; `$1` is `sync` or `metasync`.
    fn new(body: &str) -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let vdir = dir.path().join("vdir");
        fs::create_dir_all(vdir.join("c1")).unwrap();
        let fake = dir.path().join("fake.sh");
        write_script(&fake, body);
        let mut cfg = Config::with_vdir(vdir);
        cfg.state_dir = Some(dir.path().join("state"));
        cfg.sync = Some(SyncConfig {
            command: fake.to_str().unwrap().to_string(),
            metasync: false,
            ..SyncConfig::default()
        });
        Fx { dir, cfg }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn state(&self) -> PathBuf {
        bind_vdir_state(&self.cfg.state_dir(), &self.cfg.vdir).unwrap()
    }

    fn sync_mut(&mut self) -> &mut SyncConfig {
        self.cfg.sync.as_mut().unwrap()
    }
}

/// Writes an executable `sh` script and waits until it can be executed. A script written
/// while another test thread forks can briefly fail with "Text file busy" (ETXTBSY): the
/// forked child holds a copy of the write descriptor until it execs. Every script answers
/// `__probe` by exiting at once, so probing has no side effects.
fn write_script(path: &Path, body: &str) {
    fs::write(
        path,
        format!("#!/bin/sh\n[ \"$1\" = __probe ] && exit 0\n{body}\n"),
    )
    .unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match std::process::Command::new(path).arg("__probe").status() {
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

fn manual() -> RunOptions {
    RunOptions {
        background: false,
        force_delete: Vec::new(),
    }
}

fn background() -> RunOptions {
    RunOptions {
        background: true,
        force_delete: Vec::new(),
    }
}

/// Waits up to 5 s for `p` to exist.
fn wait_for(p: &Path) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if p.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    p.exists()
}

#[test]
fn not_configured_skips() {
    let mut fx = Fx::new("exit 0");
    fx.cfg.sync = None;
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert!(matches!(r.skipped, Some(Skip::NotConfigured)));
    assert!(!fx.path("state").exists(), "a skipped run creates nothing");
}

#[test]
fn clean_run_sets_last_success() {
    let fx = Fx::new(&format!(
        "cat '{}' >&2\nexit 0",
        fixture("clean.txt").display()
    ));
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert!(r.skipped.is_none());
    assert!(r.transient.is_none(), "{:?}", r.transient);
    assert_eq!(r.collections.get("c1"), Some(&CollState::Ok));
    assert_eq!(r.collections.get("Twydo test"), Some(&CollState::Ok));
    let st = SyncStatus::load(&fx.state()).unwrap();
    assert_eq!(st.last_attempt, Some(now()));
    assert_eq!(st.last_success, Some(now()));
    assert_eq!(st.failures, 0);
    assert_eq!(st.next_attempt, None);
    assert_eq!(st.collections, r.collections);
    let json = fs::read_to_string(fx.state().join("sync/status.json")).unwrap();
    assert!(json.contains("\n  \"last_success\""), "pretty JSON: {json}");
}

#[test]
fn vdirsyncer_config_is_passed_as_env() {
    let mut fx = Fx::new("");
    let out = fx.path("env.txt");
    let fake = fx.sync_mut().command.clone();
    write_script(
        Path::new(&fake),
        &format!(
            "printf '%s|%s' \"$VDIRSYNCER_CONFIG\" \"$TWYDO_SYNC_HOOKLOG\" > '{}'",
            out.display()
        ),
    );
    fx.sync_mut().vdirsyncer_config = Some(PathBuf::from("/some/vdirsyncer/config"));
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    let got = fs::read_to_string(&out).unwrap_or_else(|e| panic!("{e}: {r:?}"));
    let hook_log = fx.state().join("sync/hook.log");
    assert_eq!(
        got,
        format!("/some/vdirsyncer/config|{}", hook_log.display())
    );
}

#[test]
fn failure_backs_off_and_background_respects_it() {
    let fx = Fx::new("echo 'error: Connection refused' >&2\nexit 1");
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert_eq!(r.transient.as_deref(), Some("Connection refused"));
    let st = SyncStatus::load(&fx.state()).unwrap();
    assert_eq!(st.failures, 1);
    assert_eq!(st.next_attempt, Some(now() + chrono::Duration::seconds(60)));
    assert_eq!(st.last_error.as_deref(), Some("Connection refused"));
    assert_eq!(st.last_success, None);

    let later = now() + chrono::Duration::seconds(30);
    let r = run(&fx.cfg, &background(), later).unwrap();
    match r.skipped {
        Some(Skip::Backoff { until }) => {
            assert_eq!(until, now() + chrono::Duration::seconds(60))
        }
        other => panic!("expected backoff, got {other:?}"),
    }
    assert_eq!(SyncStatus::load(&fx.state()).unwrap().failures, 1);

    let r = run(&fx.cfg, &manual(), later).unwrap();
    assert!(r.skipped.is_none());
    let st = SyncStatus::load(&fx.state()).unwrap();
    assert_eq!(st.failures, 2);
    assert_eq!(
        st.next_attempt,
        Some(later + chrono::Duration::seconds(120))
    );
}

#[test]
fn backoff_caps_at_max() {
    let max = Duration::from_secs(1800);
    assert_eq!(backoff(0, max), Duration::ZERO);
    assert_eq!(backoff(1, max), Duration::from_secs(60));
    assert_eq!(backoff(2, max), Duration::from_secs(120));
    assert_eq!(backoff(5, max), Duration::from_secs(960));
    assert_eq!(backoff(6, max), max);
    assert_eq!(backoff(64, max), max);
    assert_eq!(backoff(u32::MAX, max), max);
}

#[test]
fn hung_command_is_killed_after_timeout() {
    let mut fx = Fx::new("sleep 30");
    fx.sync_mut().timeout_secs = 1;
    let start = Instant::now();
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "took {:?}",
        start.elapsed()
    );
    assert!(r.transient.is_some());
    assert_eq!(SyncStatus::load(&fx.state()).unwrap().failures, 1);
    let _lock = VdirLock::exclusive(&fx.state()).unwrap();
}

#[test]
fn busy_background_run_requests_rerun() {
    let fx = Fx::new("");
    let count = fx.path("count");
    let fake = fx.cfg.sync.as_ref().unwrap().command.clone();
    write_script(
        Path::new(&fake),
        &format!("echo run >> '{}'", count.display()),
    );
    let sync_dir = fx.state().join("sync");
    fs::create_dir_all(&sync_dir).unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(sync_dir.join("run.lock"))
        .unwrap();
    held.lock().unwrap();

    let r = run(&fx.cfg, &background(), now()).unwrap();
    assert!(matches!(r.skipped, Some(Skip::Busy)));
    assert!(sync_dir.join("requested").exists());
    assert!(!count.exists(), "a busy run does not sync");

    let cfg = fx.cfg.clone();
    let t = std::thread::spawn(move || run(&cfg, &manual(), now()).unwrap());
    std::thread::sleep(Duration::from_millis(100));
    held.unlock().unwrap();
    drop(held);
    let r = t.join().unwrap();
    assert!(r.skipped.is_none());
    let runs = fs::read_to_string(&count).unwrap();
    assert_eq!(runs.lines().count(), 2, "ran {runs:?}");
    assert!(!sync_dir.join("requested").exists());
}

#[test]
fn stuck_journal_entry_becomes_conflict() {
    let fx = Fx::new("exit 0");
    let file = fx.cfg.vdir.join("c1/a.ics");
    let third = task("u1", "third");
    fs::write(&file, &third).unwrap();
    let before = task("u1", "before");
    let after = task("u1", "after");
    let state = fx.state();
    let journal = Journal::open(&state).unwrap();
    journal
        .prepare(&Entry {
            format: 1,
            id: "20261101T090000Z-0000abcd".into(),
            created: now(),
            kind: "edit".into(),
            uids: vec!["u1".into()],
            stage: Stage::Prepared,
            inverse_of: None,
            synced: None,
            writes: vec![Write {
                path: "c1/a.ics".into(),
                before: Some(before),
                after: Some(after.clone()),
            }],
        })
        .unwrap();

    let r = run(&fx.cfg, &manual(), now()).unwrap();

    assert_eq!(fs::read_to_string(&file).unwrap(), third, "vdir untouched");
    let e = journal.get("20261101T090000Z-0000abcd").unwrap().unwrap();
    assert_eq!(e.stage, Stage::Abandoned);
    let store = ConflictStore::open(&state).unwrap();
    let open = store.unresolved().unwrap();
    assert_eq!(open.len(), 1);
    let c = &open[0];
    assert_eq!(c.kind, ConflictKind::Journal);
    assert!(!c.held);
    assert_eq!(c.vdir_holds, Side::Remote);
    assert_eq!(c.uid, "u1");
    assert_eq!(c.collection, "c1");
    assert_eq!(c.path, "c1/a.ics");
    assert_eq!(c.local.as_deref(), Some(after.as_str()));
    assert_eq!(c.remote.as_deref(), Some(third.as_str()));
    assert_eq!(r.new_conflicts, vec![c.id.clone()]);
}

#[test]
fn writers_wait_for_sync() {
    let fx = Fx::new("");
    let started = fx.path("started");
    let done = fx.path("done");
    let fake = fx.cfg.sync.as_ref().unwrap().command.clone();
    write_script(
        Path::new(&fake),
        &format!(
            "touch '{}'\nsleep 1\ntouch '{}'",
            started.display(),
            done.display()
        ),
    );
    let state = fx.state();
    let cfg = fx.cfg.clone();
    let t = std::thread::spawn(move || run(&cfg, &manual(), now()).unwrap());
    assert!(wait_for(&started), "fake never started");

    let shared = VdirLock::shared(&state).unwrap();
    assert!(!done.exists(), "a reader does not wait for vdirsyncer");
    drop(shared);

    let excl = VdirLock::exclusive(&state).unwrap();
    assert!(done.exists(), "a writer waits until vdirsyncer is done");
    drop(excl);
    t.join().unwrap();
}
