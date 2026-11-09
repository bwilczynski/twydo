//! The blocking client: one reader thread routes responses to the waiting callers by id and
//! notifications to [`Client::notifications`]. Nothing here reads the configuration.

use std::collections::HashMap;
use std::io::{BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::protocol::events::{EventEnvelope, Heartbeat, Missed};
use crate::protocol::methods::{HelloParams, HelloResult};
use crate::protocol::rpc::{LineError, RpcError, read_line};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{}", .0.message)]
    Rpc(RpcError),
    #[error("the daemon closed the connection")]
    Closed,
    #[error("unexpected reply: {0}")]
    Decode(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Notification {
    Event(EventEnvelope),
    Missed(Missed),
    Heartbeat(Heartbeat),
}

type Waiter = Sender<Result<Value, RpcError>>;

struct Inner {
    writer: Mutex<UnixStream>,
    /// `None` once the connection is closed: no further waiter can register.
    pending: Mutex<Option<HashMap<u64, Waiter>>>,
    next_id: AtomicU64,
    counter: AtomicU64,
    /// The session id and a per-connection nonce after hello, else a random prefix. The
    /// nonce keeps request ids unique when a later connection resumes the same session (its
    /// counter starts again at 1).
    prefix: Mutex<String>,
    /// What the daemon answered to hello, once it has.
    hello: Mutex<Option<HelloResult>>,
    stream: UnixStream,
}

/// A cloneable, thread-safe handle for making calls on a [`Client`]'s connection.
#[derive(Clone)]
pub struct Caller {
    inner: Arc<Inner>,
}

pub struct Client {
    caller: Caller,
    notes: Receiver<Notification>,
}

impl Client {
    /// Connects and says hello.
    pub fn connect(
        socket: &Path,
        hello: HelloParams,
    ) -> Result<(Client, HelloResult), ClientError> {
        let c = Client::connect_raw(socket)?;
        let r: HelloResult = c.call("hello", hello)?;
        *c.caller.inner.prefix.lock().unwrap() = format!(
            "{}-{}",
            r.session,
            &uuid::Uuid::new_v4().simple().to_string()[..6]
        );
        *c.caller.inner.hello.lock().unwrap() = Some(r.clone());
        Ok((c, r))
    }

    /// Connects without saying hello (for lifecycle calls and tests).
    pub fn connect_raw(socket: &Path) -> Result<Client, ClientError> {
        let stream = UnixStream::connect(socket)?;
        let reader = stream.try_clone()?;
        let inner = Arc::new(Inner {
            writer: Mutex::new(stream.try_clone()?),
            pending: Mutex::new(Some(HashMap::new())),
            next_id: AtomicU64::new(1),
            counter: AtomicU64::new(1),
            prefix: Mutex::new(format!(
                "r-{}",
                &uuid::Uuid::new_v4().simple().to_string()[..8]
            )),
            hello: Mutex::new(None),
            stream,
        });
        let (tx, rx) = mpsc::channel();
        let for_reader = Arc::clone(&inner);
        std::thread::spawn(move || read_loop(reader, for_reader, tx));
        Ok(Client {
            caller: Caller { inner },
            notes: rx,
        })
    }

    /// Calls `method`; safe to use from several threads at once through [`Client::caller`].
    pub fn call<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<R, ClientError> {
        self.caller.call(method, params)
    }

    /// A handle that can be cloned into other threads.
    pub fn caller(&self) -> Caller {
        self.caller.clone()
    }

    /// Events, `missed` and heartbeats, in arrival order.
    pub fn notifications(&self) -> &Receiver<Notification> {
        &self.notes
    }

    /// `"<session>-<counter>"`: unique for this connection's session.
    pub fn next_request_id(&self) -> String {
        self.caller.next_request_id()
    }

    /// The session hello opened or resumed; `None` before hello.
    pub fn session(&self) -> Option<String> {
        self.caller.session()
    }

    /// The daemon's epoch from hello; `None` before hello.
    pub fn epoch(&self) -> Option<String> {
        self.caller.epoch()
    }
}

impl Caller {
    pub fn call<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<R, ClientError> {
        let params =
            serde_json::to_value(params).map_err(|e| ClientError::Decode(e.to_string()))?;
        let inner = &self.inner;
        let id = inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        match inner.pending.lock().unwrap().as_mut() {
            Some(map) => map.insert(id, tx),
            None => return Err(ClientError::Closed),
        };
        let mut line =
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        line.push('\n');
        let sent = inner.writer.lock().unwrap().write_all(line.as_bytes());
        if let Err(e) = sent {
            if let Some(map) = inner.pending.lock().unwrap().as_mut() {
                map.remove(&id);
            }
            return Err(e.into());
        }
        match rx.recv() {
            Ok(Ok(v)) => serde_json::from_value(v).map_err(|e| ClientError::Decode(e.to_string())),
            Ok(Err(e)) => Err(ClientError::Rpc(e)),
            Err(_) => Err(ClientError::Closed),
        }
    }

    pub fn session(&self) -> Option<String> {
        self.inner
            .hello
            .lock()
            .unwrap()
            .as_ref()
            .map(|h| h.session.clone())
    }

    pub fn epoch(&self) -> Option<String> {
        self.inner
            .hello
            .lock()
            .unwrap()
            .as_ref()
            .map(|h| h.epoch.clone())
    }

    pub fn next_request_id(&self) -> String {
        let n = self.inner.counter.fetch_add(1, Ordering::Relaxed);
        format!("{}-{n}", self.inner.prefix.lock().unwrap())
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.caller.inner.stream.shutdown(Shutdown::Both);
    }
}

fn read_loop(stream: UnixStream, inner: Arc<Inner>, notes: Sender<Notification>) {
    let mut r = BufReader::new(stream);
    loop {
        let line = match read_line(&mut r) {
            Ok(Some(l)) => l,
            Ok(None) | Err(LineError::TooLong) | Err(LineError::Io(_)) => break,
            Err(LineError::InvalidUtf8) => continue,
        };
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(method) = v.get("method").and_then(Value::as_str) {
            let params = v.get("params").cloned().unwrap_or(Value::Null);
            let n = match method {
                "event" => serde_json::from_value(params).map(Notification::Event),
                "missed" => serde_json::from_value(params).map(Notification::Missed),
                "heartbeat" => serde_json::from_value(params).map(Notification::Heartbeat),
                _ => continue,
            };
            if let Ok(n) = n {
                let _ = notes.send(n);
            }
            continue;
        }
        let Some(id) = v.get("id").and_then(Value::as_u64) else {
            continue;
        };
        let waiter = inner
            .pending
            .lock()
            .unwrap()
            .as_mut()
            .and_then(|m| m.remove(&id));
        let Some(waiter) = waiter else { continue };
        let outcome = match v.get("error") {
            Some(e) => match serde_json::from_value::<RpcError>(e.clone()) {
                Ok(e) => Err(e),
                Err(_) => Err(RpcError {
                    code: -32603,
                    message: "malformed error reply".into(),
                    data: None,
                }),
            },
            None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
        };
        let _ = waiter.send(outcome);
    }
    // Closed: fail every waiter (dropping its sender) and refuse new ones.
    inner.pending.lock().unwrap().take();
}
