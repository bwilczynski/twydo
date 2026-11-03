//! Runs the TUI's write requests through the store and turns the result into a message.

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use twydo_core::compat::Matrix;
use twydo_core::config::Config;
use twydo_core::edit::Op;
use twydo_core::model::Status;
use twydo_core::store::{Store, WriteError};
use twydo_core::sync::running::{StopResult, clear_stop, request_stop, running};
use twydo_core::views::Snapshot;

/// A write the TUI asks for.
// `Op::Create` carries a whole `RepeatSpec`; requests are sent one at a time.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Apply(Op),
    Undo,
    Redo,
}

/// How long a TUI write waits for a running sync before it gives up (issue #10).
pub const LOCK_WAIT: Duration = Duration::from_secs(2);
/// How long a write waits for a sync it asked to stop.
pub const STOP_WAIT: Duration = Duration::from_secs(10);

/// How long saves must pause before the after-edit sync starts (issue #19). A sync started
/// at once holds the vdir, so the next quick save (closing checklist items one by one)
/// would wait on it.
pub const SYNC_DELAY: Duration = Duration::from_secs(3);

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

/// Starts the after-edit sync; called once saves pause, when `[sync] after_edit` is on.
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
    lock_wait: Duration,
    stop_wait: Duration,
    /// When the last write that wants an after-edit sync happened; None: no sync is due.
    sync_due: Option<Instant>,
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
                .map(|(s, _)| s)
                .map_err(|e| format!("cannot write: {e}")),
            config: cfg.clone(),
            after_write,
            done: Vec::new(),
            undone: Vec::new(),
            lock_wait: LOCK_WAIT,
            stop_wait: STOP_WAIT,
            sync_due: None,
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
            self.sync_soon();
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
        // before the after-edit sync starts, which would otherwise stop itself.
        let _ = clear_stop(&self.config);
        if written.is_ok() {
            self.sync_soon();
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
            Request::Undo => store.undo_last(now),
            Request::Redo => store.redo_last(now),
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

    /// Schedules the after-edit sync when `[sync] after_edit` is on: it starts once nothing
    /// was written for [`SYNC_DELAY`] (see [`Writer::sync_if_quiet`]). Each call pushes it back.
    pub fn sync_soon(&mut self) {
        if self.config.sync.as_ref().is_some_and(|s| s.after_edit) {
            self.sync_due = Some(Instant::now());
        }
    }

    /// Starts the scheduled sync if [`SYNC_DELAY`] has passed since the last write by `now`.
    pub fn sync_if_quiet(&mut self, now: Instant) {
        if self
            .sync_due
            .is_some_and(|t| now.saturating_duration_since(t) >= SYNC_DELAY)
        {
            self.flush_sync();
        }
    }

    /// Starts the scheduled sync now, if one is due (on quit).
    pub fn flush_sync(&mut self) {
        if self.sync_due.take().is_some() {
            (self.after_write)(&self.config);
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
