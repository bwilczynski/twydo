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
                version: env!("CARGO_PKG_VERSION").into(),
                protocol: PROTOCOL,
                pid: std::process::id(),
                uptime_secs: shared.started.elapsed().as_secs(),
                vdir: shared.vdir.clone(),
                epoch: shared.epoch.clone(),
            }),
            "daemon.shutdown" => {
                // Nothing long runs yet, so `when_idle` changes nothing. The reply is queued
                // by the caller before stopping: see `conn::serve`.
                let _: ShutdownParams = parse(params)?;
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
    // Later tasks add arms here: op.*, refusals.ack.
    match method {
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
    if p.protocol != PROTOCOL {
        let mut data = ErrorData::new(ErrorKind::Protocol);
        data.daemon = Some(env!("CARGO_PKG_VERSION").into());
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
    if p.instance != shared.instance {
        return Err(RpcError::refused(
            ErrorKind::WrongInstance,
            format!(
                "this daemon serves {}, not the instance you asked for",
                shared.vdir.display()
            ),
        ));
    }
    // Task 7 creates or resumes sessions; for now every hello gets a fresh id.
    let session = format!("s-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    *conn.session.lock().unwrap() = Some(session.clone());
    to_result(HelloResult {
        daemon: env!("CARGO_PKG_VERSION").into(),
        protocol: PROTOCOL,
        epoch: shared.epoch.clone(),
        session,
        vdir: shared.vdir.clone(),
    })
}

/// True when the request is the shutdown call (the connection layer stops the daemon after
/// the reply is queued).
pub fn is_shutdown(method: &str) -> bool {
    method == "daemon.shutdown"
}
