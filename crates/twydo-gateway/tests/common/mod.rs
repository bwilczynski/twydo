//! An in-process daemon over a synthetic vdir, a gateway in front of it and a WebSocket
//! client. Every path is inside tempdirs; no test reaches a real socket or configuration.
#![allow(dead_code)]

use std::fs;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use serde_json::{Value, json};
use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::frame::CloseFrame;
use tungstenite::{Message, WebSocket};
use twydo_core::config::{Config, WriteMode};
use twydo_daemon::client::Client;
use twydo_daemon::protocol::methods::{HelloParams, HelloResult, Instance};
use twydo_daemon::server::{Daemon, DaemonHandle, DaemonOptions};
use twydo_gateway::Connector;
use twydo_gateway::http::{ServeOptions, serve_on};
use twydo_gateway::state::{Role, Store};

pub const ORIGIN: &str = "https://tasks.example.test";

pub type Hook = Arc<dyn Fn(&Store) + Send + Sync>;

pub struct SocketConnector {
    pub socket: PathBuf,
    pub instance: Instance,
    /// Runs inside `connect`, before the daemon is reached.
    pub hook: Option<(Hook, Store)>,
}

impl Connector for SocketConnector {
    fn connect(&self, session: Option<String>) -> Result<(Client, HelloResult), String> {
        if let Some((hook, store)) = &self.hook {
            hook(store);
        }
        Client::connect(
            &self.socket,
            HelloParams {
                client: "tdo-gateway".into(),
                version: "0".into(),
                protocol: twydo_daemon::PROTOCOL,
                session,
                instance: self.instance.clone(),
            },
        )
        .map_err(|e| e.to_string())
    }
}

pub struct Env {
    pub root: tempfile::TempDir,
    _sock_dir: tempfile::TempDir,
    pub daemon: DaemonHandle,
    pub addr: SocketAddr,
    pub store: Store,
}

pub fn ics(uid: &str, summary: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\nUID:{uid}\r\n\
         SUMMARY:{summary}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
    )
}

impl Env {
    /// Inbox tasks `t1`..: `(uid, summary)`.
    pub fn new(tasks: &[(&str, &str)]) -> Env {
        Env::with_hook(tasks, None)
    }

    pub fn with_hook(tasks: &[(&str, &str)], hook: Option<Hook>) -> Env {
        let root = tempfile::tempdir().unwrap();
        let vdir = root.path().join("vdir");
        fs::create_dir_all(vdir.join("inbox")).unwrap();
        for (uid, summary) in tasks {
            fs::write(vdir.join(format!("inbox/{uid}.ics")), ics(uid, summary)).unwrap();
        }
        let mut cfg = Config::with_vdir(vdir.clone());
        cfg.state_dir = Some(root.path().join("state"));
        cfg.writes.mode = WriteMode::Test;
        cfg.timezone = Some("UTC".into());
        let sock_dir = tempfile::Builder::new()
            .prefix("td")
            .tempdir_in("/tmp")
            .unwrap();
        fs::set_permissions(sock_dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let socket = sock_dir.path().join("s").join("tdo.sock");
        let instance = Instance {
            config: root.path().join("config.toml"),
            vdir: Some(vdir),
        };
        let mut opts = DaemonOptions::new(socket.clone());
        opts.schedule = false;
        let daemon = Daemon::start(cfg, instance.clone(), opts).expect("start daemon");
        assert!(
            daemon.wait_ready(Duration::from_secs(10)),
            "daemon not ready"
        );

        let state = root.path().join("gw");
        let store = Store::open(&state).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let sopts = ServeOptions {
            listen: addr,
            origin: Some(ORIGIN.into()),
            insecure: false,
            assets_dir: None,
            state_dir: state,
        };
        let hook = hook.map(|h| (h, Store::open(&root.path().join("gw")).unwrap()));
        let connector = Arc::new(SocketConnector {
            socket,
            instance,
            hook,
        });
        std::thread::spawn(move || {
            let _ = serve_on(listener, sopts, connector);
        });
        Env {
            root,
            _sock_dir: sock_dir,
            daemon,
            addr,
            store,
        }
    }

    pub fn vdir(&self) -> PathBuf {
        self.root.path().join("vdir")
    }

    /// Pairs a device and returns its key.
    pub fn pair(&self, name: &str, role: Role) -> String {
        let code = self.store.new_code(name, role, Utc::now()).unwrap();
        self.store.redeem(&code, Utc::now()).unwrap().1
    }

    pub fn connect(&self, key: &str) -> Sock {
        self.try_connect(Some(key), Some(ORIGIN)).expect("upgrade")
    }

    pub fn try_connect(
        &self,
        key: Option<&str>,
        origin: Option<&str>,
    ) -> Result<Sock, tungstenite::Error> {
        let mut req = format!("ws://{}/ws", self.addr)
            .into_client_request()
            .unwrap();
        if let Some(k) = key {
            req.headers_mut()
                .insert("Cookie", format!("twydo_device={k}").parse().unwrap());
        }
        if let Some(o) = origin {
            req.headers_mut().insert("Origin", o.parse().unwrap());
        }
        let stream = TcpStream::connect(self.addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let (ws, _) = tungstenite::client(req, stream).map_err(|e| match e {
            tungstenite::HandshakeError::Failure(e) => e,
            tungstenite::HandshakeError::Interrupted(_) => {
                tungstenite::Error::Io(std::io::ErrorKind::TimedOut.into())
            }
        })?;
        Ok(Sock { ws, next: 1 })
    }

    /// Files in the inbox whose text contains `needle`.
    pub fn inbox_with(&self, needle: &str) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for e in fs::read_dir(self.vdir().join("inbox")).unwrap() {
            let p = e.unwrap().path();
            if fs::read_to_string(&p).unwrap_or_default().contains(needle) {
                out.push(p);
            }
        }
        out
    }

    pub fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.vdir().join(rel)).unwrap()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        self.daemon.shutdown();
        self.daemon.wait_stopped();
    }
}

pub const WAIT: Duration = Duration::from_secs(10);

pub struct Sock {
    pub ws: WebSocket<TcpStream>,
    next: u64,
}

pub enum Got {
    Json(Value),
    Closed(Option<CloseFrame>),
}

impl Sock {
    pub fn send_raw(&mut self, text: &str) {
        self.ws.send(Message::text(text)).unwrap();
    }

    /// The next JSON text frame or the close, within `limit`.
    pub fn next(&mut self, limit: Duration) -> Got {
        let deadline = Instant::now() + limit;
        loop {
            assert!(Instant::now() < deadline, "nothing within {limit:?}");
            match self.ws.read() {
                Ok(Message::Text(t)) => {
                    return Got::Json(serde_json::from_str(t.as_str()).unwrap());
                }
                Ok(Message::Close(f)) => return Got::Closed(f),
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(tungstenite::Error::ConnectionClosed) => return Got::Closed(None),
                Err(e) => panic!("socket error: {e}"),
            }
        }
    }

    /// Sends a request and returns the matching reply (`result` or `error` object whole).
    pub fn rpc(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.send_raw(
            &json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string(),
        );
        loop {
            match self.next(WAIT) {
                Got::Json(v) if v["id"] == json!(id) => return v,
                Got::Json(_) => {}
                Got::Closed(f) => panic!("closed while waiting: {f:?}"),
            }
        }
    }

    /// `result` of a successful request.
    pub fn ok(&mut self, method: &str, params: Value) -> Value {
        let v = self.rpc(method, params);
        assert!(v.get("error").is_none(), "{method}: {v}");
        v["result"].clone()
    }

    /// The `based_on` of the current inbox view.
    pub fn based_on(&mut self) -> Value {
        let v = self.ok("views.get", json!({"view": "inbox"}));
        json!({"epoch": v["epoch"], "revision": v["revision"]})
    }

    pub fn submit(&mut self, request_id: &str, op: Value) -> Value {
        let based_on = self.based_on();
        self.rpc(
            "op.submit",
            json!({"request_id": request_id, "based_on": based_on, "op": op}),
        )
    }
}

pub fn exists(p: &Path) -> bool {
    p.exists()
}
