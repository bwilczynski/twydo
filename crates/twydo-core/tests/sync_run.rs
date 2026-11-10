//! The sync runner against fake vdirsyncer commands (small `sh` scripts).
#![cfg(unix)]

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};
use twydo_core::config::WriteMode;
use twydo_core::config::{Config, SyncConfig};
use twydo_core::conflicts::{ConflictKind, ConflictStore, Side};
use twydo_core::edit::{Change, Op};
use twydo_core::store::bind_vdir_state;
use twydo_core::store::fsio::VdirLock;
use twydo_core::store::journal::{Entry, Journal, Stage, Write};
use twydo_core::store::queue::Queue;
use twydo_core::store::{DrainOutcome, Store, Submitted};
use twydo_core::sync::ended;
use twydo_core::sync::manifest::Manifest;
use twydo_core::sync::output::parse;
use twydo_core::sync::reconcile::reconcile;
use twydo_core::sync::run::{RunOptions, Skip, run};
use twydo_core::sync::running::{
    Reaped, Running, StopResult, clear_stop, process_start, reap_orphan, request_stop, running,
};
use twydo_core::sync::status::{CollState, SyncStatus, backoff, coll_states};
use twydo_core::views::Snapshot;

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
        ..RunOptions::default()
    }
}

#[test]
fn failed_rediscovery_blocks_sync_and_is_retried_before_sync() {
    let fx = Fx::new(
        r#"
        dir=$(dirname "$TWYDO_SYNC_HOOKLOG")
        echo "$1" >> "$dir/calls"
        if [ "$1" = discover ] && [ ! -f "$dir/allow" ]; then
            echo 'error: discovery unavailable' >&2
            exit 1
        fi
        [ "$1" = sync ] && echo 'Syncing twydo/c1' >&2
        exit 0
    "#,
    );
    let state = fx.state();
    fs::create_dir_all(state.join("sync")).unwrap();
    fs::write(state.join("sync/rediscover"), "backup-id").unwrap();
    let first = run(&fx.cfg, &manual(), now()).unwrap();
    assert!(first.transient.as_deref().unwrap().contains("discovery"));
    assert!(
        first.transient.as_deref().unwrap().contains(":forget-list"),
        "{:?}",
        first.transient
    );
    assert!(
        first
            .transient
            .as_deref()
            .unwrap()
            .contains("discovery unavailable"),
        "{:?}",
        first.transient
    );
    assert_eq!(
        SyncStatus::load(&state).unwrap().last_error,
        first.transient
    );
    assert_eq!(
        fs::read_to_string(state.join("sync/calls")).unwrap(),
        "discover\n"
    );
    assert!(state.join("sync/rediscover").exists());
    fs::write(state.join("sync/allow"), "").unwrap();
    let second = run(&fx.cfg, &manual(), now()).unwrap();
    assert!(second.transient.is_none(), "{:?}", second.transient);
    assert!(!state.join("sync/rediscover").exists());
    assert_eq!(
        fs::read_to_string(state.join("sync/calls")).unwrap(),
        "discover\ndiscover\nsync\n"
    );
}

fn background() -> RunOptions {
    RunOptions {
        background: true,
        ..RunOptions::default()
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
            "printf '%s|%s|%s' \"$VDIRSYNCER_CONFIG\" \"$TWYDO_SYNC_HOOKLOG\" \"$TWYDO_SYNC_VDIR\" > '{}'",
            out.display()
        ),
    );
    fx.sync_mut().vdirsyncer_config = Some(PathBuf::from("/some/vdirsyncer/config"));
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    let got = fs::read_to_string(&out).unwrap_or_else(|e| panic!("{e}: {r:?}"));
    let hook_log = fx.state().join("sync/hook.log");
    assert_eq!(
        got,
        format!(
            "/some/vdirsyncer/config|{}|{}",
            hook_log.display(),
            fx.cfg.vdir.display()
        )
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
    // A wrapper that does not `exec`: the hung grandchild must die with it.
    let mut fx = Fx::new("");
    let pidfile = fx.path("grandchild.pid");
    let fake = fx.sync_mut().command.clone();
    write_script(
        Path::new(&fake),
        &format!("sleep 30 &\necho $! > '{}'\nwait", pidfile.display()),
    );
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
    let pid = fs::read_to_string(&pidfile).unwrap();
    let pid = pid.trim();
    let alive = || {
        std::process::Command::new("kill")
            .args(["-0", pid])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    // Killed at once; allow a moment for init to reap the orphan.
    let deadline = Instant::now() + Duration::from_secs(3);
    while alive() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!alive(), "grandchild {pid} survived the timeout");
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
            from_queue: None,
            request: None,
            synced: None,
            holds: Vec::new(),
            family: None,
            families: Vec::new(),
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

/// An Applied journal entry with `writes` as (path, before, after).
fn applied(id: &str, writes: &[(&str, Option<&str>, Option<&str>)]) -> Entry {
    Entry {
        format: 1,
        id: id.into(),
        created: now(),
        kind: "edit".into(),
        uids: vec!["u1".into()],
        stage: Stage::Applied,
        inverse_of: None,
        from_queue: None,
        request: None,
        synced: None,
        holds: Vec::new(),
        family: None,
        families: Vec::new(),
        writes: writes
            .iter()
            .map(|(p, b, a)| Write {
                path: (*p).into(),
                before: b.map(str::to_string),
                after: a.map(str::to_string),
            })
            .collect(),
    }
}

/// Writes `entry` to the journal and its `after` contents to the vdir.
fn apply(fx: &Fx, entry: &Entry) {
    Journal::open(&fx.state()).unwrap().prepare(entry).unwrap();
    for w in &entry.writes {
        let f = fx.cfg.vdir.join(&w.path);
        match &w.after {
            Some(a) => fs::write(&f, a).unwrap(),
            None => {
                let _ = fs::remove_file(&f);
            }
        }
    }
}

fn synced(fx: &Fx, id: &str) -> bool {
    let e = Journal::open(&fx.state())
        .unwrap()
        .get(id)
        .unwrap()
        .unwrap();
    e.synced.is_some()
}

fn cat_fixture(name: &str) -> String {
    format!("cat '{}' >&2", fixture(name).display())
}

#[test]
fn clean_run_confirms_edit() {
    let fx = Fx::new(&format!("{}\nexit 0", cat_fixture("clean.txt")));
    let (b, a) = (task("u1", "before"), task("u1", "after"));
    let e = applied(
        "20261101T090000Z-00000001",
        &[("c1/u1.ics", Some(&b), Some(&a))],
    );
    apply(&fx, &e);
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert_eq!(r.confirmed, vec![e.id.clone()]);
    assert!(synced(&fx, &e.id));
}

#[test]
fn unattributed_error_confirms_nothing() {
    let fx = Fx::new(&format!(
        "{}\necho 'error: Connection refused' >&2\nexit 1",
        cat_fixture("clean.txt")
    ));
    let (b, a) = (task("u1", "before"), task("u1", "after"));
    let e = applied(
        "20261101T090000Z-00000001",
        &[("c1/u1.ics", Some(&b), Some(&a))],
    );
    apply(&fx, &e);
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert!(r.transient.is_some());
    assert!(r.confirmed.is_empty(), "{r:?}");
    assert!(!synced(&fx, &e.id));
}

#[test]
fn errored_collection_stays_pending_then_confirms() {
    let mut fx =
        Fx::new("printf 'Syncing twydo/c1\\nerror: twydo/c1: Connection reset\\n' >&2\nexit 1");
    // A repeating completion: a new history copy and the advanced live task.
    let (live0, live1) = (task("u1", "due 1"), task("u1", "due 2"));
    let copy = task("u1-h", "done 1");
    let e = applied(
        "20261101T090000Z-00000001",
        &[
            ("c1/u1-h.ics", None, Some(&copy)),
            ("c1/u1.ics", Some(&live0), Some(&live1)),
        ],
    );
    apply(&fx, &e);
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert!(matches!(
        r.collections.get("c1"),
        Some(CollState::Error { .. })
    ));
    assert!(r.confirmed.is_empty(), "{r:?}");
    assert!(!synced(&fx, &e.id));

    let fake = fx.sync_mut().command.clone();
    write_script(
        Path::new(&fake),
        &format!("{}\nexit 0", cat_fixture("clean.txt")),
    );
    let later = now() + chrono::Duration::seconds(60);
    let r = run(&fx.cfg, &manual(), later).unwrap();
    assert_eq!(r.confirmed, vec![e.id.clone()]);
    assert!(synced(&fx, &e.id));
    let mut files: Vec<String> = fs::read_dir(fx.cfg.vdir.join("c1"))
        .unwrap()
        .map(|d| d.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    assert_eq!(files, ["u1-h.ics", "u1.ics"], "no second history copy");
    assert!(
        ConflictStore::open(&fx.state())
            .unwrap()
            .list()
            .unwrap()
            .is_empty()
    );
}

/// Calls `reconcile` directly as `run` would, with vdirsyncer's stderr `stderr`.
fn reconcile_with(
    fx: &Fx,
    pending: &[Entry],
    stderr: &str,
) -> twydo_core::sync::reconcile::Reconciled {
    let state = fx.state();
    let sync = fx.cfg.sync.as_ref().unwrap();
    let out = parse(&sync.pair, stderr);
    let states = coll_states(&out, &[], &sync.local_storage);
    let _lock = VdirLock::exclusive(&state).unwrap();
    reconcile(
        &fx.cfg.vdir,
        &Journal::open(&state).unwrap(),
        &ConflictStore::open(&state).unwrap(),
        sync,
        &states,
        pending,
        &out,
        &[],
        now(),
    )
    .unwrap()
}

#[test]
fn write_between_locks_stays_pending() {
    let fx = Fx::new("exit 0");
    let (v0, v1, v2) = (task("u1", "0"), task("u1", "1"), task("u1", "2"));
    let first = applied(
        "20261101T090000Z-00000001",
        &[("c1/u1.ics", Some(&v0), Some(&v1))],
    );
    apply(&fx, &first);
    // vdirsyncer ran on `v1`; then a writer slipped in before the exclusive lock.
    let newer = applied(
        "20261101T090001Z-00000002",
        &[("c1/u1.ics", Some(&v1), Some(&v2))],
    );
    apply(&fx, &newer);
    let r = reconcile_with(&fx, std::slice::from_ref(&first), "Syncing twydo/c1\n");
    assert!(r.confirmed.is_empty(), "{:?}", r.confirmed);
    assert!(r.new_conflicts.is_empty());
    assert!(!synced(&fx, &first.id));
    assert!(!synced(&fx, &newer.id));
}

#[test]
fn line_ending_only_difference_is_confirmed() {
    let fx = Fx::new("exit 0");
    let (b, a) = (task("u1", "before"), task("u1", "after"));
    let e = applied(
        "20261101T090000Z-00000001",
        &[("c1/u1.ics", Some(&b), Some(&a))],
    );
    apply(&fx, &e);
    fs::write(fx.cfg.vdir.join("c1/u1.ics"), a.replace("\r\n", "\n")).unwrap();
    let r = reconcile_with(&fx, std::slice::from_ref(&e), "Syncing twydo/c1\n");
    assert_eq!(r.confirmed, vec![e.id.clone()]);
    assert!(r.new_conflicts.is_empty());
}

#[test]
fn server_change_seen_late_is_confirmed_with_a_warning() {
    // The edit reached the server in a run Twydo did not see and the server changed it
    // since, downloaded in that run too: nothing will ever confirm it, so confirm it now.
    let fx = Fx::new("exit 0");
    let (b, a) = (task("u1", "before"), task("u1", "after"));
    let e = applied(
        "20261101T090000Z-00000001",
        &[("c1/u1.ics", Some(&b), Some(&a))],
    );
    apply(&fx, &e);
    fs::write(fx.cfg.vdir.join("c1/u1.ics"), task("u1", "other")).unwrap();
    let r = reconcile_with(&fx, std::slice::from_ref(&e), "Syncing twydo/c1\n");
    assert_eq!(r.confirmed, vec![e.id.clone()]);
    assert!(r.new_conflicts.is_empty());
    assert!(r.unexpected.is_empty(), "{:?}", r.unexpected);
    assert_eq!(r.superseded, vec!["c1/u1.ics".to_string()]);
    assert!(synced(&fx, &e.id));
    // Synced now, so the next run says nothing.
    let r = reconcile_with(&fx, &[], "Syncing twydo/c1\n");
    assert!(r.superseded.is_empty() && r.unexpected.is_empty(), "{r:?}");
}

#[test]
fn file_still_holding_before_stays_pending() {
    let fx = Fx::new("exit 0");
    let (b, a) = (task("u1", "before"), task("u1", "after"));
    let e = applied(
        "20261101T090000Z-00000001",
        &[("c1/u1.ics", Some(&b), Some(&a))],
    );
    apply(&fx, &e);
    fs::write(fx.cfg.vdir.join("c1/u1.ics"), &b).unwrap();
    let r = reconcile_with(&fx, std::slice::from_ref(&e), "Syncing twydo/c1\n");
    assert!(r.confirmed.is_empty());
    assert!(r.new_conflicts.is_empty());
    assert!(r.superseded.is_empty());
    assert_eq!(r.unexpected, vec!["c1/u1.ics".to_string()]);
    assert!(!synced(&fx, &e.id));
}

#[test]
fn unexpected_difference_with_open_conflict_stays_pending() {
    let fx = Fx::new("exit 0");
    let (b, a) = (task("u1", "before"), task("u1", "after"));
    let e = applied(
        "20261101T090000Z-00000001",
        &[("c1/u1.ics", Some(&b), Some(&a))],
    );
    apply(&fx, &e);
    let other = task("u1", "other");
    fs::write(fx.cfg.vdir.join("c1/u1.ics"), &other).unwrap();
    ConflictStore::open(&fx.state())
        .unwrap()
        .record(
            twydo_core::conflicts::NewConflict {
                uid: "u1".into(),
                collection: "c1".into(),
                path: "c1/u1.ics".into(),
                kind: ConflictKind::Journal,
                held: false,
                vdir_holds: Side::Remote,
                local: Some(a.clone()),
                remote: Some(other),
            },
            now(),
        )
        .unwrap();
    let r = reconcile_with(&fx, std::slice::from_ref(&e), "Syncing twydo/c1\n");
    assert!(r.confirmed.is_empty(), "{r:?}");
    assert!(r.superseded.is_empty());
    assert_eq!(r.unexpected, vec!["c1/u1.ics".to_string()]);
}

#[test]
fn unexpected_difference_in_errored_collection_stays_pending() {
    let fx = Fx::new("exit 0");
    let (b, a) = (task("u1", "before"), task("u1", "after"));
    let e = applied(
        "20261101T090000Z-00000001",
        &[("c1/u1.ics", Some(&b), Some(&a))],
    );
    apply(&fx, &e);
    fs::write(fx.cfg.vdir.join("c1/u1.ics"), task("u1", "other")).unwrap();
    let r = reconcile_with(
        &fx,
        std::slice::from_ref(&e),
        "Syncing twydo/c1\nerror: twydo/c1: Connection reset\n",
    );
    assert!(r.confirmed.is_empty(), "{r:?}");
    assert!(r.superseded.is_empty());
    assert!(!synced(&fx, &e.id));
}

#[test]
fn remote_update_after_earlier_upload_confirms() {
    // The edit reached the server in a run Twydo did not see (vdirsyncer by hand); the
    // server changed it since, and this run brought the newer version down without a
    // conflict.
    let fx = Fx::new("exit 0");
    let (b, a) = (task("u1", "before"), task("u1", "after"));
    let e = applied(
        "20261101T090000Z-00000001",
        &[("c1/u1.ics", Some(&b), Some(&a))],
    );
    apply(&fx, &e);
    fs::write(fx.cfg.vdir.join("c1/u1.ics"), task("u1", "server")).unwrap();
    let r = reconcile_with(
        &fx,
        std::slice::from_ref(&e),
        "Syncing twydo/c1\nCopying (updating) item u1 to twydo_local/c1\n",
    );
    assert_eq!(r.confirmed, vec![e.id.clone()]);
    assert!(r.new_conflicts.is_empty());
}

#[test]
fn deletion_with_unfound_restore_stays_pending() {
    let fx = Fx::new("exit 0");
    let b = task("u1", "before");
    let e = applied(
        "20261101T090000Z-00000001",
        &[("c1/u1.ics", Some(&b), None)],
    );
    apply(&fx, &e);
    // vdirsyncer says it restored u1 locally, but no single file holds it.
    fs::write(fx.cfg.vdir.join("c1/x.ics"), task("u1", "server")).unwrap();
    fs::write(fx.cfg.vdir.join("c1/y.ics"), task("u1", "server")).unwrap();
    let r = reconcile_with(
        &fx,
        std::slice::from_ref(&e),
        "Syncing twydo/c1\nCopying (uploading) item u1 to twydo_local/c1\n",
    );
    assert!(r.confirmed.is_empty(), "{:?}", r.confirmed);
    assert!(r.new_conflicts.is_empty());
    assert_eq!(r.unexpected, vec!["c1/u1.ics".to_string()]);
    assert!(!synced(&fx, &e.id));
}

#[test]
fn delete_conflict_of_pending_entry_is_recorded_once() {
    let fx = Fx::new("exit 0");
    fs::create_dir_all(fx.cfg.vdir.join("c2")).unwrap();
    let (b, x0, x1) = (task("u1", "before"), task("u2", "0"), task("u2", "1"));
    // One path deleted in c1 (the server edited it), one edit in c2, which keeps failing.
    let e = applied(
        "20261101T090000Z-00000001",
        &[
            ("c1/u1.ics", Some(&b), None),
            ("c2/u2.ics", Some(&x0), Some(&x1)),
        ],
    );
    apply(&fx, &e);
    let remote = task("u1", "server");
    fs::write(fx.cfg.vdir.join("c1/u1.ics"), &remote).unwrap();
    let stderr = "Syncing twydo/c1\nCopying (uploading) item u1 to twydo_local/c1\n\
                  Syncing twydo/c2\nerror: twydo/c2: Connection reset\n";
    let r = reconcile_with(&fx, std::slice::from_ref(&e), stderr);
    assert_eq!(r.new_conflicts.len(), 1);
    assert!(!synced(&fx, &e.id), "c2 is not confirmed");
    let store = ConflictStore::open(&fx.state()).unwrap();

    let r = reconcile_with(
        &fx,
        std::slice::from_ref(&e),
        "Syncing twydo/c1\nSyncing twydo/c2\nerror: twydo/c2: Connection reset\n",
    );
    assert!(r.new_conflicts.is_empty(), "{:?}", r.new_conflicts);
    assert_eq!(store.list().unwrap().len(), 1);

    let id = store.list().unwrap()[0].id.clone();
    store.mark_resolved(&id, now()).unwrap();
    let r = reconcile_with(
        &fx,
        std::slice::from_ref(&e),
        "Syncing twydo/c1\nSyncing twydo/c2\nerror: twydo/c2: Connection reset\n",
    );
    assert!(r.new_conflicts.is_empty(), "{:?}", r.new_conflicts);
    let all = store.list().unwrap();
    assert_eq!(all.len(), 1, "{all:?}");
    assert!(all[0].resolved.is_some());
}

/// Runs a pending edit of `c1/u1.ics` through a run whose fake prints `first` (vdirsyncer
/// re-uploading the edit to a server that deleted it, in a run that does not count), then
/// through a clean run.
fn remote_delete_in_uncounted_run(first: &str, timeout: Option<u64>) {
    let mut fx = Fx::new(first);
    if let Some(t) = timeout {
        fx.sync_mut().timeout_secs = t;
    }
    let (b, a) = (task("u1", "before"), task("u1", "after"));
    let e = applied(
        "20261101T090000Z-00000001",
        &[("c1/u1.ics", Some(&b), Some(&a))],
    );
    apply(&fx, &e);
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert!(r.transient.is_some(), "{r:?}");
    assert_eq!(r.new_conflicts.len(), 1, "{r:?}");
    assert!(!synced(&fx, &e.id), "an uncounted run confirms nothing");
    let store = ConflictStore::open(&fx.state()).unwrap();
    let open = store.unresolved().unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].kind, ConflictKind::RemoteDeleteLocalEdit);
    assert_eq!(open[0].local.as_deref(), Some(a.as_str()));
    assert_eq!(open[0].remote, None);

    let fake = fx.sync_mut().command.clone();
    write_script(Path::new(&fake), "printf 'Syncing twydo/c1\\n' >&2\nexit 0");
    let later = now() + chrono::Duration::seconds(60);
    let r = run(&fx.cfg, &manual(), later).unwrap();
    assert!(r.transient.is_none(), "{r:?}");
    assert_eq!(r.confirmed, vec![e.id.clone()]);
    assert!(r.new_conflicts.is_empty(), "{r:?}");
    assert_eq!(store.unresolved().unwrap().len(), 1, "the conflict stays");
}

#[test]
fn remote_delete_in_errored_collection_is_recorded() {
    remote_delete_in_uncounted_run(
        "printf 'Syncing twydo/c1\\nCopying (uploading) item u1 to twydo_remote/c1\\n\
         error: twydo/c1: Connection reset\\n' >&2\nexit 1",
        None,
    );
}

#[test]
fn remote_delete_in_timed_out_run_is_recorded() {
    remote_delete_in_uncounted_run(
        "printf 'Syncing twydo/c1\\nCopying (uploading) item u1 to twydo_remote/c1\\n' >&2\n\
         sleep 30",
        Some(1),
    );
}

#[test]
fn remote_delete_with_unattributed_error_is_recorded() {
    remote_delete_in_uncounted_run(
        "printf 'Syncing twydo/c1\\nCopying (uploading) item u1 to twydo_remote/c1\\n\
         error: Connection refused\\n' >&2\nexit 1",
        None,
    );
}

/// Records a held both-changed conflict for `c1/u1.ics`.
fn held_conflict(fx: &Fx) -> String {
    ConflictStore::open(&fx.state())
        .unwrap()
        .record(
            twydo_core::conflicts::NewConflict {
                uid: "u1".into(),
                collection: "c1".into(),
                path: "c1/u1.ics".into(),
                kind: ConflictKind::BothChanged,
                held: true,
                vdir_holds: Side::Local,
                local: Some(task("u1", "here")),
                remote: Some(task("u1", "there")),
            },
            now(),
        )
        .unwrap()
        .id
}

#[test]
fn held_conflict_closes_when_both_sides_deleted() {
    let fx = Fx::new("exit 0");
    let id = held_conflict(&fx);
    let r = reconcile_with(&fx, &[], "Syncing twydo/c1\n");
    assert_eq!(r.resolved, vec![id]);
    assert!(
        ConflictStore::open(&fx.state())
            .unwrap()
            .unresolved()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn held_conflict_stays_when_its_task_moved_to_another_file() {
    let fx = Fx::new("exit 0");
    held_conflict(&fx);
    fs::write(fx.cfg.vdir.join("c1/other.ics"), task("u1", "third")).unwrap();
    let r = reconcile_with(&fx, &[], "Syncing twydo/c1\n");
    assert!(r.resolved.is_empty(), "{r:?}");
}

#[test]
fn stuck_entry_records_its_unapplied_writes_too() {
    let fx = Fx::new("exit 0");
    let (a_file, b_file) = (fx.cfg.vdir.join("c1/a.ics"), fx.cfg.vdir.join("c1/b.ics"));
    let third = task("u1", "third");
    fs::write(&a_file, &third).unwrap();
    // The second write was never applied: its file still holds `before`.
    let b0 = task("u2", "before");
    fs::write(&b_file, &b0).unwrap();
    let (a0, a1, b1) = (
        task("u1", "before"),
        task("u1", "after"),
        task("u2", "after"),
    );
    let state = fx.state();
    let journal = Journal::open(&state).unwrap();
    journal
        .prepare(&Entry {
            format: 1,
            id: "20261101T090000Z-0000abcd".into(),
            created: now(),
            kind: "edit".into(),
            uids: vec!["u1".into(), "u2".into()],
            stage: Stage::Prepared,
            inverse_of: None,
            from_queue: None,
            request: None,
            synced: None,
            holds: Vec::new(),
            family: None,
            families: Vec::new(),
            writes: vec![
                Write {
                    path: "c1/a.ics".into(),
                    before: Some(a0),
                    after: Some(a1),
                },
                Write {
                    path: "c1/b.ics".into(),
                    before: Some(b0.clone()),
                    after: Some(b1.clone()),
                },
            ],
        })
        .unwrap();

    let r = run(&fx.cfg, &manual(), now()).unwrap();

    assert_eq!(fs::read_to_string(&b_file).unwrap(), b0, "vdir untouched");
    assert_eq!(r.new_conflicts.len(), 2, "{r:?}");
    let open = ConflictStore::open(&state).unwrap().unresolved().unwrap();
    let c = open
        .iter()
        .find(|c| c.path == "c1/b.ics")
        .expect("b.ics recorded");
    assert_eq!(c.kind, ConflictKind::Journal);
    assert_eq!(c.uid, "u2");
    assert_eq!(c.vdir_holds, Side::Remote);
    assert_eq!(c.local.as_deref(), Some(b1.as_str()));
    assert_eq!(c.remote.as_deref(), Some(b0.as_str()));
}

#[test]
fn vdirsyncer_keeps_the_shared_lock_after_the_runner_lets_go() {
    // The fake leaves a process behind with its stdin (as an orphaned vdirsyncer would be
    // after a Ctrl-C killed tdo): it holds the shared vdir lock until it exits, so the
    // reconciliation's exclusive lock, and any writer, waits for it.
    let fx = Fx::new("");
    let done = fx.path("done");
    let fake = fx.cfg.sync.as_ref().unwrap().command.clone();
    write_script(
        Path::new(&fake),
        &format!(
            "exec 3<&0\n(sleep 1; touch '{}') <&3 >/dev/null 2>&1 &\nexit 0",
            done.display()
        ),
    );
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert!(r.transient.is_none(), "{r:?}");
    assert!(done.exists(), "the run waited for what kept the lock");
    let _excl = VdirLock::exclusive(&fx.state()).unwrap();
}

#[test]
fn upload_of_created_then_edited_task_is_not_a_conflict() {
    // Pins a documented limit (docs/sync.md, "A create edited before it was confirmed"): a
    // pending create makes every upload of the path look like the create's own, even when
    // the create reached the server in an earlier run that did not count and the phone has
    // deleted it since.
    let fx = Fx::new("exit 0");
    let (v1, v2) = (task("u1", "new"), task("u1", "edited"));
    let create = applied(
        "20261101T090000Z-00000001",
        &[("c1/u1.ics", None, Some(&v1))],
    );
    let mut edit = applied(
        "20261101T090001Z-00000002",
        &[("c1/u1.ics", Some(&v1), Some(&v2))],
    );
    edit.created = now() + chrono::Duration::seconds(1);
    apply(&fx, &create);
    apply(&fx, &edit);
    let r = reconcile_with(
        &fx,
        &[create.clone(), edit.clone()],
        "Syncing twydo/c1\nCopying (uploading) item u1 to twydo_remote/c1\n",
    );
    assert!(r.new_conflicts.is_empty(), "{r:?}");
    assert_eq!(r.confirmed, vec![create.id.clone(), edit.id.clone()]);
}

#[test]
fn metasync_error_is_the_first_error_line_not_the_first_line() {
    let mut fx = Fx::new(
        "if [ \"$1\" = metasync ]; then\n\
         echo 'Metasyncing twydo/c1' >&2\n\
         echo 'Metasyncing twydo/c2' >&2\n\
         echo 'error: twydo/c2: Metasync failed' >&2\n\
         echo 'error: Use `-vdebug` to see the full traceback.' >&2\n\
         exit 1\n\
         fi\n\
         echo 'Syncing twydo/c1' >&2\nexit 0",
    );
    fx.sync_mut().metasync = true;
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert_eq!(
        r.metasync_error.as_deref(),
        Some("twydo/c2: Metasync failed")
    );
    let st = SyncStatus::load(&fx.state()).unwrap();
    assert_eq!(
        st.metasync_error.as_deref(),
        Some("twydo/c2: Metasync failed")
    );
}

#[test]
fn metasync_failure_without_error_line_reports_first_nonempty_line() {
    let mut fx = Fx::new(
        "if [ \"$1\" = metasync ]; then\n\
         echo >&2\necho 'Metasyncing twydo/c1' >&2\nexit 1\nfi\n\
         exit 0",
    );
    fx.sync_mut().metasync = true;
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert_eq!(r.metasync_error.as_deref(), Some("Metasyncing twydo/c1"));
}

#[test]
fn critical_line_counts_as_metasync_error() {
    let mut fx = Fx::new(
        "if [ \"$1\" = metasync ]; then\n\
         echo 'Metasyncing twydo/c1' >&2\n\
         echo 'critical: Detected change in config file' >&2\nexit 1\nfi\n\
         exit 0",
    );
    fx.sync_mut().metasync = true;
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert_eq!(
        r.metasync_error.as_deref(),
        Some("Detected change in config file")
    );
}

#[test]
fn last_error_ignores_syncing_progress_lines() {
    let fx = Fx::new(
        "echo 'Syncing twydo/c1' >&2\necho 'Syncing twydo/c2' >&2\n\
         echo 'error: twydo/c2: boom' >&2\nexit 1",
    );
    run(&fx.cfg, &manual(), now()).unwrap();
    let st = SyncStatus::load(&fx.state()).unwrap();
    assert_eq!(st.last_error.as_deref(), Some("c2: boom"));
}

/// A run that reports a conflict for u2 in c1 and is then killed before it uploads the
/// pending edit of u1 in the same collection confirms nothing there.
#[test]
fn timed_out_run_confirms_nothing_in_conflicted_collection() {
    let mut fx = Fx::new(
        "printf 'Syncing twydo/c1\\n\
         error: twydo/c1: One item changed on both sides. Resolve this conflict manually, \
         or by setting the `conflict_resolution` parameter in your config file.\\n\
         error: Item ID: u2\\nerror: Item href on side A: u2.ics\\n\
         error: Item href on side B: u2.ics\\n' >&2\n\
         sleep 30",
    );
    fx.sync_mut().timeout_secs = 1;
    let (b, a) = (task("u1", "before"), task("u1", "after"));
    let e = applied(
        "20261101T090000Z-00000001",
        &[("c1/u1.ics", Some(&b), Some(&a))],
    );
    apply(&fx, &e);
    fs::write(fx.cfg.vdir.join("c1/u2.ics"), task("u2", "here")).unwrap();
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert!(r.transient.is_some(), "{r:?}");
    assert!(
        matches!(r.collections.get("c1"), Some(CollState::Error { .. })),
        "{r:?}"
    );
    assert!(r.confirmed.is_empty(), "{r:?}");
    assert!(!synced(&fx, &e.id), "a timed-out run confirms nothing");
}

/// A manifest that can be neither read nor written (here: a directory) only costs warnings:
/// the run still reconciles and saves its status.
#[test]
fn unusable_manifest_does_not_stop_the_run() {
    let fx = Fx::new(&format!("{}\nexit 0", cat_fixture("clean.txt")));
    let (b, a) = (task("u1", "before"), task("u1", "after"));
    let e = applied(
        "20261101T090000Z-00000001",
        &[("c1/u1.ics", Some(&b), Some(&a))],
    );
    apply(&fx, &e);
    fs::create_dir_all(fx.state().join("sync/manifest.json")).unwrap();
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert_eq!(r.confirmed, vec![e.id.clone()]);
    assert!(r.transient.is_none(), "{r:?}");
    assert_eq!(r.warnings.len(), 2, "{:?}", r.warnings);
    assert!(
        r.warnings[0].starts_with("sync/manifest.json is unreadable ("),
        "{:?}",
        r.warnings
    );
    assert!(
        r.warnings[1].starts_with("sync/manifest.json could not be written ("),
        "{:?}",
        r.warnings
    );
    let st = SyncStatus::load(&fx.state()).unwrap();
    assert_eq!(st.last_success, Some(now()));
}

/// Seeds the manifest with entries for c1, c2 and c3, runs `fake`, and returns the
/// collections left in the manifest.
fn manifest_after(fake: &str, timeout: Option<u64>) -> Vec<String> {
    let mut fx = Fx::new(fake);
    if let Some(t) = timeout {
        fx.sync_mut().timeout_secs = t;
    }
    let mut m = Manifest::default();
    for c in ["c1", "c2", "c3"] {
        m.0.insert(c.into(), vec![format!("{c}/a.ics")]);
    }
    m.save(&fx.state()).unwrap();
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert!(r.transient.is_some(), "{r:?}");
    assert_eq!(
        r.collections.get("c1"),
        Some(&CollState::EmptiedLocal),
        "{r:?}"
    );
    Manifest::load(&fx.state()).unwrap().0.into_keys().collect()
}

#[test]
fn timed_out_run_drops_evidence_but_keeps_emptied_local() {
    let kept = manifest_after(
        &format!(
            "{}\nprintf 'Syncing twydo/c2\\n' >&2\nsleep 30",
            cat_fixture("emptied-local.txt")
        ),
        Some(1),
    );
    assert_eq!(kept, vec!["c1".to_string()]);
}

#[test]
fn unattributed_error_drops_evidence_but_keeps_emptied_local() {
    let kept = manifest_after(
        &format!(
            "{}\nprintf 'Syncing twydo/c2\\nerror: Connection refused\\n' >&2\nexit 1",
            cat_fixture("emptied-local.txt")
        ),
        None,
    );
    assert_eq!(kept, vec!["c1".to_string()]);
}

#[test]
fn errored_collection_drops_its_evidence_only() {
    let kept = manifest_after(
        &format!(
            "{}\nprintf 'Syncing twydo/c2\\nerror: twydo/c2: Connection reset\\n' >&2\nexit 1",
            cat_fixture("emptied-local.txt")
        ),
        None,
    );
    // "Twydo test" synced cleanly in the fixture, so it gains an entry.
    assert_eq!(kept, vec!["Twydo test", "c1", "c3"]);
}

/// The evidence is consumed before vdirsyncer runs: a run that dies after vdirsyncer (here
/// the hook log is unreadable) leaves no manifest for the next run to force from.
#[test]
fn run_failing_after_vdirsyncer_leaves_no_manifest() {
    let fx = Fx::new(&format!(
        "{}\nrm -f \"$TWYDO_SYNC_HOOKLOG\"\nmkdir \"$TWYDO_SYNC_HOOKLOG\"\nexit 0",
        cat_fixture("emptied-local.txt")
    ));
    let mut m = Manifest::default();
    m.0.insert("c1".into(), vec!["c1/a.ics".into()]);
    m.save(&fx.state()).unwrap();
    assert!(run(&fx.cfg, &manual(), now()).is_err());
    assert!(
        !Manifest::path(&fx.state()).exists(),
        "a run that did not finish its update leaves no evidence"
    );
}

/// Issue #10: a sync that failed (e.g. no password) must not run metasync too, which would
/// wait for the same credentials again while holding the vdir lock.
#[test]
fn metasync_is_skipped_when_sync_failed() {
    let fx = Fx::new("");
    let calls = fx.path("calls");
    let fake = fx.cfg.sync.as_ref().unwrap().command.clone();
    write_script(
        Path::new(&fake),
        &format!(
            "echo \"$1\" >> '{}'\n\
             [ \"$1\" = sync ] && {{ echo 'error: Connection refused' >&2; exit 1; }}\n\
             exit 0",
            calls.display()
        ),
    );
    let mut fx = fx;
    fx.sync_mut().metasync = true;
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert_eq!(r.transient.as_deref(), Some("Connection refused"));
    assert_eq!(fs::read_to_string(&calls).unwrap(), "sync\n");
    assert_eq!(r.metasync_error, None);
}

#[test]
fn metasync_still_runs_after_a_clean_sync() {
    let fx = Fx::new("");
    let calls = fx.path("calls");
    let fake = fx.cfg.sync.as_ref().unwrap().command.clone();
    write_script(
        Path::new(&fake),
        &format!("echo \"$1\" >> '{}'\nexit 0", calls.display()),
    );
    let mut fx = fx;
    fx.sync_mut().metasync = true;
    run(&fx.cfg, &manual(), now()).unwrap();
    assert_eq!(fs::read_to_string(&calls).unwrap(), "sync\nmetasync\n");
}

// Issue #10: a running sync is recorded in `sync/running.json` and stops on request.

fn running_file(fx: &Fx) -> PathBuf {
    fx.state().join("sync").join("running.json")
}

/// Waits up to 5 s until the recorded sync has a child.
fn wait_for_child(fx: &Fx) -> Running {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(r) = running(&fx.cfg).unwrap()
            && r.child_pgid.is_some()
        {
            return r;
        }
        assert!(Instant::now() < deadline, "no running sync recorded");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn pid_alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

fn wait_dead(pid: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    while pid_alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    !pid_alive(pid)
}

#[test]
fn running_is_reported_while_vdirsyncer_runs() {
    let fx = Fx::new("sleep 1");
    let cfg = fx.cfg.clone();
    let t = std::thread::spawn(move || run(&cfg, &manual(), now()).unwrap());
    let r = wait_for_child(&fx);
    assert_eq!(r.runner_pid, std::process::id());
    let age = Utc::now() - r.started;
    assert!(
        age >= chrono::Duration::zero() && age < chrono::Duration::seconds(5),
        "{r:?}"
    );
    t.join().unwrap();
    assert_eq!(running(&fx.cfg).unwrap(), None);
    assert!(!running_file(&fx).exists());
}

#[test]
fn stop_request_kills_the_run_and_backs_off() {
    let fx = Fx::new("");
    let pidfile = fx.path("grandchild.pid");
    let fake = fx.cfg.sync.as_ref().unwrap().command.clone();
    write_script(
        Path::new(&fake),
        &format!("sleep 30 &\necho $! > '{}'\nwait", pidfile.display()),
    );
    let cfg = fx.cfg.clone();
    let start = Instant::now();
    let t = std::thread::spawn(move || run(&cfg, &manual(), now()).unwrap());
    wait_for_child(&fx);
    assert!(wait_for(&pidfile));
    assert_eq!(request_stop(&fx.cfg).unwrap(), StopResult::Requested);
    let r = t.join().unwrap();
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "{:?}",
        start.elapsed()
    );
    assert!(r.stopped);
    assert_eq!(r.transient.as_deref(), Some("sync stopped"));
    let st = SyncStatus::load(&fx.state()).unwrap();
    assert_eq!(st.failures, 1);
    assert_eq!(st.last_error.as_deref(), Some("sync stopped"));
    let pid = fs::read_to_string(&pidfile).unwrap();
    assert!(wait_dead(pid.trim()), "grandchild survived the stop");
    assert!(!running_file(&fx).exists());
    // The request stays until the requester has written, then it clears it.
    assert!(fx.state().join("sync").join("stop").exists());
    clear_stop(&fx.cfg).unwrap();
    assert!(!fx.state().join("sync").join("stop").exists());
}

#[test]
fn stopped_run_lists_are_errors() {
    let fx = Fx::new("echo 'Syncing twydo/c1' >&2\nsleep 30");
    let cfg = fx.cfg.clone();
    let t = std::thread::spawn(move || run(&cfg, &manual(), now()).unwrap());
    wait_for_child(&fx);
    // Let the fake print its line before the stop.
    std::thread::sleep(Duration::from_millis(200));
    request_stop(&fx.cfg).unwrap();
    let r = t.join().unwrap();
    assert_eq!(
        r.collections.get("c1"),
        Some(&CollState::Error {
            message: "sync stopped".into()
        })
    );
}

#[test]
fn stopped_run_does_not_rerun() {
    let fx = Fx::new("");
    let count = fx.path("count");
    let fake = fx.cfg.sync.as_ref().unwrap().command.clone();
    write_script(
        Path::new(&fake),
        &format!("echo run >> '{}'\nsleep 30", count.display()),
    );
    let cfg = fx.cfg.clone();
    let t = std::thread::spawn(move || run(&cfg, &manual(), now()).unwrap());
    wait_for_child(&fx);
    fs::write(fx.state().join("sync").join("requested"), b"").unwrap();
    request_stop(&fx.cfg).unwrap();
    let r = t.join().unwrap();
    assert!(r.stopped);
    assert_eq!(fs::read_to_string(&count).unwrap().lines().count(), 1);
}

#[test]
fn stale_stop_file_does_not_stop_the_next_run() {
    let fx = Fx::new("sleep 0.3\nexit 0");
    let sync_dir = fx.state().join("sync");
    fs::create_dir_all(&sync_dir).unwrap();
    let stop = fs::File::create(sync_dir.join("stop")).unwrap();
    stop.set_modified(std::time::SystemTime::now() - Duration::from_secs(60))
        .unwrap();
    drop(stop);
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert!(!r.stopped);
    assert_eq!(r.transient, None);
    assert!(!sync_dir.join("stop").exists());
}

#[test]
fn stop_with_a_dead_runner_kills_the_recorded_group() {
    use std::os::unix::process::CommandExt;
    let fx = Fx::new("");
    let mut orphan = std::process::Command::new("sleep")
        .arg("30")
        .process_group(0)
        .spawn()
        .unwrap();
    let mut gone = std::process::Command::new("true").spawn().unwrap();
    gone.wait().unwrap();
    let sync_dir = fx.state().join("sync");
    fs::create_dir_all(&sync_dir).unwrap();
    let rec = Running {
        runner_pid: gone.id(),
        runner_start: None,
        child_pgid: Some(orphan.id()),
        child_start: process_start(orphan.id()),
        started: Utc::now(),
    };
    fs::write(running_file(&fx), serde_json::to_vec(&rec).unwrap()).unwrap();
    assert_eq!(running(&fx.cfg).unwrap(), Some(rec));
    assert_eq!(request_stop(&fx.cfg).unwrap(), StopResult::Killed);
    let deadline = Instant::now() + Duration::from_secs(3);
    while orphan.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "orphan survived the stop");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!running_file(&fx).exists());
    // A runner queued behind the orphan must not take the vdir before the write either.
    assert!(sync_dir.join("stop").exists());
    // PR review 4: no runner is left to record the stop, so the stop records it.
    let st = SyncStatus::load(&fx.state()).unwrap();
    assert_eq!(st.failures, 1);
    assert!(st.next_attempt.is_some());
    assert_eq!(st.last_error.as_deref(), Some("sync stopped"));
}

/// PR review 2: a record whose process ids now belong to other processes (a runner killed
/// long ago, pids reused) must not lead to signalling them.
#[test]
fn stale_record_with_reused_pids_kills_nothing() {
    use std::os::unix::process::CommandExt;
    let fx = Fx::new("");
    let mut unrelated = std::process::Command::new("sleep")
        .arg("30")
        .process_group(0)
        .spawn()
        .unwrap();
    let sync_dir = fx.state().join("sync");
    fs::create_dir_all(&sync_dir).unwrap();
    let rec = Running {
        // Both ids are live, but neither started when the record says.
        runner_pid: unrelated.id(),
        runner_start: Some("Thu Jan  1 00:00:00 1970".into()),
        child_pgid: Some(unrelated.id()),
        child_start: Some("Thu Jan  1 00:00:00 1970".into()),
        started: Utc::now(),
    };
    fs::write(running_file(&fx), serde_json::to_vec(&rec).unwrap()).unwrap();
    assert_eq!(
        running(&fx.cfg).unwrap(),
        None,
        "a stale record is no running sync"
    );
    assert_eq!(request_stop(&fx.cfg).unwrap(), StopResult::NothingRunning);
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        unrelated.try_wait().unwrap().is_none(),
        "an unrelated process group was killed"
    );
    assert!(!running_file(&fx).exists(), "the stale record is removed");
    unrelated.kill().unwrap();
    unrelated.wait().unwrap();
}

#[test]
fn process_start_identifies_a_live_process_only() {
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let start = process_start(child.id());
    assert!(start.is_some());
    assert_eq!(
        process_start(child.id()),
        start,
        "stable for the same process"
    );
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(process_start(child.id()), None);
}

#[test]
fn stop_with_nothing_running() {
    let fx = Fx::new("");
    assert_eq!(request_stop(&fx.cfg).unwrap(), StopResult::NothingRunning);
    assert!(!fx.state().join("sync").join("stop").exists());
}

/// Review minor 1: `y` between two passes finds no record, but the next pass must still not
/// take the vdir before the write.
#[test]
fn stop_with_nothing_recorded_still_holds_off_the_next_sync() {
    let fx = Fx::new("");
    let count = fx.path("count");
    let fake = fx.cfg.sync.as_ref().unwrap().command.clone();
    write_script(
        Path::new(&fake),
        &format!("echo run >> '{}'", count.display()),
    );
    fs::create_dir_all(fx.state().join("sync")).unwrap();
    assert_eq!(request_stop(&fx.cfg).unwrap(), StopResult::NothingRunning);
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert!(r.stopped);
    assert!(!count.exists(), "vdirsyncer ran");
    clear_stop(&fx.cfg).unwrap();
    assert!(!fx.state().join("sync").join("stop").exists());
}

/// Review minor 4: if checking for a stop request fails, the runner must not return and
/// leave vdirsyncer running with no record of it.
#[test]
fn failed_stop_check_does_not_leave_vdirsyncer_running() {
    use std::os::unix::fs::PermissionsExt;
    let fx = Fx::new("");
    let pidfile = fx.path("child.pid");
    let fake = fx.cfg.sync.as_ref().unwrap().command.clone();
    write_script(
        Path::new(&fake),
        &format!(
            "echo $$ > '{}'
sleep 1",
            pidfile.display()
        ),
    );
    let sync_dir = fx.state().join("sync");
    let cfg = fx.cfg.clone();
    let t = std::thread::spawn(move || run(&cfg, &manual(), now()).is_ok());
    assert!(wait_for(&pidfile));
    // No search permission: stat of sync/stop fails with EACCES.
    fs::set_permissions(&sync_dir, fs::Permissions::from_mode(0o600)).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let _ = t.join();
    fs::set_permissions(&sync_dir, fs::Permissions::from_mode(0o755)).unwrap();
    let pid = fs::read_to_string(&pidfile).unwrap();
    assert!(
        !pid_alive(pid.trim()),
        "the runner returned while vdirsyncer was still running"
    );
}

/// A runner that starts while a stop request is pending (it was waiting behind the stopped
/// sync) stops before vdirsyncer, so the writer that asked gets the vdir.
#[test]
fn fresh_stop_request_stops_a_new_run_before_vdirsyncer() {
    let fx = Fx::new("");
    let count = fx.path("count");
    let fake = fx.cfg.sync.as_ref().unwrap().command.clone();
    write_script(
        Path::new(&fake),
        &format!("echo run >> '{}'", count.display()),
    );
    let sync_dir = fx.state().join("sync");
    fs::create_dir_all(&sync_dir).unwrap();
    fs::write(sync_dir.join("stop"), b"").unwrap();
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert!(r.stopped);
    assert_eq!(r.transient.as_deref(), Some("sync stopped"));
    assert!(!count.exists(), "vdirsyncer ran");
    assert!(sync_dir.join("stop").exists(), "the requester clears it");
}

/// PR review 3: a stopped metasync is a stopped run, so it backs off too.
#[test]
fn stopped_metasync_backs_off() {
    let mut fx = Fx::new("");
    fx.sync_mut().metasync = true;
    let marker = fx.path("metasync.started");
    let fake = fx.cfg.sync.as_ref().unwrap().command.clone();
    write_script(
        Path::new(&fake),
        &format!(
            "if [ \"$1\" = sync ]; then echo 'Syncing twydo/c1' >&2; exit 0; fi\ntouch '{}'\nsleep 30",
            marker.display()
        ),
    );
    let cfg = fx.cfg.clone();
    let t = std::thread::spawn(move || run(&cfg, &manual(), now()).unwrap());
    assert!(wait_for(&marker));
    request_stop(&fx.cfg).unwrap();
    let r = t.join().unwrap();
    let st = SyncStatus::load(&fx.state()).unwrap();
    assert!(r.stopped);
    assert_eq!(r.transient.as_deref(), Some("sync stopped"));
    assert_eq!(st.failures, 1);
    assert!(st.next_attempt.is_some());
}

/// PR re-review: a pass that was already under way when a stop was asked for (here waiting
/// for the vdir in recovery) must not start vdirsyncer later, even after the requester has
/// withdrawn its request; it records the stop with backoff.
#[test]
fn pass_begun_before_a_stop_does_not_start_vdirsyncer() {
    let fx = Fx::new("");
    let count = fx.path("count");
    let fake = fx.cfg.sync.as_ref().unwrap().command.clone();
    write_script(
        Path::new(&fake),
        &format!("echo run >> '{}'", count.display()),
    );
    let state = fx.state();
    fs::create_dir_all(state.join("sync")).unwrap();
    // Recovery needs the exclusive lock; hold it so the pass waits there.
    fs::create_dir_all(state.join("journal")).unwrap();
    let held = VdirLock::exclusive(&state).unwrap();
    let cfg = fx.cfg.clone();
    let t = std::thread::spawn(move || run(&cfg, &manual(), now()).unwrap());
    std::thread::sleep(Duration::from_millis(300));
    request_stop(&fx.cfg).unwrap();
    clear_stop(&fx.cfg).unwrap();
    drop(held);
    let r = t.join().unwrap();
    assert!(r.stopped);
    assert!(!count.exists(), "vdirsyncer ran after the stop");
    let st = SyncStatus::load(&state).unwrap();
    assert_eq!(st.failures, 1);
    assert!(st.next_attempt.is_some());
}

/// A stop applies to passes begun before it, not to later ones.
#[test]
fn pass_begun_after_a_stop_runs() {
    let fx = Fx::new("");
    let count = fx.path("count");
    let fake = fx.cfg.sync.as_ref().unwrap().command.clone();
    write_script(
        Path::new(&fake),
        &format!("echo run >> '{}'", count.display()),
    );
    fs::create_dir_all(fx.state().join("sync")).unwrap();
    request_stop(&fx.cfg).unwrap();
    clear_stop(&fx.cfg).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert!(!r.stopped);
    assert!(count.exists());
}

#[test]
fn changed_journal_read_dependency_never_becomes_parent_write_conflict() {
    let fx = Fx::new("exit 0");
    let parent = task("parent", "remote parent edit");
    let item = task("item", "synthetic item");
    fs::write(fx.cfg.vdir.join("c1/parent.ics"), &parent).unwrap();
    fs::write(fx.cfg.vdir.join("c1/item.ics"), &item).unwrap();
    let journal = Journal::open(&fx.state()).unwrap();
    journal
        .prepare(&Entry {
            format: 1,
            id: "20261101T090000Z-0000abce".into(),
            created: now(),
            kind: "delete".into(),
            uids: vec!["item".into()],
            stage: Stage::Prepared,
            inverse_of: None,
            from_queue: None,
            request: None,
            synced: None,
            holds: vec![("c1/parent.ics".into(), task("parent", "loaded parent"))],
            family: None,
            families: Vec::new(),
            writes: vec![Write {
                path: "c1/item.ics".into(),
                before: Some(item.clone()),
                after: None,
            }],
        })
        .unwrap();
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert_eq!(r.new_conflicts.len(), 1);
    assert_eq!(
        fs::read_to_string(fx.cfg.vdir.join("c1/parent.ics")).unwrap(),
        parent
    );
    assert_eq!(
        fs::read_to_string(fx.cfg.vdir.join("c1/item.ics")).unwrap(),
        item
    );
    let conflicts = ConflictStore::open(&fx.state())
        .unwrap()
        .unresolved()
        .unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].path, "c1/item.ics");
    assert!(conflicts[0].local.is_none());
    assert_eq!(conflicts[0].remote.as_deref(), Some(item.as_str()));
}

#[test]
fn clean_run_writes_ended() {
    let fx = Fx::new("exit 0");
    assert!(ended::read(&fx.cfg).unwrap().is_none());
    run(&fx.cfg, &manual(), now()).unwrap();
    assert_eq!(ended::read(&fx.cfg).unwrap().unwrap().at, now());
}

#[test]
fn failed_run_writes_ended() {
    let fx = Fx::new("exit 1");
    run(&fx.cfg, &manual(), now()).unwrap();
    assert!(ended::read(&fx.cfg).unwrap().is_some());
}

#[test]
fn skipped_runs_write_no_ended() {
    // Not configured.
    let mut fx = Fx::new("exit 0");
    let configured = fx.cfg.sync.take();
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert!(matches!(r.skipped, Some(Skip::NotConfigured)));
    assert!(ended::read(&fx.cfg).unwrap().is_none());
    fx.cfg.sync = configured;

    // Backoff.
    let st = SyncStatus {
        next_attempt: Some(now() + chrono::Duration::seconds(60)),
        ..SyncStatus::default()
    };
    let state = fx.state();
    st.save(&state).unwrap();
    let r = run(&fx.cfg, &background(), now()).unwrap();
    assert!(matches!(r.skipped, Some(Skip::Backoff { .. })));
    assert!(ended::read(&fx.cfg).unwrap().is_none());
    SyncStatus::default().save(&state).unwrap();

    // Busy.
    let sync_dir = state.join("sync");
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
    assert!(ended::read(&fx.cfg).unwrap().is_none());
}

#[test]
fn each_run_has_a_new_run_id() {
    let fx = Fx::new("exit 0");
    run(&fx.cfg, &manual(), now()).unwrap();
    let first = ended::read(&fx.cfg).unwrap().unwrap();
    run(&fx.cfg, &manual(), now()).unwrap();
    let second = ended::read(&fx.cfg).unwrap().unwrap();
    assert_ne!(first.run_id, second.run_id);
}

#[test]
fn switched_off_background_run_is_skipped() {
    let fx = Fx::new("touch \"$(dirname \"$0\")/marker\"");
    twydo_core::sync::switch::set(&fx.cfg, true).unwrap();
    let r = run(&fx.cfg, &background(), now()).unwrap();
    assert_eq!(r.skipped, Some(Skip::Off));
    assert!(!fx.path("marker").exists());
    assert!(ended::read(&fx.cfg).unwrap().is_none());
}

#[test]
fn switched_off_manual_run_still_runs() {
    let fx = Fx::new("exit 0");
    twydo_core::sync::switch::set(&fx.cfg, true).unwrap();
    let r = run(&fx.cfg, &manual(), now()).unwrap();
    assert!(r.skipped.is_none());
    assert!(ended::read(&fx.cfg).unwrap().is_some());
}

#[test]
fn switch_survives_and_clears() {
    use twydo_core::sync::switch::{is_off, set};
    let fx = Fx::new("exit 0");
    assert!(!is_off(&fx.cfg).unwrap());
    assert!(!fx.path("state").exists());
    set(&fx.cfg, true).unwrap();
    assert!(is_off(&fx.cfg).unwrap());
    set(&fx.cfg, false).unwrap();
    assert!(!is_off(&fx.cfg).unwrap());
}

#[test]
fn failed_rediscovery_keeps_other_missing_lists_until_a_real_sync_updates_them() {
    let fx = Fx::new(
        r#"
        dir=$(dirname "$TWYDO_SYNC_HOOKLOG")
        if [ "$1" = discover ] && [ ! -f "$dir/allow" ]; then
            echo 'warning: No collection "c2" found for storage twydo_remote.' >&2
            exit 1
        fi
        [ "$1" = sync ] && printf 'Syncing twydo/c2\nSyncing twydo/c3\n' >&2
        exit 0
    "#,
    );
    for id in ["c2", "c3", "healthy"] {
        fs::create_dir_all(fx.cfg.vdir.join(id)).unwrap();
        fs::write(
            fx.cfg.vdir.join(id).join("task.ics"),
            task(id, "Synthetic task"),
        )
        .unwrap();
    }
    let state = fx.state();
    let missing = CollState::Error {
        message: "Unknown error occurred: Not Found".into(),
    };
    SyncStatus {
        collections: std::collections::BTreeMap::from([
            ("c1".into(), missing.clone()),
            ("c2".into(), missing.clone()),
            ("c3".into(), missing.clone()),
            ("ghost".into(), missing.clone()),
            ("healthy".into(), CollState::Ok),
        ]),
        ..Default::default()
    }
    .save(&state)
    .unwrap();
    twydo_core::retired::forget(
        &fx.cfg,
        &twydo_core::retired::preview(&fx.cfg, "c1").unwrap(),
        now(),
    )
    .unwrap();
    let first = run(&fx.cfg, &manual(), now()).unwrap();
    assert!(first.transient.as_deref().unwrap().contains("discovery"));
    assert!(
        matches!(first.collections.get("c2"), Some(CollState::Error { message }) if message.ends_with("Not Found")),
        "{:?}",
        first.collections
    );
    assert_eq!(first.collections.get("c3"), Some(&missing));
    for id in ["c1", "ghost", "healthy"] {
        assert!(
            !first.collections.contains_key(id),
            "must not retain {id}: {:?}",
            first.collections
        );
    }
    assert_eq!(
        SyncStatus::load(&state).unwrap().collections,
        first.collections
    );
    let retry = run(&fx.cfg, &manual(), now()).unwrap();
    assert_eq!(retry.collections, first.collections);
    fs::write(state.join("sync/allow"), "").unwrap();
    let recovered = run(&fx.cfg, &manual(), now()).unwrap();
    assert!(recovered.transient.is_none());
    assert_eq!(recovered.collections.get("c2"), Some(&CollState::Ok));
    assert_eq!(recovered.collections.get("c3"), Some(&CollState::Ok));
    assert!(!fx.cfg.vdir.join("c1").exists());
    assert_eq!(
        fs::read_to_string(fx.cfg.vdir.join("c3/task.ics")).unwrap(),
        task("c3", "Synthetic task")
    );
}

#[test]
fn discovery_marks_only_active_lists_missing_from_remote_storage() {
    let fx = Fx::new(
        r#"
        echo 'warning: No collection "c2" found for storage twydo_remote.' >&2
        echo 'warning: No collection "c1" found for storage twydo_local.' >&2
        echo 'warning: No collection "ghost" found for storage twydo_remote.' >&2
        echo 'warning: No collection not-json found for storage twydo_remote.' >&2
        exit 1
    "#,
    );
    fs::create_dir_all(fx.cfg.vdir.join("c2")).unwrap();
    let state = fx.state();
    fs::create_dir_all(state.join("sync")).unwrap();
    fs::write(state.join("sync/rediscover"), "backup-id").unwrap();
    let report = run(&fx.cfg, &manual(), now()).unwrap();
    assert_eq!(report.collections.len(), 1, "{:?}", report.collections);
    assert!(
        matches!(report.collections.get("c2"), Some(CollState::Error { message }) if message.ends_with("Not Found"))
    );
    assert!(
        report
            .transient
            .as_deref()
            .unwrap()
            .contains("discovery failed")
    );
    assert!(state.join("sync/rediscover").exists());
}

const QA: &str = "0a04f22aeb294b719633c978fe1975e5";

/// Copies the synthetic plan 4 tasks into `c1` and lets the drain's gate pass (test mode).
fn seed_tasks(fx: &mut Fx) {
    let src =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase0/v1/plan4-list1/02-edit");
    for e in fs::read_dir(src).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "ics") {
            fs::copy(&p, fx.cfg.vdir.join("c1").join(p.file_name().unwrap())).unwrap();
        }
    }
    fx.cfg.timezone = Some("America/New_York".to_string());
    fx.cfg.writes.mode = WriteMode::Test;
}

fn qa_path(fx: &Fx) -> PathBuf {
    fx.cfg.vdir.join("c1").join(format!("{QA}.ics"))
}

/// Queues a title edit of `QA` the way a writer does while a sync holds the lock; returns
/// the queue item's file name.
fn queue_rename(fx: &Fx) -> String {
    let matrix = common::matrix(
        &["one-time", "monthly", "date-only", "timed", "start-date"],
        "unverified",
        &[],
    );
    let (mut store, _) = Store::open_with(&fx.cfg, matrix).unwrap();
    let snap = Snapshot::new(
        twydo_core::vdir::load(&fx.cfg.vdir).unwrap(),
        fx.cfg.clone(),
    );
    let op = Op::Edit {
        uid: QA.into(),
        changes: vec![Change::Summary("renamed".into())],
    };
    let held = VdirLock::exclusive(&fx.state()).unwrap();
    let got = store
        .submit(
            &snap,
            &op,
            "rename",
            "op",
            now() - chrono::Duration::minutes(5),
        )
        .unwrap();
    drop(held);
    let Submitted::Queued(item) = got else {
        panic!("should queue")
    };
    format!("{}.json", item.id)
}

#[test]
fn runner_drains_before_syncing() {
    let mut fx = Fx::new("");
    seed_tasks(&mut fx);
    let seen = fx.path("seen.ics");
    let fake = fx.cfg.sync.as_ref().unwrap().command.clone();
    write_script(
        Path::new(&fake),
        &format!(
            "cp '{}' '{}'\n{}\nexit 0",
            qa_path(&fx).display(),
            seen.display(),
            cat_fixture("clean.txt")
        ),
    );
    queue_rename(&fx);
    let r = run(&fx.cfg, &background(), now()).unwrap();
    assert!(
        fs::read_to_string(&seen)
            .unwrap()
            .contains("SUMMARY:renamed")
    );
    let [d] = r.drained.as_slice() else {
        panic!("{:?}", r.drained)
    };
    let DrainOutcome::Applied(id) = &d.outcome else {
        panic!("{d:?}")
    };
    assert_eq!(r.confirmed, vec![id.clone()]);
    assert!(synced(&fx, id));
    assert!(Queue::open(&fx.state()).unwrap().is_empty().unwrap());
}

#[test]
fn runner_drains_and_reruns_to_send() {
    let mut fx = Fx::new("");
    seed_tasks(&mut fx);
    let file = queue_rename(&fx);
    // Stage the item outside the queue; the fake's first call puts it back, as a writer that
    // found the lock busy would while vdirsyncer runs.
    let queue_dir = fx.state().join("queue");
    let staged = fx.path("staged.json");
    fs::rename(queue_dir.join(&file), &staged).unwrap();
    let calls = fx.path("calls");
    let seen = fx.path("seen.ics");
    let fake = fx.cfg.sync.as_ref().unwrap().command.clone();
    write_script(
        Path::new(&fake),
        &format!(
            "echo call >> '{calls}'\nif [ -e '{staged}' ]; then cp '{staged}' '{q}/{file}'; rm '{staged}'; \
             else cp '{qa}' '{seen}'; fi\n{clean}\nexit 0",
            calls = calls.display(),
            staged = staged.display(),
            q = queue_dir.display(),
            qa = qa_path(&fx).display(),
            seen = seen.display(),
            clean = cat_fixture("clean.txt"),
        ),
    );
    let r = run(&fx.cfg, &background(), now()).unwrap();
    assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 2);
    assert!(
        fs::read_to_string(&seen)
            .unwrap()
            .contains("SUMMARY:renamed")
    );
    let [d] = r.drained.as_slice() else {
        panic!("{:?}", r.drained)
    };
    let DrainOutcome::Applied(id) = &d.outcome else {
        panic!("{d:?}")
    };
    assert!(r.confirmed.contains(id), "{:?}", r.confirmed);
    assert!(synced(&fx, id));
}

#[test]
fn queued_final_task_deletion_is_automatically_sent_on_the_rerun() {
    let mut fx = Fx::new("");
    seed_tasks(&mut fx);
    for entry in fs::read_dir(fx.cfg.vdir.join("c1")).unwrap() {
        let path = entry.unwrap().path();
        if path != qa_path(&fx) {
            fs::remove_file(path).unwrap();
        }
    }
    let (mut store, _) = Store::open(&fx.cfg).unwrap();
    let snap = Snapshot::new(
        twydo_core::vdir::load(&fx.cfg.vdir).unwrap(),
        fx.cfg.clone(),
    );
    let held = VdirLock::shared(&fx.state()).unwrap();
    let Submitted::Queued(item) = store
        .submit(&snap, &Op::Delete { uid: QA.into() }, "delete", "op", now())
        .unwrap()
    else {
        panic!("held lock")
    };
    drop(held);
    // Publish the queued deletion during the first sync, after its pre-sync drain.
    let queued = fx.state().join("queue").join(format!("{}.json", item.id));
    let staged = fx.path("staged.json");
    fs::rename(&queued, &staged).unwrap();
    let calls = fx.path("calls");
    write_script(
        Path::new(&fx.cfg.sync.as_ref().unwrap().command),
        &format!(
            "echo \"$*\" >> '{calls}'\n\
         if [ -e '{staged}' ]; then cp '{staged}' '{queued}'; rm '{staged}'; {clean}; \
         elif [ \"$2\" = --force-delete ]; then {clean}; \
         else {empty}; fi\nexit 0",
            calls = calls.display(),
            staged = staged.display(),
            queued = queued.display(),
            clean = cat_fixture("clean.txt"),
            empty = cat_fixture("emptied-local.txt"),
        ),
    );

    let report = run(&fx.cfg, &manual(), now()).unwrap();

    assert_eq!(report.auto_forced, ["c1"]);
    assert_eq!(report.collections.get("c1"), Some(&CollState::Ok));
    assert!(
        fs::read_to_string(calls)
            .unwrap()
            .contains("--force-delete twydo/c1")
    );
    assert!(!qa_path(&fx).exists());
    let [drained] = report.drained.as_slice() else {
        panic!("{report:?}")
    };
    let DrainOutcome::Applied(id) = &drained.outcome else {
        panic!("{drained:?}")
    };
    assert!(synced(&fx, id));
}

#[test]
fn drain_refusal_does_not_fail_the_run() {
    let mut fx = Fx::new(&format!("{}\nexit 0", cat_fixture("clean.txt")));
    seed_tasks(&mut fx);
    queue_rename(&fx);
    // The file changes after the edit was queued: the drain refuses it.
    let path = qa_path(&fx);
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replacen("SUMMARY:", "SUMMARY:other ", 1)).unwrap();
    let r = run(&fx.cfg, &background(), now()).unwrap();
    assert!(r.skipped.is_none());
    assert!(r.transient.is_none(), "{:?}", r.transient);
    let [d] = r.drained.as_slice() else {
        panic!("{:?}", r.drained)
    };
    assert!(matches!(d.outcome, DrainOutcome::Refused(_)), "{d:?}");
    let q = Queue::open(&fx.state()).unwrap();
    let [refusal] = q
        .unshown()
        .unwrap()
        .try_into()
        .unwrap_or_else(|v: Vec<_>| panic!("{v:?}"));
    assert!(refusal.stale);
    assert!(fs::read_to_string(&path).unwrap().contains("SUMMARY:other"));
}

#[test]
fn stopped_pass_that_drained_leaves_no_rerun_request() {
    let mut fx = Fx::new("");
    seed_tasks(&mut fx);
    let file = queue_rename(&fx);
    let queue_dir = fx.state().join("queue");
    let staged = fx.path("staged.json");
    fs::rename(queue_dir.join(&file), &staged).unwrap();
    let fake = fx.cfg.sync.as_ref().unwrap().command.clone();
    write_script(
        Path::new(&fake),
        &format!(
            "cp '{}' '{}/{file}'\nsleep 30",
            staged.display(),
            queue_dir.display()
        ),
    );
    let cfg = fx.cfg.clone();
    let t = std::thread::spawn(move || run(&cfg, &manual(), now()).unwrap());
    wait_for_child(&fx);
    // Let the fake queue its item before the stop.
    assert!(wait_for(&queue_dir.join(&file)));
    request_stop(&fx.cfg).unwrap();
    let r = t.join().unwrap();
    assert!(r.stopped);
    assert!(
        r.drained
            .iter()
            .any(|d| matches!(d.outcome, DrainOutcome::Applied(_))),
        "{:?}",
        r.drained
    );
    assert!(!fx.state().join("sync/requested").exists());
}

// Daemon plan 2, Task 6: a vdirsyncer left by a runner that is gone, reaped at startup.

/// A finished process: its pid names no live runner.
fn dead_pid() -> u32 {
    let mut gone = std::process::Command::new("true").spawn().unwrap();
    gone.wait().unwrap();
    gone.id()
}

fn write_record(fx: &Fx, rec: &Running) {
    fs::create_dir_all(fx.state().join("sync")).unwrap();
    fs::write(running_file(fx), serde_json::to_vec(rec).unwrap()).unwrap();
}

#[test]
fn reap_orphan_kills_a_child_whose_runner_is_gone() {
    use std::os::unix::process::CommandExt;
    let fx = Fx::new("");
    let mut orphan = std::process::Command::new("sh")
        .args(["-c", "sleep 30"])
        .process_group(0)
        .spawn()
        .unwrap();
    write_record(
        &fx,
        &Running {
            runner_pid: dead_pid(),
            runner_start: None,
            child_pgid: Some(orphan.id()),
            child_start: process_start(orphan.id()),
            started: Utc::now(),
        },
    );
    assert_eq!(reap_orphan(&fx.cfg).unwrap(), Reaped::Killed);
    let deadline = Instant::now() + Duration::from_secs(3);
    while orphan.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "orphan survived the reap");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!running_file(&fx).exists());
    assert!(
        !fx.state().join("sync/stop").exists(),
        "the reaper withdraws its stop request"
    );
}

#[test]
fn reap_orphan_leaves_a_live_runner() {
    let fx = Fx::new("");
    let me = std::process::id();
    let rec = Running {
        runner_pid: me,
        runner_start: process_start(me),
        child_pgid: None,
        child_start: None,
        started: Utc::now(),
    };
    write_record(&fx, &rec);
    assert_eq!(reap_orphan(&fx.cfg).unwrap(), Reaped::LiveRunner);
    assert_eq!(running(&fx.cfg).unwrap(), Some(rec), "the record is kept");
    assert!(!fx.state().join("sync/stop").exists());
}

#[test]
fn reap_orphan_removes_a_stale_record() {
    let fx = Fx::new("");
    assert_eq!(reap_orphan(&fx.cfg).unwrap(), Reaped::Nothing);
    write_record(
        &fx,
        &Running {
            runner_pid: dead_pid(),
            runner_start: Some("Thu Jan  1 00:00:00 1970".into()),
            child_pgid: Some(dead_pid()),
            child_start: Some("Thu Jan  1 00:00:00 1970".into()),
            started: Utc::now(),
        },
    );
    assert_eq!(reap_orphan(&fx.cfg).unwrap(), Reaped::Stale);
    assert!(!running_file(&fx).exists());
    assert!(!fx.state().join("sync/stop").exists());
    // Nothing was stopped, so nothing backs off.
    assert_eq!(SyncStatus::load(&fx.state()).unwrap().failures, 0);
}

#[test]
fn a_held_queue_is_left_on_disk() {
    let mut fx = Fx::new("");
    seed_tasks(&mut fx);
    let fake = fx.cfg.sync.as_ref().unwrap().command.clone();
    write_script(
        Path::new(&fake),
        &format!("{}\nexit 0", cat_fixture("clean.txt")),
    );
    queue_rename(&fx);
    let opts = manual();
    opts.hold_queue
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let r = run(&fx.cfg, &opts, now()).unwrap();
    assert!(r.drained.is_empty(), "{:?}", r.drained);
    assert!(!Queue::open(&fx.state()).unwrap().is_empty().unwrap());
    assert!(
        !fs::read_to_string(qa_path(&fx))
            .unwrap()
            .contains("SUMMARY:renamed")
    );
}

/// Task 6 review: a run that a shutting-down daemon gave up on, blocked on the run lock,
/// writes nothing once it gets the lock: no recovery (a `Prepared` entry stays as it is), no
/// status, no ended event, no vdirsyncer.
#[test]
fn a_held_run_waiting_for_the_run_lock_writes_nothing() {
    let fx = Fx::new("");
    let count = fx.path("count");
    let fake = fx.cfg.sync.as_ref().unwrap().command.clone();
    write_script(
        Path::new(&fake),
        &format!("echo run >> '{}'", count.display()),
    );
    let file = fx.cfg.vdir.join("c1/a.ics");
    let third = task("u1", "third");
    fs::write(&file, &third).unwrap();
    let state = fx.state();
    let entry = Entry {
        format: 1,
        id: "20261101T090000Z-0000beef".into(),
        created: now(),
        kind: "edit".into(),
        uids: vec!["u1".into()],
        stage: Stage::Prepared,
        inverse_of: None,
        from_queue: None,
        request: None,
        synced: None,
        holds: Vec::new(),
        family: None,
        families: Vec::new(),
        writes: vec![Write {
            path: "c1/a.ics".into(),
            before: Some(task("u1", "before")),
            after: Some(task("u1", "after")),
        }],
    };
    Journal::open(&state).unwrap().prepare(&entry).unwrap();
    let sync_dir = state.join("sync");
    fs::create_dir_all(&sync_dir).unwrap();
    let lock = twydo_core::sync::lock::LockFile::take(&sync_dir.join("run.lock"), true)
        .unwrap()
        .expect("run lock free");
    let opts = manual();
    let hold = std::sync::Arc::clone(&opts.hold_queue);
    let cfg = fx.cfg.clone();
    let t = std::thread::spawn(move || run(&cfg, &opts, now()).unwrap());
    // The run waits for the lock; the caller gives up on it, then the lock frees.
    std::thread::sleep(Duration::from_millis(200));
    hold.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(lock);
    let r = t.join().unwrap();
    assert!(r.stopped);
    assert_eq!(r.transient, None);
    assert!(r.drained.is_empty());
    assert!(!count.exists(), "vdirsyncer ran");
    assert_eq!(fs::read_to_string(&file).unwrap(), third, "vdir untouched");
    let kept = Journal::open(&state)
        .unwrap()
        .get(&entry.id)
        .unwrap()
        .unwrap();
    assert_eq!(kept.stage, Stage::Prepared, "recovery ran");
    assert!(
        ConflictStore::open(&state)
            .unwrap()
            .list()
            .unwrap()
            .is_empty()
    );
    assert!(!state.join("sync/status.json").exists(), "status written");
    assert_eq!(SyncStatus::load(&state).unwrap(), SyncStatus::default());
    assert!(
        ended::read(&fx.cfg).unwrap().is_none(),
        "ended event written"
    );
}

/// Codex review: a run past the early `hold_queue` check, blocked on the vdir lock in its
/// recovery step while the daemon starts shutting down, writes nothing once it gets the
/// lock: no recovery (a `Prepared` entry stays as it is), no status, no ended event, no
/// vdirsyncer.
#[test]
fn a_held_run_waiting_for_the_vdir_lock_in_recovery_writes_nothing() {
    let fx = Fx::new("");
    let count = fx.path("count");
    let fake = fx.cfg.sync.as_ref().unwrap().command.clone();
    write_script(
        Path::new(&fake),
        &format!("echo run >> '{}'", count.display()),
    );
    let file = fx.cfg.vdir.join("c1/a.ics");
    let third = task("u1", "third");
    fs::write(&file, &third).unwrap();
    let state = fx.state();
    let entry = Entry {
        format: 1,
        id: "20261101T090000Z-0000beef".into(),
        created: now(),
        kind: "edit".into(),
        uids: vec!["u1".into()],
        stage: Stage::Prepared,
        inverse_of: None,
        from_queue: None,
        request: None,
        synced: None,
        holds: Vec::new(),
        family: None,
        families: Vec::new(),
        writes: vec![Write {
            path: "c1/a.ics".into(),
            before: Some(task("u1", "before")),
            after: Some(task("u1", "after")),
        }],
    };
    Journal::open(&state).unwrap().prepare(&entry).unwrap();
    let lock = twydo_core::store::fsio::VdirLock::exclusive(&state).unwrap();
    let opts = manual();
    let hold = std::sync::Arc::clone(&opts.hold_queue);
    let cfg = fx.cfg.clone();
    let t = std::thread::spawn(move || run(&cfg, &opts, now()).unwrap());
    // The run passes the early check and waits for the vdir lock; the caller starts shutting
    // down, then the lock frees.
    std::thread::sleep(Duration::from_millis(200));
    hold.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(lock);
    let r = t.join().unwrap();
    assert!(r.stopped);
    assert_eq!(r.transient, None);
    assert!(r.drained.is_empty());
    assert!(!count.exists(), "vdirsyncer ran");
    assert_eq!(fs::read_to_string(&file).unwrap(), third, "vdir untouched");
    let kept = Journal::open(&state)
        .unwrap()
        .get(&entry.id)
        .unwrap()
        .unwrap();
    assert_eq!(kept.stage, Stage::Prepared, "recovery ran");
    assert!(
        ConflictStore::open(&state)
            .unwrap()
            .list()
            .unwrap()
            .is_empty()
    );
    assert!(!state.join("sync/status.json").exists(), "status written");
    assert_eq!(SyncStatus::load(&state).unwrap(), SyncStatus::default());
    assert!(
        ended::read(&fx.cfg).unwrap().is_none(),
        "ended event written"
    );
}
