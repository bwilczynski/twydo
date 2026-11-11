//! The conflict methods (daemon spec §6.8): `conflicts.list` and `conflicts.resolve`, and the
//! journal methods that are not task writes: `journal.list`, `journal.recover` and
//! `journal.abandon` (`journal.revert` lives with the writes, in `writes.rs`).
//!
//! A resolution is a write the queue cannot hold, so it waits behind a running job
//! ([`Lane::write_waiting`](super::lane::Lane::write_waiting)), as `journal.recover` and
//! `journal.abandon` do; for the locks they take, see the lock order in `writes.rs`. The
//! conflict records the request, so a resend after a restart replays
//! ([`ConflictStore::by_request`]).

use std::sync::Arc;

use chrono::Utc;
use serde_json::Value;
use twydo_core::conflicts::{ConflictStore, ResolvedBy};
use twydo_core::retired;
use twydo_core::store::Store;
use twydo_core::store::existing_vdir_state;
use twydo_core::store::journal::{Journal, RequestRef, try_recover};
use twydo_core::sync::resolve::{Keep, Resolution, ResolveError, resolve_with};
use twydo_core::sync::switch;

use super::Shared;
use super::dispatch::ConnState;
use super::replay::Begin;
use super::writes::{refusal_of, require_request_id};
use crate::protocol::admin::{
    AbandonResult, ConflictDetail, ConflictList, ConflictView, ConflictsResolveParams,
    ConflictsShowParams, InputField, InputParseParams, InputParsed, JournalAbandonParams,
    JournalEntryView, ParsedRepeat, RecoverResult, ResolveResult, VersionView, WireKeep,
};
use crate::protocol::events::Cause;
use crate::protocol::rpc::{ErrorKind, RpcError};
use crate::protocol::sync::SyncReason;

/// How long `journal.recover` waits for the vdir lock before answering `unavailable`.
const RECOVER_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

fn internal(e: impl std::fmt::Display) -> RpcError {
    RpcError::refused(ErrorKind::Internal, e.to_string())
}

fn to_value<T: serde::Serialize>(r: T) -> Result<Value, RpcError> {
    serde_json::to_value(r).map_err(internal)
}

/// `conflicts.list`: the unresolved conflicts, as `tdo conflicts` shows them (those of an
/// archived list are left out, as the sync summary leaves them out).
pub fn conflicts_list(shared: &Arc<Shared>) -> Result<Value, RpcError> {
    let found = (|| -> std::io::Result<Vec<ConflictView>> {
        let cfg = &shared.cfg;
        let Some(state) = existing_vdir_state(&cfg.state_dir(), &cfg.vdir)? else {
            return Ok(Vec::new());
        };
        let Some(store) = ConflictStore::existing(&state)? else {
            return Ok(Vec::new());
        };
        let archived = retired::list(cfg)?;
        Ok(store
            .unresolved()?
            .iter()
            .filter(|c| !archived.iter().any(|b| b.conflicts.contains(&c.id)))
            .map(ConflictView::of)
            .collect())
    })()
    .map_err(internal)?;
    to_value(ConflictList { conflicts: found })
}

/// `conflicts.show`: both versions of one conflict and the task as the vdir holds it now.
/// Read-only; an unknown id is `not_found`.
pub fn conflicts_show(shared: &Arc<Shared>, p: ConflictsShowParams) -> Result<Value, RpcError> {
    let cfg = &shared.cfg;
    let not_found = || RpcError::refused(ErrorKind::NotFound, format!("no conflict {}", p.id));
    let Some(state) = existing_vdir_state(&cfg.state_dir(), &cfg.vdir).map_err(internal)? else {
        return Err(not_found());
    };
    let Some(store) = ConflictStore::existing(&state).map_err(internal)? else {
        return Err(not_found());
    };
    let Some(c) = store.get(&p.id).map_err(internal)? else {
        return Err(not_found());
    };
    let side = |text: &Option<String>, name: &str| {
        text.as_deref()
            .map(|t| VersionView::of(t, Some(state.join("conflicts").join(&c.id).join(name))))
    };
    let vdir_file = cfg.vdir.join(&c.path);
    let current = match std::fs::read(&vdir_file) {
        Ok(b) => Some(VersionView::of(&String::from_utf8_lossy(&b), None)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(internal(format!("{}: {e}", vdir_file.display()))),
    };
    to_value(ConflictDetail {
        conflict: ConflictView::of(&c),
        local: side(&c.local, "local.ics"),
        remote: side(&c.remote, "remote.ics"),
        current,
        vdir_file,
    })
}

/// What a recorded resolution answered: from the conflict's own record, else from the journal
/// entry the request wrote (a crash between `restore` and recording the resolution leaves only
/// that), in which case the conflict is closed now, as the first run would have.
fn replay_of(
    shared: &Shared,
    request_id: &str,
    conflict: &str,
) -> Option<Result<ResolveResult, RpcError>> {
    let store = match ConflictStore::existing(&shared.state_dir) {
        Ok(s) => s?,
        Err(e) => return Some(Err(internal(e))),
    };
    match store.by_request(request_id) {
        Ok(Some((_, by))) => return Some(Ok(ResolveResult::of(&by.resolution))),
        Ok(None) => {}
        Err(e) => return Some(Err(internal(e))),
    }
    if !shared.state_dir.join("journal").is_dir() {
        return None;
    }
    let entry = match Journal::open(&shared.state_dir).and_then(|j| j.by_request(request_id)) {
        Ok(Some(e)) => e,
        Ok(None) => return None,
        Err(e) => return Some(Err(internal(e))),
    };
    let resolution = Resolution::Written(entry.id.clone());
    let by = entry.request.map(|request| ResolvedBy {
        request,
        op_id: Some(entry.id),
        resolution: resolution.clone(),
    });
    if let Err(e) = store.mark_resolved_by(conflict, Utc::now(), by) {
        return Some(Err(internal(e)));
    }
    Some(Ok(ResolveResult::of(&resolution)))
}

fn refusal(e: ResolveError) -> RpcError {
    let kind = match &e {
        ResolveError::NotFound(_) => ErrorKind::NotFound,
        ResolveError::NotUnresolved(_) | ResolveError::Held(_) | ResolveError::Changed { .. } => {
            ErrorKind::Invalid
        }
        ResolveError::Write(twydo_core::store::WriteError::Busy) => ErrorKind::Unavailable,
        ResolveError::Write(_) => ErrorKind::Internal,
        ResolveError::Io(e) if e.kind() == std::io::ErrorKind::InvalidInput => ErrorKind::Invalid,
        ResolveError::Io(_) => ErrorKind::Internal,
    };
    RpcError::refused(kind, e.to_string())
}

/// `conflicts.resolve`: keeps one version of a conflict. Replays from the request log, then
/// from the conflict's own record, before anything else is looked at.
pub fn conflicts_resolve(
    shared: &Arc<Shared>,
    conn: &ConnState,
    p: ConflictsResolveParams,
) -> Result<Value, RpcError> {
    require_request_id(&p.request_id)?;
    let ticket = match shared.requests.begin(&p.request_id) {
        Begin::Done(r) => return r,
        Begin::New(t) => t,
    };
    if let Some(found) = replay_of(shared, &p.request_id, &p.id) {
        return ticket.finish(found.and_then(to_value));
    }
    let Some(session) = conn.session.lock().unwrap().clone() else {
        return ticket.forget(Err(RpcError::refused(
            ErrorKind::Invalid,
            "say hello first",
        )));
    };
    ticket.finish(run_resolve(shared, session, &p).and_then(to_value))
}

fn run_resolve(
    shared: &Arc<Shared>,
    session: String,
    p: &ConflictsResolveParams,
) -> Result<ResolveResult, RpcError> {
    let keep = match p.keep {
        WireKeep::Local => Keep::Local,
        WireKeep::Remote => Keep::Remote,
        WireKeep::Current => Keep::Current,
    };
    let request = RequestRef {
        session,
        request_id: p.request_id.clone(),
    };
    let lane = shared.lane.write_waiting();
    let resolution = {
        let Some(_gate) = shared.commit_gate() else {
            return Err(RpcError::refused(
                ErrorKind::Unavailable,
                "the daemon is shutting down",
            ));
        };
        resolve_with(&shared.cfg, &p.id, keep, Utc::now(), Some(request)).map_err(refusal)?
    };
    let uids = conflict_uids(shared, &p.id)?;
    if let Err(e) = shared.publisher.reload(Cause::Op, uids) {
        eprintln!("tdo daemon: reload after a conflict resolution failed: {e}");
    }
    shared.publisher.refresh_status();
    drop(lane);
    let auto = shared.cfg.sync.is_some() && !switch::is_off(&shared.cfg).unwrap_or(false);
    if resolution == Resolution::Recorded && auto {
        // A recorded choice applies only on a sync.
        shared.sync_now(SyncReason::Resolve);
    } else {
        shared.poke_scheduler();
    }
    Ok(ResolveResult::of(&resolution))
}

/// The uid of conflict `id`, for the reload's hint; an unreadable record is an error, not a
/// publish without uids.
fn conflict_uids(shared: &Shared, id: &str) -> Result<Vec<String>, RpcError> {
    let Some(store) = ConflictStore::existing(&shared.state_dir).map_err(internal)? else {
        return Ok(Vec::new());
    };
    Ok(store
        .get(id)
        .map_err(internal)?
        .map(|c| vec![c.uid])
        .unwrap_or_default())
}

/// `journal.list`: every journal entry, oldest first, as `tdo op journal` lists them.
pub fn journal_list(shared: &Arc<Shared>) -> Result<Value, RpcError> {
    let entries = (|| -> std::io::Result<Vec<JournalEntryView>> {
        let Some(state) = existing_vdir_state(&shared.cfg.state_dir(), &shared.cfg.vdir)? else {
            return Ok(Vec::new());
        };
        Ok(Journal::open(&state)?
            .list()?
            .iter()
            .map(JournalEntryView::of)
            .collect())
    })()
    .map_err(internal)?;
    to_value(entries)
}

/// `journal.recover`: rolls every unfinished entry forward, as a daemon start does. Waits for
/// a running job (recovery needs the vdir), publishes with cause `recovery` when something
/// resumed and keeps the stuck ids for the status.
pub fn journal_recover(shared: &Arc<Shared>) -> Result<Value, RpcError> {
    let lane = shared.lane.write_waiting();
    let report = {
        let Some(_gate) = shared.commit_gate() else {
            return Err(RpcError::refused(
                ErrorKind::Unavailable,
                "the daemon is shutting down",
            ));
        };
        // Not `Store::open_with`: it reports a busy vdir as an empty report, which would
        // wipe the stuck list. A reload or an outside `tdo` may hold the lock briefly.
        let state = shared.state_dir.as_path();
        let deadline = std::time::Instant::now() + RECOVER_WAIT;
        loop {
            match try_recover(&shared.cfg.vdir, state).map_err(internal)? {
                Some(report) => break report,
                None if std::time::Instant::now() >= deadline => {
                    return Err(RpcError::refused(
                        ErrorKind::Unavailable,
                        "the vdir is busy; try again",
                    ));
                }
                None => std::thread::sleep(std::time::Duration::from_millis(25)),
            }
        }
    };
    shared
        .publisher
        .set_stuck(report.stuck.iter().map(|(id, _)| id.clone()).collect());
    if !report.resumed.is_empty()
        && let Err(e) = shared.publisher.reload(Cause::Recovery, Vec::new())
    {
        eprintln!("tdo daemon: reload after a recovery failed: {e}");
    }
    shared.publisher.refresh_status();
    drop(lane);
    // Writes the unfinished entries held back may now go through (as after abandon).
    shared.request_drain();
    if !report.resumed.is_empty() {
        shared.poke_scheduler();
    }
    to_value(RecoverResult::of(&report))
}

/// `journal.abandon`: gives up on an unfinished entry so writes can continue. Idempotent (an
/// entry already abandoned answers `abandoned: false`), so the request log's replay is enough.
pub fn journal_abandon(shared: &Arc<Shared>, p: JournalAbandonParams) -> Result<Value, RpcError> {
    require_request_id(&p.request_id)?;
    let ticket = match shared.requests.begin(&p.request_id) {
        Begin::Done(r) => return r,
        Begin::New(t) => t,
    };
    ticket.finish(run_abandon(shared, &p).and_then(to_value))
}

fn run_abandon(shared: &Arc<Shared>, p: &JournalAbandonParams) -> Result<AbandonResult, RpcError> {
    let lane = shared.lane.write_waiting();
    let abandoned = {
        let Some(_gate) = shared.commit_gate() else {
            return Err(RpcError::refused(
                ErrorKind::Unavailable,
                "the daemon is shutting down",
            ));
        };
        let mut store = Store::without_recovery(&shared.cfg, shared.matrix.clone())
            .map_err(|e| refusal_of(e, shared))?;
        store.abandon(&p.op_id).map_err(|e| refusal_of(e, shared))?
    };
    shared.publisher.unstick(&p.op_id);
    shared.publisher.refresh_status();
    drop(lane);
    // Writes the stuck entry held back may now go through.
    shared.request_drain();
    Ok(AbandonResult { abandoned })
}

/// `input.parse`: reads typed text of a due, start or repeat field with the code the TUI uses
/// (`dateinput`, `repeatinput`), against the live configuration's time zone and work days. A
/// parse error is `invalid`, with the parser's message.
pub fn input_parse(shared: &Arc<Shared>, p: InputParseParams) -> Result<Value, RpcError> {
    use twydo_core::{dateinput, repeatinput};
    let cfg = shared.publisher.config();
    let today = p
        .today
        .unwrap_or_else(|| Utc::now().with_timezone(&cfg.tz()).date_naive());
    let invalid = |m: String| RpcError::refused(ErrorKind::Invalid, m);
    let out = match p.field {
        InputField::Due | InputField::Start => {
            match dateinput::parse(&p.text, today).map_err(invalid)? {
                Some(d) => InputParsed {
                    due: Some(d.due()),
                    repeat: None,
                    cleared: false,
                    description: dateinput::describe(&d, today),
                },
                None => cleared(),
            }
        }
        InputField::Repeat => {
            match repeatinput::parse(&p.text, today, &cfg.work_days).map_err(invalid)? {
                Some(spec) => InputParsed {
                    due: None,
                    repeat: Some(ParsedRepeat {
                        rrule: twydo_core::edit::complete::rule_text(&spec.rule),
                        from_completion: spec.from_completion,
                        spec: spec.clone(),
                    }),
                    cleared: false,
                    description: repeatinput::describe(&spec, p.anchor),
                },
                None => cleared(),
            }
        }
    };
    to_value(out)
}

fn cleared() -> InputParsed {
    InputParsed {
        due: None,
        repeat: None,
        cleared: true,
        description: String::new(),
    }
}

/// Runs the journal recovery a daemon deferred at startup because an unfinished list operation
/// fenced the vdir, once that operation has ended. Does nothing when none was deferred or the
/// fence still holds. Called at the end of every list job.
pub fn retry_deferred_recovery(shared: &Arc<Shared>) {
    if !shared.publisher.recovery_deferred() {
        return;
    }
    if matches!(
        twydo_core::lists::operation::fenced_collection(&shared.state_dir),
        Ok(Some(_))
    ) {
        return;
    }
    match try_recover(&shared.cfg.vdir, &shared.state_dir) {
        Ok(Some(report)) => {
            shared.publisher.set_recovery_deferred(None);
            shared
                .publisher
                .set_stuck(report.stuck.iter().map(|(id, _)| id.clone()).collect());
            if !report.resumed.is_empty()
                && let Err(e) = shared.publisher.reload(Cause::Recovery, Vec::new())
            {
                eprintln!("tdo daemon: reload after a recovery failed: {e}");
            }
            shared.publisher.refresh_status();
            shared.request_drain();
        }
        // The vdir is busy: the next list job (or `journal.recover`) tries again.
        Ok(None) => {}
        Err(e) => eprintln!("tdo daemon: deferred recovery failed: {e}"),
    }
}
