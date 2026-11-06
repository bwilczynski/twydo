//! The sync runner: one serialized vdirsyncer run per call, with recovery before it,
//! reconciliation after it, and backoff on transient failures.
//!
//! Locks, always in this order: the run lock `<state>/sync/run.lock`, the vdir lock, the
//! conflicts lock. vdirsyncer runs under the shared vdir lock only (readers continue, writers
//! wait), and never while this process holds the conflicts lock, because the conflict hook
//! takes that lock itself.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use super::SyncError;
use super::hook::{HOOK_LOG_ENV, VDIR_ENV, read_hook_log, uid_of};
use super::lock::LockFile;
use super::manifest::Manifest;
use super::output::{SyncOutput, parse};
use super::reconcile::reconcile;
use super::running::{self, Running, running_path, stop_requested};
use super::status::{CollState, SyncStatus, backoff, coll_states};
use crate::config::{Config, SyncConfig};
use crate::conflicts::{ConflictKind, ConflictStore, NewConflict, Side};
use crate::store::bind_vdir_state;
use crate::store::fsio::{VdirLock, remove_durable};
use crate::store::journal::{Journal, Stage, recover_locked};

/// How `run` was asked to run.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Timer, startup, waiter or batch-full sync: never wait for another runner, honour
    /// backoff.
    pub background: bool,
    /// Collection ids to sync with `--force-delete` (the user's confirmation).
    pub force_delete: Vec<String>,
}

/// Why a run did nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum Skip {
    NotConfigured,
    /// A background run while automatic sync is switched off.
    Off,
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
    /// Held conflicts closed by this run.
    pub resolved: Vec<String>,
    /// Paths that changed in the vdir with no conflict to explain it (left pending).
    pub unexpected: Vec<String>,
    /// Paths the server changed again after their edit synced, confirmed with the server's
    /// version (see [`super::reconcile::Reconciled::superseded`]).
    pub superseded: Vec<String>,
    /// Journal entries and conflicts pruned.
    pub pruned: usize,
    pub forced: Vec<String>,
    /// The part of `forced` that the runner chose itself (a list Twydo emptied).
    pub auto_forced: Vec<String>,
    pub metasync_error: Option<String>,
    /// A pass was stopped on request (see [`super::running::request_stop`]); no rerun
    /// followed it.
    pub stopped: bool,
    /// Problems that did not stop the run (an unreadable or unwritable manifest).
    pub warnings: Vec<String>,
}

/// Grace for the stderr reader threads after the child is gone (a grandchild may still
/// hold the pipe).
const DRAIN_GRACE: Duration = Duration::from_secs(1);
/// How often a running child is polled for exit.
const POLL: Duration = Duration::from_millis(25);
/// A requested rerun happens at most this many times per call.
const MAX_RERUNS: usize = 2;

/// The exclusive lock on `<state>/sync/run.lock` (see [`LockFile::take`]).
pub(crate) fn run_lock(sync_dir: &Path, try_only: bool) -> io::Result<Option<LockFile>> {
    LockFile::take(&sync_dir.join("run.lock"), try_only)
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
/// 5. Under the exclusive vdir lock: reconciles the result with the journal and the
///    conflicts (see [`reconcile`]), then updates the manifest: the files of every counted
///    collection, no entry for one this run may have changed without counting (see
///    [`Manifest::update`]).
/// 6. Classifies each collection and saves `status.json`. A run whose command could not
///    run, timed out, exited non-zero without explanation, printed unattributed errors, or
///    left a collection in `Error`, is a transient failure and backs off.
/// 7. Repeats from 3, at most twice, while `sync/requested` exists.
pub fn run(cfg: &Config, opts: &RunOptions, now: DateTime<Utc>) -> Result<RunReport, SyncError> {
    // Capture before taking run.lock: an orphan stop cannot record backoff while we hold
    // that lock, even if we have not reached recovery yet. Every pass must honor that stop.
    let started = Utc::now();
    let Some(sync) = &cfg.sync else {
        return Ok(skipped(Skip::NotConfigured));
    };
    if opts.background && super::switch::is_off(cfg)? {
        return Ok(skipped(Skip::Off));
    }
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir)?;
    let sync_dir = state.join("sync");
    fs::create_dir_all(&sync_dir)?;
    let requested = sync_dir.join("requested");

    // Step 1: the run lock, held until this function returns.
    let Some(_run_lock) = run_lock(&sync_dir, opts.background)? else {
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

    // Created after the backoff check: skipped runs record no event. Dropped on every exit,
    // including an error from a pass.
    let _ended = EndedGuard {
        state: &state,
        at: now,
    };
    let mut report = RunReport::default();
    let mut force = opts.force_delete.clone();
    for pass in 0..=MAX_RERUNS {
        one_pass(
            cfg,
            sync,
            &state,
            &force,
            now,
            started,
            &mut status,
            &mut report,
        )?;
        force.clear();
        // Step 7.
        // A stopped pass is not rerun: the user stopped the sync so that they can write.
        if report.stopped || pass == MAX_RERUNS || !requested.exists() {
            break;
        }
        fs::remove_file(&requested)?;
    }
    Ok(report)
}

/// Records the sync-ended event when a run that got past the backoff check ends.
struct EndedGuard<'a> {
    state: &'a Path,
    at: DateTime<Utc>,
}

impl Drop for EndedGuard<'_> {
    fn drop(&mut self) {
        let _ = super::ended::write(self.state, self.at);
    }
}

/// Steps 3 to 6.
#[allow(clippy::too_many_arguments)]
fn one_pass(
    cfg: &Config,
    sync: &SyncConfig,
    state: &Path,
    force: &[String],
    now: DateTime<Utc>,
    started: DateTime<Utc>,
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
    let guard = RefCell::new(RunningGuard::start(state, started)?);
    let journal = Journal::open(state)?;
    let pending = journal.pending()?;
    // The evidence is consumed before vdirsyncer runs: read into memory (past that point an
    // error would skip reconciliation), then removed from disk, so a run that does not reach
    // its update in step 5 (killed, failed, or unable to save) leaves none behind. The copy
    // in memory, which predates this run, decides this run's auto-force. An unreadable or
    // unremovable manifest is no evidence (nothing is auto-forced).
    let mut manifest = match Manifest::load(state) {
        Ok(m) => m,
        Err(e) => {
            report.warnings.push(format!(
                "sync/manifest.json is unreadable ({e}); emptied lists will not be applied \
                 automatically until the next sync rewrites it"
            ));
            Manifest::default()
        }
    };
    if let Err(e) = remove_durable(&Manifest::path(state))
        && manifest != Manifest::default()
    {
        report.warnings.push(format!(
            "sync/manifest.json could not be removed before the sync ({e}); emptied lists \
             will not be applied automatically in this run"
        ));
        manifest = Manifest::default();
    }
    fs::write(&hook_log, b"")?;
    let mut args = vec!["sync".to_string()];
    if force.is_empty() {
        args.push(sync.pair.clone());
    } else {
        args.push("--force-delete".into());
        args.extend(force.iter().map(|c| format!("{}/{c}", sync.pair)));
    }
    let child_env = ChildEnv {
        sync,
        hook_log: &hook_log,
        vdir: &cfg.vdir,
        state,
        running: &guard,
    };
    let rediscover = state.join("sync/rediscover");
    let discovery = if rediscover.exists() {
        let mut ran = run_command(&child_env, &["discover".into(), sync.pair.clone()])?;
        let success = matches!(&ran.outcome, Outcome::Exited(st) if st.success())
            && !ran
                .stderr
                .lines()
                .any(|l| l.starts_with("error: ") || l.starts_with("critical: "));
        if success {
            remove_durable(&rediscover)?;
            None
        } else {
            ran.stderr = format!(
                "critical: list discovery failed; backup retained; check vdirsyncer configuration before retrying\n{}",
                ran.stderr
            );
            Some(ran)
        }
    } else {
        None
    };
    let mut ran = match discovery {
        Some(failed) => failed,
        None => run_command(&child_env, &args)?,
    };
    let mut out = parse(&sync.pair, &ran.stderr);
    report.forced.extend(force.iter().cloned());
    // Local emptied by Twydo itself: rerun forced, once, still under the shared lock.
    let auto = auto_force(
        sync,
        &out,
        &pending,
        &manifest,
        ConflictStore::existing(state)?,
        force,
    )?;
    if !auto.is_empty() && matches!(ran.outcome, Outcome::Exited(_)) {
        let mut args = vec!["sync".to_string(), "--force-delete".to_string()];
        args.extend(auto.iter().map(|c| format!("{}/{c}", sync.pair)));
        let second = run_command(&child_env, &args)?;
        merge_forced(&mut out, parse(&sync.pair, &second.stderr), &auto);
        ran.outcome = second.outcome;
        report.forced.extend(auto.iter().cloned());
        report.auto_forced.extend(auto);
    }
    // Not after a failed sync: a failure that repeats (no password, no network) would make
    // metasync wait the same way while writers still wait for the lock.
    let sync_failed = || {
        let states = coll_states(&out, &[], &sync.local_storage);
        transient(sync, &ran, &out, &states).is_some()
    };
    report.metasync_error = match ran.outcome {
        Outcome::Exited(_) if sync.metasync && !sync_failed() => metasync(&child_env)?,
        _ => None,
    };
    let hook = read_hook_log(&hook_log)?;
    let pass_stopped = guard.borrow().stopped;
    report.stopped |= pass_stopped;
    drop(guard);
    drop(shared);

    let mut collections = coll_states(&out, &hook, &sync.local_storage);
    let killed = match ran.outcome {
        Outcome::TimedOut => Some("sync timed out"),
        Outcome::Stopped => Some("sync stopped"),
        _ => None,
    };
    if let Some(message) = killed {
        // A killed run may have stopped in the middle of any collection it reported,
        // conflicted ones included (reconciliation counts those too).
        for s in collections.values_mut() {
            if matches!(s, CollState::Ok | CollState::Conflict) {
                *s = CollState::Error {
                    message: message.into(),
                };
            }
        }
    }

    // Step 5: reconcile under the exclusive lock (the conflicts lock nests inside it).
    {
        let _excl = VdirLock::exclusive(state)?;
        let r = reconcile(
            &cfg.vdir,
            &journal,
            &ConflictStore::open(state)?,
            sync,
            &collections,
            &pending,
            &out,
            &hook,
            now,
        )?;
        report.confirmed.extend(r.confirmed);
        report.new_conflicts.extend(r.new_conflicts);
        report.resolved.extend(r.resolved);
        report.unexpected.extend(r.unexpected);
        report.superseded.extend(r.superseded);
        report.pruned += r.pruned;
        // The evidence a later auto-force needs (see [`Manifest::update`]). A failure here
        // is a warning, so the status below is still saved; the file was removed before
        // vdirsyncer ran, so no stale evidence is left behind.
        let uncertain = !out.unattributed.is_empty() || killed.is_some();
        if let Err(e) = manifest
            .update(&cfg.vdir, &collections, uncertain)
            .and_then(|()| manifest.save(state))
        {
            report.warnings.push(format!(
                "sync/manifest.json could not be written ({e}); emptied lists will not be \
                 applied automatically until a sync writes it"
            ));
        }
    }

    // Step 6: status.
    // A pass stopped on request backs off even when only metasync was stopped: the next
    // attempt would most likely get stuck the same way.
    let transient = transient(sync, &ran, &out, &collections)
        .or_else(|| pass_stopped.then(|| "sync stopped".to_string()));
    status.last_attempt = Some(now);
    status.collections = collections.clone();
    status.metasync_error = report.metasync_error.clone();
    match &transient {
        Some(msg) => record_failure(status, sync, msg, now),
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

/// Counts a transient failure in `status` and sets its backoff.
fn record_failure(status: &mut SyncStatus, sync: &SyncConfig, msg: &str, now: DateTime<Utc>) {
    status.failures = status.failures.saturating_add(1);
    let delay = backoff(status.failures, Duration::from_secs(sync.max_backoff_secs));
    let delay = chrono::Duration::from_std(delay).unwrap_or(chrono::Duration::MAX);
    status.next_attempt = now.checked_add_signed(delay);
    status.last_error = Some(msg.lines().next().unwrap_or_default().to_string());
}

/// Records a sync whose runner was gone and whose vdirsyncer [`super::running::request_stop`]
/// killed: nobody else will, and without a backoff the sync after the write could start another
/// stuck run at once. Under the run lock, like every status write; when another runner holds
/// it, its run began before the stop, so it stops before vdirsyncer (`sync/last_stop`) and
/// records the stop itself.
pub(crate) fn record_orphan_stop(cfg: &Config, state: &Path) -> io::Result<()> {
    let Some(sync) = &cfg.sync else {
        return Ok(());
    };
    let sync_dir = state.join("sync");
    fs::create_dir_all(&sync_dir)?;
    let Some(_run_lock) = run_lock(&sync_dir, true)? else {
        return Ok(());
    };
    let mut status = SyncStatus::load(state)?;
    let now = Utc::now();
    status.last_attempt = Some(now);
    record_failure(&mut status, sync, "sync stopped", now);
    status.save(state)
}

/// Collections to force-delete automatically: emptied on the local side, not forced
/// explicitly, with no unresolved conflict, and whose every file at the last counted run
/// (the manifest) a pending entry removed (see [`Manifest::emptied_by`]). An emptied
/// collection the journal does not fully account for (files removed by hand, a broken
/// vdir, no Twydo sync yet) is never forced.
fn auto_force(
    sync: &SyncConfig,
    out: &SyncOutput,
    pending: &[crate::store::journal::Entry],
    manifest: &Manifest,
    conflicts: Option<ConflictStore>,
    explicit: &[String],
) -> Result<Vec<String>, SyncError> {
    let unresolved = match conflicts {
        Some(store) => store.unresolved()?,
        None => Vec::new(),
    };
    let states = coll_states(out, &[], &sync.local_storage);
    Ok(out
        .emptied
        .keys()
        .filter(|c| states.get(*c) == Some(&CollState::EmptiedLocal))
        .filter(|c| !explicit.contains(c))
        .filter(|c| !unresolved.iter().any(|u| &u.collection == *c))
        .filter(|c| manifest.emptied_by(c, pending))
        .cloned()
        .collect())
}

/// Replaces what `first` says about the `forced` collections with what `second` says.
fn merge_forced(first: &mut SyncOutput, second: SyncOutput, forced: &[String]) {
    for c in forced {
        first.synced.remove(c);
        first.emptied.remove(c);
        first.errors.remove(c);
        first.unhooked.remove(c);
    }
    first.transfers.retain(|t| !forced.contains(&t.collection));
    first.conflict_items.extend(second.conflict_items);
    first.synced.extend(second.synced);
    first.transfers.extend(second.transfers);
    first.unhooked.extend(second.unhooked);
    first.emptied.extend(second.emptied);
    first.errors.extend(second.errors);
    first.unattributed.extend(second.unattributed);
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
        Outcome::Stopped => return Some("sync stopped".into()),
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

/// Step 3: rolls Prepared entries forward. For a stuck entry, every write whose file does
/// not hold its `after` (it holds the `before`, or a third version) becomes a `Journal`
/// conflict (the vdir keeps the file as it is), and the entry is abandoned. Writes already
/// applied stay. Entries that cannot be read or have an invalid path are left stuck. Returns
/// the new conflict ids.
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
            // An applied write stays. One not applied (the file still holds `before`) is
            // recorded too, so abandoning the entry drops none of its writes unseen.
            if holds(&w.after) {
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
fn metasync(env: &ChildEnv) -> Result<Option<String>, SyncError> {
    let sync = env.sync;
    let args = ["metasync".to_string(), sync.pair.clone()];
    let ran = run_command(env, &args)?;
    // The first `error:`/`critical:` line (progress lines like `Metasyncing …` come first),
    // else the first non-empty line.
    let message = || {
        let lines = || ran.stderr.lines().map(str::trim).filter(|l| !l.is_empty());
        lines()
            .find_map(|l| {
                l.strip_prefix("error: ")
                    .or_else(|| l.strip_prefix("critical: "))
            })
            .or_else(|| lines().next())
            .map(|l| l.trim().to_string())
    };
    Ok(match &ran.outcome {
        Outcome::Exited(st) if st.success() => None,
        Outcome::Exited(st) => {
            Some(message().unwrap_or_else(|| format!("metasync exited with {st}")))
        }
        Outcome::TimedOut => Some(format!("metasync timed out after {} s", sync.timeout_secs)),
        Outcome::Stopped => Some("metasync stopped".into()),
        Outcome::SpawnFailed(msg) => Some(msg.clone()),
    })
}

enum Outcome {
    Exited(ExitStatus),
    TimedOut,
    /// Killed on request (`sync/stop`).
    Stopped,
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

/// Kills `child` and, on Unix, its whole process group (it leads one: see
/// [`run_command`]), so a wrapper's vdirsyncer cannot keep writing the vdir after the
/// runner gives up its shared lock. No libc here: the group goes through `kill(1)` (see
/// [`running::kill_group`]); the direct kill is the fallback.
fn kill_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    running::kill_group(child.id());
    let _ = child.kill();
}

/// What every vdirsyncer child of one pass gets.
struct ChildEnv<'a> {
    sync: &'a SyncConfig,
    hook_log: &'a Path,
    vdir: &'a Path,
    state: &'a Path,
    running: &'a RefCell<RunningGuard>,
}

/// Keeps `sync/running.json` for the pass that created it, and removes it on drop.
struct RunningGuard {
    state: PathBuf,
    rec: Running,
    /// A child of this pass was killed on request.
    stopped: bool,
}

impl RunningGuard {
    /// Records the pass of the run begun at `started`, with no child yet.
    fn start(state: &Path, started: DateTime<Utc>) -> io::Result<RunningGuard> {
        let g = RunningGuard {
            state: state.to_path_buf(),
            rec: Running {
                runner_pid: std::process::id(),
                runner_start: running::process_start(std::process::id()),
                child_pgid: None,
                child_start: None,
                started,
            },
            stopped: false,
        };
        g.save()?;
        Ok(g)
    }

    fn set_child(&mut self, pgid: Option<u32>) -> io::Result<()> {
        self.rec.child_pgid = pgid;
        self.rec.child_start = pgid.and_then(running::process_start);
        self.save()
    }

    /// Written whole through a rename, so a reader never sees half a record.
    fn save(&self) -> io::Result<()> {
        let path = running_path(&self.state);
        let tmp = path.with_extension("json.tmp");
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(&self.rec).map_err(io::Error::other)?,
        )?;
        fs::rename(&tmp, &path)
    }
}

impl Drop for RunningGuard {
    fn drop(&mut self) {
        let _ = remove_if_exists(&running_path(&self.state));
    }
}

fn remove_if_exists(p: &Path) -> io::Result<()> {
    match fs::remove_file(p) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Runs `<command> args…` with `VDIRSYNCER_CONFIG` (when configured), the hook log and the
/// vdir in the environment. Its stdin is a shared lock on the vdir of its own (the lock file
/// is empty, so it reads EOF as from null): the kernel keeps that lock until the child and
/// whatever inherited it exit, so a child orphaned by a killed runner (Ctrl-C) still keeps
/// writers and the next run's recovery waiting. stdout is drained and dropped; stderr is
/// returned. The child is killed after `timeout_secs`, or when a stop is requested (see
/// [`running`]); with a stop already pending it is not started.
fn run_command(env: &ChildEnv, args: &[String]) -> Result<Ran, SyncError> {
    let sync = env.sync;
    let since = env.running.borrow().rec.started;
    if stop_requested(env.state, since)? {
        env.running.borrow_mut().stopped = true;
        return Ok(Ran {
            stderr: String::new(),
            outcome: Outcome::Stopped,
        });
    }
    let lock = VdirLock::shared_for_child(env.state)?;
    let mut cmd = Command::new(&sync.command);
    cmd.args(args)
        .env(HOOK_LOG_ENV, env.hook_log)
        .env(VDIR_ENV, env.vdir)
        .stdin(Stdio::from(lock))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(vc) = &sync.vdirsyncer_config {
        cmd.env("VDIRSYNCER_CONFIG", vc);
    }
    // Its own process group, so a timeout also kills what a wrapper script started. A
    // Ctrl-C at the terminal therefore reaches only tdo; the stdin lock covers that.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Ok(Ran {
                stderr: String::new(),
                outcome: Outcome::SpawnFailed(format!("cannot run {}: {e}", sync.command)),
            });
        }
    };
    // Best effort: without the record the sync cannot be stopped from elsewhere, but it runs.
    let _ = env.running.borrow_mut().set_child(Some(child.id()));
    let readers = (
        child.stdout.take().map(reader).transpose(),
        child.stderr.take().map(reader).transpose(),
    );
    let (out_rx, err_rx) = match readers {
        (Ok(o), Ok(e)) => (o, e),
        (Err(err), _) | (_, Err(err)) => {
            kill_tree(&mut child);
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
        // An unreadable request counts as none: returning here would leave the child
        // running with no record of it.
        if stop_requested(env.state, since).unwrap_or(false) {
            kill_tree(&mut child);
            child.wait()?;
            env.running.borrow_mut().stopped = true;
            break Outcome::Stopped;
        }
        if Instant::now() >= deadline {
            kill_tree(&mut child);
            child.wait()?;
            break Outcome::TimedOut;
        }
        std::thread::sleep(POLL);
    };
    let _ = env.running.borrow_mut().set_child(None);
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
