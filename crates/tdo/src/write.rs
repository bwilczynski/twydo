//! Runs the TUI's write requests through the store and turns the result into a message.

use std::time::Duration;

use chrono::{DateTime, Utc};
use twydo_core::compat::Matrix;
use twydo_core::config::Config;
use twydo_core::edit::Op;
use twydo_core::model::Status;
use twydo_core::store::queue::{Queue, refusal_message};
use twydo_core::store::{DrainOutcome, Store, Submitted, WriteError, existing_vdir_state};
use twydo_core::sync::batch::{Trigger, after_write};
use twydo_core::views::Snapshot;

/// A write the TUI asks for.
// `Op::Create` carries a whole `RepeatSpec`; requests are sent one at a time.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Apply(Op),
    Recover { backup: String, to: String },
    Undo,
    Redo,
}

/// How long `:recover` waits for a running sync before it gives up (issue #10). Saves never
/// wait: during a sync they are queued.
pub const LOCK_WAIT: Duration = Duration::from_secs(2);

/// The message of a queued save, after its success text.
const QUEUED: &str = " · queued (a sync is running)";

/// Why a write did not happen: the message for the status line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    Other(String),
}

impl Refused {
    pub fn message(&self) -> String {
        match self {
            Refused::Other(m) => m.clone(),
        }
    }
}

/// Starts what a write asked for (see [`after_write`]): a sync now or the batch waiter.
pub type SyncTrigger = Box<dyn FnMut(&Config, Trigger)>;

/// The TUI's store for the session, holding its undo/redo stack.
pub struct Writer {
    store: Result<Store, String>,
    config: Config,
    trigger: SyncTrigger,
    /// Titles of the writes `u` would undo and `Ctrl-r` would redo, in step with the store's
    /// session stacks, so the messages can name the task.
    done: Vec<Titled>,
    undone: Vec<Titled>,
    lock_wait: Duration,
}

/// A title on an undo or redo stack. `queued`: the id of the queue item that stands for the
/// write, so a refusal can drop the step the store skips.
#[derive(Debug, Clone)]
struct Titled {
    title: String,
    queued: Option<String>,
}

impl Titled {
    fn new(title: impl Into<String>, queued: Option<String>) -> Titled {
        Titled {
            title: title.into(),
            queued,
        }
    }
}

impl Writer {
    /// The embedded matrix; starts a background sync or the batch waiter after writes.
    pub fn open(cfg: &Config) -> Writer {
        Writer::open_with(
            cfg,
            Matrix::embedded().clone(),
            Box::new(|c, t| crate::cli::sync::spawn(c, Some(&c.vdir), t)),
        )
    }

    /// Never fails: an open error is kept and returned by every [`Writer::run`].
    pub fn open_with(cfg: &Config, matrix: Matrix, trigger: SyncTrigger) -> Writer {
        Writer {
            store: Store::open_with(cfg, matrix)
                .map(|(s, _)| s)
                .map_err(|e| format!("cannot write: {e}")),
            config: cfg.clone(),
            trigger,
            done: Vec::new(),
            undone: Vec::new(),
            lock_wait: LOCK_WAIT,
        }
    }

    /// Replaces [`LOCK_WAIT`] (tests).
    pub fn set_lock_wait(&mut self, lock: Duration) {
        self.lock_wait = lock;
    }

    /// Ok: the success message (a save during a sync is queued and says so). Err: why nothing
    /// was written.
    pub fn run(
        &mut self,
        snap: &Snapshot,
        req: &Request,
        now: DateTime<Utc>,
    ) -> Result<String, Refused> {
        let (message, applied) = self.write(snap, req, now)?;
        if applied {
            self.sync_after(now);
        }
        Ok(message)
    }

    /// Writes the queued edits when no sync holds the vdir (a busy vdir is not an error:
    /// the sync that holds it drains the queue), then returns [`Writer::unshown`]. Starts what
    /// [`after_write`] says when something was written.
    pub fn drain(&mut self, now: DateTime<Utc>) -> Vec<String> {
        let Ok(store) = self.store.as_mut() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        match store.drain(now) {
            Ok(drained) => {
                if drained
                    .iter()
                    .any(|d| matches!(d.outcome, DrainOutcome::Applied(_)))
                {
                    self.sync_after(now);
                }
            }
            Err(WriteError::Busy) => {}
            Err(e) => out.push(format!("queued edits not saved: {e}")),
        }
        out.extend(self.unshown());
        out
    }

    /// The messages of refused queued edits the user has not been told about, whoever drained
    /// them. Each is marked shown, so it is returned once, and its step leaves the undo and
    /// redo stacks, as the store skips it.
    pub fn unshown(&mut self) -> Vec<String> {
        let state = existing_vdir_state(&self.config.state_dir(), &self.config.vdir)
            .ok()
            .flatten();
        let Some(queue) = state.and_then(|s| Queue::existing(&s)) else {
            return Vec::new();
        };
        let Ok(refusals) = queue.unshown() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for r in refusals {
            let id = &r.item.id;
            self.done.retain(|t| t.queued.as_ref() != Some(id));
            self.undone.retain(|t| t.queued.as_ref() != Some(id));
            if queue.mark_shown(id).is_ok() {
                out.push(refusal_message(&r));
            }
        }
        out
    }

    /// The success message, and whether a write reached the vdir (a queued one has not).
    fn write(
        &mut self,
        snap: &Snapshot,
        req: &Request,
        now: DateTime<Utc>,
    ) -> Result<(String, bool), Refused> {
        let store = self.store.as_mut().map_err(|e| Refused::Other(e.clone()))?;
        let submitted = match req {
            Request::Apply(op) => {
                let label = op_title(snap, op);
                store.submit(snap, op, &label, "tui", now)
            }
            Request::Recover { backup, to } => {
                store.set_lock_wait(Some(self.lock_wait));
                match store.recover_list(snap, backup, to, now) {
                    Ok((_, false)) => {
                        return Ok(("already recovered; backup retained".into(), false));
                    }
                    Ok((id, true)) => Ok(Submitted::Applied(id)),
                    Err(WriteError::Busy) => {
                        return Err(Refused::Other("a sync is running; try again".into()));
                    }
                    Err(e) => Err(e),
                }
            }
            Request::Undo => store.undo_last(now),
            Request::Redo => store.redo_last(now),
        };
        let submitted = submitted.map_err(|e| Refused::Other(failure(snap, e)))?;
        let queued_id = match &submitted {
            Submitted::Queued(item) => Some(item.id.clone()),
            _ => None,
        };
        let applied = matches!(submitted, Submitted::Applied(_));
        let message = match req {
            Request::Recover { to, .. } => {
                self.done.push(Titled::new("recovered list", None));
                self.undone.clear();
                format!("recovered local snapshot into {to}; backup retained")
            }
            Request::Apply(op) => {
                self.done.push(Titled::new(op_title(snap, op), queued_id));
                self.undone.clear();
                format!("{}{}", success(snap, op), queued_note(&submitted))
            }
            Request::Undo => format!(
                "{}{}",
                step("undone", &mut self.done, &mut self.undone, queued_id),
                queued_note(&submitted)
            ),
            Request::Redo => format!(
                "{}{}",
                step("redone", &mut self.undone, &mut self.done, queued_id),
                queued_note(&submitted)
            ),
        };
        Ok((message, applied))
    }

    /// Starts a sync now when the batch is due, else makes sure the batch waiter runs.
    fn sync_after(&mut self, now: DateTime<Utc>) {
        if let Some(t) = after_write(&self.config, now) {
            (self.trigger)(&self.config, t);
        }
    }
}

/// The note a queued write adds to its message.
fn queued_note(s: &Submitted) -> &'static str {
    if matches!(s, Submitted::Queued(_)) {
        QUEUED
    } else {
        ""
    }
}

/// Move the top title from `from` to `to` and name it in the message. `queued`: the queue
/// item now standing for the write, if it was queued.
fn step(
    verb: &str,
    from: &mut Vec<Titled>,
    to: &mut Vec<Titled>,
    queued: Option<String>,
) -> String {
    match from.pop() {
        Some(t) => {
            let message = format!("{verb}: {}", t.title);
            to.push(Titled::new(t.title, queued));
            message
        }
        None => verb.to_string(),
    }
}

/// The title of the task an operation writes, as it is before the write.
pub fn op_title(snap: &Snapshot, op: &Op) -> String {
    let uid = match op {
        Op::Create(t) | Op::CreateAt { task: t, .. } => return t.summary.trim().to_string(),
        Op::Edit { uid, .. }
        | Op::Save { uid, .. }
        | Op::Complete { uid }
        | Op::Reopen { uid }
        | Op::Reparent { uid, .. }
        | Op::Move { uid, .. }
        | Op::Delete { uid }
        | Op::Reorder { uid, .. } => uid,
    };
    snap.find(uid)
        .map_or_else(|| uid.clone(), |i| snap.tasks[i].summary.clone())
}

/// The message for a refused write. A stale file is named by its task's title: the path
/// means nothing in the TUI.
fn failure(snap: &Snapshot, e: WriteError) -> String {
    if let WriteError::Stale(path) = &e
        && let Some(t) = snap.tasks.iter().find(|t| &t.path == path)
    {
        return format!("\"{}\" changed on disk; reload and try again", t.summary);
    }
    e.to_string()
}

/// How many children an operation writes with the task, from the snapshot before the write.
fn family_size(snap: &Snapshot, op: &Op) -> usize {
    let (Op::Complete { uid } | Op::Reopen { uid } | Op::Move { uid, .. } | Op::Delete { uid }) =
        op
    else {
        return 0;
    };
    let Some(idx) = snap.find(uid) else { return 0 };
    let kids = snap.children(idx);
    match op {
        Op::Complete { .. } if snap.tasks[idx].rrule.is_some() => kids.len(),
        Op::Complete { .. } => snap.open_items(idx),
        Op::Reopen { .. } => kids
            .iter()
            .filter(|&&c| snap.tasks[c].status == Status::Completed)
            .count(),
        _ => kids.len(),
    }
}

fn success(snap: &Snapshot, op: &Op) -> String {
    let n = family_size(snap, op);
    let title = op_title(snap, op);
    let items = if n == 1 { "item" } else { "items" };
    let with = |verb: &str| {
        if n > 0 {
            format!("{verb}: \"{title}\" and {n} {items}")
        } else {
            verb.to_string()
        }
    };
    match op {
        Op::Complete { .. } => with("done"),
        Op::Reopen { .. } => with("reopened"),
        Op::Delete { .. } if n > 0 => format!("deleted \"{title}\" and {n} {items} · u to undo"),
        Op::Delete { .. } => "deleted · u to undo".into(),
        Op::Edit { .. } | Op::Save { .. } => "saved".into(),
        Op::Create(_) | Op::CreateAt { .. } => "added".into(),
        Op::Reorder { .. } => "reordered · u to undo".into(),
        Op::Reparent { .. } => "relocated · u to undo".into(),
        Op::Move { to, .. } => {
            let name = snap
                .collections
                .iter()
                .find(|c| &c.id == to)
                .and_then(|c| c.display_name.clone())
                .unwrap_or_else(|| to.clone());
            if n > 0 {
                format!("moved \"{title}\" and {n} {items} to {name}")
            } else {
                format!("moved to {name}")
            }
        }
    }
}
