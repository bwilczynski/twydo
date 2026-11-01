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
use twydo_core::sync::output::parse;
use twydo_core::sync::reconcile::reconcile;
use twydo_core::sync::run::{RunOptions, Skip, run};
use twydo_core::sync::status::{CollState, SyncStatus, backoff, coll_states};

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
        synced: None,
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
            synced: None,
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
