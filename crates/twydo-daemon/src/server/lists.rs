//! List actions as jobs (daemon spec §7.4): `lists.create`, `lists.update`, `lists.archive`,
//! `lists.unarchive`, `lists.preview_delete`, `lists.delete`, `lists.reconcile`,
//! `lists.export`, `lists.forget` and `lists.recover`. Each runs on the job runner as a list
//! job holding the vdir lane and calls the [`ListService`] method for that action, then
//! publishes the vdir with cause `list`.
//! Edits saved meanwhile are queued and drained when the job ends; the collection fence is
//! checked there ([`twydo_core::store::Store::set_fence_at_drain`]).

use std::sync::Arc;

use chrono::Utc;
use serde_json::Value;
use twydo_core::lists::{ListError, ListRequest, ListResult, ListService, ListState, backup};
use twydo_core::retired;
use twydo_core::store::journal::RequestRef;
use twydo_core::store::{self, Ctx, Submitted};
use twydo_core::sync::run::RunOptions;

use super::Shared;
use super::dispatch::ConnState;
use super::jobs::{JobBody, NotStarted};
use super::lane::LongKind;
use super::syncjob::run_sync;
use super::writes::{refusal_of, unknown_session};
use crate::protocol::events::Cause;
use crate::protocol::jobs::{JobKind, JobResult};
use crate::protocol::lists::{
    BackupPreviewParams, Backups, DeletedBackupView, ForgetPreview, ListArchiveParams,
    ListCreateParams, ListDeleteParams, ListExportParams, ListForgetParams,
    ListPreviewDeleteParams, ListPreviewForgetParams, ListReconcileParams, ListRecoverParams,
    ListUpdateParams, PendingListOp, RetiredBackupView, WireListState, parse_color,
};
use crate::protocol::rpc::{ErrorKind, RpcError};
use crate::protocol::sync::SyncReason;
use crate::protocol::wire::{WireSnapshot, to_wire};

/// What a list job does with the service.
type Action = Box<dyn FnOnce(&Shared, &mut ListService) -> Result<ListResult, ListError> + Send>;

fn internal(e: impl std::fmt::Display) -> RpcError {
    RpcError::refused(ErrorKind::Internal, e.to_string())
}

fn invalid(m: String) -> RpcError {
    RpcError::refused(ErrorKind::Invalid, m)
}

/// A refusal fails the job as `blocked` with its message; an I/O error as `internal`.
fn list_error(e: ListError) -> RpcError {
    match e {
        ListError::Refused(m) => RpcError::refused(ErrorKind::Blocked, m),
        ListError::Io(e) => internal(e),
    }
}

fn job_result(r: ListResult) -> JobResult {
    match r {
        ListResult::Saved { operation, state } => JobResult::ListSaved {
            operation,
            state: state.into(),
        },
        ListResult::DeletePreview(p) => JobResult::DeletePreview(p.into()),
        ListResult::Exported {
            backup,
            destination,
        } => JobResult::Exported {
            backup,
            destination,
        },
        ListResult::Deleted {
            operation,
            collection,
            name,
            backup,
            ..
        } => JobResult::ListDeleted {
            operation,
            collection,
            name,
            backup,
        },
    }
}

fn expected(w: WireListState) -> Result<ListState, RpcError> {
    ListState::try_from(w).map_err(invalid)
}

/// The service on the daemon's binding configuration, with the instance's config file when
/// it exists (archiving writes it) and the daemon's compatibility matrix.
fn service(shared: &Shared) -> Result<ListService, RpcError> {
    let path = &shared.instance.config;
    let path = path.is_file().then_some(path.as_path());
    ListService::open_with(&shared.cfg, path, shared.matrix.clone()).map_err(internal)
}

/// Starts list job `kind` for `request_id`. `prepare` checks the params (not on a resend) and
/// returns the action; the job runs it, then publishes the vdir as it left it.
fn start(
    shared: &Arc<Shared>,
    conn: &ConnState,
    request_id: &str,
    kind: JobKind,
    prepare: impl FnOnce() -> Result<Action, RpcError>,
) -> Result<Value, RpcError> {
    let session = conn.session.lock().unwrap().clone();
    let prepare = || -> Result<JobBody, NotStarted> {
        let action = prepare()?;
        Ok(Box::new(move |shared: &Arc<Shared>| {
            let result = service(shared)
                .and_then(|mut s| action(shared, &mut s).map_err(list_error))
                .map(job_result);
            // A failed action may have changed the vdir too (a partial operation).
            if let Err(e) = shared.publisher.reload(Cause::List, Vec::new()) {
                eprintln!("tdo daemon: reload after a list job failed: {e}");
            }
            super::admin::retry_deferred_recovery(shared);
            result
        }))
    };
    shared.start_job_with(
        request_id,
        session.as_deref(),
        kind,
        LongKind::List,
        prepare,
    )
}

pub fn create(
    shared: &Arc<Shared>,
    conn: &ConnState,
    p: ListCreateParams,
) -> Result<Value, RpcError> {
    let rid = p.request_id.clone();
    start(shared, conn, &rid, JobKind::ListCreate, || {
        let color = parse_color(&p.color).map_err(invalid)?;
        let request = ListRequest::Create {
            id: p.id,
            name: p.name,
            color,
        };
        Ok(Box::new(move |_, s| s.execute(request)))
    })
}

pub fn update(
    shared: &Arc<Shared>,
    conn: &ConnState,
    p: ListUpdateParams,
) -> Result<Value, RpcError> {
    let rid = p.request_id.clone();
    start(shared, conn, &rid, JobKind::ListUpdate, || {
        let color = p
            .color
            .as_deref()
            .map(parse_color)
            .transpose()
            .map_err(invalid)?;
        let request = ListRequest::Edit {
            id: p.id,
            name: p.name,
            color,
            expected: expected(p.expected)?,
        };
        Ok(Box::new(move |_, s| s.execute(request)))
    })
}

/// `lists.archive` (`archived` true) and `lists.unarchive`.
pub fn archive(
    shared: &Arc<Shared>,
    conn: &ConnState,
    p: ListArchiveParams,
    archived: bool,
) -> Result<Value, RpcError> {
    let kind = if archived {
        JobKind::ListArchive
    } else {
        JobKind::ListUnarchive
    };
    let rid = p.request_id.clone();
    start(shared, conn, &rid, kind, || {
        let expected = expected(p.expected)?;
        Ok(Box::new(move |_, s| {
            s.set_archived(&expected.id, archived, &expected)
                .map(|state| ListResult::Saved {
                    operation: String::new(),
                    state,
                })
        }))
    })
}

pub fn preview_delete(
    shared: &Arc<Shared>,
    conn: &ConnState,
    p: ListPreviewDeleteParams,
) -> Result<Value, RpcError> {
    let rid = p.request_id.clone();
    start(shared, conn, &rid, JobKind::ListPreviewDelete, || {
        Ok(Box::new(move |_, s| {
            s.preview_delete(&p.id).map(ListResult::DeletePreview)
        }))
    })
}

pub fn delete(
    shared: &Arc<Shared>,
    conn: &ConnState,
    p: ListDeleteParams,
) -> Result<Value, RpcError> {
    let rid = p.request_id.clone();
    start(shared, conn, &rid, JobKind::ListDelete, || {
        let preview = p.preview.into();
        Ok(Box::new(move |_, s| s.delete_list(&preview)))
    })
}

pub fn reconcile(
    shared: &Arc<Shared>,
    conn: &ConnState,
    p: ListReconcileParams,
) -> Result<Value, RpcError> {
    let rid = p.request_id.clone();
    start(shared, conn, &rid, JobKind::ListReconcile, || {
        Ok(Box::new(move |_, s| {
            if p.keep_server {
                s.keep_server(&p.operation)
            } else {
                s.reconcile(&p.operation)
            }
        }))
    })
}

pub fn export(
    shared: &Arc<Shared>,
    conn: &ConnState,
    p: ListExportParams,
) -> Result<Value, RpcError> {
    let rid = p.request_id.clone();
    start(shared, conn, &rid, JobKind::ListExport, || {
        Ok(Box::new(move |shared, _| {
            backup::export(&shared.cfg, &p.backup, &p.destination)?;
            Ok(ListResult::Exported {
                backup: p.backup,
                destination: p.destination,
            })
        }))
    })
}

/// An I/O error from the retired-list functions: a bad name or id is the caller's.
fn io_refusal(e: std::io::Error) -> RpcError {
    match e.kind() {
        std::io::ErrorKind::InvalidInput => RpcError::refused(ErrorKind::Invalid, e.to_string()),
        std::io::ErrorKind::NotFound => RpcError::refused(ErrorKind::NotFound, e.to_string()),
        _ => internal(e),
    }
}

/// What a list job runs, apart from the service-based ones.
type Body = Box<dyn FnOnce(&Arc<Shared>) -> Result<JobResult, RpcError> + Send>;

/// Starts a list job for `request_id`. `prepare` checks the params (not on a resend) and
/// returns the body; the job runs it, then publishes the vdir with cause `list`.
fn start_body(
    shared: &Arc<Shared>,
    conn: &ConnState,
    request_id: &str,
    kind: JobKind,
    prepare: impl FnOnce() -> Result<Body, NotStarted>,
) -> Result<Value, RpcError> {
    let session = conn.session.lock().unwrap().clone();
    shared.start_job_with(request_id, session.as_deref(), kind, LongKind::List, || {
        let body = prepare()?;
        Ok(Box::new(move |shared: &Arc<Shared>| {
            let result = body(shared);
            if let Err(e) = shared.publisher.reload(Cause::List, Vec::new()) {
                eprintln!("tdo daemon: reload after a list job failed: {e}");
            }
            super::admin::retry_deferred_recovery(shared);
            shared.poke_scheduler();
            result
        }))
    })
}

/// `lists.preview_forget`: what forgetting list `name` would preserve.
pub fn preview_forget(shared: &Arc<Shared>, p: ListPreviewForgetParams) -> Result<Value, RpcError> {
    let preview = retired::preview(&shared.cfg, &p.name).map_err(io_refusal)?;
    serde_json::to_value(ForgetPreview::of(&preview)).map_err(internal)
}

/// `lists.forget`: preserves the list, then (holding the same lane) runs the manual sync the
/// TUI runs after it. The sync's end is published once the lane is let go.
pub fn forget(
    shared: &Arc<Shared>,
    conn: &ConnState,
    p: ListForgetParams,
) -> Result<Value, RpcError> {
    let rid = p.request_id.clone();
    start_body(shared, conn, &rid, JobKind::ListForget, || {
        Ok(Box::new(move |shared: &Arc<Shared>| {
            let preview = retired::preview(&shared.cfg, &p.name).map_err(io_refusal)?;
            if preview.collection != p.collection {
                return Err(RpcError::refused(
                    ErrorKind::Blocked,
                    "list changed; reload and try again",
                ));
            }
            let b = retired::forget(&shared.cfg, &preview, Utc::now()).map_err(|e| {
                RpcError::refused(ErrorKind::Blocked, format!("list was not removed: {e}"))
            })?;
            let mut message = format!(
                "preserved {:?} as {}; recover with :recover {} <destination-list>",
                b.name, b.id, b.id
            );
            let sync = shared.cfg.sync.is_some().then(|| {
                let job_id = shared.runner.current_job();
                let done = run_sync(
                    shared,
                    job_id.as_deref(),
                    SyncReason::List,
                    RunOptions::default(),
                );
                let result = done.result.clone();
                shared.runner.defer_sync_end(done);
                Box::new(result)
            });
            if let Some(r) = &sync
                && !r.problems.is_empty()
            {
                message = format!("{message} \u{b7} {}", r.problems.join("; "));
            }
            Ok(JobResult::Forgot {
                backup: b.id,
                name: b.name,
                message,
                sync,
            })
        }))
    })
}

/// `lists.recover`: copies a preserved list's tasks into list `to`. Waits behind a running
/// sync like any list job.
pub fn recover(
    shared: &Arc<Shared>,
    conn: &ConnState,
    p: ListRecoverParams,
) -> Result<Value, RpcError> {
    let rid = p.request_id.clone();
    start_body(shared, conn, &rid, JobKind::ListRecover, || {
        // An unknown session is not recorded: the request resent under a new one runs.
        let session = shared
            .sessions
            .get(&p.session)
            .ok_or_else(|| NotStarted::Unrecorded(unknown_session()))?;
        Ok(Box::new(move |shared: &Arc<Shared>| {
            // The recovery goes on the calling session's store and undo stack, so its `op.undo`
            // undoes it (as `u` after `:recover` does in the TUI). The job holds the lane long,
            // and session holders never wait for a long job.
            let mut s = super::sessions::lock(&session);
            shared.test_point("recover");
            let snap = Arc::clone(&shared.publisher.current().1);
            let ctx = Ctx {
                now: Utc::now(),
                request: Some(RequestRef {
                    session: p.session,
                    request_id: p.request_id,
                }),
                config: &shared.cfg,
            };
            let r = shared.writer.lock_waiting().and_then(|locked| {
                store::recover_list(&locked, &snap.snap, &p.backup, &p.to, &ctx)
            });
            match r {
                Ok((_, false)) => Ok(JobResult::Recovered {
                    op_id: None,
                    applied: false,
                    message: "already recovered; backup retained".into(),
                }),
                Ok((op_id, true)) => {
                    s.store.record_recovered(&op_id);
                    s.pushed("recovered list".into(), &Submitted::Applied(op_id.clone()));
                    Ok(JobResult::Recovered {
                        op_id: Some(op_id),
                        applied: true,
                        message: format!("recovered local snapshot into {}; backup retained", p.to),
                    })
                }
                Err(e) => Err(refusal_of(e, shared)),
            }
        }))
    })
}

/// `lists.backups`: the preserved lists and the backups list deletions kept, newest first.
pub fn backups(shared: &Arc<Shared>) -> Result<Value, RpcError> {
    let retired = retired::list(&shared.cfg)
        .map_err(io_refusal)?
        .into_iter()
        .map(|b| RetiredBackupView::of(&b))
        .collect();
    let deleted = backup::list(&shared.cfg)
        .map_err(io_refusal)?
        .into_iter()
        .map(|b| DeletedBackupView {
            id: b.id,
            collection: b.collection,
            name: b.name,
            created: b.created,
            resources: b.remote.resources.len(),
            local_files: b.local.len(),
        })
        .collect();
    serde_json::to_value(Backups { retired, deleted }).map_err(internal)
}

/// `lists.backup_preview`: the tasks of a preserved list, as a snapshot (revision 0).
pub fn backup_preview(shared: &Arc<Shared>, p: BackupPreviewParams) -> Result<Value, RpcError> {
    let snap = retired::snapshot(&shared.cfg, &p.id).map_err(io_refusal)?;
    let wire: WireSnapshot = to_wire(&snap, 0);
    serde_json::to_value(wire).map_err(internal)
}

/// `lists.pending`: the unfinished list save, if any.
pub fn pending(shared: &Arc<Shared>) -> Result<Value, RpcError> {
    let record = twydo_core::lists::operation::active(&shared.cfg).map_err(io_refusal)?;
    serde_json::to_value(record.as_ref().map(PendingListOp::of)).map_err(internal)
}
