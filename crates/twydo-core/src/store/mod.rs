//! Crash-safe storage: atomic file writes, the vdir lock, the operation journal with
//! recovery (see `docs/architecture.md`), and the [`Store`] that applies operations through them.
//!
//! An operation runs only when the gate allows it and every file it touches still holds
//! the bytes the caller loaded. It is recorded in the journal before the vdir is touched,
//! so a crash leaves it to be rolled forward by recovery, never half applied.

mod family;
pub mod fsio;
pub mod history;
pub mod journal;
mod order;
pub mod plan;
pub mod queue;
mod reparent;
mod retired;
pub mod writer;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::compat::Matrix;
use crate::config::Config;
use crate::edit::{EditError, Op};
use crate::gate::Blocked;
use crate::ical;
use crate::model::meta_edit::MetaJson;
use crate::views::Snapshot;
use journal::{Entry, FORMAT, RequestRef, Stage, new_op_id, try_recover};
use queue::{Item, Kind, Prepared};

pub use history::{Access, History, Step, StepState};
pub use journal::RecoveryReport;
pub use writer::{Ctx, Fence, Locked, Writer};

/// How long [`Store::submit`] tries the vdir lock before it queues the write.
const TRY_LOCK: Duration = Duration::from_millis(250);

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

/// Writes operations to the vdir through the [`Writer`], with a session undo/redo stack.
#[derive(Debug)]
pub struct Store {
    cfg: Config,
    writer: Writer,
    /// The steps applied, queued or undone this session.
    history: History,
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
pub(crate) struct DrainStep {
    pub(crate) drained: Drained,
    pub(crate) error: Option<WriteError>,
}

impl DrainStep {
    pub(crate) fn new(item: Item, outcome: DrainOutcome) -> DrainStep {
        DrainStep {
            drained: Drained { item, outcome },
            error: None,
        }
    }
}

pub(crate) fn no_entry(id: &str) -> WriteError {
    io_error(io::ErrorKind::NotFound, format!("no journal entry {id}"))
}

/// The JSON `uid` in the metadata of the task with iCalendar UID `parent`, if it has one.
/// A new item's `parentuid` is this value, else the parent's UID.
pub fn parent_json_uid(snap: &Snapshot, parent: &str) -> Option<String> {
    let raw = snap.tasks[snap.find(parent)?].meta.raw.as_deref()?;
    let meta = crate::model::meta_edit::MetaJson::parse(raw)?;
    meta.get_str("uid").map(str::to_string)
}

/// A [`Ctx`] for a call at `now`, built from the store's own fields so a held [`Locked`]
/// (which borrows only the writer) does not block the store's other fields.
macro_rules! ctx {
    ($s:expr, $now:expr) => {
        Ctx {
            now: $now,
            request: $s.request.clone(),
            config: &$s.cfg,
        }
    };
}

/// What became of `item` among the `steps` a drain dealt with: committed, refused (the error
/// behind it) or still queued.
pub(crate) fn drained_outcome(steps: Vec<DrainStep>, item: Item) -> Result<Submitted, WriteError> {
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

/// The vdir lock as the store waits for it: blocking when `wait` is `None`, else retried
/// for up to `wait` and [`WriteError::Busy`] if it stays held.
fn lock_for(writer: &Writer, wait: Option<Duration>) -> Result<Locked<'_>, WriteError> {
    match wait {
        None => writer.lock_waiting(),
        Some(wait) => writer.lock(wait)?.ok_or(WriteError::Busy),
    }
}

/// Keeps `items` in `sink` while collection is on.
fn keep(sink: &mut Vec<Drained>, on: bool, items: &[Drained]) {
    if on {
        sink.extend(items.iter().cloned());
    }
}

impl Store {
    /// [`Store::open_with`] the embedded compatibility matrix.
    pub fn open(cfg: &Config) -> Result<(Store, RecoveryReport), WriteError> {
        Store::open_with(cfg, Matrix::embedded().clone())
    }

    /// Runs recovery, then opens the journal. While any entry is stuck, writes return
    /// [`WriteError::Stuck`] until it is [abandoned](Store::abandon).
    pub fn open_with(cfg: &Config, matrix: Matrix) -> Result<(Store, RecoveryReport), WriteError> {
        // Recovery takes the lock itself; the store holds none between operations (R2). A sync
        // holding the vdir defers it to the first write, which recovers under the lock anyway.
        // It runs before the writer opens the journal: opening creates `journal/`, and a
        // state directory without one is skipped by recovery.
        let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir)?;
        let report = try_recover(&cfg.vdir, &state)?.unwrap_or_default();
        let writer = Writer::open(cfg, matrix)?;
        Ok((Store::shell(cfg, writer), report))
    }

    /// A store that skips recovery (the caller holds the vdir lock, or recovers itself), with
    /// `matrix`.
    pub fn without_recovery(cfg: &Config, matrix: Matrix) -> Result<Store, WriteError> {
        Ok(Store::shell(cfg, Writer::open(cfg, matrix)?))
    }

    fn shell(cfg: &Config, writer: Writer) -> Store {
        Store {
            cfg: cfg.clone(),
            writer,
            history: History::new(),
            lock_wait: None,
            request: None,
            vdir_busy: false,
            fence_at_drain: false,
            drained: Vec::new(),
            collect_drained: false,
        }
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

    /// The per-vdir state directory holding this store's journal and lock.
    pub fn state_dir(&self) -> &Path {
        self.writer.state_dir()
    }

    fn fence(&self) -> Fence {
        if self.fence_at_drain {
            Fence::AtDrain
        } else {
            Fence::Now
        }
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
        let lock = lock_for(&self.writer, self.lock_wait)?;
        let id = self.commit(&lock, prepared, now)?;
        self.history
            .record(String::new(), &Submitted::Applied(id.clone()));
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
            crate::lists::operation::ensure_idle(self.writer.state_dir())?;
        }
        let mut prepared = self.writer.plan(snap, op, now)?;
        prepared.entry.request = self.request.clone();
        Ok(prepared)
    }

    /// Commits a prepared entry under `lock`. The collection fence was checked when it was
    /// prepared (or is left to the drain, [`Store::set_fence_at_drain`]).
    fn commit(
        &self,
        lock: &Locked<'_>,
        mut p: Prepared,
        now: DateTime<Utc>,
    ) -> Result<String, WriteError> {
        let ctx = ctx!(self, now);
        if self.fence_at_drain {
            p.entry.request = ctx.request.clone();
            lock.commit_inner(p, None, now)
        } else {
            lock.commit(p, &ctx)
        }
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
            && let Some(lock) = self.writer.lock(TRY_LOCK)?
        {
            let drained = lock.drain(&ctx!(self, now))?;
            keep(&mut self.drained, self.collect_drained, &drained);
            let id = self.commit(&lock, prepared, now)?;
            let submitted = Submitted::Applied(id);
            self.history.record(String::new(), &submitted);
            return Ok(submitted);
        }
        let item = self.writer.enqueue(
            Kind::Op {
                op: op.clone(),
                prepared,
            },
            label,
            tool,
            self.fence(),
            &ctx!(self, now),
        )?;
        let submitted = self.settle_pushed(item, now, after_push)?;
        self.history.record(String::new(), &submitted);
        Ok(submitted)
    }

    /// For an `item` already pushed: tries the lock once and drains if it is free, so an item
    /// queued just as a sync released the lock is not stranded. Reports what became of `item`;
    /// the undo and redo stacks are the caller's business.
    fn settle_pushed(
        &mut self,
        item: Item,
        now: DateTime<Utc>,
        after_push: &mut dyn FnMut(),
    ) -> Result<Submitted, WriteError> {
        after_push();
        // The holder may have released the lock after our try and before the push, and its
        // drain would not have seen the item: look once more.
        if self.vdir_busy {
            return Ok(Submitted::Queued(item));
        }
        let Some(lock) = self.writer.lock(Duration::ZERO)? else {
            return Ok(Submitted::Queued(item));
        };
        // Anything but a refusal of our own item (stuck journal, I/O) leaves it queued for the
        // next drain: the caller is told it is pending, never "not saved" and applied later.
        let Ok(steps) = lock.drain_steps(&ctx!(self, now)) else {
            return Ok(Submitted::Queued(item));
        };
        if self.collect_drained {
            self.drained.extend(steps.iter().map(|s| s.drained.clone()));
        }
        drained_outcome(steps, item)
    }

    /// Commits the queued items, oldest first, once the lock is free. [`WriteError::Busy`]
    /// if a sync still holds it. For the TUI when it learns a sync ended.
    pub fn drain(&mut self, now: DateTime<Utc>) -> Result<Vec<Drained>, WriteError> {
        let lock = self.writer.lock(TRY_LOCK)?.ok_or(WriteError::Busy)?;
        let drained = lock.drain(&ctx!(self, now))?;
        keep(&mut self.drained, self.collect_drained, &drained);
        Ok(drained)
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
        writer::check_plain_path(path)?;
        let lock = lock_for(&self.writer, self.lock_wait)?;
        lock.restore(path, expect, content, uids, &ctx!(self, now))
    }

    /// Revert entry `id` with a new entry (writes swapped and reversed, `inverse_of` = `id`).
    /// Every file must still hold what `id` wrote. Applied now when the lock is free, queued
    /// while a sync holds it.
    pub fn undo(&mut self, id: &str, now: DateTime<Utc>) -> Result<Submitted, WriteError> {
        let done = self.revert_or_queue(id, "op", "undo", now)?;
        self.history.forget_applied(id);
        Ok(done)
    }

    /// Undo the last step applied, queued or redone this session. Waits at most [`TRY_LOCK`]
    /// for the lock, never for a sync: a held lock queues the revert.
    pub fn undo_last(&mut self, now: DateTime<Utc>) -> Result<Submitted, WriteError> {
        self.step_history(true, now)
    }

    /// Redo the last undo of this session by reverting it.
    pub fn redo_last(&mut self, now: DateTime<Utc>) -> Result<Submitted, WriteError> {
        self.step_history(false, now)
    }

    /// [`History::undo`] (`undo`) or [`History::redo`]: under the lock when it is free within
    /// [`TRY_LOCK`], else by queueing the revert and looking at the lock once more, so a revert
    /// queued just as a sync released it is not stranded.
    fn step_history(&mut self, undo: bool, now: DateTime<Utc>) -> Result<Submitted, WriteError> {
        let ctx = ctx!(self, now);
        let run = |h: &mut History, access: Access<'_>| {
            if undo {
                h.undo(access, "tui", &ctx)
            } else {
                h.redo(access, "tui", &ctx)
            }
        };
        if !self.vdir_busy
            && let Some(lock) = self.writer.lock(TRY_LOCK)?
        {
            let result = run(&mut self.history, Access::Locked(&lock));
            let own = self.history.take_drained();
            keep(&mut self.drained, self.collect_drained, &own);
            return result.map(|(done, _)| done);
        }
        let access = Access::Queue(&self.writer, self.fence());
        let (done, moved) = run(&mut self.history, access)?;
        let Submitted::Queued(item) = done else {
            return Ok(done);
        };
        match self.settle_pushed(item.clone(), now, &mut || {}) {
            Ok(Submitted::Applied(id)) => {
                self.history.settled(&item.id, &id);
                Ok(Submitted::Applied(id))
            }
            Ok(other) => Ok(other),
            Err(e) => {
                self.history.refused(undo, &item.id, moved);
                Err(e)
            }
        }
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
            && let Some(lock) = self.writer.lock(TRY_LOCK)?
        {
            let drained = lock.drain(&ctx!(self, now))?;
            keep(&mut self.drained, self.collect_drained, &drained);
            return Ok(Submitted::Applied(lock.revert(id, &ctx!(self, now))?));
        }
        let kind = self.writer.revert_item(id)?;
        let item = self
            .writer
            .enqueue(kind, label, tool, self.fence(), &ctx!(self, now))?;
        self.settle_pushed(item, now, &mut || {})
    }

    /// Mark the unfinished entry `id` Abandoned so writes can continue. Its files are left
    /// as they are; the entry stays in the journal for manual recovery. Returns `false` when
    /// the entry was already `Abandoned` (nothing changed), `true` when this call abandoned it.
    pub fn abandon(&mut self, id: &str) -> Result<bool, WriteError> {
        lock_for(&self.writer, self.lock_wait)?.abandon(id)
    }
}
