//! The gateway's HTTP side: static assets, pairing and device authentication.
//!
//! One thread per TCP connection, `Connection: close`. Request parsing, authentication and
//! the Origin check are plain functions so the WebSocket route can reuse them.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde_json::json;

use crate::Connector;
use crate::assets::{self, ASSETS};
use crate::state::{Device, Store};

pub const MAX_HEAD: usize = 16 * 1024;
pub const MAX_BODY: usize = 4 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const COOKIE: &str = "twydo_device";
const MAX_AGE: u64 = 34_560_000;

pub struct ServeOptions {
    pub listen: SocketAddr,
    pub origin: Option<String>,
    pub insecure: bool,
    pub assets_dir: Option<PathBuf>,
    pub state_dir: PathBuf,
}

pub enum Auth {
    Device(Device),
    None,
}

/// How a request proved who it is. Cookie requests need an Origin check; bearer ones do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Cookie,
    Bearer,
}

pub struct Request {
    pub method: String,
    /// Path without the query string.
    pub path: String,
    pub headers: Vec<(String, Vec<u8>)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .and_then(|(_, v)| std::str::from_utf8(v).ok())
    }
}

/// Why a request could not be read; maps to an HTTP status.
#[derive(Debug, PartialEq, Eq)]
pub enum ReadError {
    HeadTooLarge,
    BodyTooLarge,
    Malformed,
    Io,
}

impl ReadError {
    fn status(&self) -> (u16, &'static str) {
        match self {
            ReadError::HeadTooLarge => (431, "Request Header Fields Too Large"),
            ReadError::BodyTooLarge => (413, "Payload Too Large"),
            ReadError::Malformed => (400, "Bad Request"),
            ReadError::Io => (408, "Request Timeout"),
        }
    }
}

/// Read one request: head up to 16 KiB, body up to 4 KiB, all within `deadline`.
pub fn read_request(stream: &mut TcpStream, deadline: Instant) -> Result<Request, ReadError> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut req = httparse::Request::new(&mut headers);
        match req.parse(&buf) {
            Ok(httparse::Status::Complete(head_len)) => {
                if head_len > MAX_HEAD {
                    return Err(ReadError::HeadTooLarge);
                }
                let method = req.method.unwrap_or("").to_owned();
                let target = req.path.unwrap_or("/");
                let path = target.split('?').next().unwrap_or("/").to_owned();
                let hs: Vec<(String, Vec<u8>)> = req
                    .headers
                    .iter()
                    .map(|h| (h.name.to_owned(), h.value.to_vec()))
                    .collect();
                let len = hs
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
                    .map(|(_, v)| {
                        std::str::from_utf8(v)
                            .ok()
                            .and_then(|s| s.trim().parse::<usize>().ok())
                            .ok_or(ReadError::Malformed)
                    })
                    .transpose()?
                    .unwrap_or(0);
                if len > MAX_BODY {
                    return Err(ReadError::BodyTooLarge);
                }
                let mut body = buf[head_len..].to_vec();
                while body.len() < len {
                    let n = read_before(stream, &mut chunk, deadline)?;
                    if n == 0 {
                        return Err(ReadError::Malformed);
                    }
                    body.extend_from_slice(&chunk[..n]);
                }
                body.truncate(len);
                return Ok(Request {
                    method,
                    path,
                    headers: hs,
                    body,
                });
            }
            Ok(httparse::Status::Partial) => {
                if buf.len() >= MAX_HEAD {
                    return Err(ReadError::HeadTooLarge);
                }
            }
            Err(httparse::Error::TooManyHeaders) => return Err(ReadError::HeadTooLarge),
            Err(_) => return Err(ReadError::Malformed),
        }
        let n = read_before(stream, &mut chunk, deadline)?;
        if n == 0 {
            return Err(ReadError::Malformed);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// One read that cannot outlive the request deadline.
fn read_before(
    stream: &mut TcpStream,
    buf: &mut [u8],
    deadline: Instant,
) -> Result<usize, ReadError> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(ReadError::Io);
    }
    stream
        .set_read_timeout(Some(left.min(READ_TIMEOUT)))
        .map_err(|_| ReadError::Io)?;
    stream.read(buf).map_err(|_| ReadError::Io)
}

fn cookie_key(headers: &[httparse::Header]) -> Option<String> {
    headers
        .iter()
        .filter(|h| h.name.eq_ignore_ascii_case("cookie"))
        .filter_map(|h| std::str::from_utf8(h.value).ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|p| p.trim().split_once('='))
        .find(|(k, _)| *k == COOKIE)
        .map(|(_, v)| v.to_owned())
}

fn bearer_key(headers: &[httparse::Header]) -> Option<String> {
    headers
        .iter()
        .filter(|h| h.name.eq_ignore_ascii_case("authorization"))
        .filter_map(|h| std::str::from_utf8(h.value).ok())
        .find_map(|v| {
            let (scheme, rest) = v.trim().split_once(' ')?;
            scheme
                .eq_ignore_ascii_case("bearer")
                .then(|| rest.trim().to_owned())
        })
}

/// Which credential a request carries: the cookie if present, else a bearer token.
pub fn credential(headers: &[httparse::Header]) -> Option<(String, Via)> {
    cookie_key(headers)
        .map(|k| (k, Via::Cookie))
        .or_else(|| bearer_key(headers).map(|k| (k, Via::Bearer)))
}

/// The device a request belongs to: cookie first, else `Authorization: Bearer`.
pub fn authenticate(headers: &[httparse::Header], store: &Store) -> Auth {
    authenticate_via(headers, store).0
}

/// Like [`authenticate`], also reporting how the key was presented.
pub fn authenticate_via(headers: &[httparse::Header], store: &Store) -> (Auth, Option<Via>) {
    match credential(headers) {
        Some((key, via)) => match store.authenticate(&key) {
            Some(d) => (Auth::Device(d), Some(via)),
            None => (Auth::None, Some(via)),
        },
        None => (Auth::None, None),
    }
}

/// Cross-site WebSocket hijacking guard for cookie requests.
///
/// With `--origin` set the `Origin` header must equal it. Without it (only valid with
/// `--insecure`) it must equal `http://<Host>`; a missing Host fails closed. With
/// `require_present` false (same-origin GETs such as `/session`, where browsers omit
/// `Origin`) an absent header passes, but a present one must still match. `/ws` passes
/// `true`. Bearer requests skip the check.
pub fn origin_ok(
    headers: &[httparse::Header],
    via: Via,
    expected: Option<&str>,
    insecure: bool,
    require_present: bool,
) -> bool {
    if via == Via::Bearer {
        return true;
    }
    let value = |name: &str| {
        headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .and_then(|h| std::str::from_utf8(h.value).ok())
    };
    let Some(origin) = value("origin") else {
        return !require_present;
    };
    let Ok(origin) = normalize_origin(origin) else {
        return false;
    };
    match expected {
        Some(e) => normalize_origin(e).is_ok_and(|e| origin == e),
        None if insecure => {
            value("host").is_some_and(|h| origin == format!("http://{}", h.to_ascii_lowercase()))
        }
        None => false,
    }
}

/// Parse `scheme://host[:port]` (http or https; one trailing `/` allowed; no userinfo, path or
/// query) and lower-case scheme and host. Used for both `--origin` and the request's `Origin`.
pub fn normalize_origin(raw: &str) -> Result<String, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("the origin is empty".into());
    }
    let s = s.strip_suffix('/').unwrap_or(s);
    let Some((scheme, rest)) = s.split_once("://") else {
        return Err(format!("{raw:?} is not scheme://host[:port]"));
    };
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(format!("{raw:?} must start with http:// or https://"));
    }
    if rest.is_empty() || rest.contains(['/', '?', '#', '@', ' ']) {
        return Err(format!(
            "{raw:?} must be only scheme://host[:port], with no path, query or credentials"
        ));
    }
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) if !rest.ends_with(']') => (h, Some(p)),
        _ => (rest, None),
    };
    if host.is_empty()
        || port.is_some_and(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(format!("{raw:?} has an invalid host or port"));
    }
    Ok(format!("{scheme}://{}", rest.to_ascii_lowercase()))
}

/// `connect-src` entries for the WebSocket: derived from `--origin`.
fn ws_origin(origin: &str) -> Option<String> {
    if let Some(rest) = origin.strip_prefix("https://") {
        Some(format!("wss://{rest}"))
    } else {
        origin
            .strip_prefix("http://")
            .map(|rest| format!("ws://{rest}"))
    }
}

pub fn csp(origin: Option<&str>) -> String {
    let ws = origin.and_then(ws_origin).map(|w| format!(" {w}"));
    format!(
        "default-src 'self'; connect-src 'self'{}; img-src 'self' data:; frame-ancestors 'none'",
        ws.unwrap_or_default()
    )
}

struct Response {
    status: u16,
    reason: &'static str,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    fn new(status: u16, reason: &'static str) -> Self {
        Response {
            status,
            reason,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
    fn json(status: u16, reason: &'static str, v: serde_json::Value) -> Self {
        let mut r = Response::new(status, reason);
        r.headers
            .push(("Content-Type".into(), "application/json".into()));
        r.headers.push(("Cache-Control".into(), "no-store".into()));
        r.headers
            .push(("X-Content-Type-Options".into(), "nosniff".into()));
        r.body = v.to_string().into_bytes();
        r
    }
    fn error(status: u16, reason: &'static str, msg: &str) -> Self {
        Response::json(status, reason, json!({ "error": msg }))
    }
    fn write_to(&self, out: &mut impl Write) -> io::Result<()> {
        let mut head = format!("HTTP/1.1 {} {}\r\n", self.status, self.reason);
        for (k, v) in &self.headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            self.body.len()
        ));
        out.write_all(head.as_bytes())?;
        out.write_all(&self.body)?;
        out.flush()
    }
}

/// Resource bounds; the defaults are the production values, tests shrink them.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Overall time allowed for reading one request (head plus body).
    pub request_deadline: Duration,
    /// Concurrent connections; more are answered 503 and closed.
    pub max_connections: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            request_deadline: READ_TIMEOUT,
            max_connections: 64,
        }
    }
}

pub(crate) struct Ctx {
    pub(crate) connector: Arc<dyn Connector>,
    opts: ServeOptions,
    pub(crate) store: Store,
    limits: Limits,
    active: AtomicUsize,
}

struct Slot<'a>(&'a AtomicUsize);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Serve on an already bound listener. Blocks; one thread per connection.
pub fn serve_on(
    listener: TcpListener,
    opts: ServeOptions,
    connector: Arc<dyn Connector>,
) -> io::Result<()> {
    serve_with_limits(listener, opts, connector, Limits::default())
}

/// [`serve_on`] with explicit resource bounds.
pub fn serve_with_limits(
    listener: TcpListener,
    opts: ServeOptions,
    connector: Arc<dyn Connector>,
    limits: Limits,
) -> io::Result<()> {
    let store = Store::open(&opts.state_dir).map_err(io::Error::other)?;
    if let (Some(origin), false) = (&opts.origin, opts.insecure) {
        store.ensure_vapid(origin).map_err(io::Error::other)?;
    }
    let ctx = Arc::new(Ctx {
        connector,
        opts,
        store,
        limits,
        active: AtomicUsize::new(0),
    });
    for conn in listener.incoming() {
        let Ok(mut stream) = conn else { continue };
        if ctx.active.fetch_add(1, Ordering::SeqCst) >= ctx.limits.max_connections {
            ctx.active.fetch_sub(1, Ordering::SeqCst);
            let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
            let _ = Response::error(503, "Service Unavailable", "too many connections")
                .write_to(&mut stream);
            continue;
        }
        let ctx = Arc::clone(&ctx);
        std::thread::spawn(move || {
            let _slot = Slot(&ctx.active);
            let _ = handle(stream, &ctx);
        });
    }
    Ok(())
}

/// Bind `opts.listen`; tests read the bound address from the listener.
pub fn bind(opts: &ServeOptions) -> io::Result<TcpListener> {
    TcpListener::bind(opts.listen)
}

/// Bind `opts.listen` and serve.
pub fn serve(opts: ServeOptions, connector: Arc<dyn Connector>) -> io::Result<()> {
    let listener = bind(&opts)?;
    serve_on(listener, opts, connector)
}

fn handle(mut stream: TcpStream, ctx: &Ctx) -> io::Result<()> {
    stream.set_write_timeout(Some(READ_TIMEOUT))?;
    let deadline = Instant::now() + ctx.limits.request_deadline;
    let resp = match read_request(&mut stream, deadline) {
        Ok(req) if req.method == "GET" && req.path == "/ws" => match upgrade(&req, ctx) {
            Ok((device, key, accept)) => {
                let head = format!(
                    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\
                     Connection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
                );
                stream.write_all(head.as_bytes())?;
                stream.flush()?;
                crate::ws::run(stream, ctx, device, &key);
                return Ok(());
            }
            Err(r) => r,
        },
        Ok(req) => route(&req, ctx),
        Err(e) => {
            let (s, r) = e.status();
            Response::error(s, r, r)
        }
    };
    resp.write_to(&mut stream)
}

/// Headers of a parsed request as `httparse` values, for the shared auth functions.
fn as_httparse(req: &Request) -> Vec<httparse::Header<'_>> {
    req.headers
        .iter()
        .map(|(n, v)| httparse::Header { name: n, value: v })
        .collect()
}

fn route(req: &Request, ctx: &Ctx) -> Response {
    match (req.method.as_str(), req.path.as_str()) {
        ("POST", "/pair") => pair(req, ctx),
        ("GET", "/session") => session(req, ctx),
        ("GET" | "HEAD", _) => static_asset(req, ctx),
        _ => Response::error(405, "Method Not Allowed", "method not allowed"),
    }
}

/// Authenticate a request for a data route; `Err` is the refusal to send.
fn require_device(req: &Request, ctx: &Ctx, require_origin: bool) -> Result<Device, Response> {
    let headers = as_httparse(req);
    match authenticate_via(&headers, &ctx.store) {
        (Auth::Device(d), Some(via)) => {
            if origin_ok(
                &headers,
                via,
                ctx.opts.origin.as_deref(),
                ctx.opts.insecure,
                require_origin,
            ) {
                Ok(d)
            } else {
                Err(Response::error(403, "Forbidden", "origin not allowed"))
            }
        }
        _ => Err(Response::error(
            401,
            "Unauthorized",
            "a device key is required",
        )),
    }
}

fn session(req: &Request, ctx: &Ctx) -> Response {
    match require_device(req, ctx, false) {
        Ok(d) => Response::json(200, "OK", json!({ "device": d.name, "role": d.role })),
        Err(r) => r,
    }
}

/// Check a `/ws` request: who it is (401), where it comes from (403), and that it really is
/// a WebSocket upgrade (400). Returns the device, its key and the `Sec-WebSocket-Accept` value.
fn upgrade(req: &Request, ctx: &Ctx) -> Result<(Device, String, String), Response> {
    let device = require_device(req, ctx, true)?;
    let headers = as_httparse(req);
    let key = credential(&headers).map(|(k, _)| k).unwrap_or_default();
    let has_token = |name: &str, token: &str| {
        req.header(name)
            .is_some_and(|v| v.split(',').any(|t| t.trim().eq_ignore_ascii_case(token)))
    };
    let bad = |m: &str| Response::error(400, "Bad Request", m);
    if !has_token("connection", "upgrade") || !has_token("upgrade", "websocket") {
        return Err(bad("expected a websocket upgrade"));
    }
    if req.header("sec-websocket-version").map(str::trim) != Some("13") {
        return Err(bad("unsupported websocket version"));
    }
    let Some(ws_key) = req.header("sec-websocket-key").map(str::trim) else {
        return Err(bad("missing Sec-WebSocket-Key"));
    };
    let accept = tungstenite::handshake::derive_accept_key(ws_key.as_bytes());
    Ok((device, key, accept))
}

fn pair(req: &Request, ctx: &Ctx) -> Response {
    let code = serde_json::from_slice::<serde_json::Value>(&req.body)
        .ok()
        .and_then(|v| v["code"].as_str().map(str::to_owned));
    let Some(code) = code else {
        return Response::error(400, "Bad Request", "expected {\"code\": \"...\"}");
    };
    match ctx.store.redeem(&code, Utc::now()) {
        Ok((device, key)) => {
            eprintln!("paired device {}", device.name);
            let secure = if ctx.opts.insecure { "" } else { "; Secure" };
            let mut r = Response::json(200, "OK", json!({ "device": device.name, "key": key }));
            r.headers.push((
                "Set-Cookie".into(),
                format!(
                    "{COOKIE}={key}; HttpOnly; SameSite=Strict{secure}; Path=/; Max-Age={MAX_AGE}"
                ),
            ));
            r
        }
        Err(crate::state::StateError::CodeInvalid) => {
            Response::error(401, "Unauthorized", "invalid or expired pairing code")
        }
        Err(_) => Response::error(500, "Internal Server Error", "pairing failed"),
    }
}

/// Resolve a request path inside a directory, refusing anything that escapes it.
fn safe_join(root: &Path, url_path: &str) -> Option<PathBuf> {
    let rel = url_path.trim_start_matches('/');
    let mut out = root.to_path_buf();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(p) => out.push(p),
            _ => return None,
        }
    }
    Some(out)
}

fn lookup(ctx: &Ctx, url_path: &str) -> Option<(&'static str, Vec<u8>)> {
    let rel = url_path.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    if let Some(dir) = &ctx.opts.assets_dir {
        let p = safe_join(dir, rel)?;
        let bytes = std::fs::read(&p).ok().filter(|_| p.is_file())?;
        return Some((assets::mime_for(rel), bytes));
    }
    ASSETS
        .iter()
        .find(|a| a.path == rel)
        .map(|a| (a.mime, a.bytes.to_vec()))
}

fn static_asset(req: &Request, ctx: &Ctx) -> Response {
    let last = req.path.rsplit('/').next().unwrap_or("");
    let found = lookup(ctx, &req.path).or_else(|| {
        // SPA fallback only for paths without an extension.
        (!last.contains('.'))
            .then(|| lookup(ctx, "/index.html"))
            .flatten()
    });
    let Some((mime, bytes)) = found else {
        return Response::error(404, "Not Found", "not found");
    };
    let mut r = Response::new(200, "OK");
    r.headers.push(("Content-Type".into(), mime.into()));
    r.headers.push((
        "Content-Security-Policy".into(),
        csp(ctx.opts.origin.as_deref()),
    ));
    r.headers
        .push(("X-Content-Type-Options".into(), "nosniff".into()));
    if req.method != "HEAD" {
        r.body = bytes;
    }
    r
}
