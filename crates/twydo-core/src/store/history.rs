//! A session's undo and redo history: one list of steps, no I/O of its own beyond the
//! [`Writer`] it is lent. The caller decides how a revert reaches the vdir ([`Access`]);
//! the per-step rules (what a queued, applied or withdrawn step becomes when undone) live here.

use super::queue::{self, Item, Queue};
use super::writer::{Ctx, Fence, Locked, Writer};
use super::{DrainStep, Drained, Submitted, WriteError, drained_outcome};

/// One entry of the history.
#[derive(Debug, Clone)]
pub struct Step {
    /// What the step did, for messages; the shell records steps with an empty title.
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

/// How a revert reaches the vdir.
#[derive(Clone, Copy)]
pub enum Access<'a> {
    /// The vdir lock is held: revert now.
    Locked(&'a Locked<'a>),
    /// It is not (a sync holds it): queue the revert, checking the collection fence per `Fence`.
    Queue(&'a Writer, Fence),
}

/// The steps a session applied or queued, and the ones it undid. Never persisted.
#[derive(Debug, Default)]
pub struct History {
    /// Oldest first; an undo reverts the last.
    done: Vec<Step>,
    /// Undone steps; a redo reverts the last. A new [`History::record`] clears it.
    undone: Vec<Step>,
    /// Items the drains inside an undo or redo dealt with, until [`History::take_drained`].
    drained: Vec<Drained>,
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

    /// Undoes the last step. Under [`Access::Locked`] the queue is drained before an entry is
    /// reverted, but a queued step is withdrawn first if no drain has claimed it. Returns what became of the revert and the step that moved (its
    /// title, and the id it named before the move). A queued step a drain refused is dropped
    /// and the step before it undone. [`WriteError::NothingToUndo`] when the list is empty.
    pub fn undo(
        &mut self,
        access: Access<'_>,
        origin: &str,
        ctx: &Ctx<'_>,
    ) -> Result<(Submitted, Step), WriteError> {
        while let Some(step) = self.done.last().cloned() {
            match self.revert_step(&step, "undo", access, origin, ctx)? {
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
    ) -> Result<(Submitted, Step), WriteError> {
        while let Some(step) = self.undone.last().cloned() {
            match self.revert_step(&step, "redo", access, origin, ctx)? {
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
    pub fn forget_applied(&mut self, entry: &str) {
        self.done
            .retain(|s| !matches!(&s.state, StepState::Applied(id) if id == entry));
    }

    /// The queued item `item` was committed as `entry` after the undo or redo that queued it
    /// returned: the step it became now names the entry.
    pub fn settled(&mut self, item: &str, entry: &str) {
        for s in self.done.iter_mut().chain(self.undone.iter_mut()) {
            if matches!(&s.state, StepState::Queued(i) if i.id == item) {
                s.state = StepState::Applied(entry.to_string());
            }
        }
    }

    /// The queued item `item` was refused after the undo (`undo`) or redo that queued it
    /// returned: the step it became goes, and `moved` (as [`History::undo`] returned it)
    /// goes back where it was.
    pub fn refused(&mut self, undo: bool, item: &str, moved: Step) {
        let (from, back) = if undo {
            (&mut self.undone, &mut self.done)
        } else {
            (&mut self.done, &mut self.undone)
        };
        from.retain(|s| !matches!(&s.state, StepState::Queued(i) if i.id == item));
        back.push(moved);
    }

    /// Takes the items the drains inside [`History::undo`] and [`History::redo`] dealt with.
    pub fn take_drained(&mut self) -> Vec<Drained> {
        std::mem::take(&mut self.drained)
    }

    /// Reverts `step`: the result and the step for the other list. `None` when `step` was a
    /// queued item a drain refused, so nothing of it was ever written.
    fn revert_step(
        &mut self,
        step: &Step,
        label: &str,
        access: Access<'_>,
        origin: &str,
        ctx: &Ctx<'_>,
    ) -> Result<Option<(Submitted, Step)>, WriteError> {
        let done = match &step.state {
            StepState::Applied(id) => self.revert_entry(id, label, access, origin, ctx)?,
            StepState::Queued(item) => {
                let writer = writer_of(access);
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
                self.revert_entry(&entry.id, label, access, origin, ctx)?
            }
            StepState::Cancelled(item) => {
                // The redo is a new request: its item, and the entry drained from it, carry it.
                // A new id too, so the withdrawn item's record keeps answering for it.
                let mut item = item.clone();
                item.id = queue::new_id(ctx.now);
                item.request = ctx.request.clone();
                match access {
                    Access::Queue(writer, fence) => {
                        writer.push(&item, fence)?;
                        Submitted::Queued(item)
                    }
                    Access::Locked(lock) => {
                        lock.writer().push(&item, Fence::Now)?;
                        self.drain_own(lock, item, ctx)?
                    }
                }
            }
        };
        let state = match &done {
            Submitted::Applied(id) => StepState::Applied(id.clone()),
            Submitted::Queued(item) => StepState::Queued(item.clone()),
            Submitted::Cancelled => unreachable!("only a queued step is cancelled"),
        };
        Ok(Some((
            done,
            Step {
                title: step.title.clone(),
                state,
            },
        )))
    }

    /// Reverts journal entry `id`: under the lock, after a drain (a queued item may be about to
    /// touch the same files), else by queueing the revert.
    fn revert_entry(
        &mut self,
        id: &str,
        label: &str,
        access: Access<'_>,
        origin: &str,
        ctx: &Ctx<'_>,
    ) -> Result<Submitted, WriteError> {
        match access {
            Access::Locked(lock) => {
                let drained = lock.drain(ctx)?;
                self.drained.extend(drained);
                Ok(Submitted::Applied(lock.revert(id, ctx)?))
            }
            Access::Queue(writer, fence) => {
                let kind = writer.revert_item(id)?;
                Ok(Submitted::Queued(
                    writer.enqueue(kind, label, origin, fence, ctx)?,
                ))
            }
        }
    }

    /// Drains under `lock` right after `item` was pushed and reports what became of it. Anything
    /// but a refusal of `item` itself (stuck journal, I/O) leaves it queued for the next drain.
    fn drain_own(
        &mut self,
        lock: &Locked<'_>,
        item: Item,
        ctx: &Ctx<'_>,
    ) -> Result<Submitted, WriteError> {
        let Ok(steps) = lock.drain_steps(ctx) else {
            return Ok(Submitted::Queued(item));
        };
        self.drained
            .extend(steps.iter().map(|s: &DrainStep| s.drained.clone()));
        drained_outcome(steps, item)
    }
}

fn writer_of<'a>(access: Access<'a>) -> &'a Writer {
    match access {
        Access::Locked(lock) => lock.writer(),
        Access::Queue(writer, _) => writer,
    }
}
