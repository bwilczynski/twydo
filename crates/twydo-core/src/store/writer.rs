//! The single vdir writer: the journal and the queue behind one `&self` value with no
//! per-call state. Everything that touches the vdir is a method of [`Locked`], which exists
//! only while the exclusive vdir lock is held; per-call values (the time, the request, the
//! live configuration) arrive in a [`Ctx`].

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use super::fsio::VdirLock;
use super::journal::{
    ApplyError, Entry, FORMAT, FromQueue, Journal, RecoveryReport, RequestRef, Stage, Write,
    apply_entry, new_op_id, recover_locked,
};
use super::plan::{self, Policy};
use super::queue::{self, Item, Kind, Prepared, Queue};
use super::{DrainOutcome, DrainStep, Drained, WriteError, bind_vdir_state, io_error, no_entry};
use crate::compat::{ContractId, Matrix};
use crate::config::{Config, WriteMode};
use crate::edit::Op;
use crate::gate::{self, Blocked};
use crate::views::Snapshot;

/// How often a lock wait retries the vdir lock.
const LOCK_RETRY: Duration = Duration::from_millis(25);

/// The edge case that must pass before a repeating completion can be undone in verified mode.
const UNDO_REPEAT_EDGE: &str = "reopen-undo-before-after-sync";

/// The values that belong to one call, not to the writer.
pub struct Ctx<'a> {
    /// The time stamped on what the call creates.
    pub now: DateTime<Utc>,
    /// The client request behind the call, stamped on entries and queue items.
    pub request: Option<RequestRef>,
    /// The live configuration a drain gates on (its time zone).
    pub config: &'a Config,
}

/// When the collection-operation fence is checked for a queued item.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fence {
    /// At enqueue.
    Now,
    /// Left to the drain.
    AtDrain,
}

/// Owns the journal and the queue of one vdir. Holds no per-call state.
#[derive(Debug)]
pub struct Writer {
    pub(super) vdir: PathBuf,
    pub(super) state: PathBuf,
    pub(super) mode: WriteMode,
    pub(super) matrix: Matrix,
    pub(super) journal: Journal,
}

const _: fn() = || {
    fn sync<T: Sync>() {}
    sync::<Writer>();
};

// Held only so the lock is released when the `Locked` drops.
#[allow(dead_code)]
enum Held<'a> {
    Owned(VdirLock),
    Borrowed(&'a VdirLock),
}

/// The writer with the exclusive vdir lock held: the only way to touch the vdir.
pub struct Locked<'a> {
    writer: &'a Writer,
    _held: Held<'a>,
}

impl Writer {
    /// Binds the vdir's state directory and opens its journal. Runs no recovery and takes no
    /// lock.
    pub fn open(cfg: &Config, matrix: Matrix) -> Result<Writer, WriteError> {
        let vdir = cfg.vdir.clone();
        let state = bind_vdir_state(&cfg.state_dir(), &vdir)?;
        let journal = Journal::open(&state)?;
        Ok(Writer {
            vdir,
            state,
            mode: cfg.writes.mode,
            matrix,
            journal,
        })
    }

    /// The write policy operations are gated and planned under.
    pub fn policy(&self) -> Policy<'_> {
        Policy {
            matrix: &self.matrix,
            mode: self.mode,
        }
    }

    /// The per-vdir state directory holding the journal, the queue and the lock.
    pub fn state_dir(&self) -> &Path {
        &self.state
    }

    /// The gate and the plan of `op` on `snap` under this writer's vdir and policy: no I/O and
    /// no fence check. The entry carries no request.
    pub fn plan(
        &self,
        snap: &Snapshot,
        op: &Op,
        now: DateTime<Utc>,
    ) -> Result<Prepared, WriteError> {
        plan::plan_in(&self.vdir, snap, op, self.policy(), now)
    }

    /// The vdir lock, retried every 25 ms for up to `wait`; `None` if it stayed held.
    pub fn lock(&self, wait: Duration) -> Result<Option<Locked<'_>>, WriteError> {
        let deadline = Instant::now() + wait;
        loop {
            if let Some(lock) = VdirLock::try_exclusive(&self.state)? {
                return Ok(Some(Locked {
                    writer: self,
                    _held: Held::Owned(lock),
                }));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            std::thread::sleep(left.min(LOCK_RETRY));
        }
    }

    /// The vdir lock, blocking until it is free.
    pub fn lock_waiting(&self) -> Result<Locked<'_>, WriteError> {
        Ok(Locked {
            writer: self,
            _held: Held::Owned(VdirLock::exclusive(&self.state)?),
        })
    }

    /// A lock the caller already holds (the sync runner's). `held` must be the exclusive lock
    /// of this writer's state directory.
    pub(crate) fn with_lock<'a>(&'a self, held: &'a VdirLock) -> Locked<'a> {
        Locked {
            writer: self,
            _held: Held::Borrowed(held),
        }
    }

    /// Queues `kind`: builds the item from `ctx`, checks the collection fence when `fence` is
    /// [`Fence::Now`], and pushes it.
    pub fn enqueue(
        &self,
        kind: Kind,
        label: &str,
        origin: &str,
        fence: Fence,
        ctx: &Ctx<'_>,
    ) -> Result<Item, WriteError> {
        let item = Item {
            format: queue::FORMAT,
            id: queue::new_id(ctx.now),
            created: ctx.now,
            origin: queue::origin(origin),
            label: label.to_string(),
            request: ctx.request.clone(),
            kind,
        };
        self.push(&item, fence)?;
        Ok(item)
    }

    /// The collection fence for a write about to be queued or applied: refuses while a
    /// collection operation is unfinished when `fence` is [`Fence::Now`]; [`Fence::AtDrain`]
    /// leaves it to the drain. The one implementation `enqueue` and `push` share; a caller
    /// calls it before [`Writer::plan`] so the fence is reported before the gate.
    pub fn check_fence(&self, fence: Fence) -> Result<(), WriteError> {
        if fence == Fence::Now {
            crate::lists::operation::ensure_idle(&self.state)?;
        }
        Ok(())
    }

    /// Pushes a ready item, keeping its `created`, `origin` and `label` (which
    /// [`Writer::enqueue`] would overwrite); `History` re-queues a cancelled item with it. The
    /// fence is checked by [`Writer::check_fence`] (`Queue::push` does not check it).
    pub(super) fn push(&self, item: &Item, fence: Fence) -> Result<(), WriteError> {
        self.check_fence(fence)?;
        Queue::open(&self.state)?.push(item)?;
        Ok(())
    }

    /// The queued form of reverting journal entry `entry`.
    pub fn revert_item(&self, entry: &str) -> Result<Kind, WriteError> {
        let e = self.journal.get(entry)?.ok_or_else(|| no_entry(entry))?;
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
        Ok(Kind::Revert {
            entry: entry.to_string(),
            writes,
        })
    }

    /// Withdraws a queued item no drain has claimed; `false` when one has.
    pub fn cancel(&self, item: &Item, ctx: &Ctx<'_>) -> Result<bool, WriteError> {
        Ok(Queue::open(&self.state)?.cancel_by(item, ctx.request.clone(), ctx.now)?)
    }

    /// The file at vdir-relative `rel` must hold `want` (`None`: must not exist).
    pub(super) fn check_holds(&self, rel: &str, want: Option<&str>) -> Result<(), WriteError> {
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

/// `path` must be a plain `<collection>/<file>` vdir-relative path.
fn check_plain_path(path: &str) -> Result<(), WriteError> {
    let plain = Path::new(path).components().count() == 2
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)));
    if plain {
        Ok(())
    } else {
        Err(io_error(
            io::ErrorKind::InvalidInput,
            format!("{path} is not a <collection>/<file> path"),
        ))
    }
}

impl Locked<'_> {
    /// The writer this lock belongs to.
    pub fn writer(&self) -> &Writer {
        self.writer
    }

    /// Recovery under the held lock.
    pub fn recover(&self) -> Result<RecoveryReport, WriteError> {
        Ok(recover_locked(&self.writer.vdir, &self.writer.journal)?)
    }

    /// Commits a prepared entry: refused while a collection operation is unfinished, unless
    /// the journal is clear and every file it reads or writes still holds the bytes it was
    /// planned on. The entry gets a fresh id, `ctx.now` and `ctx.request`.
    pub fn commit(&self, mut p: Prepared, ctx: &Ctx<'_>) -> Result<String, WriteError> {
        crate::lists::operation::ensure_idle(&self.writer.state)?;
        p.entry.request = ctx.request.clone();
        self.commit_inner(p, None, ctx.now)
    }

    /// [`Locked::commit`] without the fence check; names the queued item it carries out.
    pub(super) fn commit_inner(
        &self,
        p: Prepared,
        from_queue: Option<FromQueue>,
        now: DateTime<Utc>,
    ) -> Result<String, WriteError> {
        self.check_not_stuck()?;
        for w in &p.entry.writes {
            self.writer.check_holds(&w.path, w.before.as_deref())?;
        }
        for (rel, source) in &p.holds {
            self.writer.check_holds(rel, Some(source))?;
        }
        for f in p.family.iter().chain(&p.families) {
            self.writer.check_family(f)?;
        }
        let mut entry = p.entry;
        entry.id = new_op_id(now);
        entry.created = now;
        entry.from_queue = from_queue;
        self.run(entry)
    }

    /// Reverts entry `entry` with a new entry (writes swapped and reversed, `inverse_of` =
    /// `entry`). Every file must still hold what `entry` wrote.
    pub fn revert(&self, entry: &str, ctx: &Ctx<'_>) -> Result<String, WriteError> {
        self.revert_inner(entry, None, ctx.request.clone(), ctx.now)
    }

    /// A revert's work; `from_queue` names the queued item it carries out.
    pub(super) fn revert_inner(
        &self,
        id: &str,
        from_queue: Option<FromQueue>,
        request: Option<RequestRef>,
        now: DateTime<Utc>,
    ) -> Result<String, WriteError> {
        let w = self.writer;
        self.check_not_stuck()?;
        let e = w.journal.get(id)?.ok_or_else(|| no_entry(id))?;
        if e.stage != Stage::Applied {
            return Err(io_error(
                io::ErrorKind::InvalidInput,
                format!("operation {id} was not applied"),
            ));
        }
        if w.mode == WriteMode::Verified && is_repeat_completion(&e) {
            let ok = w
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
        for write in &e.writes {
            w.check_holds(&write.path, write.after.as_deref())?;
        }
        for family in e.family.iter().chain(&e.families) {
            w.check_family(family)?;
        }
        for (rel, content) in &e.holds {
            w.check_holds(rel, Some(content))?;
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

    /// Writes one file through the journal (kind "resolve") without the gate: it restores a
    /// version that 2Do or Twydo already wrote. `path` is vdir-relative
    /// (`<collection>/<file>`). Refused with [`WriteError::Stale`] unless the file holds
    /// `expect` (`None`: must not exist). `content` `None` deletes the file.
    pub fn restore(
        &self,
        path: &str,
        expect: Option<&str>,
        content: Option<&str>,
        uids: Vec<String>,
        ctx: &Ctx<'_>,
    ) -> Result<String, WriteError> {
        check_plain_path(path)?;
        let entry = Entry {
            format: FORMAT,
            id: new_op_id(ctx.now),
            created: ctx.now,
            kind: "resolve".to_string(),
            uids,
            stage: Stage::Prepared,
            inverse_of: None,
            from_queue: None,
            request: ctx.request.clone(),
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
        self.check_not_stuck()?;
        self.writer.check_holds(path, expect)?;
        self.run(entry)
    }

    /// Marks the unfinished entry `id` Abandoned so writes can continue. Its files are left as
    /// they are. `false` when the entry was already `Abandoned`.
    pub fn abandon(&self, id: &str) -> Result<bool, WriteError> {
        let e = self.writer.journal.get(id)?.ok_or_else(|| no_entry(id))?;
        if e.stage == Stage::Abandoned {
            return Ok(false);
        }
        if e.stage != Stage::Prepared {
            return Err(io_error(
                io::ErrorKind::InvalidInput,
                format!("operation {id} is not unfinished"),
            ));
        }
        self.writer.journal.set_stage(id, Stage::Abandoned)?;
        Ok(true)
    }

    /// Commits the queued items, oldest first: [`Locked::drain_steps`] without the refusal
    /// errors.
    pub fn drain(&self, ctx: &Ctx<'_>) -> Result<Vec<Drained>, WriteError> {
        Ok(self
            .drain_steps(ctx)?
            .into_iter()
            .map(|s| s.drained)
            .collect())
    }

    /// Drains the queue (spec §6.3): per item, in id order, skip what a journal entry already
    /// names, skip what was cancelled, re-gate on the current files and commit what was
    /// planned. A refused item is recorded and the drain goes on; a stuck journal or I/O error
    /// stops it, leaving the item claimed, and is the error only if nothing drained.
    pub(crate) fn drain_steps(&self, ctx: &Ctx<'_>) -> Result<Vec<DrainStep>, WriteError> {
        let w = self.writer;
        let now = ctx.now;
        if let Err(fence) = crate::lists::operation::ensure_idle(&w.state) {
            return self.refuse_fenced(fence, now);
        }
        let queue = Queue::open(&w.state)?;
        let mut report = crate::vdir::load(&w.vdir)?;
        let mut steps: Vec<DrainStep> = Vec::new();
        for item in queue.list()? {
            if let Some(entry) = w.journal.by_queue_id(&item.id)? {
                let outcome = if entry.stage == Stage::Prepared {
                    // An interrupted commit is not yet applied. Finish recovery before
                    // dropping its claim, and refresh the gate's view for dependent items.
                    if let Err(e) = self.check_not_stuck() {
                        if steps.is_empty() {
                            return Err(e);
                        }
                        break;
                    }
                    report = crate::vdir::load(&w.vdir)?;
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
                    let snap = Snapshot::new(report.clone(), ctx.config.clone());
                    let today = item.created.with_timezone(&snap.tz()).date_naive();
                    gate::check(&snap, &w.matrix, w.mode, op, today)
                        .map_err(WriteError::Blocked)
                        .and_then(|()| self.commit_inner(prepared, Some(from_queue), now))
                }
                Kind::Revert { entry, .. } => {
                    self.revert_inner(entry, Some(from_queue), item.request.clone(), now)
                }
            };
            match result {
                Ok(id) => {
                    queue.done(&item.id)?;
                    if let Some(entry) = w.journal.get(&id)? {
                        report.overlay_writes(&w.vdir, &entry.writes);
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
        &self,
        fence: io::Error,
        now: DateTime<Utc>,
    ) -> Result<Vec<DrainStep>, WriteError> {
        let w = self.writer;
        let Ok(Some(collection)) = crate::lists::operation::fenced_collection(&w.state) else {
            return Err(fence.into());
        };
        let reason = fence.to_string();
        let queue = Queue::open(&w.state)?;
        let mut steps = Vec::new();
        for item in queue.list()? {
            let in_list = item.writes().iter().any(|write| {
                Path::new(&write.path)
                    .components()
                    .next()
                    .is_some_and(|c| c.as_os_str() == collection.as_str())
            });
            // An item a journal entry names was committed: the next drain finishes it.
            if !in_list || w.journal.by_queue_id(&item.id)?.is_some() {
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

    /// The journal protocol: prepare, apply, mark applied. A conflict leaves the entry
    /// Prepared, which blocks later writes until it is abandoned.
    pub(super) fn run(&self, mut e: Entry) -> Result<String, WriteError> {
        let w = self.writer;
        // Inverses can recreate resources too. Refuse existing identity collisions
        // before preparing any entry, so a stale undo cannot block unrelated writes.
        super::journal::check_create_identities(&w.vdir, &e).map_err(|err| match err {
            ApplyError::Conflict { path } => WriteError::Stale(w.vdir.join(path)),
            ApplyError::Io(err) => WriteError::Io(err),
            err => io_error(io::ErrorKind::InvalidInput, err.to_string()),
        })?;
        // The exclusive vdir lock serializes commits across stores and processes. A drain
        // passes the same `now` for every item, so persist a strictly increasing commit
        // time rather than letting reconciliation break ties with random journal ids.
        if let Some(last) = w.journal.list()?.iter().map(|e| e.created).max()
            && last >= e.created
        {
            e.created = last
                .checked_add_signed(chrono::Duration::nanoseconds(1))
                .ok_or_else(|| {
                    io_error(io::ErrorKind::InvalidData, "journal commit time overflow")
                })?;
        }
        w.journal.prepare(&e)?;
        match apply_entry(&w.vdir, &e) {
            Ok(()) => {}
            Err(ApplyError::Io(err)) => return Err(err.into()),
            Err(err) => return Err(WriteError::Stuck(format!("{} ({err})", e.id))),
        }
        w.journal.set_stage(&e.id, Stage::Applied)?;
        Ok(e.id)
    }

    /// Recovery under the held lock; any entry it cannot finish blocks the write.
    pub(super) fn check_not_stuck(&self) -> Result<(), WriteError> {
        let report = self.recover()?;
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
}
