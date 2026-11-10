//! Runs the TUI's write requests through the store and turns the result into a message.

use std::time::Duration;

use chrono::{DateTime, Utc};
use twydo_core::compat::Matrix;
use twydo_core::config::Config;
use twydo_core::edit::Op;
pub use twydo_core::edit::describe::op_title;
use twydo_core::store::queue::{Queue, refusal_message};
use twydo_core::store::{DrainOutcome, Store, Submitted, WriteError, existing_vdir_state};
use twydo_core::sync::batch::{Trigger, after_write};
use twydo_core::views::Snapshot;
use twydo_daemon::client::{Caller, ClientError};
use twydo_daemon::protocol::jobs::{JobGetParams, JobResult, JobStarted, JobState, JobView};
use twydo_daemon::protocol::lists::ListRecoverParams;
use twydo_daemon::protocol::methods::{BasedOn, SubmitParams, UndoParams, WriteResult};
use twydo_daemon::protocol::rpc::{ErrorKind, RpcError};

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
    matrix: Matrix,
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

    /// Never fails: a failed open is retried on the next write, after recovery may finish.
    pub fn open_with(cfg: &Config, matrix: Matrix, trigger: SyncTrigger) -> Writer {
        Writer {
            store: Store::open_with(cfg, matrix.clone())
                .map(|(s, _)| s)
                .map_err(|e| format!("cannot write: {e}")),
            config: cfg.clone(),
            matrix,
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
        // A pending collection fence can outlive the previous TUI. Once :lists
        // clears it, retry initialization without replacing a healthy store's undo stack.
        if self.store.is_err() {
            self.store = Store::open_with(&self.config, self.matrix.clone())
                .map(|(s, _)| s)
                .map_err(|e| format!("cannot write: {e}"));
        }
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
                format!(
                    "{}{}",
                    twydo_core::edit::describe::success_message(snap, op),
                    queued_note(&submitted)
                )
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

/// How long [`execute`] waits for a recovery job before it gives up.
const JOB_WAIT: Duration = Duration::from_secs(60);

/// Sends a direct write (`op.submit`, `op.undo` or `op.redo`) to the daemon. A resend uses the
/// same `request_id`, so the daemon applies it once. `Recover` is a job: see
/// [`recover_params`].
pub fn submit(
    caller: &Caller,
    session: &str,
    based_on: &BasedOn,
    req: &Request,
    request_id: &str,
    timeout: Duration,
) -> Result<WriteResult, ClientError> {
    let session = session.to_string();
    let request_id = request_id.to_string();
    match req {
        Request::Apply(op) => caller.call_timeout(
            "op.submit",
            SubmitParams {
                session,
                request_id,
                based_on: based_on.clone(),
                op: op.clone(),
            },
            timeout,
        ),
        Request::Undo | Request::Redo => caller.call_timeout(
            if *req == Request::Undo {
                "op.undo"
            } else {
                "op.redo"
            },
            UndoParams {
                session,
                request_id,
            },
            timeout,
        ),
        Request::Recover { .. } => Err(ClientError::Decode(
            "a recovery is a job, not a direct write".into(),
        )),
    }
}

/// The `lists.recover` call a `Recover` request stands for.
pub fn recover_params(
    session: &str,
    request_id: &str,
    backup: &str,
    to: &str,
) -> ListRecoverParams {
    ListRecoverParams {
        session: session.to_string(),
        request_id: request_id.to_string(),
        backup: backup.to_string(),
        to: to.to_string(),
    }
}

/// The message a finished recovery job stands for, or why it did not recover.
pub fn recover_outcome(state: JobState) -> Result<String, String> {
    match state {
        JobState::Finished {
            result: JobResult::Recovered { message, .. },
        } => Ok(message),
        JobState::Failed { message, .. } => Err(message),
        _ => Err("the daemon stopped before the job ended".into()),
    }
}

/// The message for a refusal of the daemon. A stale file is named by its task's title (found
/// by `title_of`, from the client's snapshot): the path means nothing in the TUI.
pub fn refusal_text(e: &RpcError, title_of: impl Fn(&str) -> Option<String>) -> String {
    if let Some(d) = &e.data
        && d.kind == ErrorKind::Stale
        && let Some(title) = d.uid.as_deref().and_then(title_of)
    {
        return format!("\"{title}\" changed on disk; reload and try again");
    }
    e.message.clone()
}

/// [`refusal_text`] with the titles of `snap`.
pub fn refusal_in(e: &RpcError, snap: &Snapshot) -> String {
    refusal_text(e, |uid| {
        snap.tasks
            .iter()
            .find(|t| t.uid == uid)
            .map(|t| t.summary.clone())
    })
}

/// Runs a write on the daemon and returns its message (a queued write says so). A recovery
/// waits for its job to end by polling `job.get`: a blocking convenience for tests and the
/// CLI, while the TUI uses the link's event-driven path. `snap` names the task of a stale refusal.
pub fn execute(
    caller: &Caller,
    session: &str,
    based_on: &BasedOn,
    snap: &Snapshot,
    req: &Request,
    request_id: &str,
) -> Result<String, String> {
    let failed = |e: ClientError| match e {
        ClientError::Rpc(e) => refusal_in(&e, snap),
        e => e.to_string(),
    };
    if let Request::Recover { backup, to } = req {
        let params = recover_params(session, request_id, backup, to);
        let started: JobStarted = caller
            .call_timeout("lists.recover", params, JOB_WAIT)
            .map_err(failed)?;
        let deadline = std::time::Instant::now() + JOB_WAIT;
        loop {
            let view: JobView = caller
                .call_timeout(
                    "job.get",
                    JobGetParams {
                        job_id: Some(started.job_id.clone()),
                        request_id: None,
                    },
                    JOB_WAIT,
                )
                .map_err(failed)?;
            if view.state.is_final() {
                return recover_outcome(view.state);
            }
            if std::time::Instant::now() > deadline {
                return Err("the recovery did not finish; try again".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    submit(caller, session, based_on, req, request_id, JOB_WAIT)
        .map(|r| r.message)
        .map_err(failed)
}
