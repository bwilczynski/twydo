//! The client's write contract, independent of the daemon's gate, store and queue behavior.
mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;

use serde_json::{Value, json};
use tdo::write::{Request, execute};
use twydo_core::config::Config;
use twydo_core::edit::Op;
use twydo_core::views::Snapshot;
use twydo_daemon::client::Client;
use twydo_daemon::protocol::methods::BasedOn;
use twydo_daemon::protocol::rpc::{ErrorKind, RpcError};

#[test]
fn forwards_request_identity_and_daemon_messages_and_translates_stale_titles() {
    let dir = common::short_socket_dir();
    let socket = dir.path().join("adapter.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let op = Op::Complete { uid: "u1".into() };
    let sent_op = op.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        for (i, method) in ["op.submit", "op.undo", "op.redo", "op.submit", "op.submit"]
            .into_iter()
            .enumerate()
        {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], method);
            let mut expected = json!({"session": "s-client", "request_id": format!("request-{i}")});
            if method == "op.submit" {
                expected["based_on"] = json!({"epoch": "d-client", "revision": 17});
                expected["op"] = serde_json::to_value(&sent_op).unwrap();
            }
            assert_eq!(request["params"], expected);
            let mut reply = json!({"jsonrpc": "2.0", "id": request["id"]});
            if i < 3 {
                reply["result"] = json!({"outcome": if i == 0 { "queued" } else { "applied" }, "op_id": "op-sentinel", "revision": 18, "message": format!("daemon message {i}")});
            } else {
                let mut error = RpcError::refused(
                    if i == 3 {
                        ErrorKind::Blocked
                    } else {
                        ErrorKind::Stale
                    },
                    "daemon refusal",
                );
                error.data.as_mut().unwrap().uid = Some("u1".into());
                reply["error"] = serde_json::to_value(error).unwrap();
            }
            writeln!(stream, "{reply}").unwrap();
        }
    });
    let client = Client::connect_raw(&socket).unwrap();
    let cal = twydo_core::ical::parse(b"BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:u1\r\nSUMMARY:Client title\r\nEND:VTODO\r\nEND:VCALENDAR\r\n").unwrap();
    let snap = Snapshot::new(
        twydo_core::vdir::LoadReport {
            tasks: twydo_core::model::tasks_from_calendar(&cal, "inbox", Path::new("inbox/u1.ics")),
            ..Default::default()
        },
        Config::with_vdir("/unused".into()),
    );
    let based = BasedOn {
        epoch: "d-client".into(),
        revision: 17,
    };
    for (i, (request, expected)) in [
        (Request::Apply(op.clone()), Ok("daemon message 0".into())),
        (Request::Undo, Ok("daemon message 1".into())),
        (Request::Redo, Ok("daemon message 2".into())),
        (Request::Apply(op.clone()), Err("daemon refusal".into())),
        (
            Request::Apply(op),
            Err("\"Client title\" changed on disk; reload and try again".into()),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            execute(
                &client.caller(),
                "s-client",
                &based,
                &snap,
                &request,
                &format!("request-{i}")
            ),
            expected
        );
    }
    server.join().unwrap();
}
