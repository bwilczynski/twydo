//! Runs the TUI's write requests through the store and turns the result into a message.

use chrono::{DateTime, Utc};
use twydo_core::compat::Matrix;
use twydo_core::config::Config;
use twydo_core::edit::Op;
use twydo_core::store::{Store, WriteError};
use twydo_core::views::Snapshot;

/// A write the TUI asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Apply(Op),
    Undo,
    Redo,
}

/// Called after each successful write when `[sync] after_edit` is on.
pub type SyncTrigger = Box<dyn FnMut(&Config)>;

/// The TUI's store for the session, holding its undo/redo stack.
pub struct Writer {
    store: Result<Store, String>,
    config: Config,
    after_write: SyncTrigger,
    /// Titles of the writes `u` would undo and `Ctrl-r` would redo, in step with the store's
    /// session stacks, so the messages can name the task.
    done: Vec<String>,
    undone: Vec<String>,
}

impl Writer {
    /// The embedded matrix; syncs in the background after writes.
    pub fn open(cfg: &Config) -> Writer {
        Writer::open_with(
            cfg,
            Matrix::embedded().clone(),
            Box::new(|c| crate::cli::sync::spawn_background(c, Some(&c.vdir))),
        )
    }

    /// Never fails: an open error is kept and returned by every [`Writer::run`].
    pub fn open_with(cfg: &Config, matrix: Matrix, after_write: SyncTrigger) -> Writer {
        Writer {
            store: Store::open_with(cfg, matrix)
                .map(|(mut s, _)| {
                    s.set_no_wait(true);
                    s
                })
                .map_err(|e| format!("cannot write: {e}")),
            config: cfg.clone(),
            after_write,
            done: Vec::new(),
            undone: Vec::new(),
        }
    }

    /// Ok: the success message. Err: why nothing was written.
    pub fn run(
        &mut self,
        snap: &Snapshot,
        req: &Request,
        now: DateTime<Utc>,
    ) -> Result<String, String> {
        let store = self.store.as_mut().map_err(|e| e.clone())?;
        let done = match req {
            Request::Apply(op) => store.apply(snap, op, now),
            Request::Undo => store.undo_last(now),
            Request::Redo => store.redo_last(now),
        };
        done.map_err(|e| failure(snap, e))?;
        let message = match req {
            Request::Apply(op) => {
                self.done.push(op_title(snap, op));
                self.undone.clear();
                success(snap, op)
            }
            Request::Undo => step("undone", &mut self.done, &mut self.undone),
            Request::Redo => step("redone", &mut self.undone, &mut self.done),
        };
        if self.config.sync.as_ref().is_some_and(|s| s.after_edit) {
            (self.after_write)(&self.config);
        }
        Ok(message)
    }
}

/// Move the top title from `from` to `to` and name it in the message.
fn step(verb: &str, from: &mut Vec<String>, to: &mut Vec<String>) -> String {
    match from.pop() {
        Some(title) => {
            let message = format!("{verb}: {title}");
            to.push(title);
            message
        }
        None => verb.to_string(),
    }
}

/// The title of the task an operation writes, as it is before the write.
fn op_title(snap: &Snapshot, op: &Op) -> String {
    let uid = match op {
        Op::Create(t) => return t.summary.trim().to_string(),
        Op::Edit { uid, .. }
        | Op::Complete { uid }
        | Op::Reopen { uid }
        | Op::Move { uid, .. }
        | Op::Delete { uid } => uid,
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

fn success(snap: &Snapshot, op: &Op) -> String {
    match op {
        Op::Complete { .. } => "done".into(),
        Op::Reopen { .. } => "reopened".into(),
        Op::Delete { .. } => "deleted · u to undo".into(),
        Op::Edit { .. } => "saved".into(),
        Op::Create(_) => "added".into(),
        Op::Move { to, .. } => {
            let name = snap
                .collections
                .iter()
                .find(|c| &c.id == to)
                .and_then(|c| c.display_name.clone())
                .unwrap_or_else(|| to.clone());
            format!("moved to {name}")
        }
    }
}
