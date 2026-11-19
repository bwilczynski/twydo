//! Pure request and event filtering between a device and the daemon: no I/O.

use serde_json::{Value, json};
use twydo_core::edit::{Due, Op};
use twydo_core::model::TaskKind;
use twydo_daemon::client::Notification;
use twydo_daemon::protocol::events::Event;
use twydo_daemon::protocol::methods::{SubmitParams, UndoParams};
use twydo_daemon::protocol::rpc::{ErrorKind, RpcError};
use twydo_daemon::protocol::views::{ViewName, ViewsGetParams};

use crate::state::{Device, Role};

const PHONE: &[&str] = &[
    "views.get",
    "events.subscribe",
    "events.unsubscribe",
    "op.submit",
    "op.undo",
];
const DISPLAY: &[&str] = &["views.get", "events.subscribe", "events.unsubscribe"];

pub struct Ctx<'a> {
    pub device: &'a Device,
    pub session: &'a str,
}

/// The prefix the gateway puts on a device's request ids: `g<id>-`. `Device.id` is fixed-width
/// (8 lowercase hex digits) and so cannot contain `-`: one device's prefix is never a prefix of
/// another's, and `outbound_note`'s `strip_prefix` identifies exactly one device.
pub fn request_prefix(device: &Device) -> String {
    format!("g{}-", device.id)
}

fn refusal() -> RpcError {
    RpcError::refused(ErrorKind::Invalid, "not allowed for this device")
}

fn invalid(message: &str) -> RpcError {
    RpcError::refused(ErrorKind::Invalid, message)
}

fn allowed(role: Role) -> &'static [&'static str] {
    match role {
        Role::Phone => PHONE,
        Role::Display => DISPLAY,
    }
}

fn phone_may_submit(op: &Op) -> bool {
    match op {
        Op::Create(t) => {
            t.parent.is_none()
                && t.repeat.is_none()
                && t.start.is_none()
                && t.kind == TaskKind::Task
                && matches!(t.due, None | Some(Due::Date(_)))
        }
        Op::Complete { .. } => true,
        _ => false,
    }
}

/// Take the client's `request_id` out of `obj` and return it prefixed, or an error.
fn prefixed_request_id(
    ctx: &Ctx,
    obj: &serde_json::Map<String, Value>,
) -> Result<String, RpcError> {
    let id = obj
        .get("request_id")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("request_id is required"))?;
    let ok = (1..=100).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if !ok {
        return Err(invalid(
            "request_id must be 1-100 characters of A-Za-z0-9_-",
        ));
    }
    Ok(format!("{}{id}", request_prefix(ctx.device)))
}

/// A client request checked and rewritten for the daemon, or the error to answer with.
pub fn inbound(
    ctx: &Ctx,
    method: &str,
    params: Option<Value>,
) -> Result<(String, Value), RpcError> {
    if !allowed(ctx.device.role).contains(&method) {
        return Err(refusal());
    }
    match method {
        "views.get" => {
            let p: ViewsGetParams = serde_json::from_value(params.unwrap_or(Value::Null))
                .map_err(|_| invalid("invalid views.get params"))?;
            if p.view == ViewName::Unknown {
                return Err(invalid("invalid views.get params"));
            }
            return Ok((
                method.to_string(),
                serde_json::to_value(p).map_err(|_| invalid("invalid views.get params"))?,
            ));
        }
        "events.subscribe" | "events.unsubscribe" => {
            return Ok((method.to_string(), json!({})));
        }
        _ => {}
    }
    let Some(Value::Object(obj)) = params else {
        return Err(invalid("params must be an object"));
    };
    let rid = prefixed_request_id(ctx, &obj)?;
    let mut obj = obj;
    obj.insert("request_id".into(), Value::String(rid));
    obj.insert("session".into(), Value::String(ctx.session.to_string()));
    let value = Value::Object(obj);
    if method == "op.submit" {
        let p: SubmitParams =
            serde_json::from_value(value).map_err(|_| invalid("invalid op.submit params"))?;
        if ctx.device.role == Role::Phone && !phone_may_submit(&p.op) {
            return Err(refusal());
        }
        let v = serde_json::to_value(&p).map_err(|_| invalid("invalid op.submit params"))?;
        Ok((method.to_string(), v))
    } else {
        let p: UndoParams =
            serde_json::from_value(value).map_err(|_| invalid("invalid op.undo params"))?;
        let v = serde_json::to_value(&p).map_err(|_| invalid("invalid op.undo params"))?;
        Ok((method.to_string(), v))
    }
}

fn note(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": method, "params": params})
}

/// A daemon notification as the device may see it, or None to drop it.
pub fn outbound_note(ctx: &Ctx, n: &Notification) -> Option<Value> {
    match n {
        Notification::Missed(m) => Some(note("missed", serde_json::to_value(m).ok()?)),
        Notification::Heartbeat(h) => Some(note("heartbeat", serde_json::to_value(h).ok()?)),
        Notification::Event(env) => match &env.event {
            Event::DataChanged { .. } | Event::StatusChanged { .. } => {
                Some(note("event", serde_json::to_value(env).ok()?))
            }
            Event::OpFinished {
                op_id,
                request_id: Some(rid),
                state,
            } if ctx.device.role == Role::Phone => {
                let rest = rid.strip_prefix(&request_prefix(ctx.device))?;
                let mut env = env.clone();
                env.event = Event::OpFinished {
                    op_id: op_id.clone(),
                    request_id: Some(rest.to_string()),
                    state: state.clone(),
                };
                Some(note("event", serde_json::to_value(&env).ok()?))
            }
            _ => None,
        },
    }
}
