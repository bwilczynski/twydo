//! Request dispatch: the lifecycle methods and the table later tasks extend.

use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::Shared;
use crate::PROTOCOL;
use crate::protocol::methods::{DaemonInfo, HelloParams, HelloResult, ShutdownParams};
use crate::protocol::rpc::{ErrorData, ErrorKind, INVALID_PARAMS, METHOD_NOT_FOUND, RpcError};
use crate::schema;

fn invalid_params(e: impl std::fmt::Display) -> RpcError {
    RpcError {
        code: INVALID_PARAMS,
        message: format!("invalid params: {e}"),
        data: Some(ErrorData::new(ErrorKind::Invalid)),
    }
}

fn parse<P: DeserializeOwned>(params: Option<Value>) -> Result<P, RpcError> {
    serde_json::from_value(params.unwrap_or_else(|| json!({}))).map_err(invalid_params)
}

fn to_result<R: serde::Serialize>(r: R) -> Result<Value, RpcError> {
    serde_json::to_value(r).map_err(|e| RpcError::refused(ErrorKind::Internal, e.to_string()))
}

/// Per-connection state the dispatch needs.
pub struct ConnState {
    pub id: u64,
    pub session: Mutex<Option<String>>,
    /// The connection's outbound queue; the bus sends notifications into it.
    pub out: SyncSender<String>,
}

impl ConnState {
    pub fn new(id: u64, out: SyncSender<String>) -> ConnState {
        ConnState {
            id,
            session: Mutex::new(None),
            out,
        }
    }
}

/// Is `method` answered without a successful hello?
fn is_lifecycle(method: &str) -> Option<bool> {
    schema::METHODS
        .iter()
        .find(|m| m.name == method)
        .map(|m| m.lifecycle)
}

/// Runs one request. Methods the later tasks implement get an arm in the final `match`.
pub fn dispatch(
    shared: &Arc<Shared>,
    conn: &ConnState,
    method: &str,
    params: Option<Value>,
) -> Result<Value, RpcError> {
    let Some(lifecycle) = is_lifecycle(method) else {
        return Err(RpcError {
            code: METHOD_NOT_FOUND,
            message: format!("method not found: {method}"),
            data: None,
        });
    };
    if lifecycle {
        return match method {
            "hello" => hello(shared, conn, parse(params)?),
            "rpc.discover" => Ok(schema::document()),
            "daemon.info" => to_result(DaemonInfo {
                version: shared.version.clone(),
                protocol: PROTOCOL,
                pid: std::process::id(),
                uptime_secs: shared.started.elapsed().as_secs(),
                vdir: shared.vdir.clone(),
                epoch: shared.epoch.clone(),
            }),
            "daemon.shutdown" => {
                // The reply is queued by the caller before stopping: see `conn::serve`. With
                // `when_idle`, new jobs are refused from here on, before the reply.
                let p: ShutdownParams = parse(params)?;
                if p.when_idle {
                    shared.runner.set_closing();
                }
                Ok(json!({}))
            }
            _ => unreachable!("lifecycle table and dispatch disagree"),
        };
    }
    if conn.session.lock().unwrap().is_none() {
        return Err(RpcError::refused(ErrorKind::Invalid, "say hello first"));
    }
    if shared.is_stopping() {
        return Err(RpcError::refused(
            ErrorKind::Unavailable,
            "the daemon is shutting down",
        ));
    }
    if let Some(e) = shared.startup_error.lock().unwrap().as_ref() {
        return Err(RpcError::refused(
            ErrorKind::Internal,
            format!("the daemon failed to start: {e}"),
        ));
    }
    if !shared.is_ready() {
        return Err(RpcError::refused(
            ErrorKind::Unavailable,
            "the daemon is starting",
        ));
    }
    match method {
        "refusals.ack" => super::writes::ack(shared, parse(params)?),
        "op.submit" => super::writes::submit(shared, parse(params)?),
        "op.undo" => super::writes::undo(shared, parse(params)?, false),
        "op.redo" => super::writes::undo(shared, parse(params)?, true),
        "op.get" => super::writes::get(shared, parse(params)?),
        "op.check" => super::writes::check(shared, parse(params)?),
        "sync.run" => super::syncjob::sync_run(shared, conn, parse(params)?),
        "sync.stop" => super::syncjob::sync_stop(shared),
        "sync.set_auto" => super::syncjob::sync_set_auto(shared, parse(params)?),
        "conflicts.list" => super::admin::conflicts_list(shared),
        "conflicts.resolve" => super::admin::conflicts_resolve(shared, conn, parse(params)?),
        "journal.list" => super::admin::journal_list(shared),
        "journal.recover" => super::admin::journal_recover(shared),
        "journal.revert" => super::writes::journal_revert(shared, parse(params)?),
        "journal.abandon" => super::admin::journal_abandon(shared, parse(params)?),
        "input.parse" => super::admin::input_parse(shared, parse(params)?),
        "job.get" => super::jobs::get(shared, parse(params)?),
        "lists.create" => super::lists::create(shared, conn, parse(params)?),
        "lists.update" => super::lists::update(shared, conn, parse(params)?),
        "lists.archive" => super::lists::archive(shared, conn, parse(params)?, true),
        "lists.unarchive" => super::lists::archive(shared, conn, parse(params)?, false),
        "lists.preview_delete" => super::lists::preview_delete(shared, conn, parse(params)?),
        "lists.delete" => super::lists::delete(shared, conn, parse(params)?),
        "lists.reconcile" => super::lists::reconcile(shared, conn, parse(params)?),
        "lists.export" => super::lists::export(shared, conn, parse(params)?),
        "lists.forget" => super::lists::forget(shared, conn, parse(params)?),
        "lists.recover" => super::lists::recover(shared, conn, parse(params)?),
        "lists.preview_forget" => super::lists::preview_forget(shared, parse(params)?),
        "lists.pending" => super::lists::pending(shared),
        "lists.backups" => super::lists::backups(shared),
        "lists.backup_preview" => super::lists::backup_preview(shared, parse(params)?),
        "events.subscribe" => to_result(shared.publisher.subscribe(conn.id, conn.out.clone())),
        "events.unsubscribe" => {
            shared.bus.unsubscribe(conn.id);
            Ok(json!({}))
        }
        "snapshot.get" => to_result(&shared.publisher.current().1.wire),
        "status.get" => match shared.publisher.status() {
            Some(v) => to_result(v),
            None => Err(RpcError::refused(
                ErrorKind::Unavailable,
                "the daemon is starting",
            )),
        },
        _ => Err(RpcError {
            code: METHOD_NOT_FOUND,
            message: format!("method not implemented yet: {method}"),
            data: None,
        }),
    }
}

fn hello(shared: &Arc<Shared>, conn: &ConnState, p: HelloParams) -> Result<Value, RpcError> {
    // The instance is checked first: a `protocol` refusal therefore means the client asked
    // for this very daemon, which is what lets a newer client replace it (spec §10.3).
    if p.instance != shared.instance {
        return Err(RpcError::refused(
            ErrorKind::WrongInstance,
            format!(
                "this daemon serves {}, not the instance you asked for",
                shared.vdir.display()
            ),
        ));
    }
    if p.protocol != PROTOCOL {
        let mut data = ErrorData::new(ErrorKind::Protocol);
        data.daemon = Some(shared.version.clone());
        data.protocol = Some(PROTOCOL);
        return Err(RpcError {
            code: crate::protocol::rpc::REFUSED,
            message: format!(
                "protocol {} is not supported; this daemon speaks protocol {PROTOCOL}",
                p.protocol
            ),
            data: Some(data),
        });
    }
    // Opens (or resumes) the session attached to this connection; a second hello on the same
    // connection lets go of the first.
    let session = shared.sessions.open(p.session.as_deref()).map_err(|e| {
        RpcError::refused(ErrorKind::Internal, format!("cannot open a session: {e}"))
    })?;
    if let Some(old) = conn.session.lock().unwrap().replace(session.clone()) {
        shared.sessions.detach(&old);
    }
    to_result(HelloResult {
        daemon: shared.version.clone(),
        protocol: PROTOCOL,
        epoch: shared.epoch.clone(),
        session,
        vdir: shared.vdir.clone(),
    })
}

/// For the shutdown call, its `when_idle`; `None` for any other request. The connection
/// layer stops the daemon after the reply is queued, or once idle.
pub fn is_shutdown(method: &str, params: Option<&Value>) -> Option<bool> {
    (method == "daemon.shutdown").then(|| {
        params
            .and_then(|p| p.get("when_idle"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    })
}
