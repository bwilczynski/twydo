//! Task writes (daemon spec §6.6, §6.7): `op.submit`, `op.undo`, `op.redo`, `op.get` and
//! `op.check`, the replay of resent requests and the mapping of write errors to refusals; the
//! write queue's drain after a long job and at startup (§7.3) and `refusals.ack` (§6.5).
//!
//! # Lock order
//!
//! The one description of the daemon's lock order; other modules point here. Each path below
//! takes its locks in the order given and never the other way round. "Then" means the earlier
//! lock is still held unless it says it was let go.
//!
//! ## Task writes (`op.submit`, `op.undo`, `op.redo`, `journal.revert`)
//!
//! 1. the [`RequestLog`](super::replay::RequestLog) slot (`begin` may wait for an in-flight
//!    duplicate; it waits holding no other lock);
//! 2. the session's mutex (serialises the requests of one session);
//! 3. the lane token ([`Lane::write`](super::lane::Lane::write): waits only for another
//!    write, never for a long job);
//! 4. the commit gate's read side ([`Shared::commit_gate`]), taken by [`with_vdir`] (or
//!    [`with_gate`] when a long job holds the lane) and held around the writer call only;
//! 5. the vdir file lock, taken inside that call by the [`Writer`] (or by the session's
//!    `History`, lazily, for an undo or redo) and released before it returns;
//! 6. the publisher's `reload_lock`, then its `state`, then the bus (after the gate is let go).
//!    `Publisher::reload` takes the shared vdir lock *before* `reload_lock` (step 5 before 6),
//!    and nothing waits for the vdir lock while holding `reload_lock`, so a reload stuck
//!    behind a sync never blocks the queue overlay.
//!
//! The live configuration (`Publisher::config`, the `cfg` leaf) is cloned out before the lane
//! is taken, for the write's [`Ctx`]; nothing holds it while waiting.
//!
//! A queued write (the lane answered `LongBusy`) takes neither the vdir lock nor
//! `reload_lock` while it holds the session mutex: its reply hands the overlay and the status
//! to the background refresher ([`Shared::request_overlay_publish`]).
//!
//! The session table's mutex is held only for lookups, never while waiting.
//!
//! ## Writes that wait through a long job
//!
//! `conflicts.resolve`, `journal.recover` and `journal.abandon` cannot be queued, so they take the
//! lane with [`Lane::write_waiting`](super::lane::Lane::write_waiting), which waits for a running
//! long job: the request log slot (not for `journal.recover`), then the lane, then the commit
//! gate around the writer or core call, then the vdir lock (inside that call), then, with the
//! gate let go and the lane still held, the publisher (`reload` takes the vdir reader lock and
//! then `reload_lock`; then `state` and the bus). They take no session mutex: a long job's
//! drain hook takes session mutexes while it holds the lane, so a session holder must never
//! wait for a long job. After letting the lane go they may request a drain or a sync (below).
//!
//! ## Long jobs (the job runner, [`super::jobs`])
//!
//! The runner thread takes the lane long holding nothing; the job's body then takes, under it:
//!
//! - a sync ([`super::syncjob::run_sync`]; a `sync.run` job, a background sync, and the sync
//!   `lists.forget` runs after preserving its list): the stop gate (to mark the sync active, let
//!   go at once), then the publisher (`state` to report the sync
//!   running, then `reload_lock` and `state` for the status, then the bus), let go before the
//!   sync runs; the core runner then takes the sync run lock (`<state>/sync/run.lock`), then
//!   the vdir lock, both released before it returns; then it publishes the items it drained
//!   as the drain hook does. The sync stays reported running (the status, the stop gate's
//!   active sync) until its reload and `sync.finished` ([`super::syncjob::SyncDone::finish`])
//!   run after the lane is let go, so after the drain hook; see the stop gate below.
//! - a list job ([`super::lists`]): the `ListService` takes the vdir lock exclusively
//!   (released before it returns); then the publisher (`reload`, as above).
//! - `lists.recover`: the calling session's mutex, then the vdir lock (taken by the job), and
//!   the publisher's `state` to read the current snapshot; the session mutex is let go before
//!   the reload. Taking a session mutex under the long lane is safe: a holder of a session
//!   mutex never waits for a long job (task writes get `LongBusy` at once; the waiting writes
//!   take no session mutex).
//! - the job records' `JobStore.lock`: a leaf, held around one record's read-modify-write.
//!
//! When the job lets the lane go, its drain hook ([`drain_queue`]) runs while the lane is
//! still held and takes, in order: the commit gate's read side (around the drain passes
//! only), the vdir file lock (released before each pass returns), then, with the gate let go,
//! the publisher (`reload_lock`, `state`) and the bus. No session mutex: a refused queued step
//! is dropped lazily by the session's history when it is next undone. The hook never takes the
//! lane itself.
//!
//! ## The stop gate and the end gate
//!
//! `syncjob::stop_running_sync` (`sync.stop` and teardown) takes the runner's stop gate (the
//! active sync and its stop request), then writes the stop file, all holding the gate. A sync's
//! end ([`super::syncjob::SyncDone::finish`], after its `sync.finished` is sent) withdraws its
//! request, and the runner putting things right after a panic does the same, under the same
//! gate and never holding the publisher's locks (the gate is taken after them, not around
//! them). The end gate (`JobRunner::end_gate`) is held by a job's end while it records the
//! final state and emits `job.finished`, and by `job.get` while it reads. Both take the end
//! gate first, then the job store's lock (a leaf, let go before the emit), then, in the job's
//! end, the bus; neither takes the stop gate or the publisher while holding it.
//!
//! ## The publisher and the configuration reload
//!
//! `Publisher::finish_sync` (the end of a sync) takes `reload_lock`, then `state` (to report
//! the sync idle, build and store the status), emits `status.changed` and `sync.finished`
//! while holding `state`, so a status reader (`state`) waits for both. `Publisher::set_config`
//! takes `reload_lock` alone, then the `cfg` leaf, so a reload or overlay never builds half
//! of its snapshot with each configuration. `Publisher.cfg` is a leaf: it is cloned out and
//! never held across I/O. `Shared::config_seen` is taken first by the configuration reload
//! ([`super::config::changed`]) and held throughout; `reload_lock` and the vdir lock follow
//! it, never the other way round. `before_queued_reply` (a test hook) runs with the session
//! mutex held and takes nothing else.
//!
//! ## Background threads
//!
//! The background drainer ([`Shared::request_drain`]) and the startup drain hold nothing when they
//! start, take the lane as a write ([`Lane::write`](super::lane::Lane::write)) for the drain passes
//! only ([`drain_passes`]), and publish and report ([`finish_drain`]) after letting it go.
//!
//! The scheduler's poke ([`Shared::poke_scheduler`], after a publish and a sync's end) takes
//! the scheduler slot and then the scheduler's own mutex (`core.inner`), both held only to set
//! a flag. [`Shared::sync_now`] (the scheduler's `sync_now`) takes the slot, then `core.inner`
//! (let go), then the runner's `state` (to queue the sync), then `core.inner` again. The
//! scheduler thread holds neither while it reads the journal or posts to the job runner.
//!
//! The configuration reload ([`super::config::changed`]) takes `config_seen` and holds it
//! throughout; then the publisher: `reload` takes the vdir reader lock, then `reload_lock`.
//!
//! Teardown first waits for the job runner to finish its current work item (`stop_jobs`),
//! holding none of the above and taking only the stop gate (above) to ask a running sync to
//! stop, at every poll once the grace is over. It has no give-up path: a long job, a list
//! job's helper included, may write until it ends, so the daemon lock stays held until then
//! (a job that never ends is ended by the service manager's stop timeout). Teardown then takes
//! the commit gate's write side holding none of the above, so it waits only for writer calls
//! already inside the gate (each bounded by its lock try), and it closes the gate
//! before the daemon lock is released.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use serde_json::Value;
use twydo_core::edit::Op;
use twydo_core::edit::describe::{op_title, success_message};
use twydo_core::gate;
use twydo_core::store::journal::{Entry, Journal, RequestRef, Stage};
use twydo_core::store::queue::{Item, Kind, Queue, Queued, refusal_message};
use twydo_core::store::{
    Access, Ctx, DrainOutcome, Drained, Fence, Step, StepState, Submitted, WriteError, Writer,
};
use twydo_core::views::Snapshot as CoreSnapshot;

use super::Shared;
use super::lane::{LaneHeld, LongBusy, LongKind, WriteGuard};
use super::publish::Snapshot;
use super::replay::Begin;
use super::sessions::{QUEUED, Session, queued_note};
use crate::protocol::admin::JournalRevertParams;
use crate::protocol::events::{Cause, Event};
use crate::protocol::methods::{
    AckParams, AckResult, BasedOn, CheckParams, CheckResult, Created, OpGetParams, OpState,
    SubmitParams, Undid, UndoParams, WriteOutcome, WriteResult,
};
use crate::protocol::rpc::{ErrorData, ErrorKind, REFUSED, RpcError};

const RELOAD: &str = "reload and try again";
const ABANDONED: &str = "the operation was abandoned; it was not applied";
const UNDO_CUT_SHORT: &str = "the undo was interrupted; it was not applied";

/// The commit gate is closed: the daemon is stopping and commits nothing more.
fn stopping() -> RpcError {
    RpcError::refused(ErrorKind::Unavailable, "the daemon is shutting down")
}

/// A journal entry still `Prepared`: its writes may be partly on disk.
fn stuck_entry(id: &str) -> RpcError {
    RpcError::refused(
        ErrorKind::Stuck,
        format!("operation {id} is unfinished; recover or abandon it"),
    )
}

fn to_value(r: WriteResult) -> Result<Value, RpcError> {
    serde_json::to_value(r).map_err(|e| RpcError::refused(ErrorKind::Internal, e.to_string()))
}

/// Maps a write error to a refusal. `view` is the snapshot the write was planned against
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
    // A file the client's view does not hold (it joined a family after the view) is named
    // through the latest snapshot.
    if let WriteError::Stale(path) = &e
        && let Some(t) = view
            .tasks
            .iter()
            .find(|t| &t.path == path)
            .or_else(|| current.tasks.iter().find(|t| &t.path == path))
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

pub(super) fn require_request_id(id: &str) -> Result<(), RpcError> {
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
            // The request queued an item an undo withdrew since: it was queued, as answered
            // then; `op.get` now says cancelled.
            Ok(Some(Queued::Cancelled(c))) => {
                return Some(Ok(WriteResult {
                    outcome: WriteOutcome::Queued,
                    op_id: c.item.id,
                    revision,
                    message: "already queued; since undone".into(),
                    created: None,
                    undid: None,
                }));
            }
            Ok(None) => {}
            Err(e) => return Some(Err(RpcError::refused(ErrorKind::Internal, e.to_string()))),
        }
        // The request was an undo that withdrew a queued item.
        match q.cancelled_by(request_id) {
            Ok(Some(c)) => return Some(withdrawn(shared, &q, c.item.id, revision)),
            Ok(None) => {}
            Err(e) => return Some(Err(RpcError::refused(ErrorKind::Internal, e.to_string()))),
        }
    }
    match Journal::open(&shared.state_dir).and_then(|j| j.by_request(request_id)) {
        // Only an applied entry is applied; a stuck one is not (yet), an abandoned one never.
        Ok(Some(e)) => Some(match e.stage {
            Stage::Applied => Ok(WriteResult {
                outcome: WriteOutcome::Applied,
                created: replayed_created(shared, &e),
                op_id: e.id,
                revision,
                message: "already applied".into(),
                undid: None,
            }),
            // Recovery left it stuck; else a commit is in progress (the same state `op.get`
            // calls pending). The id is not waited for: a transient refusal is not recorded,
            // so the client's next resend finds the entry applied.
            Stage::Prepared if shared.publisher.is_stuck(&e.id) => Err(stuck_entry(&e.id)),
            Stage::Prepared => Err(RpcError::refused(
                ErrorKind::Unavailable,
                "the write is still being applied; try again",
            )),
            Stage::Abandoned => Err(RpcError::refused(ErrorKind::Blocked, ABANDONED)),
        }),
        Ok(None) => None,
        Err(e) => Some(Err(RpcError::refused(ErrorKind::Internal, e.to_string()))),
    }
}

/// The task a replayed create made: the uid the journal entry names and the file's current
/// rev (it may have changed since the create).
fn replayed_created(shared: &Shared, e: &Entry) -> Option<Created> {
    if !matches!(e.kind.as_str(), "create" | "insert") || e.inverse_of.is_some() {
        return None;
    }
    let uid = e.uids.first()?;
    let (_, snap) = shared.publisher.current();
    let t = snap.wire.tasks.iter().find(|t| &t.task.uid == uid)?;
    Some(Created {
        uid: uid.clone(),
        rev: t.rev.clone(),
    })
}

/// The answer to a resent undo that withdrew queued item `item_id`: applied if the item is
/// gone unapplied, else (a crash cut the undo short, so the item stayed and may have drained)
/// not applied.
fn withdrawn(
    shared: &Shared,
    q: &Queue,
    item_id: String,
    revision: u64,
) -> Result<WriteResult, RpcError> {
    let internal = |e: std::io::Error| RpcError::refused(ErrorKind::Internal, e.to_string());
    let committed = Journal::open(&shared.state_dir)
        .and_then(|j| j.by_queue_id(&item_id))
        .map_err(internal)?
        .is_some();
    match q.state(&item_id).map_err(internal)? {
        Some(Queued::Cancelled(_)) if !committed => Ok(WriteResult {
            outcome: WriteOutcome::Applied,
            op_id: item_id,
            revision,
            message: "already undone".into(),
            created: None,
            undid: None,
        }),
        _ => Err(RpcError::refused(ErrorKind::Blocked, UNDO_CUT_SHORT)),
    }
}

pub(super) fn unknown_session() -> RpcError {
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

/// Publishes what a commit changed (`entry`, if any, and any entries drained before it) and
/// refreshes the status. Returns the revision after the reload. Never fails: the commit is
/// done, so a failed reload only delays the publish until the next one.
///
/// Accepted for now: if an outside process takes the vdir lock between our commit and this
/// reload, the reload (and so the applied reply, or the release of the lane after a drain)
/// waits for it to be released.
fn publish_applied(shared: &Shared, entry: Option<&str>, drained: &[Drained]) -> u64 {
    let mut uids = Vec::new();
    if let Ok(j) = Journal::open(&shared.state_dir) {
        let ids = drained
            .iter()
            .filter_map(|d| match &d.outcome {
                DrainOutcome::Applied(id) => Some(id.as_str()),
                _ => None,
            })
            .chain(entry);
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
    // An applied edit may make a batch due.
    shared.poke_scheduler();
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
        // SIGKILL, not abort(): no core dump and no crash report on every test run.
        // SAFETY: signals our own process; nothing after this runs.
        unsafe {
            libc::kill(libc::getpid(), libc::SIGKILL);
        }
    }
}

/// How long a task write waits for an outside holder of the vdir lock before it queues.
const TRY_LOCK: Duration = Duration::from_millis(250);

/// Runs `f` on the writer inside the commit gate's read side: `Err` (the daemon is shutting
/// down) once teardown has closed it. The lane guard is the proof that the caller holds the
/// lane. `f` takes the vdir lock itself, and must not outlive the gate with a `Locked`.
pub(super) fn with_vdir<T>(
    shared: &Shared,
    _lane: &impl LaneHeld,
    f: impl FnOnce(&Writer) -> T,
) -> Result<T, RpcError> {
    with_gate(shared, f)
}

/// [`with_vdir`] without a guard to show: for a write that queues under a long job (the lane
/// guard is not ours, the long job's is) and for the drain hook, which runs while the long
/// job's guard is being dropped. Both still run inside the commit gate.
pub(super) fn with_gate<T>(shared: &Shared, f: impl FnOnce(&Writer) -> T) -> Result<T, RpcError> {
    match shared.commit_gate() {
        Some(_gate) => Ok(f(&shared.writer)),
        None => Err(stopping()),
    }
}

/// [`with_vdir`] or [`with_gate`] by what the lane answered; `f` is told which long job holds
/// it, if any.
fn with_lane<T>(
    shared: &Shared,
    lane: &Result<WriteGuard<'_>, LongBusy>,
    f: impl FnOnce(&Writer, Option<LongKind>) -> T,
) -> Result<T, RpcError> {
    match lane {
        Ok(guard) => with_vdir(shared, guard, |w| f(w, None)),
        Err(LongBusy(kind)) => with_gate(shared, |w| f(w, Some(*kind))),
    }
}

/// The collection fence of what a write queues: during a list job it waits for the drain
/// (daemon spec §7.4), else it is checked at once.
fn fence_for(long: Option<LongKind>) -> Fence {
    if long == Some(LongKind::List) {
        Fence::AtDrain
    } else {
        Fence::Now
    }
}

/// The request an RPC's write is stamped with.
fn request_of(session: &str, request_id: &str) -> RequestRef {
    RequestRef {
        session: session.to_string(),
        request_id: request_id.to_string(),
    }
}

/// The id `undid.op_id` names for a step: its journal entry, or its queue item.
fn step_op_id(step: &Step) -> String {
    match &step.state {
        StepState::Applied(id) => id.clone(),
        StepState::Queued(item) | StepState::Cancelled(item) => item.id.clone(),
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
    // Cloned out before the lane is taken: the live configuration is a leaf, never held.
    let cfg = shared.publisher.config();
    let mut s = super::sessions::lock(&session);
    let lane = shared.lane.write();
    let ctx = Ctx {
        now: Utc::now(),
        request: Some(request_of(&p.session, &p.request_id)),
        config: &cfg,
    };
    let mut drained = Vec::new();
    let outcome = with_lane(shared, &lane, |writer, long| {
        if let Some(hook) = &shared.in_commit {
            hook();
        }
        // A long job holds the vdir: plan and queue at once, without trying the lock.
        let prepared = writer.plan(&view.snap, &p.op, ctx.now)?;
        if long.is_none()
            && let Some(locked) = writer.lock(TRY_LOCK)?
        {
            drained.extend(locked.drain(&ctx)?);
            return Ok(Submitted::Applied(locked.commit(prepared, &ctx)?));
        }
        let kind = Kind::Op {
            op: p.op.clone(),
            prepared,
        };
        let item = writer.enqueue(kind, &label, "daemon", fence_for(long), &ctx)?;
        Ok(Submitted::Queued(item))
    })?;
    let submitted = match outcome {
        Ok(x) => x,
        Err(e) => {
            finish_drained(shared, drained);
            return Err(refusal(e, &view.snap, &shared.publisher.current().1.snap));
        }
    };
    s.history.record(label, &submitted);
    let message = format!(
        "{}{}",
        success_message(&view.snap, &p.op),
        queued_note(&submitted)
    );
    let result = match &submitted {
        Submitted::Applied(id) => {
            crash_point(shared);
            let revision = publish_applied(shared, Some(id), &drained);
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
        Submitted::Queued(item) => queued(shared, item, message, None),
        Submitted::Cancelled => unreachable!("a submit never cancels"),
    };
    drop(lane);
    drop(s);
    finish_drained(shared, drained);
    Ok(result)
}

/// The rows a queued item changes, for the `data.changed` hint of its overlay.
fn item_uids(shared: &Shared, item: &Item) -> Vec<String> {
    match &item.kind {
        Kind::Op { prepared, .. } => prepared.entry.uids.clone(),
        Kind::Revert { entry, .. } => Journal::open(&shared.state_dir)
            .ok()
            .and_then(|j| j.get(entry).ok().flatten())
            .map(|e| e.uids)
            .unwrap_or_default(),
    }
}

fn queued(shared: &Arc<Shared>, item: &Item, message: String, undid: Option<Undid>) -> WriteResult {
    if let Some(hook) = &shared.before_queued_reply {
        hook();
    }
    // The overlay (the row pending) and the status are published in the background: a queued
    // reply never waits on `reload_lock` or the vdir lock, which a reload blocked behind an
    // outside lock holder may hold for minutes. The reply carries the revision before it.
    shared.request_overlay_publish(item_uids(shared, item));
    // The long job ended between our `LongBusy` and the push, after its drain's last pass:
    // nothing else would drain the item. (If it is still running, its release hook sees the
    // item, which is on disk by now.)
    if shared.lane.current_long().is_none() {
        shared.request_drain();
    }
    WriteResult {
        outcome: WriteOutcome::Queued,
        op_id: item.id.clone(),
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
    let cfg = shared.publisher.config();
    let mut s = super::sessions::lock(&session);
    let lane = shared.lane.write();
    let ctx = Ctx {
        now: Utc::now(),
        request: Some(request_of(&p.session, &p.request_id)),
        config: &cfg,
    };
    let mut drained = Vec::new();
    // The history takes the vdir lock itself, and only when a step needs it; under a long job
    // it never locks and queues the revert.
    let r = with_lane(shared, &lane, |writer, long| {
        let fence = fence_for(long);
        let access = match long {
            None => Access::Try {
                writer,
                wait: TRY_LOCK,
                fence,
            },
            Some(_) => Access::Queue(writer, fence),
        };
        if redo {
            s.history.redo(access, "daemon", &ctx, &mut drained)
        } else {
            s.history.undo(access, "daemon", &ctx, &mut drained)
        }
    })?;
    let (submitted, moved) = match r {
        Ok(x) => x,
        Err(e) => {
            finish_drained(shared, drained);
            let cur = shared.publisher.current().1;
            return Err(refusal(e, &cur.snap, &cur.snap));
        }
    };
    let verb = if redo { "redone" } else { "undone" };
    let message = format!("{verb}: {}{}", moved.title, queued_note(&submitted));
    let undid = Some(Undid {
        op_id: Some(step_op_id(&moved)),
        title: moved.title.clone(),
    });
    let result = match &submitted {
        Submitted::Applied(id) => {
            crash_point(shared);
            let revision = publish_applied(shared, Some(id), &drained);
            WriteResult {
                outcome: WriteOutcome::Applied,
                op_id: id.clone(),
                revision,
                message,
                created: None,
                undid,
            }
        }
        Submitted::Queued(item) => queued(shared, item, message, undid),
        Submitted::Cancelled => {
            // The undo removed a still-queued item: nothing reached the vdir. As for a queued
            // reply, publishing the overlay without it must not hold up the answer.
            shared.request_overlay_publish(Vec::new());
            WriteResult {
                outcome: WriteOutcome::Applied,
                op_id: step_op_id(&moved),
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

/// `journal.revert`: undoes the applied entry `op_id` whoever wrote it. It uses no session's
/// history, so it is on nobody's undo or redo stack (D7). Replays from the
/// request log, then the journal and the queue, like `op.submit`; during a long job it queues.
pub fn journal_revert(shared: &Arc<Shared>, p: JournalRevertParams) -> Result<Value, RpcError> {
    let (rid, sid) = (p.request_id.clone(), p.session.clone());
    replayed(shared, &rid, &sid, |_session| run_revert(shared, &p))
}

fn run_revert(shared: &Arc<Shared>, p: &JournalRevertParams) -> Result<WriteResult, RpcError> {
    let internal = |e: std::io::Error| RpcError::refused(ErrorKind::Internal, e.to_string());
    let entry = Journal::open(&shared.state_dir)
        .and_then(|j| j.get(&p.op_id))
        .map_err(internal)?;
    let Some(entry) = entry else {
        return Err(RpcError::refused(
            ErrorKind::NotFound,
            format!("no operation {}", p.op_id),
        ));
    };
    // The label `op.undo` would give: the task's title before the edit being undone (for a
    // create, the title it made), from the entry; the current title if it holds no readable
    // task.
    let title = entry_title(&entry).unwrap_or_else(|| {
        let snap = shared.publisher.current().1;
        entry
            .uids
            .first()
            .map(|u| {
                snap.snap
                    .find(u)
                    .map_or_else(|| u.clone(), |i| snap.snap.tasks[i].summary.clone())
            })
            .unwrap_or_default()
    });
    let cfg = shared.publisher.config();
    let lane = shared.lane.write();
    let ctx = Ctx {
        now: Utc::now(),
        request: Some(request_of(&p.session, &p.request_id)),
        config: &cfg,
    };
    let mut drained = Vec::new();
    let outcome = with_lane(shared, &lane, |writer, long| {
        if long.is_none()
            && let Some(locked) = writer.lock(TRY_LOCK)?
        {
            drained.extend(locked.drain(&ctx)?);
            return Ok(Submitted::Applied(locked.revert(&p.op_id, &ctx)?));
        }
        let kind = writer.revert_item(&p.op_id)?;
        let item = writer.enqueue(kind, "undo", "daemon", fence_for(long), &ctx)?;
        Ok(Submitted::Queued(item))
    })?;
    let submitted = match outcome {
        Ok(x) => x,
        Err(e) => {
            finish_drained(shared, drained);
            return Err(refusal_of(e, shared));
        }
    };
    let message = format!("undone: {title}{}", queued_note(&submitted));
    let result = match &submitted {
        Submitted::Applied(id) => {
            crash_point(shared);
            let revision = publish_applied(shared, Some(id), &drained);
            WriteResult {
                outcome: WriteOutcome::Applied,
                op_id: id.clone(),
                revision,
                message,
                created: None,
                undid: None,
            }
        }
        Submitted::Queued(item) => queued(shared, item, message, None),
        Submitted::Cancelled => unreachable!("a revert never cancels"),
    };
    drop(lane);
    finish_drained(shared, drained);
    Ok(result)
}

/// The title of the task `entry` is about, as the entry's write for it held before the edit
/// (or, for a create, after it).
fn entry_title(entry: &Entry) -> Option<String> {
    let uid = entry.uids.first()?;
    entry.writes.iter().find_map(|w| {
        let text = w.before.as_ref().or(w.after.as_ref())?;
        let cal = twydo_core::ical::parse(text.as_bytes()).ok()?;
        cal.root
            .children_named("VTODO")
            .find(|t| t.prop("UID").is_some_and(|p| p.value.trim() == uid))
            .and_then(|t| t.prop("SUMMARY"))
            .map(|p| twydo_core::ical::value::unescape_text(&p.value))
    })
}

/// A write error from a journal method, against the latest published snapshot. A missing
/// entry is `not_found` and an entry in the wrong stage `invalid`.
pub(super) fn refusal_of(e: WriteError, shared: &Shared) -> RpcError {
    if let WriteError::Io(io) = &e {
        match io.kind() {
            std::io::ErrorKind::NotFound => {
                return RpcError::refused(ErrorKind::NotFound, io.to_string());
            }
            std::io::ErrorKind::InvalidInput => {
                return RpcError::refused(ErrorKind::Invalid, io.to_string());
            }
            _ => {}
        }
    }
    let cur = shared.publisher.current().1;
    refusal(e, &cur.snap, &cur.snap)
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

/// What a journal entry says about its operation: applied only once `Applied`. A `Prepared`
/// entry is stuck only once recovery has left it so; any other is mid-commit, so pending.
fn entry_state(shared: &Shared, e: Entry) -> Result<OpState, RpcError> {
    match e.stage {
        Stage::Applied => Ok(OpState::Applied { op_id: e.id }),
        Stage::Prepared if !shared.publisher.is_stuck(&e.id) => Ok(OpState::Pending),
        Stage::Prepared => Err(stuck_entry(&e.id)),
        Stage::Abandoned => Ok(OpState::Refused {
            kind: ErrorKind::Blocked,
            message: ABANDONED.into(),
        }),
    }
}

/// Where operation `op_id` (a journal entry or queue item id) stands.
fn state_of(shared: &Shared, op_id: &str) -> Result<OpState, RpcError> {
    let internal = |e: std::io::Error| RpcError::refused(ErrorKind::Internal, e.to_string());
    let journal = Journal::open(&shared.state_dir).map_err(internal)?;
    if let Ok(Some(e)) = journal.get(op_id) {
        return entry_state(shared, e);
    }
    if let Some(e) = journal.by_queue_id(op_id).map_err(internal)? {
        return entry_state(shared, e);
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
            Queued::Cancelled(_) => OpState::Cancelled,
        });
    }
    if is_queue_id(op_id) {
        // Only an undo removes an item; its record may have been pruned since.
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
                // A recorded answer that is no write result belongs to another kind of
                // request (a sync's job): this is not a write.
                Some(Some(Ok(v))) => Some(
                    serde_json::from_value::<WriteResult>(v)
                        .map(Some)
                        .map_err(|_| {
                            RpcError::refused(ErrorKind::NotFound, format!("no request {rid}"))
                        }),
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
                    // The entry is mid-commit.
                    Some(ErrorKind::Unavailable) => OpState::Pending,
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

/// At most this many drain passes per release. A pass that drained something is followed by
/// another, so an error that stopped it partway (and was swallowed because it had progress)
/// is seen; each pass takes at least one item off the queue.
const DRAIN_PASSES: usize = 64;

/// How many times a drain that finds the vdir lock held tries again (each try waits up to
/// the vdir lock's 250 ms try) before leaving the items for the next release. The holder may be one
/// of our own reloads (a shared lock held for the length of a vdir load), not only another
/// process.
const DRAIN_BUSY_TRIES: usize = 4;

/// Drains the write queue (daemon spec §7.3): when a long job releases the lane (the lane's
/// drain hook) and once at startup. The caller holds the lane, so no write slips in between.
///
/// The writer drains in id order. Another process holding the vdir lock
/// ([`WriteError::Busy`]) is not an error: the items wait for the next drain. An error that
/// leaves items (a stuck journal, I/O) is reported in the status and retried at the next
/// release. For what was drained: the result is published (`Cause::Op`) and the status
/// refreshed, and then each item gets its `op.finished`, so a client acting on the event
/// already finds both up to date.
pub fn drain_queue(shared: &Arc<Shared>) {
    let drained = drain_passes(shared);
    finish_drain(shared, drained);
}

/// The part of [`drain_queue`] that needs the vdir: run it under the lane (a long job's, or a
/// write's), then call [`finish_drain`] with what it returns once the lane is let go, if the
/// lane was taken as a write. It takes no session mutex: a session holder may wait for a write.
pub fn drain_passes(shared: &Arc<Shared>) -> Vec<Drained> {
    if shared.is_stopping() {
        return Vec::new();
    }
    if let Some(hook) = &shared.before_drain_passes {
        hook();
    }
    let empty = Queue::existing(&shared.state_dir)
        .map(|q| q.is_empty())
        .unwrap_or(Ok(true));
    if matches!(empty, Ok(true)) {
        shared.publisher.set_drain_error(None);
        shared.request_status_refresh();
        return Vec::new();
    }
    let cfg = shared.publisher.config();
    let ctx = Ctx {
        now: Utc::now(),
        request: None,
        config: &cfg,
    };
    let mut drained = Vec::new();
    // `None`: keep what the status says (the vdir was busy, nothing was learnt).
    let mut error: Option<Option<String>> = Some(None);
    let mut busy = 0;
    // Closed: the daemon is stopping; the items wait for the next daemon.
    let passes = with_gate(shared, |writer| {
        for _ in 0..DRAIN_PASSES {
            let pass = writer
                .lock(TRY_LOCK)
                .and_then(|locked| locked.ok_or(WriteError::Busy))
                .and_then(|locked| locked.drain(&ctx));
            match pass {
                Ok(d) if d.is_empty() => break,
                Ok(d) => drained.extend(d),
                Err(WriteError::Busy) if drained.is_empty() && busy + 1 < DRAIN_BUSY_TRIES => {
                    busy += 1;
                }
                Err(WriteError::Busy) => {
                    if drained.is_empty() {
                        error = None;
                    }
                    break;
                }
                Err(e) => {
                    eprintln!("tdo daemon: draining the write queue stopped: {e}");
                    error = Some(Some(e.to_string()));
                    break;
                }
            }
        }
    });
    if passes.is_err() {
        return Vec::new();
    }
    if let Some(hook) = &shared.after_drain_passes {
        hook();
    }
    if let Some(e) = error {
        shared.publisher.set_drain_error(e);
    }
    drained
}

/// What follows a drain's passes: the result is published and each item gets its
/// `op.finished`. Takes no lane.
pub fn finish_drain(shared: &Arc<Shared>, drained: Vec<Drained>) {
    if drained.is_empty() {
        shared.request_status_refresh();
        return;
    }
    publish_applied(shared, None, &drained);
    finish_drained(shared, drained);
}

/// `refusals.ack`: marks the named refusals shown, so they leave `status.refusals`. Replayed
/// from the in-memory log only: the refusal record itself is the durable state. Unknown ids
/// (and ids already acknowledged) are ignored; `acknowledged` counts the ones marked now.
pub fn ack(shared: &Arc<Shared>, p: AckParams) -> Result<Value, RpcError> {
    require_request_id(&p.request_id)?;
    let ticket = match shared.requests.begin(&p.request_id) {
        Begin::Done(r) => return r,
        Begin::New(t) => t,
    };
    ticket.finish(run_ack(shared, &p.ids))
}

fn run_ack(shared: &Arc<Shared>, ids: &[String]) -> Result<Value, RpcError> {
    let mut acknowledged = 0;
    let mut failed = None;
    if let Some(q) = Queue::existing(&shared.state_dir) {
        for id in ids {
            let Ok(Some(Queued::Refused(r))) = q.state(id) else {
                continue;
            };
            if r.shown {
                continue;
            }
            match q.mark_shown(id) {
                Ok(()) => acknowledged += 1,
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
    }
    if acknowledged > 0 {
        shared.request_status_refresh();
    }
    if let Some(e) = failed {
        return Err(RpcError::refused(ErrorKind::Internal, e.to_string()));
    }
    serde_json::to_value(AckResult { acknowledged })
        .map_err(|e| RpcError::refused(ErrorKind::Internal, e.to_string()))
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
