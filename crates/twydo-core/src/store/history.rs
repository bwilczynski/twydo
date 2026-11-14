//! A session's undo and redo history: one list of steps, no I/O of its own beyond the
//! [`Writer`] it is lent. The caller decides how a revert reaches the vdir ([`Access`]);
//! the per-step rules (what a queued, applied or withdrawn step becomes when undone) live here.

use std::time::Duration;

use super::queue::{self, Item, Queue};
use super::writer::{Ctx, Fence, Writer};
use super::{Drained, Submitted, WriteError, drained_outcome};

/// One entry of the history.
#[derive(Debug, Clone)]
pub struct Step {
    /// What the step did, for messages; a caller with no title leaves it empty.
    pub title: String,
    pub state: StepState,
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum StepState {
    /// A committed journal entry.
    Applied(String),
    /// An item waiting in the queue (or committed since, which the journal then names).
    Queued(Item),
    /// An item an undo removed from the queue; a redo puts it back.
    Cancelled(Item),
}

/// How a revert reaches the vdir. `fence` is the collection fence of anything queued: checked
/// when the item is pushed for [`Fence::Now`], left to the drain for [`Fence::AtDrain`].
#[derive(Clone, Copy)]
pub enum Access<'a> {
    /// The vdir may be free: take the lock, waiting at most `wait`, only when a step needs it;
    /// queue when someone outside holds it.
    Try {
        writer: &'a Writer,
        wait: Duration,
        fence: Fence,
    },
    /// A long job holds the vdir: never lock, always queue.
    Queue(&'a Writer, Fence),
}

/// The steps a session applied or queued, and the ones it undid. Never persisted.
#[derive(Debug, Default)]
pub struct History {
    /// Oldest first; an undo reverts the last.
    done: Vec<Step>,
    /// Undone steps; a redo reverts the last. A new [`History::record`] clears it.
    undone: Vec<Step>,
}

impl History {
    pub fn new() -> History {
        History::default()
    }

    /// Records what a submit did. A new step clears the redo list; a cancelled submit is no
    /// step.
    pub fn record(&mut self, title: String, s: &Submitted) {
        let state = match s {
            Submitted::Applied(id) => StepState::Applied(id.clone()),
            Submitted::Queued(item) => StepState::Queued(item.clone()),
            Submitted::Cancelled => return,
        };
        self.done.push(Step { title, state });
        self.undone.clear();
    }

    /// Undoes the last step. Returns what became of the revert and the step that moved (its
    /// title, and the id it named before the move). A queued step a drain refused is dropped
    /// and the step before it undone. [`WriteError::NothingToUndo`] when the list is empty.
    ///
    /// A queued step is withdrawn without the lock if no drain has claimed it. Under
    /// [`Access::Try`] the lock is taken only to revert an entry, after draining the queue.
    /// Every drain run is appended to `drained`, also when the call then fails.
    pub fn undo(
        &mut self,
        access: Access<'_>,
        origin: &str,
        ctx: &Ctx<'_>,
        drained: &mut Vec<Drained>,
    ) -> Result<(Submitted, Step), WriteError> {
        while let Some(step) = self.done.last().cloned() {
            match revert_step(&step, "undo", access, origin, ctx, drained)? {
                Some(moved) => {
                    self.done.pop();
                    self.undone.push(moved.1);
                    return Ok((moved.0, step));
                }
                None => {
                    self.done.pop();
                }
            }
        }
        Err(WriteError::NothingToUndo)
    }

    /// Redoes the last undo by reverting it; the mirror of [`History::undo`].
    pub fn redo(
        &mut self,
        access: Access<'_>,
        origin: &str,
        ctx: &Ctx<'_>,
        drained: &mut Vec<Drained>,
    ) -> Result<(Submitted, Step), WriteError> {
        while let Some(step) = self.undone.last().cloned() {
            match revert_step(&step, "redo", access, origin, ctx, drained)? {
                Some(moved) => {
                    self.undone.pop();
                    self.done.push(moved.1);
                    return Ok((moved.0, step));
                }
                None => {
                    self.undone.pop();
                }
            }
        }
        Err(WriteError::NothingToRedo)
    }

    /// Forgets the applied step naming `entry`: it was reverted by name, not by [`History::undo`].
    /// For a caller that reverts an entry by id outside the history (`Writer::revert`); today
    /// only the test wrapper.
    pub fn forget_applied(&mut self, entry: &str) {
        self.done
            .retain(|s| !matches!(&s.state, StepState::Applied(id) if id == entry));
    }
}

/// Reverts `step`: the result and the step for the other list. `None` when `step` was a queued
/// item a drain refused, so nothing of it was ever written.
fn revert_step(
    step: &Step,
    label: &str,
    access: Access<'_>,
    origin: &str,
    ctx: &Ctx<'_>,
    drained: &mut Vec<Drained>,
) -> Result<Option<(Submitted, Step)>, WriteError> {
    let done = match &step.state {
        StepState::Applied(id) => revert_entry(id, label, access, origin, ctx, drained)?,
        StepState::Queued(item) => {
            let (writer, _) = parts(access);
            if writer.cancel(item, ctx)? {
                let state = StepState::Cancelled(item.clone());
                let title = step.title.clone();
                return Ok(Some((Submitted::Cancelled, Step { title, state })));
            }
            // A drainer got there first: the entry it commits names the item. Until that
            // entry exists the drainer is mid-commit, unless it refused the item.
            let Some(entry) = writer.journal.by_queue_id(&item.id)? else {
                if Queue::open(&writer.state)?.is_refused(&item.id)? {
                    return Ok(None);
                }
                return Err(WriteError::Busy);
            };
            revert_entry(&entry.id, label, access, origin, ctx, drained)?
        }
        StepState::Cancelled(item) => {
            // The redo is a new request: its item, and the entry drained from it, carry it.
            // A new id too, so the withdrawn item's record keeps answering for it.
            let mut item = item.clone();
            item.id = queue::new_id(ctx.now);
            item.request = ctx.request.clone();
            let (writer, fence) = parts(access);
            writer.push(&item, fence)?;
            settle(access, item, ctx, drained)?
        }
    };
    let state = match &done {
        Submitted::Applied(id) => StepState::Applied(id.clone()),
        Submitted::Queued(item) => StepState::Queued(item.clone()),
        Submitted::Cancelled => unreachable!("only a queued step is cancelled"),
    };
    let title = step.title.clone();
    Ok(Some((done, Step { title, state })))
}

fn parts<'a>(access: Access<'a>) -> (&'a Writer, Fence) {
    match access {
        Access::Try { writer, fence, .. } | Access::Queue(writer, fence) => (writer, fence),
    }
}

/// After `item` was pushed: under [`Access::Try`] looks at the lock once without waiting, so an
/// item queued just as a holder released it is not stranded, and drains if it is free. Reports
/// what became of `item`; a refusal of it is the error. Anything but a refusal of `item` itself
/// (stuck journal, I/O) leaves it queued for the next drain.
fn settle(
    access: Access<'_>,
    item: Item,
    ctx: &Ctx<'_>,
    drained: &mut Vec<Drained>,
) -> Result<Submitted, WriteError> {
    let Access::Try { writer, .. } = access else {
        return Ok(Submitted::Queued(item));
    };
    let Some(lock) = writer.lock(Duration::ZERO)? else {
        return Ok(Submitted::Queued(item));
    };
    let Ok(steps) = lock.drain_steps(ctx) else {
        return Ok(Submitted::Queued(item));
    };
    drained.extend(steps.iter().map(|s| s.drained.clone()));
    drained_outcome(steps, item)
}

/// Reverts journal entry `id`: under [`Access::Try`] with the lock (waiting at most `wait`),
/// after a drain (a queued item may be about to touch the same files); else by queueing the
/// revert.
fn revert_entry(
    id: &str,
    label: &str,
    access: Access<'_>,
    origin: &str,
    ctx: &Ctx<'_>,
    drained: &mut Vec<Drained>,
) -> Result<Submitted, WriteError> {
    if let Access::Try { writer, wait, .. } = access
        && let Some(lock) = writer.lock(wait)?
    {
        drained.extend(lock.drain(ctx)?);
        return Ok(Submitted::Applied(lock.revert(id, ctx)?));
    }
    let (writer, fence) = parts(access);
    let kind = writer.revert_item(id)?;
    let item = writer.enqueue(kind, label, origin, fence, ctx)?;
    settle(access, item, ctx, drained)
}
