//! The sync runner: one serialized vdirsyncer run per call, with recovery before it,
//! reconciliation after it, and backoff on transient failures.
//!
//! Locks, always in this order: the run lock `<state>/sync/run.lock`, the vdir lock, the
//! conflicts lock. vdirsyncer runs under the shared vdir lock only (readers continue, writers
//! wait), and never while this process holds the conflicts lock, because the conflict hook
//! takes that lock itself.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use super::SyncError;
use super::hook::{HOOK_LOG_ENV, HookRecord, read_hook_log, uid_of};
use super::output::{SyncOutput, parse};
use super::status::{CollState, SyncStatus, backoff, coll_states};
use crate::config::{Config, SyncConfig};
use crate::conflicts::{Conflict, ConflictKind, ConflictStore, NewConflict, Side};
use crate::store::bind_vdir_state;
use crate::store::fsio::VdirLock;
use crate::store::journal::{Entry, Journal, Stage, recover_locked};

/// How `run` was asked to run.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Timer or after-edit trigger: never wait for another runner, honour backoff.
    pub background: bool,
    /// Collection ids to sync with `--force-delete` (the user's confirmation).
    pub force_delete: Vec<String>,
}

/// Why a run did nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum Skip {
    NotConfigured,
    /// Another runner holds the run lock; it was asked to run once more.
    Busy,
    /// A background run during backoff.
    Backoff {
        until: DateTime<Utc>,
    },
}

/// What a run did. With several passes (reruns requested while running), the lists collect
/// every pass and the rest describe the last one.
#[derive(Debug, Clone, Default)]
pub struct RunReport {
    pub skipped: Option<Skip>,
    pub collections: BTreeMap<String, CollState>,
    /// First line of a transient failure (see [`run`]).
    pub transient: Option<String>,
    pub new_conflicts: Vec<String>,
    pub confirmed: Vec<String>,
    pub pruned: usize,
    pub forced: Vec<String>,
    pub metasync_error: Option<String>,
}

/// Grace for the stderr reader threads after the child is gone (a grandchild may still
/// hold the pipe).
const DRAIN_GRACE: Duration = Duration::from_secs(1);
/// How often a running child is polled for exit.
const POLL: Duration = Duration::from_millis(25);
/// A requested rerun happens at most this many times per call.
const MAX_RERUNS: usize = 2;

/// The exclusive lock on `<state>/sync/run.lock`, released on drop. It unlocks explicitly
/// rather than relying on close: a child forked by another thread holds a copy of the
/// descriptor until it execs, and would keep the lock alive.
struct RunLock(File);

impl RunLock {
    /// Blocks for the lock, or with `try_only` returns `None` when another runner holds it.
    fn take(sync_dir: &Path, try_only: bool) -> io::Result<Option<RunLock>> {
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(sync_dir.join("run.lock"))?;
        if !try_only {
            f.lock()?;
            return Ok(Some(RunLock(f)));
        }
        match f.try_lock() {
            Ok(()) => Ok(Some(RunLock(f))),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(e)) => Err(e),
        }
    }
}

impl Drop for RunLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn skipped(s: Skip) -> RunReport {
    RunReport {
        skipped: Some(s),
        ..RunReport::default()
    }
}

/// Runs a sync for `cfg`:
///
/// 1. Takes the run lock (background: without waiting; when busy, asks the holder for a
///    rerun through `sync/requested` and returns [`Skip::Busy`]).
/// 2. In background mode, skips while `status.next_attempt > now`.
/// 3. Recovers the journal; a stuck entry's conflicting writes become `Journal` conflicts
///    and the entry is abandoned.
/// 4. Under the shared vdir lock: runs `<command> sync <pair>` (or `--force-delete` for the
///    forced collections), killed after `timeout_secs`, then `metasync` when configured.
/// 5. Under the exclusive vdir lock: reconciles the result with the journal.
/// 6. Classifies each collection and saves `status.json`. A run whose command could not
///    run, timed out, exited non-zero without explanation, printed unattributed errors, or
///    left a collection in `Error`, is a transient failure and backs off.
/// 7. Repeats from 3, at most twice, while `sync/requested` exists.
pub fn run(cfg: &Config, opts: &RunOptions, now: DateTime<Utc>) -> Result<RunReport, SyncError> {
    let Some(sync) = &cfg.sync else {
        return Ok(skipped(Skip::NotConfigured));
    };
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir)?;
    let sync_dir = state.join("sync");
    fs::create_dir_all(&sync_dir)?;
    let requested = sync_dir.join("requested");

    // Step 1: the run lock, held until this function returns.
    let Some(_run_lock) = RunLock::take(&sync_dir, opts.background)? else {
        File::create(&requested)?;
        return Ok(skipped(Skip::Busy));
    };

    // Step 2: backoff.
    let mut status = SyncStatus::load(&state)?;
    if opts.background
        && let Some(until) = status.next_attempt.filter(|t| *t > now)
    {
        return Ok(skipped(Skip::Backoff { until }));
    }

    let mut report = RunReport::default();
    let mut force = opts.force_delete.clone();
    for pass in 0..=MAX_RERUNS {
        one_pass(cfg, sync, &state, &force, now, &mut status, &mut report)?;
        force.clear();
        // Step 7.
        if pass == MAX_RERUNS || !requested.exists() {
            break;
        }
        fs::remove_file(&requested)?;
    }
    Ok(report)
}

/// Steps 3 to 6.
fn one_pass(
    cfg: &Config,
    sync: &SyncConfig,
    state: &Path,
    force: &[String],
    now: DateTime<Utc>,
    status: &mut SyncStatus,
    report: &mut RunReport,
) -> Result<(), SyncError> {
    // Step 3: recovery under the exclusive lock.
    report
        .new_conflicts
        .extend(recover_and_park(&cfg.vdir, state, now)?);

    // Step 4: vdirsyncer under the shared lock.
    let hook_log = state.join("sync").join("hook.log");
    let shared = VdirLock::shared(state)?;
    let journal = Journal::open(state)?;
    let pending = journal.pending()?;
    let unresolved = ConflictStore::open(state)?.unresolved()?;
    fs::write(&hook_log, b"")?;
    let mut args = vec!["sync".to_string()];
    if force.is_empty() {
        args.push(sync.pair.clone());
    } else {
        args.push("--force-delete".into());
        args.extend(force.iter().map(|c| format!("{}/{c}", sync.pair)));
    }
    let ran = run_command(sync, &args, &hook_log)?;
    let out = parse(&sync.pair, &ran.stderr);
    // Emptied collections are only classified here (step 6); forcing them is Task 8.
    report.forced.extend(force.iter().cloned());
    report.metasync_error = match ran.outcome {
        Outcome::Exited(_) if sync.metasync => metasync(sync, &hook_log)?,
        _ => None,
    };
    let hook = read_hook_log(&hook_log)?;
    drop(shared);

    let mut collections = coll_states(&out, &hook, &sync.local_storage);
    if matches!(ran.outcome, Outcome::TimedOut) {
        // A killed run may have stopped in the middle of any collection it reported.
        for s in collections.values_mut() {
            if *s == CollState::Ok {
                *s = CollState::Error {
                    message: "sync timed out".into(),
                };
            }
        }
    }

    // Step 5: reconcile under the exclusive lock.
    {
        let _excl = VdirLock::exclusive(state)?;
        // Task 7 plugs in here: `reconcile(&cfg.vdir, &journal, &conflicts, sync, &pending,
        // &out, &hook, now)` fills `confirmed`, `new_conflicts` and `pruned`. Until then
        // nothing is confirmed.
        let confirmed = reconcile_stub(&pending, &unresolved, &out, &hook);
        report.confirmed.extend(confirmed);
    }

    // Step 6: status.
    let transient = transient(sync, &ran, &out, &collections);
    status.last_attempt = Some(now);
    status.collections = collections.clone();
    status.metasync_error = report.metasync_error.clone();
    match &transient {
        Some(msg) => {
            status.failures = status.failures.saturating_add(1);
            let delay = backoff(status.failures, Duration::from_secs(sync.max_backoff_secs));
            let delay = chrono::Duration::from_std(delay).unwrap_or(chrono::Duration::MAX);
            status.next_attempt = now.checked_add_signed(delay);
            status.last_error = Some(msg.lines().next().unwrap_or_default().to_string());
        }
        None => {
            status.last_success = Some(now);
            status.failures = 0;
            status.next_attempt = None;
            status.last_error = None;
        }
    }
    status.save(state)?;
    report.collections = collections;
    report.transient = transient;
    Ok(())
}

/// Placeholder for Task 7's `reconcile`: confirms nothing.
fn reconcile_stub(
    _pending: &[Entry],
    _unresolved: &[Conflict],
    _out: &SyncOutput,
    _hook: &[HookRecord],
) -> Vec<String> {
    Vec::new()
}

/// The transient failure of a run, if any (see [`run`], step 6).
fn transient(
    sync: &SyncConfig,
    ran: &Ran,
    out: &SyncOutput,
    collections: &BTreeMap<String, CollState>,
) -> Option<String> {
    match &ran.outcome {
        Outcome::SpawnFailed(msg) => return Some(msg.clone()),
        Outcome::TimedOut => {
            return Some(format!(
                "{} sync timed out after {} s",
                sync.command, sync.timeout_secs
            ));
        }
        Outcome::Exited(_) => {}
    }
    if let Some(line) = out.unattributed.first() {
        return Some(line.clone());
    }
    if let Some((c, msg)) = collections.iter().find_map(|(c, s)| match s {
        CollState::Error { message } => Some((c, message)),
        _ => None,
    }) {
        return Some(format!("{c}: {msg}"));
    }
    match &ran.outcome {
        Outcome::Exited(st)
            if !st.success() && collections.values().all(|s| *s == CollState::Ok) =>
        {
            Some(format!("{} sync exited with {st}", sync.command))
        }
        _ => None,
    }
}

/// Step 3: rolls Prepared entries forward. For a stuck entry, every write whose file holds
/// neither its `before` nor its `after` becomes a `Journal` conflict (the vdir keeps the
/// file as it is), and the entry is abandoned. Writes already applied stay. Entries that
/// cannot be read or have an invalid path are left stuck. Returns the new conflict ids.
fn recover_and_park(vdir: &Path, state: &Path, now: DateTime<Utc>) -> io::Result<Vec<String>> {
    let _lock = VdirLock::exclusive(state)?;
    let journal = Journal::open(state)?;
    let report = recover_locked(vdir, &journal)?;
    let mut ids = Vec::new();
    if report.stuck.is_empty() {
        return Ok(ids);
    }
    let conflicts = ConflictStore::open(state)?;
    for (id, _reason) in &report.stuck {
        let Ok(Some(e)) = journal.get(id) else {
            continue;
        };
        if e.stage != Stage::Prepared || &e.id != id {
            continue;
        }
        let Some(files) = e
            .writes
            .iter()
            .map(|w| safe_path(vdir, &w.path))
            .collect::<Option<Vec<PathBuf>>>()
        else {
            continue;
        };
        for (w, file) in e.writes.iter().zip(files) {
            let current = match fs::read(&file) {
                Ok(b) => Some(b),
                Err(err) if err.kind() == io::ErrorKind::NotFound => None,
                Err(err) => return Err(err),
            };
            let holds =
                |want: &Option<String>| current.as_deref() == want.as_ref().map(String::as_bytes);
            if holds(&w.before) || holds(&w.after) {
                continue;
            }
            let current = current.map(|b| String::from_utf8_lossy(&b).into_owned());
            let uid = [&w.after, &w.before, &current]
                .into_iter()
                .flatten()
                .find_map(|text| uid_of(text, &file).ok())
                .or_else(|| e.uids.first().cloned())
                .unwrap_or_default();
            let collection = Path::new(&w.path)
                .parent()
                .and_then(Path::to_str)
                .unwrap_or_default()
                .to_string();
            let c = conflicts.record(
                NewConflict {
                    uid,
                    collection,
                    path: w.path.clone(),
                    kind: ConflictKind::Journal,
                    held: false,
                    vdir_holds: Side::Remote,
                    local: w.after.clone(),
                    remote: current,
                },
                now,
            )?;
            ids.push(c.id);
        }
        journal.set_stage(id, Stage::Abandoned)?;
    }
    Ok(ids)
}

/// `vdir/rel` when `rel` is a plain relative path inside the vdir.
fn safe_path(vdir: &Path, rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel);
    let ok = !rel.is_empty()
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)));
    ok.then(|| vdir.join(p))
}

/// Runs `<command> metasync <pair>`; returns its failure, if any.
fn metasync(sync: &SyncConfig, hook_log: &Path) -> Result<Option<String>, SyncError> {
    let args = ["metasync".to_string(), sync.pair.clone()];
    let ran = run_command(sync, &args, hook_log)?;
    let first_line = || {
        ran.stderr
            .lines()
            .map(|l| l.strip_prefix("error: ").unwrap_or(l).trim())
            .find(|l| !l.is_empty())
            .map(str::to_string)
    };
    Ok(match &ran.outcome {
        Outcome::Exited(st) if st.success() => None,
        Outcome::Exited(st) => {
            Some(first_line().unwrap_or_else(|| format!("metasync exited with {st}")))
        }
        Outcome::TimedOut => Some(format!("metasync timed out after {} s", sync.timeout_secs)),
        Outcome::SpawnFailed(msg) => Some(msg.clone()),
    })
}

enum Outcome {
    Exited(ExitStatus),
    TimedOut,
    SpawnFailed(String),
}

struct Ran {
    stderr: String,
    outcome: Outcome,
}

/// Starts a thread that reads `r` to the end and sends what it read.
fn reader<R: Read + Send + 'static>(mut r: R) -> Result<mpsc::Receiver<String>, SyncError> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("sync-output".into())
        .spawn(move || {
            let mut buf = Vec::new();
            let _ = r.read_to_end(&mut buf);
            let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
        })
        .map_err(|e| SyncError::Spawn(format!("cannot start an output reader: {e}")))?;
    Ok(rx)
}

/// Runs `<command> args…` with stdin null, `VDIRSYNCER_CONFIG` (when configured) and the
/// hook log in the environment. stdout is drained and dropped; stderr is returned. The
/// child is killed after `timeout_secs`.
fn run_command(sync: &SyncConfig, args: &[String], hook_log: &Path) -> Result<Ran, SyncError> {
    let mut cmd = Command::new(&sync.command);
    cmd.args(args)
        .env(HOOK_LOG_ENV, hook_log)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(vc) = &sync.vdirsyncer_config {
        cmd.env("VDIRSYNCER_CONFIG", vc);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Ok(Ran {
                stderr: String::new(),
                outcome: Outcome::SpawnFailed(format!("cannot run {}: {e}", sync.command)),
            });
        }
    };
    let readers = (
        child.stdout.take().map(reader).transpose(),
        child.stderr.take().map(reader).transpose(),
    );
    let (out_rx, err_rx) = match readers {
        (Ok(o), Ok(e)) => (o, e),
        (Err(err), _) | (_, Err(err)) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(err);
        }
    };
    // A timeout too large for `Instant` is ~forever (a year is plenty).
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(sync.timeout_secs))
        .unwrap_or_else(|| Instant::now() + Duration::from_secs(365 * 24 * 3600));
    let outcome = loop {
        if let Some(st) = child.try_wait()? {
            break Outcome::Exited(st);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            child.wait()?;
            break Outcome::TimedOut;
        }
        std::thread::sleep(POLL);
    };
    // After a normal exit, wait for the rest of stderr until the deadline: a run whose
    // output is incomplete must not look clean. After a kill, give it a short grace only.
    let wait = match outcome {
        Outcome::Exited(_) => deadline.saturating_duration_since(Instant::now()),
        _ => Duration::ZERO,
    }
    .max(DRAIN_GRACE);
    if let Some(rx) = out_rx {
        let _ = rx.recv_timeout(wait);
    }
    let (stderr, outcome) = match err_rx.map(|rx| rx.recv_timeout(wait)) {
        None => (String::new(), outcome),
        Some(Ok(s)) => (s, outcome),
        Some(Err(_)) => (String::new(), Outcome::TimedOut),
    };
    Ok(Ran { stderr, outcome })
}
