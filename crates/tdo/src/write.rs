//! Runs the TUI's write requests through the store and turns the result into a message.

use std::time::Duration;

use chrono::{DateTime, Utc};
use twydo_core::compat::Matrix;
use twydo_core::config::Config;
use twydo_core::edit::Op;
use twydo_core::model::Status;
use twydo_core::store::{Store, WriteError};
use twydo_core::sync::batch::{Trigger, after_write};
use twydo_core::sync::running::{StopResult, clear_stop, request_stop, running};
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

/// How long a TUI write waits for a running sync before it gives up (issue #10).
pub const LOCK_WAIT: Duration = Duration::from_secs(2);
/// How long a write waits for a sync it asked to stop.
pub const STOP_WAIT: Duration = Duration::from_secs(10);

/// Why a write did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// The vdir lock stayed held for the whole wait. `running_for`: how long the recorded
    /// sync has run (None: no `running.json`, nothing the TUI can stop).
    Busy {
        running_for: Option<Duration>,
    },
    Other(String),
}

impl Refused {
    /// The message for the status line.
    pub fn message(&self) -> String {
        match self {
            Refused::Busy { .. } => WriteError::Busy.to_string(),
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
    done: Vec<String>,
    undone: Vec<String>,
    lock_wait: Duration,
    stop_wait: Duration,
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
            stop_wait: STOP_WAIT,
        }
    }

    /// Replaces [`LOCK_WAIT`] and [`STOP_WAIT`] (tests).
    pub fn set_waits(&mut self, lock: Duration, stop: Duration) {
        self.lock_wait = lock;
        self.stop_wait = stop;
    }

    /// Ok: the success message. Err: why nothing was written. Waits up to the lock wait for
    /// a running sync.
    pub fn run(
        &mut self,
        snap: &Snapshot,
        req: &Request,
        now: DateTime<Utc>,
    ) -> Result<String, Refused> {
        let wait = self.lock_wait;
        let written = self.write(snap, req, now, wait);
        if written.is_ok() {
            self.sync_after(now);
        }
        written
    }

    /// Asks the running sync to stop, then writes, waiting up to the stop wait for it to let
    /// go of the vdir. The message says so when a sync was stopped.
    pub fn stop_and_run(
        &mut self,
        snap: &Snapshot,
        req: &Request,
        now: DateTime<Utc>,
    ) -> Result<String, Refused> {
        let stopped = match request_stop(&self.config) {
            Ok(StopResult::NothingRunning) => false,
            Ok(StopResult::Requested | StopResult::Killed) => true,
            Err(e) => return Err(Refused::Other(format!("cannot stop the sync: {e}"))),
        };
        let wait = self.stop_wait;
        let written = self.write(snap, req, now, wait);
        // The request held off other syncs until this write; withdraw it either way, and
        // before the sync after the write starts, which would otherwise stop itself.
        let _ = clear_stop(&self.config);
        if written.is_ok() {
            self.sync_after(now);
        }
        match written {
            Ok(m) if stopped => Ok(format!("{m} · sync stopped")),
            Err(Refused::Busy { .. }) => {
                Err(Refused::Other("not saved; the sync did not stop".into()))
            }
            other => other,
        }
    }

    fn write(
        &mut self,
        snap: &Snapshot,
        req: &Request,
        now: DateTime<Utc>,
        wait: Duration,
    ) -> Result<String, Refused> {
        let store = self.store.as_mut().map_err(|e| Refused::Other(e.clone()))?;
        store.set_lock_wait(Some(wait));
        let done = match req {
            Request::Apply(op) => store.apply(snap, op, now),
            Request::Recover { backup, to } => match store.recover_list(snap, backup, to, now) {
                Ok((_, false)) => return Ok("already recovered; backup retained".into()),
                Ok((id, true)) => Ok(id),
                Err(e) => Err(e),
            },
            Request::Undo => store.undo_last(now).map(|_| String::new()),
            Request::Redo => store.redo_last(now).map(|_| String::new()),
        };
        match done {
            Ok(_) => {}
            Err(WriteError::Busy) => {
                return Err(Refused::Busy {
                    running_for: running_for(&self.config),
                });
            }
            Err(e) => return Err(Refused::Other(failure(snap, e))),
        }
        let message = match req {
            Request::Recover { to, .. } => {
                self.done.push("recovered list".into());
                self.undone.clear();
                format!("recovered local snapshot into {to}; backup retained")
            }
            Request::Apply(op) => {
                self.done.push(op_title(snap, op));
                self.undone.clear();
                success(snap, op)
            }
            Request::Undo => step("undone", &mut self.done, &mut self.undone),
            Request::Redo => step("redone", &mut self.undone, &mut self.done),
        };
        Ok(message)
    }

    /// Starts a sync now when the batch is due, else makes sure the batch waiter runs.
    fn sync_after(&mut self, now: DateTime<Utc>) {
        if let Some(t) = after_write(&self.config, now) {
            (self.trigger)(&self.config, t);
        }
    }
}

/// How long the recorded sync has run; None when none is recorded (or unreadable).
fn running_for(cfg: &Config) -> Option<Duration> {
    let rec = running(cfg).ok().flatten()?;
    Some(
        (Utc::now() - rec.started)
            .to_std()
            .unwrap_or(Duration::ZERO),
    )
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
        | Op::Save { uid, .. }
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
        Op::Create(_) => "added".into(),
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
