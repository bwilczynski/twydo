use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;

use chrono::Utc;
use twydo_gateway::http::{Limits, ServeOptions, serve_with_limits};
use twydo_gateway::state::{Role, Store};

struct NoDaemon;

impl twydo_gateway::Connector for NoDaemon {
    fn connect(
        &self,
        _: Option<String>,
    ) -> Result<
        (
            twydo_daemon::client::Client,
            twydo_daemon::protocol::methods::HelloResult,
        ),
        String,
    > {
        Err("no daemon".into())
    }
}

const ORIGIN: &str = "https://tasks.example.test";

struct Srv {
    addr: SocketAddr,
    store: Store,
    _dir: tempfile::TempDir,
    dir: PathBuf,
}

fn start(insecure: bool, assets_dir: Option<PathBuf>) -> Srv {
    start_with(insecure, assets_dir, Some(ORIGIN), Limits::default())
}

fn start_with(
    insecure: bool,
    assets_dir: Option<PathBuf>,
    origin: Option<&str>,
    limits: Limits,
) -> Srv {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("gw");
    let store = Store::open(&state).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let opts = ServeOptions {
        listen: addr,
        origin: origin.map(str::to_owned),
        insecure,
        assets_dir,
        state_dir: state,
    };
    std::thread::spawn(move || {
        let _ = serve_with_limits(listener, opts, std::sync::Arc::new(NoDaemon), limits);
    });
    let d = dir.path().to_path_buf();
    Srv {
        addr,
        store,
        _dir: dir,
        dir: d,
    }
}

struct Resp {
    status: u16,
    head: String,
    body: String,
}

fn raw(addr: SocketAddr, req: &[u8]) -> Resp {
    let mut s = TcpStream::connect(addr).unwrap();
    s.write_all(req).unwrap();
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .split(' ')
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    Resp {
        status,
        head: head.to_owned(),
        body: body.to_owned(),
    }
}

fn get(addr: SocketAddr, path: &str, extra: &str) -> Resp {
    raw(
        addr,
        format!("GET {path} HTTP/1.1\r\nHost: x\r\n{extra}\r\n").as_bytes(),
    )
}

fn pair(srv: &Srv) -> (String, Resp) {
    let code = srv
        .store
        .new_code("Pixel", Role::Phone, Utc::now())
        .unwrap();
    let body = format!("{{\"code\":\"{code}\"}}");
    let r = raw(
        srv.addr,
        format!(
            "POST /pair HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .as_bytes(),
    );
    let v: serde_json::Value = serde_json::from_str(&r.body).unwrap_or_default();
    let key = v["key"].as_str().unwrap_or("").to_owned();
    (key, r)
}

#[test]
fn serves_index_and_manifest() {
    let s = start(false, None);
    let r = get(s.addr, "/", "");
    assert_eq!(r.status, 200);
    assert!(r.head.to_lowercase().contains("content-type: text/html"));
    assert!(r.head.contains("X-Content-Type-Options: nosniff"));
    assert!(r.head.contains("Content-Security-Policy: default-src 'self'; connect-src 'self' wss://tasks.example.test; img-src 'self' data:; frame-ancestors 'none'"));
    // The manifest is only present in a real build.
    if !twydo_gateway::assets::PLACEHOLDER {
        assert_eq!(get(s.addr, "/manifest.webmanifest", "").status, 200);
    }
}

#[test]
fn json_responses_are_nosniff() {
    let s = start(false, None);
    let r = get(s.addr, "/session", "");
    assert_eq!(r.status, 401);
    assert!(
        r.head.contains("X-Content-Type-Options: nosniff"),
        "{}",
        r.head
    );
}

#[test]
fn origin_normaliser() {
    use twydo_gateway::http::normalize_origin as n;
    assert_eq!(
        n("https://Tasks.Example.test/").unwrap(),
        "https://tasks.example.test"
    );
    assert_eq!(n("HTTPS://a.test:8443").unwrap(), "https://a.test:8443");
    assert!(n("").is_err());
    assert!(n("https://a.test/app").is_err());
    assert!(n("https://a.test//").is_err());
    assert!(n("https://a.test?x=1").is_err());
    assert!(n("ftp://a.test").is_err());
    assert!(n("a.test").is_err());
    assert!(n("https://a.test:").is_err());
}

#[test]
fn unknown_route_falls_back_to_index() {
    let s = start(false, None);
    let r = get(s.addr, "/some/route", "");
    assert_eq!(r.status, 200);
    assert!(r.head.to_lowercase().contains("text/html"));
    assert_eq!(get(s.addr, "/missing.png", "").status, 404);
}

#[test]
fn pair_sets_cookie_with_flags() {
    let s = start(false, None);
    let (key, r) = pair(&s);
    assert_eq!(r.status, 200);
    assert!(!key.is_empty());
    assert!(r.body.contains("\"device\":\"Pixel\""));
    let cookie = r
        .head
        .lines()
        .find(|l| l.to_lowercase().starts_with("set-cookie:"))
        .unwrap();
    for flag in [
        "HttpOnly",
        "SameSite=Strict",
        "Secure",
        "Max-Age=34560000",
        "Path=/",
    ] {
        assert!(cookie.contains(flag), "{flag} in {cookie}");
    }
    assert!(cookie.contains(&format!("twydo_device={key}")));
}

#[test]
fn insecure_drops_secure_flag() {
    let s = start(true, None);
    let (_, r) = pair(&s);
    let cookie = r
        .head
        .lines()
        .find(|l| l.to_lowercase().starts_with("set-cookie:"))
        .unwrap();
    assert!(!cookie.contains("Secure"));
    assert!(cookie.contains("HttpOnly"));
}

#[test]
fn wrong_code_is_401() {
    let s = start(false, None);
    let body = r#"{"code":"AAAA-AAAA"}"#;
    let r = raw(
        s.addr,
        format!(
            "POST /pair HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .as_bytes(),
    );
    assert_eq!(r.status, 401);
    assert!(r.body.contains("\"error\""));
    assert!(!r.head.to_lowercase().contains("set-cookie"));
}

#[test]
fn session_requires_key() {
    let s = start(false, None);
    assert_eq!(get(s.addr, "/session", "").status, 401);
    assert_eq!(
        get(s.addr, "/session", "Cookie: twydo_device=nope\r\n").status,
        401
    );
}

#[test]
fn session_accepts_cookie_with_origin() {
    let s = start(false, None);
    let (key, _) = pair(&s);
    let r = get(
        s.addr,
        "/session",
        &format!("Cookie: a=b; twydo_device={key}\r\nOrigin: {ORIGIN}\r\n"),
    );
    assert_eq!(r.status, 200);
    assert!(r.body.contains("\"device\":\"Pixel\""));
    assert!(r.body.contains("\"role\":\"phone\""));
}

#[test]
fn session_origin_comparison_ignores_case_and_trailing_slash() {
    let s = start(false, None);
    let (key, _) = pair(&s);
    let r = get(
        s.addr,
        "/session",
        &format!("Cookie: twydo_device={key}\r\nOrigin: HTTPS://Tasks.Example.test\r\n"),
    );
    assert_eq!(r.status, 200);
}

#[test]
fn session_rejects_foreign_origin() {
    let s = start(false, None);
    let (key, _) = pair(&s);
    let r = get(
        s.addr,
        "/session",
        &format!("Cookie: twydo_device={key}\r\nOrigin: https://evil.test\r\n"),
    );
    assert_eq!(r.status, 403);
}

#[test]
fn session_accepts_same_origin_without_origin_header() {
    let s = start(false, None);
    let (key, _) = pair(&s);
    let r = get(
        s.addr,
        "/session",
        &format!("Cookie: twydo_device={key}\r\n"),
    );
    assert_eq!(r.status, 200);
}

#[test]
fn ws_requires_origin_for_cookies() {
    let s = start(false, None);
    let (key, _) = pair(&s);
    let c = format!("Cookie: twydo_device={key}\r\n");
    assert_eq!(get(s.addr, "/ws", &c).status, 403);
    assert_eq!(
        get(s.addr, "/ws", &format!("{c}Origin: https://evil.test\r\n")).status,
        403
    );
    assert_eq!(
        get(s.addr, "/ws", &format!("{c}Origin: {ORIGIN}\r\n")).status,
        400 // authenticated, but not an upgrade request
    );
}

#[test]
fn no_origin_option_uses_host_under_insecure() {
    let s = start_with(true, None, None, Limits::default());
    let (key, _) = pair(&s);
    let c = format!("Cookie: twydo_device={key}\r\n");
    let host = s.addr.to_string();
    let ok = raw(
        s.addr,
        format!("GET /session HTTP/1.1\r\nHost: {host}\r\n{c}Origin: http://{host}\r\n\r\n")
            .as_bytes(),
    );
    assert_eq!(ok.status, 200);
    let foreign = raw(
        s.addr,
        format!("GET /session HTTP/1.1\r\nHost: {host}\r\n{c}Origin: http://evil.test\r\n\r\n")
            .as_bytes(),
    );
    assert_eq!(foreign.status, 403);
    // Missing Host fails closed when an Origin is present.
    let no_host = raw(
        s.addr,
        format!("GET /session HTTP/1.0\r\n{c}Origin: http://{host}\r\n\r\n").as_bytes(),
    );
    assert_eq!(no_host.status, 403);
}

#[test]
fn trickling_client_is_cut_off_by_the_deadline() {
    let limits = Limits {
        request_deadline: std::time::Duration::from_millis(300),
        max_connections: 64,
    };
    let s = start_with(false, None, Some(ORIGIN), limits);
    let start = std::time::Instant::now();
    let mut c = TcpStream::connect(s.addr).unwrap();
    c.write_all(b"GET / HTTP/1.1\r\n").unwrap();
    while start.elapsed() < std::time::Duration::from_secs(3) {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if c.write_all(b"X").is_err() {
            break;
        }
    }
    let mut buf = Vec::new();
    let _ = c.read_to_end(&mut buf);
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
    assert!(String::from_utf8_lossy(&buf).starts_with("HTTP/1.1 408"));
}

#[test]
fn connections_over_the_cap_get_503() {
    let limits = Limits {
        request_deadline: std::time::Duration::from_secs(5),
        max_connections: 1,
    };
    let s = start_with(false, None, Some(ORIGIN), limits);
    let _idle = TcpStream::connect(s.addr).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(get(s.addr, "/", "").status, 503);
}

#[test]
fn bearer_key_accepted_without_origin() {
    let s = start(false, None);
    let (key, _) = pair(&s);
    let r = get(
        s.addr,
        "/session",
        &format!("Authorization: Bearer {key}\r\n"),
    );
    assert_eq!(r.status, 200);
}

#[test]
fn revoked_key_refused() {
    let s = start(false, None);
    let (key, _) = pair(&s);
    s.store.revoke("Pixel").unwrap();
    let r = get(
        s.addr,
        "/session",
        &format!("Authorization: Bearer {key}\r\n"),
    );
    assert_eq!(r.status, 401);
}

#[test]
fn ws_needs_upgrade_headers_once_authenticated() {
    let s = start(false, None);
    let (key, _) = pair(&s);
    let r = get(s.addr, "/ws", &format!("Authorization: Bearer {key}\r\n"));
    assert_eq!(r.status, 400);
    assert_eq!(get(s.addr, "/ws", "").status, 401);
}

#[test]
fn oversized_head_refused() {
    let s = start(false, None);
    let big = "a".repeat(20 * 1024);
    let r = get(s.addr, "/", &format!("X-Big: {big}\r\n"));
    assert_eq!(r.status, 431);
}

#[test]
fn oversized_body_refused() {
    let s = start(false, None);
    let r = raw(
        s.addr,
        b"POST /pair HTTP/1.1\r\nHost: x\r\nContent-Length: 100000\r\n\r\n{}",
    );
    assert_eq!(r.status, 413);
}

#[test]
fn assets_dir_overrides_embedded() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("index.html"), "<p>override</p>").unwrap();
    std::fs::write(tmp.path().join("app.js"), "x").unwrap();
    let s = start(false, Some(tmp.path().to_path_buf()));
    let r = get(s.addr, "/", "");
    assert!(r.body.contains("override"));
    let js = get(s.addr, "/app.js", "");
    assert_eq!(js.status, 200);
    assert!(js.head.to_lowercase().contains("javascript"));
    assert_eq!(get(s.addr, "/../../etc/passwd.txt", "").status, 404);
    let _ = &s.dir;
}

/// A served request proves startup (including `ensure_vapid`) is done.
fn wait_started(s: &Srv) {
    raw(
        s.addr,
        b"GET /healthz HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    );
}

#[test]
fn serve_creates_the_vapid_key_with_an_origin() {
    let s = start(false, None);
    wait_started(&s);
    let v = Store::open(&s.dir.join("gw")).unwrap().vapid().unwrap();
    assert_eq!(v.expect("key created").subject, ORIGIN);
}

#[test]
fn serve_creates_no_vapid_key_when_insecure() {
    let s = start(true, None);
    wait_started(&s);
    assert!(
        Store::open(&s.dir.join("gw"))
            .unwrap()
            .vapid()
            .unwrap()
            .is_none()
    );
}

// --- push routes ---

fn send(addr: SocketAddr, method: &str, path: &str, extra: &str, body: &str) -> Resp {
    raw(
        addr,
        format!(
            "{method} {path} HTTP/1.1\r\nHost: x\r\n{extra}Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .as_bytes(),
    )
}

fn auth(key: &str) -> String {
    format!("Authorization: Bearer {key}\r\n")
}

fn cookie(key: &str) -> String {
    format!("Cookie: twydo_device={key}\r\n")
}

fn with_origin(key: &str) -> String {
    format!("{}Origin: {ORIGIN}\r\n", cookie(key))
}

/// A subscription body whose `p256dh` is a real P-256 point (the server's own VAPID key).
fn sub_body(s: &Srv, endpoint: &str) -> String {
    let p256dh = s.store.vapid().unwrap().expect("key").key.public_b64();
    serde_json::json!({
        "endpoint": endpoint,
        "expirationTime": null,
        "keys": { "p256dh": p256dh, "auth": "AAAAAAAAAAAAAAAAAAAAAA" }
    })
    .to_string()
}

const ENDPOINT: &str = "https://push.example.test/send/abc";

fn started() -> Srv {
    let s = start(false, None);
    wait_started(&s);
    s
}

#[test]
fn push_key_needs_a_device() {
    let s = started();
    assert_eq!(get(s.addr, "/push/key", "").status, 401);
}

#[test]
fn push_key_returns_the_vapid_public_key() {
    let s = started();
    let (key, _) = pair(&s);
    let r = get(s.addr, "/push/key", &auth(&key));
    assert_eq!(r.status, 200);
    let v: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    let want = s.store.vapid().unwrap().unwrap().key.public_b64();
    assert_eq!(v["key"], want);
}

#[test]
fn put_subscription_stores_it() {
    let s = started();
    let (key, _) = pair(&s);
    let r = send(
        s.addr,
        "PUT",
        "/push/subscription",
        &with_origin(&key),
        &sub_body(&s, ENDPOINT),
    );
    assert_eq!(r.status, 204);
    assert_eq!(r.body, "");
    let subs = s.store.subscriptions().unwrap();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].1.endpoint, ENDPOINT);
}

#[test]
fn put_without_origin_is_refused() {
    let s = started();
    let (key, _) = pair(&s);
    let r = send(
        s.addr,
        "PUT",
        "/push/subscription",
        &cookie(&key),
        &sub_body(&s, ENDPOINT),
    );
    assert_eq!(r.status, 403);
    assert!(s.store.subscriptions().unwrap().is_empty());
}

#[test]
fn put_with_foreign_origin_is_refused() {
    let s = started();
    let (key, _) = pair(&s);
    let r = send(
        s.addr,
        "PUT",
        "/push/subscription",
        &format!("{}Origin: https://evil.example.test\r\n", cookie(&key)),
        &sub_body(&s, ENDPOINT),
    );
    assert_eq!(r.status, 403);
}

#[test]
fn put_rejects_http_endpoint() {
    let s = started();
    let (key, _) = pair(&s);
    for ep in [
        "http://push.example.test/x".to_owned(),
        format!("https://push.example.test/{}", "a".repeat(2048)),
    ] {
        let r = send(
            s.addr,
            "PUT",
            "/push/subscription",
            &with_origin(&key),
            &sub_body(&s, &ep),
        );
        assert_eq!(r.status, 400, "{}", &ep[..20]);
    }
}

#[test]
fn put_rejects_bad_keys() {
    let s = started();
    let (key, _) = pair(&s);
    let good = s.store.vapid().unwrap().unwrap().key.public_b64();
    let cases = [
        ("AAAA", "AAAAAAAAAAAAAAAAAAAAAA"),
        (good.as_str(), "AAAA"),
        (good.as_str(), "not base64!"),
        ("", ""),
    ];
    for (p256dh, authk) in cases {
        let body = serde_json::json!({
            "endpoint": ENDPOINT, "keys": { "p256dh": p256dh, "auth": authk }
        })
        .to_string();
        let r = send(
            s.addr,
            "PUT",
            "/push/subscription",
            &with_origin(&key),
            &body,
        );
        assert_eq!(r.status, 400, "{p256dh} / {authk}");
    }
    let r = send(
        s.addr,
        "PUT",
        "/push/subscription",
        &with_origin(&key),
        "{}",
    );
    assert_eq!(r.status, 400);
    assert!(s.store.subscriptions().unwrap().is_empty());
}

#[test]
fn display_role_is_forbidden() {
    let s = started();
    let code = s.store.new_code("Wall", Role::Display, Utc::now()).unwrap();
    let (_, key) = s.store.redeem(&code, Utc::now()).unwrap();
    assert_eq!(get(s.addr, "/push/key", &auth(&key)).status, 403);
    let r = send(
        s.addr,
        "PUT",
        "/push/subscription",
        &with_origin(&key),
        &sub_body(&s, ENDPOINT),
    );
    assert_eq!(r.status, 403);
    let r = send(
        s.addr,
        "DELETE",
        "/push/subscription",
        &with_origin(&key),
        "",
    );
    assert_eq!(r.status, 403);
}

#[test]
fn delete_clears_it() {
    let s = started();
    let (key, _) = pair(&s);
    let h = with_origin(&key);
    send(
        s.addr,
        "PUT",
        "/push/subscription",
        &h,
        &sub_body(&s, ENDPOINT),
    );
    assert_eq!(s.store.subscriptions().unwrap().len(), 1);
    let r = send(s.addr, "DELETE", "/push/subscription", &h, "");
    assert_eq!(r.status, 204);
    assert_eq!(r.body, "");
    assert!(s.store.subscriptions().unwrap().is_empty());
    let r = send(s.addr, "DELETE", "/push/subscription", &cookie(&key), "");
    assert_eq!(r.status, 403, "delete also needs an origin");
}

#[test]
fn insecure_has_no_push_routes() {
    let s = start(true, None);
    wait_started(&s);
    let (key, _) = pair(&s);
    let h = auth(&key);
    assert_eq!(get(s.addr, "/push/key", &h).status, 404);
    assert_eq!(
        send(s.addr, "PUT", "/push/subscription", &h, "{}").status,
        404
    );
    assert_eq!(
        send(s.addr, "DELETE", "/push/subscription", &h, "").status,
        404
    );
}
