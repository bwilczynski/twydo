//! `tdo gateway` against the built binary. Synthetic content; every path is inside a tempdir
//! and the daemon is the environment's own (stopped on drop).

mod common;

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::*;
use serde_json::{Value, json};
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;

fn gw(env: &TestEnv, state: &std::path::Path) -> Command {
    let mut c = env.tdo_cmd();
    c.arg("gateway").arg("--state-dir").arg(state);
    c
}

fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

#[test]
fn serve_requires_origin() {
    let env = TestEnv::new("");
    let state = env.root.path().join("gw");
    let out = gw(&env, &state).arg("serve").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains(
        "tdo gateway: --origin is required (or --insecure for plain HTTP on a trusted network)"
    ));
}

fn serve_err(args: &[&str]) -> (Option<i32>, String) {
    let env = TestEnv::new("");
    let state = env.root.path().join("gw");
    let out = gw(&env, &state).arg("serve").args(args).output().unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn serve_rejects_a_bad_origin() {
    for args in [
        &["--origin", ""][..],
        &["--origin", "https://a.test/app"],
        &["--origin", "http://a.test"],
        &["--origin", "a.test"],
    ] {
        let (code, err) = serve_err(args);
        assert_eq!(code, Some(2), "{args:?}: {err}");
        assert!(err.contains("tdo gateway:"), "{args:?}: {err}");
    }
}

#[test]
fn serve_rejects_an_empty_origin_from_the_environment() {
    let env = TestEnv::new("");
    let state = env.root.path().join("gw");
    let out = gw(&env, &state)
        .arg("serve")
        .env("TWYDO_GATEWAY_ORIGIN", "")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn pair_prints_code_and_devices_lists_it() {
    let env = TestEnv::new("");
    let state = env.root.path().join("gw");
    let out = gw(&env, &state).args(["pair", "Pixel"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("Pairing code for Pixel: "), "{text}");
    assert!(text.contains("(valid 10 minutes)"), "{text}");
    let code = text
        .trim_start_matches("Pairing code for Pixel: ")
        .split(' ')
        .next()
        .unwrap();
    assert_eq!(code.len(), 9);
    assert_eq!(code.as_bytes()[4], b'-');
    // A pending code is not yet a device.
    let out = gw(&env, &state).arg("devices").output().unwrap();
    assert!(out.status.success());
    assert!(!String::from_utf8_lossy(&out.stdout).contains("Pixel"));
}

#[test]
fn revoke_unknown_device_fails() {
    let env = TestEnv::new("");
    let state = env.root.path().join("gw");
    let out = gw(&env, &state)
        .args(["revoke", "Nobody"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
}

fn http(addr: &str, req: &str) -> Option<String> {
    let mut s = TcpStream::connect(addr).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(req.as_bytes()).ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).ok()?;
    Some(out)
}

#[test]
fn end_to_end_pair_and_views_get() {
    let env = TestEnv::new("");
    fs::write(
        env.vdir().join("inbox/t1.ics"),
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\nUID:t1\r\n\
         SUMMARY:Buy milk\r\nEND:VTODO\r\nEND:VCALENDAR\r\n",
    )
    .unwrap();
    let _daemon = env.spawn(&[]);
    env.connect(&env.socket());

    let state = env.root.path().join("gw");
    let port = free_port();
    let addr = format!("127.0.0.1:{port}");
    let origin = format!("http://{addr}");
    let mut c = gw(&env, &state);
    c.args([
        "serve",
        "--insecure",
        "--origin",
        &origin,
        "--listen",
        &addr,
    ])
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    let _gateway = Proc(c.spawn().unwrap());

    let deadline = Instant::now() + WAIT;
    while TcpStream::connect(&addr).is_err() {
        assert!(Instant::now() < deadline, "gateway never accepted");
        std::thread::sleep(Duration::from_millis(50));
    }

    let out = gw(&env, &state).args(["pair", "Pixel"]).output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    let code = text
        .trim_start_matches("Pairing code for Pixel: ")
        .split(' ')
        .next()
        .unwrap()
        .to_string();

    let body = json!({ "code": code }).to_string();
    let resp = http(
        &addr,
        &format!(
            "POST /pair HTTP/1.1\r\nHost: {addr}\r\nOrigin: {origin}\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        ),
    )
    .expect("pair response");
    assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
    let payload: Value = serde_json::from_str(resp.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    let key = payload["key"].as_str().unwrap().to_string();

    let devices = gw(&env, &state).arg("devices").output().unwrap();
    assert!(String::from_utf8_lossy(&devices.stdout).contains("Pixel"));

    let mut req = format!("ws://{addr}/ws").into_client_request().unwrap();
    req.headers_mut()
        .insert("Cookie", format!("twydo_device={key}").parse().unwrap());
    req.headers_mut().insert("Origin", origin.parse().unwrap());
    let stream = TcpStream::connect(&addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let (mut ws, _) = tungstenite::client(req, stream).expect("upgrade");
    ws.send(Message::text(
        json!({"jsonrpc":"2.0","id":1,"method":"views.get","params":{"view":"inbox"}}).to_string(),
    ))
    .unwrap();
    let deadline = Instant::now() + WAIT;
    loop {
        assert!(Instant::now() < deadline, "no views.get reply");
        match ws.read() {
            Ok(Message::Text(t)) => {
                let v: Value = serde_json::from_str(t.as_str()).unwrap();
                if v["id"] == json!(1) {
                    assert_eq!(v["result"]["sections"][0]["rows"][0]["uid"], "t1", "{v}");
                    break;
                }
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => panic!("socket error: {e}"),
        }
    }

    let out = gw(&env, &state).args(["revoke", "Pixel"]).output().unwrap();
    assert!(out.status.success());
    env.stop_daemon();
}
