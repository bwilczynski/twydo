//! Task writes (daemon spec §6.6, §6.7): `op.submit`, `op.undo`, `op.redo`, `op.get` and
//! `op.check`, the replay of resent requests and the mapping of store errors to refusals.
//!
//! # Lock order
//!
//! A write takes, in this order and never the other way round:
//!
//! 1. the [`RequestLog`](super::replay::RequestLog) slot (`begin` may wait for an in-flight
//!    duplicate; it waits holding no other lock);
//! 2. the session's mutex (serialises the requests of one session);
//! 3. the lane token ([`Lane::write`](super::lane::Lane::write): waits only for another
//!    write, never for a long job);
//! 4. the store's vdir file lock (inside `Store`, released before it returns);
//! 5. the publisher's `reload_lock`, then its `state`, then the bus.
//!
//! The session table's mutex is held only for lookups, never while waiting. A long job takes
//! only the lane; its drain hook (Task 8) must not take a session mutex or the lane.

use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use serde_json::Value;
use twydo_core::edit::Op;
use twydo_core::edit::describe::{op_title, success_message};
use twydo_core::gate;
use twydo_core::store::journal::{Journal, RequestRef};
use twydo_core::store::queue::{Queue, Queued, refusal_message};
use twydo_core::store::{DrainOutcome, Drained, Submitted, WriteError};
use twydo_core::views::Snapshot as CoreSnapshot;

use super::Shared;
use super::publish::Snapshot;
use super::replay::Begin;
use super::sessions::{QUEUED, Session, queued_note, step};
use crate::protocol::events::{Cause, Event};
use crate::protocol::methods::{
    BasedOn, CheckParams, CheckResult, Created, OpGetParams, OpState, SubmitParams, Undid,
    UndoParams, WriteOutcome, WriteResult,
};
use crate::protocol::rpc::{ErrorData, ErrorKind, REFUSED, RpcError};

const RELOAD: &str = "reload and try again";

fn to_value(r: WriteResult) -> Result<Value, RpcError> {
    serde_json::to_value(r).map_err(|e| RpcError::refused(ErrorKind::Internal, e.to_string()))
}

/// Maps a store error to a refusal. `view` is the snapshot the write was planned against
/// (names the task of a stale file), `current` the latest published one (its `current_rev`).
pub fn refusal(e: WriteError, view: &CoreSnapshot, current: &CoreSnapshot) -> RpcError {
    let kind = match &e {
        WriteError::Blocked(_) => ErrorKind::Blocked,
        WriteError::Edit(_) | WriteError::NothingToUndo | WriteError::NothingToRedo => {
            ErrorKind::Invalid
        }
        WriteError::Stale(_) => ErrorKind::Stale,
        WriteError::Stuck(_) => ErrorKind::Stuck,
        WriteError::Busy => ErrorKind::Unavailable,
        WriteError::Io(_) => ErrorKind::Internal,
    };
    if let WriteError::Stale(path) = &e
        && let Some(t) = view.tasks.iter().find(|t| &t.path == path)
    {
        let mut data = ErrorData::new(kind);
        data.uid = Some(t.uid.clone());
        data.current_rev = current
            .find(&t.uid)
            .and_then(|i| current.rev(i))
            .map(str::to_string);
        return RpcError {
            code: REFUSED,
            message: format!("\"{}\" changed on disk; {RELOAD}", t.summary),
            data: Some(data),
        };
    }
    RpcError::refused(kind, e.to_string())
}

/// The published snapshot `based_on` names: ours and still kept, else `stale`.
fn planning_view(shared: &Shared, b: &BasedOn) -> Result<Arc<Snapshot>, RpcError> {
    if b.epoch != shared.epoch {
        return Err(RpcError::refused(ErrorKind::Stale, RELOAD));
    }
    shared
        .publisher
        .at(b.revision)
        .ok_or_else(|| RpcError::refused(ErrorKind::Stale, RELOAD))
}

fn require_request_id(id: &str) -> Result<(), RpcError> {
    if id.trim().is_empty() {
        return Err(RpcError::refused(
            ErrorKind::Invalid,
            "request_id is required",
        ));
    }
    Ok(())
}

/// Runs a changing request at most once per `request_id`: the in-memory log first (waiting
/// for an in-flight duplicate), then the durable records, and only then `run`.
///
/// The session check comes after the replay and its refusal is never recorded: after a
/// restart a client resends under its old session (refused), says hello again and resends
/// the same request under the new one, which must run.
fn replayed(
    shared: &Shared,
    request_id: &str,
    session_id: &str,
    run: impl FnOnce(Arc<Mutex<Session>>) -> Result<WriteResult, RpcError>,
) -> Result<Value, RpcError> {
    require_request_id(request_id)?;
    let ticket = match shared.requests.begin(request_id) {
        Begin::Done(r) => return r,
        Begin::New(t) => t,
    };
    if let Some(found) = durable(shared, request_id) {
        return ticket.finish(found.and_then(to_value));
    }
    let Some(session) = shared.sessions.get(session_id) else {
        return ticket.forget(Err(unknown_session()));
    };
    ticket.finish(run(session).and_then(to_value))
}

fn refused_kind(stale: bool) -> ErrorKind {
    if stale {
        ErrorKind::Stale
    } else {
        ErrorKind::Blocked
    }
}

/// What the journal and the queue recorded for `request_id` (a request from before a restart
/// or from earlier in this epoch). The queue is read first: an item leaves it only after the
/// entry naming it (or its refusal) is written, so nothing is missed in between.
fn durable(shared: &Shared, request_id: &str) -> Option<Result<WriteResult, RpcError>> {
    let revision = shared.publisher.current().0;
    if let Some(q) = Queue::existing(&shared.state_dir) {
        match q.by_request(request_id) {
            Ok(Some(Queued::Pending(i) | Queued::Claimed(i))) => {
                return Some(Ok(WriteResult {
                    outcome: WriteOutcome::Queued,
                    op_id: i.id,
                    revision,
                    message: format!("already queued{QUEUED}"),
                    created: None,
                    undid: None,
                }));
            }
            Ok(Some(Queued::Refused(r))) => {
                return Some(Err(RpcError::refused(
                    refused_kind(r.stale),
                    refusal_message(&r),
                )));
            }
            Ok(None) => {}
            Err(e) => return Some(Err(RpcError::refused(ErrorKind::Internal, e.to_string()))),
        }
    }
    match Journal::open(&shared.state_dir).and_then(|j| j.by_request(request_id)) {
        Ok(Some(e)) => Some(Ok(WriteResult {
            outcome: WriteOutcome::Applied,
            op_id: e.id,
            revision,
            message: "already applied".into(),
            created: None,
            undid: None,
        })),
        Ok(None) => None,
        Err(e) => Some(Err(RpcError::refused(ErrorKind::Internal, e.to_string()))),
    }
}

fn unknown_session() -> RpcError {
    RpcError::refused(ErrorKind::NotFound, "unknown session; reconnect")
}

/// Tells subscribers what became of queued items a write drained on its way.
fn finish_drained(shared: &Shared, drained: Vec<Drained>) {
    if drained.is_empty() {
        return;
    }
    let queue = Queue::existing(&shared.state_dir);
    for d in drained {
        let state = match d.outcome {
            DrainOutcome::Applied(id) | DrainOutcome::AlreadyApplied(id) => {
                OpState::Applied { op_id: id }
            }
            DrainOutcome::Refused(reason) => {
                let refusal = queue.as_ref().and_then(|q| match q.state(&d.item.id) {
                    Ok(Some(Queued::Refused(r))) => Some(r),
                    _ => None,
                });
                match refusal {
                    Some(r) => OpState::Refused {
                        kind: refused_kind(r.stale),
                        message: refusal_message(&r),
                    },
                    None => OpState::Refused {
                        kind: ErrorKind::Blocked,
                        message: reason,
                    },
                }
            }
        };
        shared.bus.emit(Event::OpFinished {
            op_id: d.item.id,
            request_id: d.item.request.map(|r| r.request_id),
            state,
        });
    }
}

/// Publishes what a commit changed (`entry` and any entries drained before it) and refreshes
/// the status. Returns the revision after the reload. Never fails: the commit is done, so a
/// failed reload only delays the publish until the next one.
///
/// Accepted for now: if an outside process takes the vdir lock between our commit and this
/// reload, the reload (and so the applied reply) waits for it to be released.
fn publish_applied(shared: &Shared, entry: &str, drained: &[Drained]) -> u64 {
    let mut uids = Vec::new();
    if let Ok(j) = Journal::open(&shared.state_dir) {
        let ids = drained
            .iter()
            .filter_map(|d| match &d.outcome {
                DrainOutcome::Applied(id) => Some(id.as_str()),
                _ => None,
            })
            .chain(std::iter::once(entry));
        for id in ids {
            if let Ok(Some(e)) = j.get(id) {
                for u in e.uids {
                    if !uids.contains(&u) {
                        uids.push(u);
                    }
                }
            }
        }
    }
    let revision = match shared.publisher.reload(Cause::Op, uids) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("tdo daemon: reload after a write failed: {e}");
            shared.publisher.current().0
        }
    };
    shared.publisher.refresh_status();
    revision
}

/// The task a create wrote: the file the entry created, found in the newest snapshot.
fn created_task(shared: &Shared, entry: &str) -> Option<Created> {
    let e = Journal::open(&shared.state_dir).ok()?.get(entry).ok()??;
    let (_, snap) = shared.publisher.current();
    e.writes
        .iter()
        .filter(|w| w.before.is_none() && w.after.is_some())
        .find_map(|w| {
            snap.wire
                .tasks
                .iter()
                .find(|t| t.task.path == Path::new(&w.path))
        })
        .map(|t| Created {
            uid: t.task.uid.clone(),
            rev: t.rev.clone(),
        })
}

fn crash_point(shared: &Shared) {
    if shared.crash_after_commit {
        // Tests of the replay after a crash: the commit is on disk, the answer never leaves.
        std::process::abort();
    }
}

pub fn submit(shared: &Arc<Shared>, p: SubmitParams) -> Result<Value, RpcError> {
    let (rid, sid) = (p.request_id.clone(), p.session.clone());
    replayed(shared, &rid, &sid, |session| run_submit(shared, session, p))
}

fn run_submit(
    shared: &Arc<Shared>,
    session: Arc<Mutex<Session>>,
    p: SubmitParams,
) -> Result<WriteResult, RpcError> {
    let view = planning_view(shared, &p.based_on)?;
    let label = op_title(&view.snap, &p.op);
    let mut s = session.lock().unwrap();
    let lane = shared.lane.write();
    let now = Utc::now();
    s.store.set_request(Some(RequestRef {
        session: p.session.clone(),
        request_id: p.request_id.clone(),
    }));
    // A long job holds the vdir: plan and queue at once, without trying the lock.
    s.store.set_vdir_busy(lane.is_err());
    let r = s.store.submit(&view.snap, &p.op, &label, "daemon", now);
    s.store.set_vdir_busy(false);
    s.store.set_request(None);
    let drained = s.store.take_drained();
    let submitted = match r {
        Ok(x) => x,
        Err(e) => {
            finish_drained(shared, drained);
            return Err(refusal(e, &view.snap, &shared.publisher.current().1.snap));
        }
    };
    s.pushed(label, &submitted);
    let message = format!(
        "{}{}",
        success_message(&view.snap, &p.op),
        queued_note(&submitted)
    );
    let result = match &submitted {
        Submitted::Applied(id) => {
            crash_point(shared);
            let revision = publish_applied(shared, id, &drained);
            let created = match &p.op {
                Op::Create(_) | Op::CreateAt { .. } => created_task(shared, id),
                _ => None,
            };
            WriteResult {
                outcome: WriteOutcome::Applied,
                op_id: id.clone(),
                revision,
                message,
                created,
                undid: None,
            }
        }
        Submitted::Queued(item) => queued(shared, item.id.clone(), message, None),
        Submitted::Cancelled => unreachable!("a submit never cancels"),
    };
    drop(lane);
    drop(s);
    finish_drained(shared, drained);
    Ok(result)
}

fn queued(
    shared: &Arc<Shared>,
    op_id: String,
    message: String,
    undid: Option<Undid>,
) -> WriteResult {
    // The overlay of the queued item is published by the drain machinery (Task 8). The status
    // is refreshed in the background: a queued reply never waits on `reload_lock`, which a
    // reload blocked on the vdir lock (a sync) may hold for minutes.
    shared.request_status_refresh();
    WriteResult {
        outcome: WriteOutcome::Queued,
        op_id,
        revision: shared.publisher.current().0,
        message,
        created: None,
        undid,
    }
}

/// `op.undo` (`redo` false) and `op.redo`.
pub fn undo(shared: &Arc<Shared>, p: UndoParams, redo: bool) -> Result<Value, RpcError> {
    let (rid, sid) = (p.request_id.clone(), p.session.clone());
    replayed(shared, &rid, &sid, |session| {
        run_undo(shared, session, p, redo)
    })
}

fn run_undo(
    shared: &Arc<Shared>,
    session: Arc<Mutex<Session>>,
    p: UndoParams,
    redo: bool,
) -> Result<WriteResult, RpcError> {
    let mut s = session.lock().unwrap();
    s.prune_refused(Queue::existing(&shared.state_dir).as_ref());
    let lane = shared.lane.write();
    let now = Utc::now();
    s.store.set_request(Some(RequestRef {
        session: p.session.clone(),
        request_id: p.request_id.clone(),
    }));
    s.store.set_vdir_busy(lane.is_err());
    let r = if redo {
        s.store.redo_last(now)
    } else {
        s.store.undo_last(now)
    };
    s.store.set_vdir_busy(false);
    s.store.set_request(None);
    let drained = s.store.take_drained();
    let submitted = match r {
        Ok(x) => x,
        Err(e) => {
            finish_drained(shared, drained);
            let cur = shared.publisher.current().1;
            return Err(refusal(e, &cur.snap, &cur.snap));
        }
    };
    let session = &mut *s;
    let (text, moved) = if redo {
        step("redone", &mut session.undone, &mut session.done, &submitted)
    } else {
        step("undone", &mut session.done, &mut session.undone, &submitted)
    };
    let message = format!("{text}{}", queued_note(&submitted));
    let undid = moved.as_ref().map(|t| Undid {
        op_id: t.op_id.clone(),
        title: t.title.clone(),
    });
    let result = match &submitted {
        Submitted::Applied(id) => {
            crash_point(shared);
            let revision = publish_applied(shared, id, &drained);
            WriteResult {
                outcome: WriteOutcome::Applied,
                op_id: id.clone(),
                revision,
                message,
                created: None,
                undid,
            }
        }
        Submitted::Queued(item) => queued(shared, item.id.clone(), message, undid),
        Submitted::Cancelled => {
            // The undo removed a still-queued item: nothing reached the vdir. As for a queued
            // reply, the status refresh must not hold up the answer.
            let op_id = moved.and_then(|t| t.queued).unwrap_or_default();
            shared.request_status_refresh();
            WriteResult {
                outcome: WriteOutcome::Applied,
                op_id,
                revision: shared.publisher.current().0,
                message,
                created: None,
                undid,
            }
        }
    };
    drop(lane);
    drop(s);
    finish_drained(shared, drained);
    Ok(result)
}

/// Is `id` shaped like a queue item id (`<YYYYMMDDTHHMMSS%6fZ>-<8 digits>-<pid>`)?
fn is_queue_id(id: &str) -> bool {
    let mut parts = id.split('-');
    let (Some(stamp), Some(seq), Some(pid), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    stamp.len() == 22
        && stamp.as_bytes()[8] == b'T'
        && stamp.ends_with('Z')
        && digits(&stamp[..8])
        && digits(&stamp[9..21])
        && seq.len() == 8
        && digits(seq)
        && digits(pid)
}

/// Where operation `op_id` (a journal entry or queue item id) stands.
fn state_of(shared: &Shared, op_id: &str) -> Result<OpState, RpcError> {
    let internal = |e: std::io::Error| RpcError::refused(ErrorKind::Internal, e.to_string());
    let journal = Journal::open(&shared.state_dir).map_err(internal)?;
    if let Ok(Some(e)) = journal.get(op_id) {
        return Ok(OpState::Applied { op_id: e.id });
    }
    if let Some(e) = journal.by_queue_id(op_id).map_err(internal)? {
        return Ok(OpState::Applied { op_id: e.id });
    }
    if let Some(q) = Queue::existing(&shared.state_dir)
        && let Ok(Some(found)) = q.state(op_id)
    {
        return Ok(match found {
            Queued::Pending(_) | Queued::Claimed(_) => OpState::Pending,
            Queued::Refused(r) => OpState::Refused {
                kind: refused_kind(r.stale),
                message: refusal_message(&r),
            },
        });
    }
    if is_queue_id(op_id) {
        // Only an undo removes an item without leaving a record.
        return Ok(OpState::Cancelled);
    }
    Err(RpcError::refused(
        ErrorKind::NotFound,
        format!("no operation {op_id}"),
    ))
}

pub fn get(shared: &Arc<Shared>, p: OpGetParams) -> Result<Value, RpcError> {
    let state = match (p.op_id, p.request_id) {
        (Some(op_id), _) => state_of(shared, &op_id)?,
        (None, Some(rid)) => {
            let recorded = match shared.requests.peek(&rid) {
                Some(None) => Some(Ok(None)),
                Some(Some(Ok(v))) => Some(
                    serde_json::from_value::<WriteResult>(v)
                        .map(Some)
                        .map_err(|e| RpcError::refused(ErrorKind::Internal, e.to_string())),
                ),
                Some(Some(Err(e))) => Some(Err(e)),
                None => durable(shared, &rid).map(|r| r.map(Some)),
            };
            match recorded {
                // Still running.
                Some(Ok(None)) => OpState::Pending,
                Some(Ok(Some(r))) => state_of(shared, &r.op_id)?,
                Some(Err(e)) => match e.data.as_ref().map(|d| d.kind) {
                    Some(kind @ (ErrorKind::Blocked | ErrorKind::Stale)) => OpState::Refused {
                        kind,
                        message: e.message,
                    },
                    _ => return Err(e),
                },
                None => {
                    return Err(RpcError::refused(
                        ErrorKind::NotFound,
                        format!("no request {rid}"),
                    ));
                }
            }
        }
        (None, None) => {
            return Err(RpcError::refused(
                ErrorKind::Invalid,
                "op_id or request_id is required",
            ));
        }
    };
    serde_json::to_value(state).map_err(|e| RpcError::refused(ErrorKind::Internal, e.to_string()))
}

/// The gate's verdict on `op` against the snapshot `based_on` names; never writes.
pub fn check(shared: &Arc<Shared>, p: CheckParams) -> Result<Value, RpcError> {
    let view = planning_view(shared, &p.based_on)?;
    let today = Utc::now()
        .with_timezone(&view.snap.config.tz())
        .date_naive();
    let verdict = gate::check(
        &view.snap,
        &shared.matrix,
        shared.cfg.writes.mode,
        &p.op,
        today,
    );
    let r = CheckResult {
        allowed: verdict.is_ok(),
        reason: verdict.err().map(|b| b.to_string()),
    };
    serde_json::to_value(r).map_err(|e| RpcError::refused(ErrorKind::Internal, e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::is_queue_id;

    #[test]
    fn queue_ids_are_told_from_journal_ids() {
        let now = chrono::Utc::now();
        assert!(is_queue_id(&twydo_core::store::queue::new_id(now)));
        assert!(!is_queue_id(&twydo_core::store::journal::new_op_id(now)));
        assert!(!is_queue_id("x"));
    }
}
