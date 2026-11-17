//! One device WebSocket bridged to one daemon connection.
//!
//! One thread owns the socket and polls it (100 ms read timeout) together with the daemon's
//! notifications and the worker's replies. A single worker thread runs the requests, so the
//! socket thread never blocks on the daemon.

use std::net::{Shutdown, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError, TrySendError};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde_json::Value;
use tungstenite::protocol::frame::CloseFrame;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{Role as WsRole, WebSocketConfig};
use tungstenite::{Error as WsError, Message, WebSocket};
use twydo_daemon::client::{Caller, ClientError, Notification};
use twydo_daemon::protocol::rpc::{
    ErrorKind, INVALID_REQUEST, MAX_LINE, PARSE_ERROR, Response, RpcError,
};

use crate::bridge::{self, Ctx as BridgeCtx};
use crate::http::Ctx;
use crate::state::{Device, StateError};

const POLL: Duration = Duration::from_millis(100);
const PING_EVERY: Duration = Duration::from_secs(20);
const REVOKE_POLL: Duration = Duration::from_secs(2);
/// How long a closing socket waits for the peer's close frame.
const CLOSE_WAIT: Duration = Duration::from_secs(1);
/// After a revoke, how long the connection gets to finish by itself (the policy close frame)
/// before the watcher shuts the whole socket, which also ends a pump blocked in a write.
const REVOKE_GRACE: Duration = Duration::from_secs(1);
const INTERNAL: i64 = -32603;
/// Requests waiting for the worker; more are refused.
const QUEUE: usize = 32;

/// A request for the worker: the client's id, method and params.
type Job = (Value, String, Option<Value>);

fn error_reply(id: Value, e: RpcError) -> String {
    serde_json::to_string(&Response::err(id, e)).unwrap_or_default()
}

fn protocol_error(code: i64, message: &str) -> RpcError {
    RpcError {
        code,
        message: message.into(),
        data: None,
    }
}

fn close_frame(code: CloseCode, reason: &str) -> CloseFrame {
    CloseFrame {
        code,
        reason: reason.to_owned().into(),
    }
}

/// Run requests from `jobs` against the daemon, sending each reply to `out`.
fn worker(
    device: Device,
    session: String,
    caller: Caller,
    jobs: Receiver<Job>,
    out: Sender<String>,
    cancel: Arc<AtomicBool>,
    revoked: Arc<AtomicBool>,
) {
    for (id, method, params) in jobs {
        // Once the socket is being closed (revoked, daemon gone) nothing queued is sent on,
        // and a revoked device's request never reaches the daemon.
        if cancel.load(Ordering::SeqCst) || revoked.load(Ordering::SeqCst) {
            return;
        }
        let ctx = BridgeCtx {
            device: &device,
            session: &session,
        };
        let reply = match bridge::inbound(&ctx, &method, params) {
            Err(e) => error_reply(id, e),
            Ok((method, params)) => {
                let variant = op_variant(&method, &params);
                let result = caller.call::<Value, Value>(&method, params);
                if matches!(method.as_str(), "op.submit" | "op.undo") {
                    log_write(&device, &method, &variant, &result);
                }
                match result {
                    Ok(v) => serde_json::to_string(&Response::ok(id, v)).unwrap_or_default(),
                    Err(ClientError::Rpc(e)) => error_reply(id, e),
                    Err(e) => error_reply(id, protocol_error(INTERNAL, &e.to_string())),
                }
            }
        };
        if out.send(reply).is_err() {
            return;
        }
    }
}

/// The name of the `Op` variant in an `op.submit`'s params (never its contents).
fn op_variant(method: &str, params: &Value) -> String {
    if method != "op.submit" {
        return "-".into();
    }
    match &params["op"] {
        Value::String(s) => s.clone(),
        Value::Object(o) => o.keys().next().cloned().unwrap_or_else(|| "-".into()),
        _ => "-".into(),
    }
}

/// `gateway: device=<name> <method> <op variant> -> <outcome|error kind>`; no task content.
fn log_write(device: &Device, method: &str, variant: &str, result: &Result<Value, ClientError>) {
    let outcome = match result {
        Ok(v) => v["outcome"].as_str().unwrap_or("ok").to_owned(),
        Err(ClientError::Rpc(e)) => e
            .data
            .as_ref()
            .and_then(|d| serde_json::to_value(d.kind).ok())
            .and_then(|k| k.as_str().map(str::to_owned))
            .unwrap_or_else(|| "error".into()),
        Err(_) => "error".into(),
    };
    eprintln!(
        "gateway: device={} {method} {variant} -> {outcome}",
        device.name
    );
}

/// Why the socket loop ended.
enum End {
    /// Close with this frame.
    Close(CloseFrame),
    /// The peer is gone or misbehaved; nothing more to say.
    Drop,
}

/// Serve one upgraded connection until it ends. `key` is the device's credential.
pub(crate) fn run(stream: TcpStream, ctx: &Ctx, device: Device, key: &str) {
    let _ = stream.set_read_timeout(Some(POLL));
    let _ = stream.set_nodelay(true);
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_LINE))
        .max_frame_size(Some(MAX_LINE));
    let mut ws = WebSocket::from_raw_socket(stream, WsRole::Server, Some(config));

    let (client, hello) = match ctx.connector.connect(device.session.clone()) {
        Ok(c) => c,
        Err(_) => {
            close(&mut ws, close_frame(CloseCode::Error, "daemon not running"));
            return;
        }
    };
    // A device revoked while connecting no longer exists: `touch` says so.
    if let Err(StateError::NoSuchDevice) =
        ctx.store
            .touch(&device.id, Utc::now(), Some(&hello.session))
    {
        close(&mut ws, close_frame(CloseCode::Policy, "device revoked"));
        return;
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let revoked = Arc::new(AtomicBool::new(false));
    let (job_tx, job_rx) = mpsc::sync_channel::<Job>(QUEUE);
    let (out_tx, out_rx) = mpsc::channel::<String>();
    {
        let (d, s, c) = (device.clone(), hello.session.clone(), client.caller());
        let (cancel, revoked) = (Arc::clone(&cancel), Arc::clone(&revoked));
        std::thread::spawn(move || worker(d, s, c, job_rx, out_tx, cancel, revoked));
    }

    // The watcher is independent of the pump: a client trickling a partial frame keeps the
    // pump inside one blocking read, so revocation is noticed (and that read interrupted) here.
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let reader = ws.get_ref().try_clone();
    let end = std::thread::scope(|scope| {
        let revoked = &revoked;
        scope.spawn(move || {
            // Every REVOKE_POLL, until the connection ends (the stop sender is dropped).
            while let Err(mpsc::RecvTimeoutError::Timeout) = stop_rx.recv_timeout(REVOKE_POLL) {
                // Re-read unconditionally: file timestamps are too coarse to notice a revoke.
                if ctx.store.authenticate(key).is_none() {
                    revoked.store(true, Ordering::SeqCst);
                    if let Ok(r) = &reader {
                        let _ = r.shutdown(Shutdown::Read);
                        // A pump stuck writing to a peer that does not read never sees the
                        // flag; if the connection has not ended shortly, cut the transport.
                        if let Err(mpsc::RecvTimeoutError::Timeout) =
                            stop_rx.recv_timeout(REVOKE_GRACE)
                        {
                            let _ = r.shutdown(Shutdown::Both);
                        }
                    }
                    return;
                }
            }
        });
        let end = pump(
            &mut ws,
            ctx,
            &device,
            &hello.session,
            key,
            revoked,
            &client,
            &job_tx,
            &out_rx,
        );
        drop(stop_tx);
        end
    });
    cancel.store(true, Ordering::SeqCst);
    drop(job_tx);
    if let End::Close(f) = end {
        close(&mut ws, f);
    }
}

#[allow(clippy::too_many_arguments)]
fn pump(
    ws: &mut WebSocket<TcpStream>,
    ctx: &Ctx,
    device: &Device,
    session: &str,
    key: &str,
    revoked: &AtomicBool,
    client: &twydo_daemon::client::Client,
    jobs: &SyncSender<Job>,
    replies: &Receiver<String>,
) -> End {
    let bridge_ctx = BridgeCtx { device, session };
    let mut last_ping = Instant::now();
    loop {
        if revoked.load(Ordering::SeqCst) {
            return revoke_close(ws);
        }
        // Daemon notifications.
        loop {
            match client.notifications().try_recv() {
                Ok(n) => {
                    if let Some(v) = outbound(&bridge_ctx, &n)
                        && send(ws, v).is_err()
                    {
                        return End::Drop;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    return End::Close(close_frame(CloseCode::Error, "daemon not running"));
                }
            }
        }
        // Replies from the worker.
        while let Ok(text) = replies.try_recv() {
            if send(ws, text).is_err() {
                return End::Drop;
            }
        }
        if last_ping.elapsed() >= PING_EVERY {
            last_ping = Instant::now();
            if ws.send(Message::Ping(Vec::new().into())).is_err() {
                return End::Drop;
            }
        }
        let read = ws.read();
        // A frame that completed after a revoke is dropped, whatever the read returned.
        if revoked.load(Ordering::SeqCst) {
            return revoke_close(ws);
        }
        match read {
            Ok(Message::Text(t)) => {
                // Authorize each request as it is dispatched, not only at the next poll.
                if ctx.store.authenticate(key).is_none() {
                    return revoke_close(ws);
                }
                if let Some(reply) = accept(t.as_str(), jobs)
                    && send(ws, reply).is_err()
                {
                    return End::Drop;
                }
            }
            Ok(Message::Binary(_)) => {
                return End::Close(close_frame(CloseCode::Unsupported, "text frames only"));
            }
            Ok(Message::Close(_)) | Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => {
                return End::Drop;
            }
            Ok(_) => {}
            Err(WsError::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(WsError::Capacity(_)) => {
                return End::Close(close_frame(CloseCode::Size, "message too large"));
            }
            Err(_) => return End::Drop,
        }
    }
}

fn outbound(ctx: &BridgeCtx, n: &Notification) -> Option<String> {
    bridge::outbound_note(ctx, n).map(|v| v.to_string())
}

fn send(ws: &mut WebSocket<TcpStream>, text: String) -> Result<(), WsError> {
    ws.send(Message::text(text))
}

/// Parse one text frame; a request goes to the worker (no immediate reply), anything else
/// gets the JSON-RPC error to send back now.
fn accept(text: &str, jobs: &SyncSender<Job>) -> Option<String> {
    if text.len() > MAX_LINE {
        return Some(error_reply(
            Value::Null,
            protocol_error(INVALID_REQUEST, "message too large"),
        ));
    }
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return Some(error_reply(
            Value::Null,
            protocol_error(PARSE_ERROR, "parse error"),
        ));
    };
    let id = v
        .get("id")
        .cloned()
        .filter(|i| i.is_string() || i.is_number());
    let method = v.get("method").and_then(Value::as_str);
    let (Some(id), Some(method)) = (id, method) else {
        return Some(error_reply(
            v.get("id").cloned().unwrap_or(Value::Null),
            protocol_error(
                INVALID_REQUEST,
                "expected a request with an id and a method",
            ),
        ));
    };
    match jobs.try_send((id.clone(), method.to_owned(), v.get("params").cloned())) {
        Ok(()) | Err(TrySendError::Disconnected(_)) => None,
        Err(TrySendError::Full(_)) => Some(error_reply(
            id,
            RpcError::refused(ErrorKind::Unavailable, "too many pending requests"),
        )),
    }
}

/// Send a close frame and give the peer a moment to answer it.
fn close(ws: &mut WebSocket<TcpStream>, frame: CloseFrame) {
    let _ = ws.close(Some(frame));
    let deadline = Instant::now() + CLOSE_WAIT;
    while Instant::now() < deadline {
        match ws.read() {
            Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => return,
            Err(WsError::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return,
            Ok(_) => {}
        }
    }
}

/// Close a revoked device's socket. The watcher shut the read half to interrupt a blocked
/// read, which leaves tungstenite unable to send a close frame, so write one by hand.
fn revoke_close(ws: &mut WebSocket<TcpStream>) -> End {
    use std::io::Write;
    const REASON: &[u8] = b"device revoked";
    let mut frame = vec![0x88, (2 + REASON.len()) as u8];
    frame.extend_from_slice(&u16::from(CloseCode::Policy).to_be_bytes());
    frame.extend_from_slice(REASON);
    let stream = ws.get_mut();
    let _ = stream.write_all(&frame);
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);
    End::Drop
}
