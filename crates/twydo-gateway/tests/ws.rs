//! The device WebSocket against an in-process daemon. Synthetic content only.

mod common;

use std::time::{Duration, Instant};

use common::{Env, Got, ORIGIN, WAIT};
use serde_json::{Value, json};
use twydo_gateway::state::Role;

fn create(summary: &str) -> Value {
    json!({"Create": {
        "collection": "inbox", "summary": summary, "description": null,
        "due": null, "start": null, "kind": "Task", "parent": null, "repeat": null
    }})
}

fn http_status(r: Result<common::Sock, tungstenite::Error>) -> u16 {
    match r {
        Err(tungstenite::Error::Http(resp)) => resp.status().as_u16(),
        Err(e) => panic!("unexpected error {e}"),
        Ok(_) => panic!("upgrade was accepted"),
    }
}

#[test]
fn views_get_through_socket() {
    let env = Env::new(&[("t1", "Buy milk")]);
    let key = env.pair("Pixel", Role::Phone);
    let mut s = env.connect(&key);
    let v = s.ok("views.get", json!({"view": "inbox"}));
    assert_eq!(v["sections"][0]["rows"][0]["uid"], "t1");
}

#[test]
fn complete_and_undo_through_socket() {
    let env = Env::new(&[("t1", "Buy milk")]);
    let key = env.pair("Pixel", Role::Phone);
    let mut s = env.connect(&key);
    let r = s.submit("c1", json!({"Complete": {"uid": "t1"}}));
    assert!(r.get("error").is_none(), "{r}");
    assert!(env.read("inbox/t1.ics").contains("STATUS:COMPLETED"));
    let r = s.rpc("op.undo", json!({"request_id": "u1"}));
    assert!(r.get("error").is_none(), "{r}");
    assert!(!env.read("inbox/t1.ics").contains("STATUS:COMPLETED"));
}

#[test]
fn capture_creates_task_in_inbox() {
    let env = Env::new(&[]);
    let key = env.pair("Pixel", Role::Phone);
    let mut s = env.connect(&key);
    let r = s.submit("n1", create("Captured idea"));
    assert!(r.get("error").is_none(), "{r}");
    assert_eq!(env.inbox_with("SUMMARY:Captured idea").len(), 1);
}

#[test]
fn same_request_id_writes_once() {
    let env = Env::new(&[]);
    let key = env.pair("Pixel", Role::Phone);
    let mut s = env.connect(&key);
    let first = s.submit("dup", create("Only once"));
    assert!(first.get("error").is_none(), "{first}");
    let second = s.submit("dup", create("Only once"));
    assert!(second.get("error").is_none(), "{second}");
    assert_eq!(env.inbox_with("SUMMARY:Only once").len(), 1);
}

#[test]
fn two_devices_have_separate_undo() {
    let env = Env::new(&[("t1", "First"), ("t2", "Second")]);
    let ka = env.pair("PhoneA", Role::Phone);
    let kb = env.pair("PhoneB", Role::Phone);
    let mut a = env.connect(&ka);
    let mut b = env.connect(&kb);
    assert!(
        a.submit("x", json!({"Complete": {"uid": "t1"}}))
            .get("error")
            .is_none()
    );
    assert!(
        b.submit("x", json!({"Complete": {"uid": "t2"}}))
            .get("error")
            .is_none()
    );
    let r = a.rpc("op.undo", json!({"request_id": "ua"}));
    assert!(r.get("error").is_none(), "{r}");
    assert!(!env.read("inbox/t1.ics").contains("STATUS:COMPLETED"));
    assert!(env.read("inbox/t2.ics").contains("STATUS:COMPLETED"));
}

#[test]
fn events_reach_subscribed_socket() {
    let env = Env::new(&[("t1", "First")]);
    let key = env.pair("Pixel", Role::Phone);
    let mut s = env.connect(&key);
    s.ok("events.subscribe", json!({}));
    std::fs::write(
        env.vdir().join("inbox/t9.ics"),
        common::ics("t9", "Outside"),
    )
    .unwrap();
    env.daemon
        .reload_for_test(twydo_daemon::protocol::events::Cause::Outside)
        .unwrap();
    let deadline = Instant::now() + WAIT;
    loop {
        assert!(Instant::now() < deadline, "no data.changed");
        match s.next(WAIT) {
            Got::Json(v) if v["method"] == "event" && v["params"]["type"] == "data.changed" => {
                break;
            }
            Got::Json(_) => {}
            Got::Closed(f) => panic!("closed: {f:?}"),
        }
    }
}

#[test]
fn disallowed_method_never_reaches_daemon() {
    let env = Env::new(&[("t1", "First")]);
    let key = env.pair("Wall", Role::Display);
    let mut s = env.connect(&key);
    let r = s.submit("w1", json!({"Complete": {"uid": "t1"}}));
    assert_eq!(r["error"]["code"], -32000, "{r}");
    assert!(!env.read("inbox/t1.ics").contains("STATUS:COMPLETED"));
    let r = s.rpc("daemon.shutdown", json!({"when_idle": false}));
    assert_eq!(r["error"]["code"], -32000, "{r}");
    // The daemon is still serving.
    s.ok("views.get", json!({"view": "inbox"}));
    assert!(!env.daemon.is_stopped());
}

#[test]
fn revoke_closes_socket_within_5s() {
    let env = Env::new(&[]);
    let key = env.pair("Pixel", Role::Phone);
    let mut s = env.connect(&key);
    s.ok("views.get", json!({"view": "inbox"}));
    let t = Instant::now();
    env.store.revoke("Pixel").unwrap();
    match s.next(Duration::from_secs(5)) {
        Got::Closed(Some(f)) => {
            assert_eq!(u16::from(f.code), 1008);
            assert_eq!(f.reason.as_str(), "device revoked");
        }
        Got::Closed(None) => panic!("closed without a frame"),
        Got::Json(v) => panic!("unexpected {v}"),
    }
    assert!(t.elapsed() < Duration::from_secs(5));
}

#[test]
fn revoke_with_unchanged_mtime_still_closes_socket() {
    let env = Env::new(&[]);
    let key = env.pair("Pixel", Role::Phone);
    let mut s = env.connect(&key);
    s.ok("views.get", json!({"view": "inbox"}));
    let file = env.root.path().join("gw/devices.json");
    let before = std::fs::metadata(&file).unwrap().modified().unwrap();
    env.store.revoke("Pixel").unwrap();
    // Pin the timestamp to what it was: coarse clocks can make this happen for real.
    std::fs::OpenOptions::new()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(before)
        .unwrap();
    match s.next(Duration::from_secs(5)) {
        Got::Closed(Some(f)) => assert_eq!(u16::from(f.code), 1008),
        Got::Closed(None) => panic!("closed without a frame"),
        Got::Json(v) => panic!("unexpected {v}"),
    }
}

#[test]
fn daemon_restart_closes_socket() {
    let env = Env::new(&[]);
    let key = env.pair("Pixel", Role::Phone);
    let mut s = env.connect(&key);
    s.ok("views.get", json!({"view": "inbox"}));
    env.daemon.shutdown();
    env.daemon.wait_stopped();
    match s.next(Duration::from_secs(5)) {
        Got::Closed(Some(f)) => {
            assert_eq!(u16::from(f.code), 1011);
            assert_eq!(f.reason.as_str(), "daemon not running");
        }
        Got::Closed(None) => panic!("closed without a frame"),
        Got::Json(v) => panic!("unexpected {v}"),
    }
}

#[test]
fn upgrade_without_key_refused() {
    let env = Env::new(&[]);
    assert_eq!(http_status(env.try_connect(None, Some(ORIGIN))), 401);
    assert_eq!(
        http_status(env.try_connect(Some("nope"), Some(ORIGIN))),
        401
    );
}

#[test]
fn upgrade_with_foreign_origin_refused() {
    let env = Env::new(&[]);
    let key = env.pair("Pixel", Role::Phone);
    assert_eq!(
        http_status(env.try_connect(Some(&key), Some("https://evil.example.test"))),
        403
    );
    assert_eq!(http_status(env.try_connect(Some(&key), None)), 403);
}

#[test]
fn non_json_frame_gets_parse_error() {
    let env = Env::new(&[]);
    let key = env.pair("Pixel", Role::Phone);
    let mut s = env.connect(&key);
    s.send_raw("this is not json");
    match s.next(WAIT) {
        Got::Json(v) => {
            assert_eq!(v["error"]["code"], -32700, "{v}");
            assert!(v["id"].is_null());
        }
        Got::Closed(f) => panic!("closed: {f:?}"),
    }
}

#[test]
fn revoke_during_connect_closes_socket() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    // Arm the hook only for the device's own connection (the first call).
    let armed = Arc::new(AtomicBool::new(true));
    let a = Arc::clone(&armed);
    let hook: common::Hook = Arc::new(move |store| {
        if a.swap(false, Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(300));
            store.revoke("Pixel").unwrap();
            std::thread::sleep(Duration::from_millis(300));
        }
    });
    let env = Env::with_hook(&[], Some(hook));
    let key = env.pair("Pixel", Role::Phone);
    let mut s = env.connect(&key);
    match s.next(Duration::from_secs(5)) {
        Got::Closed(Some(f)) => {
            assert_eq!(u16::from(f.code), 1008);
            assert_eq!(f.reason.as_str(), "device revoked");
        }
        Got::Closed(None) => panic!("closed without a frame"),
        Got::Json(v) => panic!("unexpected {v}"),
    }
}

#[test]
fn too_many_pending_requests_are_refused() {
    let env = Env::new(&[("t1", "First")]);
    let key = env.pair("Pixel", Role::Phone);
    let mut s = env.connect(&key);
    let n = 3000u64;
    for id in 1..=n {
        s.send_raw(
            &json!({"jsonrpc":"2.0","id":id,"method":"views.get","params":{"view":"inbox"}})
                .to_string(),
        );
    }
    let (mut ok, mut refused) = (0, 0);
    while ok + refused < n {
        match s.next(WAIT) {
            Got::Json(v) if v.get("result").is_some() => ok += 1,
            Got::Json(v) => {
                assert_eq!(v["error"]["code"], -32000, "{v}");
                assert_eq!(v["error"]["message"], "too many pending requests");
                assert_eq!(v["error"]["data"]["kind"], "unavailable");
                refused += 1;
            }
            Got::Closed(f) => panic!("closed: {f:?}"),
        }
    }
    assert!(refused > 0, "queue never filled (ok={ok})");
    assert!(ok > 0);
}

#[test]
fn reconnect_resumes_the_stored_session() {
    let env = Env::new(&[("t1", "First")]);
    let key = env.pair("Pixel", Role::Phone);
    let mut s = env.connect(&key);
    let r = s.submit("c1", json!({"Complete": {"uid": "t1"}}));
    assert!(r.get("error").is_none(), "{r}");
    drop(s);
    let stored = env.store.authenticate(&key).unwrap().session;
    assert!(stored.is_some(), "session not stored");
    let mut s = env.connect(&key);
    let r = s.rpc("op.undo", json!({"request_id": "u1"}));
    assert!(r.get("error").is_none(), "undo after reconnect: {r}");
    assert!(!env.read("inbox/t1.ics").contains("STATUS:COMPLETED"));
    assert_eq!(env.store.authenticate(&key).unwrap().session, stored);
}

/// A masked text frame header for a payload of `len` (< 65536) bytes, with a zero mask key.
fn masked_text_header(len: usize) -> Vec<u8> {
    let mut h = vec![0x81];
    if len < 126 {
        h.push(0x80 | len as u8);
    } else {
        h.push(0x80 | 126);
        h.extend((len as u16).to_be_bytes());
    }
    h.extend([0, 0, 0, 0]);
    h
}

/// Writes `bytes` a few at a time, 30 ms apart, until `stop` is set or the peer is gone.
fn trickle(
    mut raw: std::net::TcpStream,
    bytes: Vec<u8>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> std::thread::JoinHandle<()> {
    use std::io::Write;
    use std::sync::atomic::Ordering;
    std::thread::spawn(move || {
        for chunk in bytes.chunks(2) {
            if stop.load(Ordering::SeqCst) || raw.write_all(chunk).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    })
}

#[test]
fn revoke_closes_socket_while_a_frame_trickles_in() {
    use std::sync::{Arc, atomic::AtomicBool, atomic::Ordering};
    let env = Env::new(&[]);
    let key = env.pair("Pixel", Role::Phone);
    let mut s = env.connect(&key);
    s.ok("views.get", json!({"view": "inbox"}));
    // An incomplete frame: 1000 declared payload bytes, delivered two per 30 ms (15 s).
    let mut bytes = masked_text_header(1000);
    bytes.extend(std::iter::repeat_n(b' ', 999));
    let stop = Arc::new(AtomicBool::new(false));
    let raw = s.ws.get_ref().try_clone().unwrap();
    let t = trickle(raw, bytes, Arc::clone(&stop));
    std::thread::sleep(Duration::from_millis(300));
    env.store.revoke("Pixel").unwrap();
    let started = Instant::now();
    // Never finish the frame: the close must come from the revocation alone.
    let got = s.next(Duration::from_secs(5));
    stop.store(true, Ordering::SeqCst);
    t.join().unwrap();
    match got {
        Got::Closed(Some(f)) => {
            assert_eq!(u16::from(f.code), 1008);
            assert_eq!(f.reason.as_str(), "device revoked");
        }
        Got::Closed(None) => panic!("closed without a frame"),
        Got::Json(v) => panic!("unexpected {v}"),
    }
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn request_finished_after_revoke_is_not_executed() {
    use std::io::Write;
    use std::sync::{Arc, atomic::AtomicBool};
    let env = Env::new(&[("t1", "First")]);
    let key = env.pair("Pixel", Role::Phone);
    let mut s = env.connect(&key);
    let based_on = s.based_on();
    let body = json!({"jsonrpc":"2.0","id":1,"method":"op.submit","params":
        {"request_id":"late","based_on":based_on,"op":{"Complete":{"uid":"t1"}}}})
    .to_string();
    let mut frame = masked_text_header(body.len());
    frame.extend(body.bytes()); // zero mask key: the payload is sent as is
    let (head, tail) = frame.split_at(frame.len() - 4);
    let mut raw = s.ws.get_ref().try_clone().unwrap();
    raw.write_all(head).unwrap();
    // Keep the connection busy so only the missing 4 bytes are outstanding.
    let _ = trickle(
        s.ws.get_ref().try_clone().unwrap(),
        Vec::new(),
        Arc::new(AtomicBool::new(false)),
    );
    env.store.revoke("Pixel").unwrap();
    match s.next(Duration::from_secs(5)) {
        Got::Closed(Some(f)) => assert_eq!(u16::from(f.code), 1008),
        other => panic!(
            "expected a policy close, got {}",
            matches!(other, Got::Json(_))
        ),
    }
    // The rest of the frame arrives too late (the peer may already be gone).
    let _ = raw.write_all(tail);
    std::thread::sleep(Duration::from_millis(500));
    assert!(!env.read("inbox/t1.ics").contains("STATUS:COMPLETED"));
}

/// The kernel's state code (hex, as in `/proc/net/tcp`) of the server end of a loopback
/// connection: the socket whose local port is `server` and remote port is `client`.
#[cfg(target_os = "linux")]
fn server_tcp_state(server: u16, client: u16) -> Option<String> {
    let table = std::fs::read_to_string("/proc/net/tcp").ok()?;
    table.lines().skip(1).find_map(|l| {
        let mut f = l.split_whitespace().skip(1);
        let (local, remote, state) = (f.next()?, f.next()?, f.next()?);
        let port = |a: &str| u16::from_str_radix(a.rsplit(':').next()?, 16).ok();
        (port(local)? == server && port(remote)? == client).then(|| state.to_owned())
    })
}

/// A peer that stops reading while the gateway sends it a huge reply leaves the pump blocked
/// in a write. Revocation must still end the transport within the bound. The peer never
/// reads (reading would unblock the writer), so the server end is observed through the
/// kernel's socket table: it leaves ESTABLISHED only once the gateway shuts the socket.
#[cfg(target_os = "linux")]
#[test]
fn revoke_ends_transport_while_pump_is_blocked_writing() {
    use std::os::fd::AsRawFd;
    let note = "x".repeat(12 * 1024 * 1024);
    let env = Env::new(&[("t1", note.as_str())]);
    let key = env.pair("Pixel", Role::Phone);
    let mut s = env.connect(&key);
    let raw = s.ws.get_ref();
    let small: libc::c_int = 4096;
    let rc = unsafe {
        libc::setsockopt(
            raw.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            (&small as *const libc::c_int).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    assert_eq!(rc, 0, "SO_RCVBUF");
    let client_port = raw.local_addr().unwrap().port();
    let server_port = env.addr.port();
    s.send_raw(r#"{"jsonrpc":"2.0","id":1,"method":"views.get","params":{"view":"inbox"}}"#);
    // The reply is far larger than both socket buffers: the pump is now stuck writing it.
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(
        server_tcp_state(server_port, client_port).as_deref(),
        Some("01"),
        "server end should be established while the peer is not reading"
    );
    env.store.revoke("Pixel").unwrap();
    let started = Instant::now();
    let limit = Duration::from_secs(5);
    loop {
        let state = server_tcp_state(server_port, client_port);
        if state.as_deref() != Some("01") {
            break;
        }
        assert!(
            started.elapsed() < limit,
            "transport still open {:?} after revoke",
            started.elapsed()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
