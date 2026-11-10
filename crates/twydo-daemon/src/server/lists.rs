//! List actions as jobs (daemon spec §7.4): `lists.create`, `lists.update`, `lists.archive`,
//! `lists.unarchive`, `lists.preview_delete`, `lists.delete`, `lists.reconcile` and
//! `lists.export`. Each runs on the job runner holding the vdir lane long as a list job and
//! calls the [`ListService`] method the TUI calls for that action
//! (`tdo/src/term/lists.rs::perform_list_action`), then publishes the vdir with cause `list`.
//! Edits saved meanwhile are queued and drained when the job ends; the collection fence is
//! checked there ([`twydo_core::store::Store::set_fence_at_drain`]).

use std::sync::Arc;

use serde_json::Value;
use twydo_core::lists::{ListError, ListRequest, ListResult, ListService, ListState, backup};

use super::Shared;
use super::dispatch::ConnState;
use super::jobs::JobBody;
use super::lane::LongKind;
use crate::protocol::events::Cause;
use crate::protocol::jobs::{JobKind, JobResult};
use crate::protocol::lists::{
    ListArchiveParams, ListCreateParams, ListDeleteParams, ListExportParams,
    ListPreviewDeleteParams, ListReconcileParams, ListUpdateParams, WireListState, parse_color,
};
use crate::protocol::rpc::{ErrorKind, RpcError};

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
    let prepare = || -> Result<JobBody, RpcError> {
        let action = prepare()?;
        Ok(Box::new(move |shared: &Arc<Shared>| {
            let result = service(shared)
                .and_then(|mut s| action(shared, &mut s).map_err(list_error))
                .map(job_result);
            // A failed action may have changed the vdir too (a partial operation).
            if let Err(e) = shared.publisher.reload(Cause::List, Vec::new()) {
                eprintln!("tdo daemon: reload after a list job failed: {e}");
            }
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
