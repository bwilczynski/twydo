//! The writes the TUI asks the daemon for, and how a request is sent and its answer worded.

use std::time::Duration;

use twydo_core::edit::Op;
pub use twydo_core::edit::describe::op_title;
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
