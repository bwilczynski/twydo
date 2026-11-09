use twydo_daemon::protocol::events::{Cause, Event, EventEnvelope};
use twydo_daemon::protocol::methods::{Instance, OpState};
use twydo_daemon::protocol::rpc::{ErrorKind, LineError, MAX_LINE, RpcError, read_line};

#[test]
fn refusal_carries_a_stable_kind() {
    let e = RpcError::refused(
        ErrorKind::Stale,
        "\"X\" changed on disk; reload and try again",
    );
    let v = serde_json::to_value(&e).unwrap();
    assert_eq!(v["code"], -32000);
    assert_eq!(v["data"]["kind"], "stale");
}

#[test]
fn events_serialize_as_in_the_spec() {
    let e = EventEnvelope {
        seq: 120,
        event: Event::DataChanged {
            revision: 43,
            cause: Cause::Sync,
            uids: vec!["a".into()],
        },
    };
    assert_eq!(
        serde_json::to_value(&e).unwrap(),
        serde_json::json!({"seq":120,"type":"data.changed","revision":43,"cause":"sync","uids":["a"]})
    );
    let back: EventEnvelope = serde_json::from_value(serde_json::to_value(&e).unwrap()).unwrap();
    assert_eq!(back, e);
}

#[test]
fn op_state_is_tagged_by_state() {
    let s = OpState::Applied { op_id: "o1".into() };
    assert_eq!(
        serde_json::to_value(&s).unwrap(),
        serde_json::json!({"state":"applied","op_id":"o1"})
    );
    let p: OpState = serde_json::from_str(r#"{"state":"pending"}"#).unwrap();
    assert_eq!(p, OpState::Pending);
}

#[test]
fn instance_is_hashable() {
    let a = Instance {
        config: "/c".into(),
        vdir: None,
    };
    let set: std::collections::HashSet<_> = [a.clone(), a].into();
    assert_eq!(set.len(), 1);
}

#[test]
fn read_line_refuses_an_oversized_line_without_buffering_it() {
    let big = vec![b'x'; MAX_LINE + 1];
    let mut r = std::io::BufReader::new(std::io::Cursor::new(big));
    assert!(matches!(read_line(&mut r), Err(LineError::TooLong)));
}

#[test]
fn read_line_splits_pipelined_messages() {
    let mut r = std::io::BufReader::new(&b"{\"a\":1}\n{\"b\":2}\n"[..]);
    assert_eq!(read_line(&mut r).unwrap().as_deref(), Some("{\"a\":1}"));
    assert_eq!(read_line(&mut r).unwrap().as_deref(), Some("{\"b\":2}"));
    assert_eq!(read_line(&mut r).unwrap(), None);
}

#[test]
fn read_line_strips_cr_and_returns_a_final_unterminated_line() {
    let mut r = std::io::BufReader::new(&b"a\r\nb"[..]);
    assert_eq!(read_line(&mut r).unwrap().as_deref(), Some("a"));
    assert_eq!(read_line(&mut r).unwrap().as_deref(), Some("b"));
    assert_eq!(read_line(&mut r).unwrap(), None);
}

#[test]
fn read_line_rejects_invalid_utf8() {
    let mut r = std::io::BufReader::new(&b"\xff\xfe\n"[..]);
    assert!(matches!(read_line(&mut r), Err(LineError::InvalidUtf8)));
}
