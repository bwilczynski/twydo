//! One connection: a reader thread (this one) that parses and dispatches, and a writer thread
//! that owns the write half and drains a bounded queue of lines.

use std::io::{BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::mpsc::{SyncSender, sync_channel};

use serde_json::Value;

use super::{ConnState, Shared, dispatch, is_shutdown};
use crate::protocol::rpc::{
    ErrorKind, INVALID_REQUEST, LineError, PARSE_ERROR, Request, Response, RpcError, read_line,
};

/// Only the daemon's own user may talk to it.
pub fn peer_allowed(peer_uid: u32, own_uid: u32) -> bool {
    peer_uid == own_uid
}

/// The uid of the process on the other end (`SO_PEERCRED` on Linux, `getpeereid` on macOS).
#[cfg(target_os = "linux")]
pub fn peer_uid(s: &UnixStream) -> std::io::Result<u32> {
    use std::os::fd::AsRawFd;
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `cred` and `len` are valid for the call and sized as getsockopt expects.
    let rc = unsafe {
        libc::getsockopt(
            s.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    if rc == 0 {
        Ok(cred.uid)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "linux"))]
pub fn peer_uid(s: &UnixStream) -> std::io::Result<u32> {
    use std::os::fd::AsRawFd;
    let (mut uid, mut gid) = (0 as libc::uid_t, 0 as libc::gid_t);
    // SAFETY: both out-pointers are valid for the call.
    let rc = unsafe { libc::getpeereid(s.as_raw_fd(), &mut uid, &mut gid) };
    if rc == 0 {
        Ok(uid)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// The text a panic carried.
fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// How long a write to a client may block before the client is dropped.
const WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

fn send_response(tx: &SyncSender<String>, r: Response) {
    if let Ok(line) = serde_json::to_string(&r) {
        let _ = tx.send(line);
    }
}

fn err_response(id: Value, code: i64, message: &str) -> Response {
    Response::err(
        id,
        RpcError {
            code,
            message: message.into(),
            data: None,
        },
    )
}

pub fn serve(shared: &Arc<Shared>, id: u64, stream: UnixStream) {
    let Ok(write_half) = stream.try_clone() else {
        return;
    };
    let (tx, rx) = sync_channel::<String>(shared.event_buffer.max(1));
    let writer = std::thread::spawn(move || {
        let mut w = write_half;
        let _ = w.set_write_timeout(Some(WRITE_TIMEOUT));
        for line in rx {
            if w.write_all(line.as_bytes()).is_err() || w.write_all(b"\n").is_err() {
                break;
            }
        }
        let _ = w.shutdown(Shutdown::Both);
    });

    let state = Arc::new(ConnState::new(id, tx.clone()));
    let mut reader = BufReader::new(stream);
    loop {
        let line = match read_line(&mut reader) {
            Ok(Some(l)) => l,
            Ok(None) | Err(LineError::TooLong) | Err(LineError::Io(_)) => break,
            Err(LineError::InvalidUtf8) => {
                send_response(
                    &tx,
                    err_response(Value::Null, PARSE_ERROR, "message is not valid UTF-8"),
                );
                continue;
            }
        };
        let value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                send_response(
                    &tx,
                    err_response(Value::Null, PARSE_ERROR, &format!("parse error: {e}")),
                );
                continue;
            }
        };
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        let valid = value.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
            && value.get("method").is_some_and(Value::is_string);
        let req: Request = match valid
            .then(|| serde_json::from_value(value))
            .and_then(Result::ok)
        {
            Some(r) => r,
            None => {
                send_response(
                    &tx,
                    err_response(id, INVALID_REQUEST, "not a JSON-RPC 2.0 request"),
                );
                continue;
            }
        };
        let inline = req.method == "hello";
        let run = {
            let shared = Arc::clone(shared);
            let state = Arc::clone(&state);
            let tx = tx.clone();
            move || {
                let shutdown = is_shutdown(&req.method, req.params.as_ref());
                // A panicking handler answers `internal`; the connection goes on.
                let method = req.method.clone();
                let result = catch_unwind(AssertUnwindSafe(|| {
                    shared.test_point(&format!("request:{method}"));
                    dispatch(&shared, &state, &method, req.params)
                }))
                .unwrap_or_else(|p| {
                    Err(RpcError::refused(
                        ErrorKind::Internal,
                        format!("internal error: {}", panic_text(p.as_ref())),
                    ))
                });
                let ok = result.is_ok();
                if let Some(id) = req.id {
                    send_response(
                        &tx,
                        match result {
                            Ok(v) => Response::ok(id, v),
                            Err(e) => Response::err(id, e),
                        },
                    );
                }
                match shutdown {
                    Some(false) if ok => shared.stop_after_reply(),
                    Some(true) if ok => shared.stop_when_idle(),
                    _ => {}
                }
            }
        };
        // `hello` runs inline so its session exists before the next request is read.
        if inline {
            run();
        } else {
            std::thread::spawn(run);
        }
    }
    // The session outlives the connection for its ttl.
    if let Some(session) = state.session.lock().unwrap().take() {
        shared.sessions.detach(&session);
    }
    // The bus and the state hold senders too: release them so the writer can finish.
    shared.bus.unsubscribe(id);
    drop(state);
    drop(tx);
    let _ = writer.join();
}
