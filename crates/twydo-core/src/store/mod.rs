//! Crash-safe storage: atomic file writes, the vdir lock, the operation journal with
//! recovery (see `docs/architecture.md`), and the [`Store`] that applies operations through them.
//!
//! An operation runs only when the gate allows it and every file it touches still holds
//! the bytes the caller loaded. It is recorded in the journal before the vdir is touched,
//! so a crash leaves it to be rolled forward by recovery, never half applied.

mod family;
pub mod fsio;
pub mod journal;
mod order;
pub mod plan;
pub mod queue;
mod reparent;
mod retired;

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use crate::compat::{ContractId, Matrix};
use crate::config::{Config, WriteMode};
use crate::edit::{EditError, Op};
use crate::gate::{self, Blocked};
use crate::ical;
use crate::model::meta_edit::MetaJson;
use crate::views::Snapshot;
use fsio::VdirLock;
use journal::{
    ApplyError, Entry, FORMAT, FromQueue, Journal, RequestRef, Stage, Write, apply_entry,
    new_op_id, recover_locked, try_recover,
};
use queue::{Item, Kind, Prepared, Queue};

pub use journal::RecoveryReport;

/// How often a write with a lock wait retries the vdir lock.
const LOCK_RETRY: Duration = Duration::from_millis(25);

/// How long [`Store::submit`] tries the vdir lock before it queues the write.
const TRY_LOCK: Duration = Duration::from_millis(250);

/// The edge case that must pass before a repeating completion can be undone in verified mode.
const UNDO_REPEAT_EDGE: &str = "reopen-undo-before-after-sync";

#[derive(thiserror::Error, Debug)]
pub enum WriteError {
    #[error("{0}")]
    Blocked(Blocked),
    #[error("{0}")]
    Edit(#[from] EditError),
    #[error("{} changed on disk; reload and try again", .0.display())]
    Stale(PathBuf),
    #[error("journal has unfinished operations: {0}")]
    Stuck(String),
    #[error("nothing to undo")]
    NothingToUndo,
    #[error("nothing to redo")]
    NothingToRedo,
    /// The vdir lock is held (a sync is running) and the store was told not to wait.
    #[error("a sync is running; try again in a moment")]
    Busy,
    #[error(transparent)]
    Io(#[from] io::Error),
}

fn io_error(kind: io::ErrorKind, msg: impl Into<String>) -> WriteError {
    WriteError::Io(io::Error::new(kind, msg.into()))
}

/// Name of the identity file in a per-vdir state directory: the canonical vdir path.
pub const VDIR_ID_FILE: &str = "vdir";

/// FNV-1a 64 of `bytes`.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn canonical_vdir(vdir: &Path) -> io::Result<PathBuf> {
    fs::canonicalize(vdir).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("cannot resolve vdir {}: {e}", vdir.display()),
        )
    })
}

fn state_dir_for(state_dir: &Path, canonical: &Path) -> PathBuf {
    let id = fnv1a64(canonical.as_os_str().as_encoded_bytes());
    state_dir.join("vdirs").join(format!("{id:016x}"))
}

fn identity_bytes(canonical: &Path) -> Vec<u8> {
    let mut b = canonical.as_os_str().as_encoded_bytes().to_vec();
    b.push(b'\n');
    b
}

/// Checks the identity file in `dir` names `canonical`. A missing file is accepted.
/// Returns whether the file exists.
fn check_identity(dir: &Path, canonical: &Path) -> io::Result<bool> {
    let found = match fs::read(dir.join(VDIR_ID_FILE)) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    if found == identity_bytes(canonical) {
        return Ok(true);
    }
    let named = String::from_utf8_lossy(&found);
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "state dir {} belongs to vdir {}, not {}; refusing to use its journal",
            dir.display(),
            named.trim_end_matches('\n'),
            canonical.display()
        ),
    ))
}

/// The state directory for `vdir` under `state_dir`: `<state_dir>/vdirs/<id>`, where `<id>`
/// is the 16-hex-digit FNV-1a 64 hash of the canonical vdir path. The journal and the vdir
/// lock live there, so each vdir has its own. Creates nothing; fails if `vdir` cannot be
/// canonicalized.
pub fn vdir_state_dir(state_dir: &Path, vdir: &Path) -> io::Result<PathBuf> {
    Ok(state_dir_for(state_dir, &canonical_vdir(vdir)?))
}

/// For writers: [`vdir_state_dir`], created, with its identity file written if missing.
/// Fails with `InvalidData` if the identity file names another vdir (a hash collision or
/// manual tampering): its journal must not be replayed into this one.
pub fn bind_vdir_state(state_dir: &Path, vdir: &Path) -> io::Result<PathBuf> {
    let canonical = canonical_vdir(vdir)?;
    let dir = state_dir_for(state_dir, &canonical);
    if !check_identity(&dir, &canonical)? {
        fs::create_dir_all(&dir)?;
        fsio::write_atomic(&dir.join(VDIR_ID_FILE), &identity_bytes(&canonical))?;
    }
    Ok(dir)
}

/// For readers: [`vdir_state_dir`] if it exists, else `None`. A vdir that does not exist has
/// no state either (`None`), so the caller reports the unreadable vdir itself. Creates
/// nothing. Fails with `InvalidData` if its identity file names another vdir.
pub fn existing_vdir_state(state_dir: &Path, vdir: &Path) -> io::Result<Option<PathBuf>> {
    let canonical = match canonical_vdir(vdir) {
        Ok(c) => c,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let dir = state_dir_for(state_dir, &canonical);
    if !dir.is_dir() {
        return Ok(None);
    }
    check_identity(&dir, &canonical)?;
    Ok(Some(dir))
}

/// Writes operations to the vdir through the journal, with a session undo/redo stack.
#[derive(Debug)]
pub struct Store {
    vdir: PathBuf,
    state: PathBuf,
    cfg: Config,
    mode: WriteMode,
    matrix: Matrix,
    journal: Journal,
    /// Entries applied (or redone) this session, oldest first; `u` reverts the last.
    done: Vec<Step>,
    /// Undo entries of this session; `Ctrl-r` reverts the last. A new apply clears it.
    undone: Vec<Step>,
    /// Refuse with [`WriteError::Busy`] instead of waiting for the vdir lock.
    lock_wait: Option<Duration>,
    /// Stamped on every entry and queue item this store creates.
    request: Option<RequestRef>,
    /// Plan and queue at once, never trying the vdir lock (a sync is known to be running).
    vdir_busy: bool,
    /// Leave the collection fence to the drain: a list job runs, so an edit is queued even
    /// while its operation is active ([`Store::set_fence_at_drain`]).
    fence_at_drain: bool,
    /// Items this store's drains dealt with, until [`Store::take_drained`]; collected only
    /// while [`Store::set_collect_drained`] is on.
    drained: Vec<Drained>,
    collect_drained: bool,
}

/// What [`Store::submit`] did with a write.
#[derive(Debug)]
// Items carry whole tasks; one is built per write, never in bulk.
#[allow(clippy::large_enum_variant)]
pub enum Submitted {
    /// Committed now; the journal entry id.
    Applied(String),
    /// A sync holds the vdir: waiting in the queue for the next lock holder.
    Queued(Item),
    /// An undo removed a queued item before anything committed it.
    Cancelled,
}

/// One step of the session's undo or redo stack.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Step {
    /// A committed journal entry.
    Applied(String),
    /// An item waiting in the queue (or committed since, which the journal then names).
    Queued(Item),
    /// An item an undo removed from the queue; a redo puts it back.
    Cancelled(Item),
}

/// A queued item a drain dealt with.
#[derive(Debug, Clone)]
pub struct Drained {
    pub item: Item,
    pub outcome: DrainOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrainOutcome {
    /// Committed; the journal entry id.
    Applied(String),
    /// A journal entry already named the item (a crash before its claim was removed).
    AlreadyApplied(String),
    /// Not committed; why.
    Refused(String),
}

/// A drained item and, for a refusal, the error behind it.
struct DrainStep {
    drained: Drained,
    error: Option<WriteError>,
}

impl DrainStep {
    fn new(item: Item, outcome: DrainOutcome) -> DrainStep {
        DrainStep {
            drained: Drained { item, outcome },
            error: None,
        }
    }
}

/// A completion of a repeating task or project: history copies are created and the live
/// task advanced, or, for the last occurrence of an ending series, a repeating task is
/// completed in place (its `RRULE` kept).
fn is_repeat_completion(e: &Entry) -> bool {
    e.kind == "complete"
        && e.writes
            .iter()
            .any(|w| w.before.is_none() || ends_repeat_in_place(w))
}

/// A write that turns an open repeating task into a completed one.
fn ends_repeat_in_place(w: &Write) -> bool {
    let todo = |text: &Option<String>| {
        let cal = crate::ical::parse(text.as_deref()?.as_bytes()).ok()?;
        cal.root.children_named("VTODO").next().cloned()
    };
    let completed = |t: &crate::ical::Component| {
        t.prop("STATUS")
            .is_some_and(|p| p.value.trim().eq_ignore_ascii_case("COMPLETED"))
    };
    match (todo(&w.before), todo(&w.after)) {
        (Some(b), Some(a)) => b.prop("RRULE").is_some() && !completed(&b) && completed(&a),
        _ => false,
    }
}

fn no_entry(id: &str) -> WriteError {
    io_error(io::ErrorKind::NotFound, format!("no journal entry {id}"))
}

/// The JSON `uid` in the metadata of the task with iCalendar UID `parent`, if it has one.
/// A new item's `parentuid` is this value, else the parent's UID.
pub fn parent_json_uid(snap: &Snapshot, parent: &str) -> Option<String> {
    let raw = snap.tasks[snap.find(parent)?].meta.raw.as_deref()?;
    let meta = crate::model::meta_edit::MetaJson::parse(raw)?;
    meta.get_str("uid").map(str::to_string)
}

impl Store {
    /// [`Store::open_with`] the embedded compatibility matrix.
    pub fn open(cfg: &Config) -> Result<(Store, RecoveryReport), WriteError> {
        Store::open_with(cfg, Matrix::embedded().clone())
    }

    /// Runs recovery, then opens the journal. While any entry is stuck, writes return
    /// [`WriteError::Stuck`] until it is [abandoned](Store::abandon).
    pub fn open_with(cfg: &Config, matrix: Matrix) -> Result<(Store, RecoveryReport), WriteError> {
        let vdir = cfg.vdir.clone();
        let state = bind_vdir_state(&cfg.state_dir(), &vdir)?;
        // Recovery takes the lock itself; the store holds none between operations (R2). A sync
        // holding the vdir defers it to the first write, which recovers under the lock anyway.
        let report = try_recover(&vdir, &state)?.unwrap_or_default();
        let journal = Journal::open(&state)?;
        let store = Store {
            vdir,
            state,
            cfg: cfg.clone(),
            mode: cfg.writes.mode,
            matrix,
            journal,
            done: Vec::new(),
            undone: Vec::new(),
            lock_wait: None,
            request: None,
            vdir_busy: false,
            fence_at_drain: false,
            drained: Vec::new(),
            collect_drained: false,
        };
        Ok((store, report))
    }

    /// A store that skips recovery (the caller holds the vdir lock, or recovers itself), with
    /// `matrix`.
    pub fn without_recovery(cfg: &Config, matrix: Matrix) -> Result<Store, WriteError> {
        let vdir = cfg.vdir.clone();
        let state = bind_vdir_state(&cfg.state_dir(), &vdir)?;
        let journal = Journal::open(&state)?;
        Ok(Store {
            vdir,
            state,
            cfg: cfg.clone(),
            mode: cfg.writes.mode,
            matrix,
            journal,
            done: Vec::new(),
            undone: Vec::new(),
            lock_wait: None,
            request: None,
            vdir_busy: false,
            fence_at_drain: false,
            drained: Vec::new(),
            collect_drained: false,
        })
    }

    /// Every entry or queue item this store creates until the next call carries `r`.
    pub fn set_request(&mut self, r: Option<RequestRef>) {
        self.request = r;
    }

    /// While `busy`, `submit`, `undo`, `undo_last` and `redo_last` never try the vdir lock:
    /// they gate, plan and queue at once.
    pub fn set_vdir_busy(&mut self, busy: bool) {
        self.vdir_busy = busy;
    }

    /// While on, planning a write skips the collection fence
    /// ([`crate::lists::operation::ensure_idle`]) and the drain checks it instead: for the
    /// daemon while a list job holds the vdir, so an edit saved during the job is queued and
    /// refused at drain only if the job leaves its operation unfinished. Use with
    /// [`Store::set_vdir_busy`], so nothing commits before the drain.
    pub fn set_fence_at_drain(&mut self, on: bool) {
        self.fence_at_drain = on;
    }

    /// Collect the items this store's drains deal with, for [`Store::take_drained`]. Off by
    /// default: a long-lived store that never takes them (the TUI's) keeps nothing.
    pub fn set_collect_drained(&mut self, on: bool) {
        self.collect_drained = on;
        if !on {
            self.drained.clear();
        }
    }

    /// The items this store's drains dealt with since the last call, oldest first (only
    /// while [`Store::set_collect_drained`] is on; empty otherwise).
    pub fn take_drained(&mut self) -> Vec<Drained> {
        std::mem::take(&mut self.drained)
    }

    /// How long a write waits for the vdir lock. `None` (the default) blocks until it is free
    /// (the CLI). `Some(d)` retries every 25 ms for up to `d`, then returns
    /// [`WriteError::Busy`] (`Some(ZERO)`: no wait). For the TUI, which must not freeze for
    /// the length of a sync.
    pub fn set_lock_wait(&mut self, wait: Option<Duration>) {
        self.lock_wait = wait;
    }

    fn lock(&self) -> Result<VdirLock, WriteError> {
        let Some(wait) = self.lock_wait else {
            return Ok(VdirLock::exclusive(&self.state)?);
        };
        self.try_lock_for(wait)?.ok_or(WriteError::Busy)
    }

    /// The vdir lock, retried every 25 ms for up to `wait`; `None` if it stayed held.
    fn try_lock_for(&self, wait: Duration) -> Result<Option<VdirLock>, WriteError> {
        let deadline = Instant::now() + wait;
        loop {
            if let Some(lock) = VdirLock::try_exclusive(&self.state)? {
                return Ok(Some(lock));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            std::thread::sleep(left.min(LOCK_RETRY));
        }
    }

    /// The per-vdir state directory holding this store's journal and lock.
    pub fn state_dir(&self) -> &Path {
        &self.state
    }

    /// Apply `op` to the tasks loaded in `snap`. Refused unless the gate allows it and every
    /// file it touches still holds the loaded bytes (a new file must not exist yet).
    /// Returns the journal entry id.
    pub fn apply(
        &mut self,
        snap: &Snapshot,
        op: &Op,
        now: DateTime<Utc>,
    ) -> Result<String, WriteError> {
        let prepared = self.prepare(snap, op, now)?;
        let _lock = self.lock()?;
        let id = self.commit_locked(prepared, None, now)?;
        self.done.push(Step::Applied(id.clone()));
        self.undone.clear();
        Ok(id)
    }

    /// The fence check, the gate and the plan: the journal entry `op` produces on the files
    /// loaded in `snap`, with the holds and family it must find unchanged when it is committed.
    pub(crate) fn prepare(
        &self,
        snap: &Snapshot,
        op: &Op,
        now: DateTime<Utc>,
    ) -> Result<Prepared, WriteError> {
        if !self.fence_at_drain {
            crate::lists::operation::ensure_idle(&self.state)?;
        }
        let policy = plan::Policy {
            matrix: &self.matrix,
            mode: self.mode,
        };
        let mut prepared = plan::plan(snap, op, policy, now)?;
        prepared.entry.request = self.request.clone();
        Ok(prepared)
    }

    /// Commits a prepared entry (the caller holds the lock): refused unless the journal is
    /// clear and every file it reads or writes still holds the bytes it was planned on. The
    /// entry gets a fresh id and `now` as its creation time.
    pub(crate) fn commit_locked(
        &mut self,
        p: Prepared,
        from_queue: Option<FromQueue>,
        now: DateTime<Utc>,
    ) -> Result<String, WriteError> {
        self.check_not_stuck()?;
        for w in &p.entry.writes {
            self.check_holds(&w.path, w.before.as_deref())?;
        }
        for (rel, source) in &p.holds {
            self.check_holds(rel, Some(source))?;
        }
        for f in p.family.iter().chain(&p.families) {
            self.check_family(f)?;
        }
        let mut entry = p.entry;
        entry.id = new_op_id(now);
        entry.created = now;
        entry.from_queue = from_queue;
        self.run(entry)
    }

    /// The one write path of the TUI and `tdo op`: applies `op` when the vdir lock is free
    /// and queues it when a sync holds it (spec §6.1). `label` names it in messages and
    /// `tool` (`"tui"` or `"op"`) says who queued it. Never returns [`WriteError::Busy`].
    pub fn submit(
        &mut self,
        view: &Snapshot,
        op: &Op,
        label: &str,
        tool: &str,
        now: DateTime<Utc>,
    ) -> Result<Submitted, WriteError> {
        self.submit_with(view, op, label, tool, now, &mut || {})
    }

    /// [`Store::submit`], calling `after_push` once the item is queued and before the lock is
    /// tried again: a test hook for the lost-wakeup window.
    #[doc(hidden)]
    pub fn submit_with(
        &mut self,
        view: &Snapshot,
        op: &Op,
        label: &str,
        tool: &str,
        now: DateTime<Utc>,
        after_push: &mut dyn FnMut(),
    ) -> Result<Submitted, WriteError> {
        let prepared = self.prepare(view, op, now)?;
        if !self.vdir_busy
            && let Some(lock) = self.try_lock_for(TRY_LOCK)?
        {
            self.drain_locked(&lock, now)?;
            let id = self.commit_locked(prepared, None, now)?;
            self.done.push(Step::Applied(id.clone()));
            self.undone.clear();
            return Ok(Submitted::Applied(id));
        }
        let item = Item {
            format: queue::FORMAT,
            id: queue::new_id(now),
            created: now,
            origin: queue::origin(tool),
            label: label.to_string(),
            request: self.request.clone(),
            kind: Kind::Op {
                op: op.clone(),
                prepared,
            },
        };
        let submitted = self.settle_queued(item, now, after_push)?;
        match &submitted {
            Submitted::Applied(id) => {
                self.done.push(Step::Applied(id.clone()));
                self.undone.clear();
            }
            Submitted::Queued(item) => {
                self.done.push(Step::Queued(item.clone()));
                self.undone.clear();
            }
            Submitted::Cancelled => {}
        }
        Ok(submitted)
    }

    /// Pushes `item`, then tries the lock once and drains if it is free, so an item queued
    /// just as a sync released the lock is not stranded. Reports what became of `item`; the
    /// undo and redo stacks are the caller's business.
    fn settle_queued(
        &mut self,
        item: Item,
        now: DateTime<Utc>,
        after_push: &mut dyn FnMut(),
    ) -> Result<Submitted, WriteError> {
        // `Queue::push` does not check the collection fence; undo and redo never plan
        // through `prepare`, so this is their save-time fence (left to the drain during a
        // daemon's list job, as in `prepare`).
        if !self.fence_at_drain {
            crate::lists::operation::ensure_idle(&self.state)?;
        }
        let queue = Queue::open(&self.state)?;
        queue.push(&item)?;
        after_push();
        // The holder may have released the lock after our try and before the push, and its
        // drain would not have seen the item: look once more.
        if self.vdir_busy {
            return Ok(Submitted::Queued(item));
        }
        let Some(lock) = VdirLock::try_exclusive(&self.state)? else {
            return Ok(Submitted::Queued(item));
        };
        // Anything but a refusal of our own item (stuck journal, I/O) leaves it queued for the
        // next drain: the caller is told it is pending, never "not saved" and applied later.
        let Ok(steps) = self.drain_steps(&lock, now) else {
            return Ok(Submitted::Queued(item));
        };
        if self.collect_drained {
            self.drained.extend(steps.iter().map(|s| s.drained.clone()));
        }
        for step in steps {
            if step.drained.item.id != item.id {
                continue;
            }
            return match (step.drained.outcome, step.error) {
                (DrainOutcome::Applied(id) | DrainOutcome::AlreadyApplied(id), _) => {
                    Ok(Submitted::Applied(id))
                }
                (DrainOutcome::Refused(reason), error) => Err(match error {
                    Some(e) => e,
                    None => WriteError::Blocked(Blocked(reason)),
                }),
            };
        }
        Ok(Submitted::Queued(item))
    }

    /// Commits the queued items, oldest first, once the lock is free. [`WriteError::Busy`]
    /// if a sync still holds it. For the TUI when it learns a sync ended.
    pub fn drain(&mut self, now: DateTime<Utc>) -> Result<Vec<Drained>, WriteError> {
        let lock = self.try_lock_for(TRY_LOCK)?.ok_or(WriteError::Busy)?;
        self.drain_locked(&lock, now)
    }

    /// [`Store::drain_steps`] without the refusal errors.
    pub(crate) fn drain_locked(
        &mut self,
        held: &VdirLock,
        now: DateTime<Utc>,
    ) -> Result<Vec<Drained>, WriteError> {
        let drained: Vec<Drained> = self
            .drain_steps(held, now)?
            .into_iter()
            .map(|s| s.drained)
            .collect();
        if self.collect_drained {
            self.drained.extend(drained.iter().cloned());
        }
        Ok(drained)
    }

    /// Drains the queue under the held lock (spec §6.3): per item, in id order, skip what a
    /// journal entry already names, skip what was cancelled, re-gate on the current files and
    /// commit what was planned. A refused item is recorded and the drain goes on; a stuck
    /// journal or I/O error stops it, leaving the item claimed, and is the error only if
    /// nothing drained.
    fn drain_steps(
        &mut self,
        _held: &VdirLock,
        now: DateTime<Utc>,
    ) -> Result<Vec<DrainStep>, WriteError> {
        if let Err(fence) = crate::lists::operation::ensure_idle(&self.state) {
            return self.refuse_fenced(fence, now);
        }
        let queue = Queue::open(&self.state)?;
        let mut report = crate::vdir::load(&self.vdir)?;
        let mut steps: Vec<DrainStep> = Vec::new();
        for item in queue.list()? {
            if let Some(entry) = self.journal.by_queue_id(&item.id)? {
                let outcome = if entry.stage == Stage::Prepared {
                    // An interrupted commit is not yet applied. Finish recovery before
                    // dropping its claim, and refresh the gate's view for dependent items.
                    if let Err(e) = self.check_not_stuck() {
                        if steps.is_empty() {
                            return Err(e);
                        }
                        break;
                    }
                    report = crate::vdir::load(&self.vdir)?;
                    DrainOutcome::Applied(entry.id)
                } else {
                    DrainOutcome::AlreadyApplied(entry.id)
                };
                queue.done(&item.id)?;
                steps.push(DrainStep::new(item, outcome));
                continue;
            }
            if !queue.claim(&item.id)? {
                continue;
            }
            if queue.is_refused(&item.id)? {
                // A crash inside `Queue::refuse` left both the claim and the refusal.
                queue.done(&item.id)?;
                continue;
            }
            let from_queue = FromQueue {
                id: item.id.clone(),
                queued: item.created,
            };
            let result = match &item.kind {
                Kind::Op { op, prepared } => {
                    let mut prepared = prepared.clone();
                    prepared.entry.request = item.request.clone();
                    let snap = Snapshot::new(report.clone(), self.cfg.clone());
                    let today = item.created.with_timezone(&snap.tz()).date_naive();
                    gate::check(&snap, &self.matrix, self.mode, op, today)
                        .map_err(WriteError::Blocked)
                        .and_then(|()| self.commit_locked(prepared, Some(from_queue), now))
                }
                Kind::Revert { entry, .. } => {
                    self.revert_locked(entry, Some(from_queue), item.request.clone(), now)
                }
            };
            match result {
                Ok(id) => {
                    queue.done(&item.id)?;
                    if let Some(entry) = self.journal.get(&id)? {
                        report.overlay_writes(&self.vdir, &entry.writes);
                    }
                    steps.push(DrainStep::new(item, DrainOutcome::Applied(id)));
                }
                Err(e @ (WriteError::Stuck(_) | WriteError::Io(_))) => {
                    if steps.is_empty() {
                        return Err(e);
                    }
                    break;
                }
                Err(e) => {
                    let reason = e.to_string();
                    let stale = matches!(e, WriteError::Stale(_));
                    queue.refuse(&item, &reason, stale, now)?;
                    steps.push(DrainStep {
                        drained: Drained {
                            item,
                            outcome: DrainOutcome::Refused(reason),
                        },
                        error: Some(e),
                    });
                }
            }
        }
        Ok(steps)
    }

    /// The drain while an unfinished collection operation fences writes: the queued items
    /// that write in its list are refused with the fence's message (they were queued during
    /// the list job: one beginning refuses while its list has queued edits), the others wait
    /// for the operation to be reconciled. The fence's error when nothing was refused.
    fn refuse_fenced(
        &mut self,
        fence: io::Error,
        now: DateTime<Utc>,
    ) -> Result<Vec<DrainStep>, WriteError> {
        let Ok(Some(collection)) = crate::lists::operation::fenced_collection(&self.state) else {
            return Err(fence.into());
        };
        let reason = fence.to_string();
        let queue = Queue::open(&self.state)?;
        let mut steps = Vec::new();
        for item in queue.list()? {
            let in_list = item.writes().iter().any(|w| {
                Path::new(&w.path)
                    .components()
                    .next()
                    .is_some_and(|c| c.as_os_str() == collection.as_str())
            });
            // An item a journal entry names was committed: the next drain finishes it.
            if !in_list || self.journal.by_queue_id(&item.id)?.is_some() {
                continue;
            }
            if !queue.claim(&item.id)? {
                continue;
            }
            queue.refuse(&item, &reason, false, now)?;
            steps.push(DrainStep::new(item, DrainOutcome::Refused(reason.clone())));
        }
        if steps.is_empty() {
            return Err(fence.into());
        }
        Ok(steps)
    }

    /// Writes one file through the journal (kind "resolve") without the gate: it restores a
    /// version that 2Do or Twydo already wrote. `path` is vdir-relative (`<collection>/<file>`).
    /// Refused with [`WriteError::Stale`] unless the file holds `expect` (`None`: must not
    /// exist). `content` `None` deletes the file. Returns the journal entry id.
    pub fn restore(
        &mut self,
        path: &str,
        expect: Option<&str>,
        content: Option<&str>,
        uids: Vec<String>,
        now: DateTime<Utc>,
    ) -> Result<String, WriteError> {
        let plain = Path::new(path).components().count() == 2
            && Path::new(path)
                .components()
                .all(|c| matches!(c, Component::Normal(_)));
        if !plain {
            return Err(io_error(
                io::ErrorKind::InvalidInput,
                format!("{path} is not a <collection>/<file> path"),
            ));
        }
        let entry = Entry {
            format: FORMAT,
            id: new_op_id(now),
            created: now,
            kind: "resolve".to_string(),
            uids,
            stage: Stage::Prepared,
            inverse_of: None,
            from_queue: None,
            request: self.request.clone(),
            synced: None,
            holds: Vec::new(),
            family: None,
            families: Vec::new(),
            writes: vec![Write {
                path: path.to_string(),
                before: expect.map(str::to_string),
                after: content.map(str::to_string),
            }],
        };
        let _lock = self.lock()?;
        self.check_not_stuck()?;
        self.check_holds(path, expect)?;
        self.run(entry)
    }

    /// Revert entry `id` with a new entry (writes swapped and reversed, `inverse_of` = `id`).
    /// Every file must still hold what `id` wrote. Applied now when the lock is free, queued
    /// while a sync holds it.
    pub fn undo(&mut self, id: &str, now: DateTime<Utc>) -> Result<Submitted, WriteError> {
        let done = self.revert_or_queue(id, "op", "undo", now)?;
        self.done
            .retain(|d| !matches!(d, Step::Applied(d) if d == id));
        Ok(done)
    }

    /// Undo the last step applied, queued or redone this session. Waits at most [`TRY_LOCK`]
    /// for the lock, never for a sync: a held lock queues the revert.
    pub fn undo_last(&mut self, now: DateTime<Utc>) -> Result<Submitted, WriteError> {
        while let Some(step) = self.done.last().cloned() {
            match self.revert_step(&step, "undo", now)? {
                Some((done, new)) => {
                    self.done.pop();
                    self.undone.push(new);
                    return Ok(done);
                }
                None => {
                    self.done.pop();
                }
            }
        }
        Err(WriteError::NothingToUndo)
    }

    /// Redo the last undo of this session by reverting it.
    pub fn redo_last(&mut self, now: DateTime<Utc>) -> Result<Submitted, WriteError> {
        while let Some(step) = self.undone.last().cloned() {
            match self.revert_step(&step, "redo", now)? {
                Some((done, new)) => {
                    self.undone.pop();
                    self.done.push(new);
                    return Ok(done);
                }
                None => {
                    self.undone.pop();
                }
            }
        }
        Err(WriteError::NothingToRedo)
    }

    /// Reverts `step`: the result and the step for the other stack. `None` when `step` was a
    /// queued item a drain refused, so nothing of it was ever written.
    fn revert_step(
        &mut self,
        step: &Step,
        label: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<(Submitted, Step)>, WriteError> {
        let done = match step {
            Step::Applied(id) => self.revert_or_queue(id, "tui", label, now)?,
            Step::Queued(item) => {
                let queue = Queue::open(&self.state)?;
                if queue.cancel_by(item, self.request.clone(), now)? {
                    return Ok(Some((Submitted::Cancelled, Step::Cancelled(item.clone()))));
                }
                // A drainer got there first: the entry it commits names the item. Until that
                // entry exists the drainer is mid-commit, unless it refused the item.
                let Some(entry) = self.journal.by_queue_id(&item.id)? else {
                    if queue.is_refused(&item.id)? {
                        return Ok(None);
                    }
                    return Err(WriteError::Busy);
                };
                self.revert_or_queue(&entry.id, "tui", label, now)?
            }
            Step::Cancelled(item) => {
                // The redo is a new request: its item, and the entry drained from it, carry it.
                // A new id too, so the withdrawn item's record keeps answering for it.
                let mut item = item.clone();
                item.id = queue::new_id(now);
                item.request = self.request.clone();
                self.settle_queued(item, now, &mut || {})?
            }
        };
        let new = match &done {
            Submitted::Applied(id) => Step::Applied(id.clone()),
            Submitted::Queued(item) => Step::Queued(item.clone()),
            Submitted::Cancelled => unreachable!("only a queued step is cancelled"),
        };
        Ok(Some((done, new)))
    }

    /// Reverts entry `id` now if the lock is free within [`TRY_LOCK`], else queues the revert.
    /// Its checks run when it commits.
    fn revert_or_queue(
        &mut self,
        id: &str,
        tool: &str,
        label: &str,
        now: DateTime<Utc>,
    ) -> Result<Submitted, WriteError> {
        if !self.vdir_busy
            && let Some(lock) = self.try_lock_for(TRY_LOCK)?
        {
            self.drain_locked(&lock, now)?;
            let request = self.request.clone();
            return Ok(Submitted::Applied(
                self.revert_locked(id, None, request, now)?,
            ));
        }
        let e = self.journal.get(id)?.ok_or_else(|| no_entry(id))?;
        let writes = e
            .writes
            .iter()
            .rev()
            .map(|w| Write {
                path: w.path.clone(),
                before: w.after.clone(),
                after: w.before.clone(),
            })
            .collect();
        let item = Item {
            format: queue::FORMAT,
            id: queue::new_id(now),
            created: now,
            origin: queue::origin(tool),
            label: label.to_string(),
            request: self.request.clone(),
            kind: Kind::Revert {
                entry: id.to_string(),
                writes,
            },
        };
        self.settle_queued(item, now, &mut || {})
    }

    /// Mark the unfinished entry `id` Abandoned so writes can continue. Its files are left
    /// as they are; the entry stays in the journal for manual recovery. Returns `false` when
    /// the entry was already `Abandoned` (nothing changed), `true` when this call abandoned it.
    pub fn abandon(&mut self, id: &str) -> Result<bool, WriteError> {
        let _lock = self.lock()?;
        let e = self.journal.get(id)?.ok_or_else(|| no_entry(id))?;
        if e.stage == Stage::Abandoned {
            return Ok(false);
        }
        if e.stage != Stage::Prepared {
            return Err(io_error(
                io::ErrorKind::InvalidInput,
                format!("operation {id} is not unfinished"),
            ));
        }
        self.journal.set_stage(id, Stage::Abandoned)?;
        Ok(true)
    }

    /// A revert's work under the held lock; `from_queue` names the queued item it
    /// carries out.
    fn revert_locked(
        &self,
        id: &str,
        from_queue: Option<FromQueue>,
        request: Option<RequestRef>,
        now: DateTime<Utc>,
    ) -> Result<String, WriteError> {
        self.check_not_stuck()?;
        let e = self.journal.get(id)?.ok_or_else(|| no_entry(id))?;
        if e.stage != Stage::Applied {
            return Err(io_error(
                io::ErrorKind::InvalidInput,
                format!("operation {id} was not applied"),
            ));
        }
        if self.mode == WriteMode::Verified && is_repeat_completion(&e) {
            let ok = self
                .matrix
                .require(ContractId::CompleteUndo, WriteMode::Verified)
                .is_ok();
            if !ok {
                return Err(WriteError::Blocked(Blocked(format!(
                    "undoing the completion of a repeating task is not verified yet \
                     ({UNDO_REPEAT_EDGE})"
                ))));
            }
        }
        for w in &e.writes {
            self.check_holds(&w.path, w.after.as_deref())?;
        }
        for family in e.family.iter().chain(&e.families) {
            self.check_family(family)?;
        }
        for (rel, content) in &e.holds {
            self.check_holds(rel, Some(content))?;
        }
        let writes = e
            .writes
            .iter()
            .rev()
            .map(|w| Write {
                path: w.path.clone(),
                before: w.after.clone(),
                after: w.before.clone(),
            })
            .collect();
        self.run(Entry {
            format: FORMAT,
            id: new_op_id(now),
            created: now,
            kind: e.kind,
            uids: e.uids,
            stage: Stage::Prepared,
            inverse_of: Some(id.to_string()),
            from_queue,
            request,
            synced: None,
            holds: e.holds,
            family: e.family,
            families: e.families,
            writes,
        })
    }

    /// The journal protocol (the caller holds the lock): prepare, apply, mark applied. A
    /// conflict leaves the entry Prepared, which blocks later writes until it is abandoned.
    fn run(&self, mut e: Entry) -> Result<String, WriteError> {
        // Inverses can recreate resources too. Refuse existing identity collisions
        // before preparing any entry, so a stale undo cannot block unrelated writes.
        journal::check_create_identities(&self.vdir, &e).map_err(|err| match err {
            ApplyError::Conflict { path } => WriteError::Stale(self.vdir.join(path)),
            ApplyError::Io(err) => WriteError::Io(err),
            err => io_error(io::ErrorKind::InvalidInput, err.to_string()),
        })?;
        // The exclusive vdir lock serializes commits across stores and processes. A drain
        // passes the same `now` for every item, so persist a strictly increasing commit
        // time rather than letting reconciliation break ties with random journal ids.
        if let Some(last) = self.journal.list()?.iter().map(|e| e.created).max()
            && last >= e.created
        {
            e.created = last
                .checked_add_signed(chrono::Duration::nanoseconds(1))
                .ok_or_else(|| {
                    io_error(io::ErrorKind::InvalidData, "journal commit time overflow")
                })?;
        }
        self.journal.prepare(&e)?;
        match apply_entry(&self.vdir, &e) {
            Ok(()) => {}
            Err(ApplyError::Io(err)) => return Err(err.into()),
            Err(err) => return Err(WriteError::Stuck(format!("{} ({err})", e.id))),
        }
        self.journal.set_stage(&e.id, Stage::Applied)?;
        Ok(e.id)
    }

    /// Recovery under the held lock; any entry it cannot finish blocks the write.
    fn check_not_stuck(&self) -> Result<(), WriteError> {
        let report = recover_locked(&self.vdir, &self.journal)?;
        if report.stuck.is_empty() {
            return Ok(());
        }
        let list: Vec<String> = report
            .stuck
            .iter()
            .map(|(id, why)| format!("{id} ({why})"))
            .collect();
        Err(WriteError::Stuck(list.join("; ")))
    }

    /// The file at vdir-relative `rel` must hold `want` (`None`: must not exist).
    fn check_holds(&self, rel: &str, want: Option<&str>) -> Result<(), WriteError> {
        let path = self.vdir.join(rel);
        let current = match fs::read(&path) {
            Ok(b) => Some(b),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        if current.as_deref() == want.map(str::as_bytes) {
            Ok(())
        } else {
            Err(WriteError::Stale(path))
        }
    }
}
