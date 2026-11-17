use serde_json::{Value, json};
use twydo_daemon::client::Notification;
use twydo_daemon::protocol::events::{EventEnvelope, Heartbeat, Missed};
use twydo_daemon::protocol::rpc::{ErrorKind, REFUSED};
use twydo_gateway::bridge::{Ctx, inbound, outbound_note, request_prefix};
use twydo_gateway::state::{Device, Role};

fn dev(role: Role) -> Device {
    Device {
        id: "abcd1234".into(),
        name: "d".into(),
        role,
        paired_at: "2026-11-17T10:00:00Z".parse().unwrap(),
        last_seen: None,
        session: None,
    }
}

fn submit(op: Value) -> Option<Value> {
    Some(json!({
        "session": "forged", "request_id": "r1",
        "based_on": {"epoch": "e", "revision": 1}, "op": op
    }))
}

fn task(extra: Value) -> Value {
    let mut t = json!({
        "collection": "inbox", "summary": "Buy milk", "description": null,
        "due": null, "start": null, "kind": "Task", "parent": null, "repeat": null
    });
    for (k, v) in extra.as_object().unwrap() {
        t[k] = v.clone();
    }
    json!({"Create": t})
}

fn repeat_json() -> Value {
    let day = "2026-11-17".parse().unwrap();
    serde_json::to_value(twydo_core::repeatinput::parse("daily", day, &[]).unwrap()).unwrap()
}

fn run(
    role: Role,
    method: &str,
    p: Option<Value>,
) -> Result<(String, Value), twydo_daemon::protocol::rpc::RpcError> {
    let d = dev(role);
    inbound(
        &Ctx {
            device: &d,
            session: "gw-sess",
        },
        method,
        p,
    )
}

fn assert_refused(r: Result<(String, Value), twydo_daemon::protocol::rpc::RpcError>) {
    let e = r.unwrap_err();
    assert_eq!(e.code, REFUSED);
    assert_eq!(e.data.unwrap().kind, ErrorKind::Invalid);
}

#[test]
fn phone_allowlist() {
    assert!(run(Role::Phone, "views.get", Some(json!({"view": "inbox"}))).is_ok());
    for m in ["events.subscribe", "events.unsubscribe"] {
        assert!(run(Role::Phone, m, Some(json!({}))).is_ok(), "{m}");
    }
    assert!(run(Role::Phone, "op.submit", submit(task(json!({})))).is_ok());
    assert!(run(Role::Phone, "op.undo", Some(json!({"request_id": "u1"}))).is_ok());
}

#[test]
fn display_is_read_only() {
    assert!(run(Role::Display, "views.get", Some(json!({"view": "today"}))).is_ok());
    assert!(run(Role::Display, "events.subscribe", None).is_ok());
    assert_refused(run(Role::Display, "op.submit", submit(task(json!({})))));
    assert_refused(run(
        Role::Display,
        "op.undo",
        Some(json!({"request_id": "u1"})),
    ));
}

#[test]
fn methods_outside_role_refused() {
    for m in [
        "lists.delete",
        "daemon.shutdown",
        "journal.recover",
        "sync.run",
        "snapshot.get",
        "hello",
    ] {
        assert_refused(run(Role::Phone, m, None));
    }
}

#[test]
fn create_with_repeat_parent_start_kind_or_timed_due_refused() {
    for extra in [
        json!({"parent": "u"}),
        json!({"start": "2026-11-20"}),
        json!({"kind": "Project"}),
        json!({"due": {"Date": "2026-11-20"}, "repeat": repeat_json()}),
        json!({"due": {"Time": "2026-11-20T10:00:00"}}),
    ] {
        assert_refused(run(Role::Phone, "op.submit", submit(task(extra))));
    }
    assert!(
        run(
            Role::Phone,
            "op.submit",
            submit(task(json!({"due": {"Date": "2026-11-20"}})))
        )
        .is_ok()
    );
    assert!(
        run(
            Role::Phone,
            "op.submit",
            submit(json!({"Complete": {"uid": "u"}}))
        )
        .is_ok()
    );
}

#[test]
fn delete_move_edit_refused() {
    for op in [
        json!({"Delete": {"uid": "u"}}),
        json!({"Edit": {"uid": "u", "changes": []}}),
        json!({"Reorder": {"uid": "u", "anchor": "a", "placement": "Before"}}),
    ] {
        assert_refused(run(Role::Phone, "op.submit", submit(op)));
    }
    // Not even a parseable op.
    assert_refused(run(Role::Phone, "op.submit", submit(json!({"Nope": 1}))));
}

#[test]
fn forged_session_replaced() {
    let (_, p) = run(Role::Phone, "op.submit", submit(task(json!({})))).unwrap();
    assert_eq!(p["session"], "gw-sess");
    let (_, p) = run(
        Role::Phone,
        "op.undo",
        Some(json!({"session": "x", "request_id": "u"})),
    )
    .unwrap();
    assert_eq!(p["session"], "gw-sess");
}

#[test]
fn request_id_prefixed_and_validated() {
    assert_eq!(request_prefix(&dev(Role::Phone)), "gabcd1234-");
    let (_, p) = run(Role::Phone, "op.submit", submit(task(json!({})))).unwrap();
    assert_eq!(p["request_id"], "gabcd1234-r1");
    for bad in ["", "a b", "a/b", &"x".repeat(101)] {
        let mut s = submit(task(json!({}))).unwrap();
        s["request_id"] = json!(bad);
        assert_refused(run(Role::Phone, "op.submit", Some(s)));
    }
    let mut s = submit(task(json!({}))).unwrap();
    s["request_id"] = json!("x".repeat(100));
    assert!(run(Role::Phone, "op.submit", Some(s)).is_ok());
    let mut s = submit(task(json!({}))).unwrap();
    s.as_object_mut().unwrap().remove("request_id");
    assert_refused(run(Role::Phone, "op.submit", Some(s)));
}

fn event(request_id: Option<&str>) -> Notification {
    let mut ev =
        json!({"seq": 3, "type": "op.finished", "op_id": "o1", "state": {"state": "pending"}});
    if let Some(r) = request_id {
        ev["request_id"] = json!(r);
    }
    Notification::Event(serde_json::from_value::<EventEnvelope>(ev).unwrap())
}

fn out(role: Role, n: &Notification) -> Option<Value> {
    let d = dev(role);
    outbound_note(
        &Ctx {
            device: &d,
            session: "s",
        },
        n,
    )
}

#[test]
fn own_op_finished_forwarded_with_prefix_stripped() {
    let v = out(Role::Phone, &event(Some("gabcd1234-r1"))).unwrap();
    assert_eq!(v["method"], "event");
    assert_eq!(v["params"]["request_id"], "r1");
    assert_eq!(v["params"]["op_id"], "o1");
    assert_eq!(v["params"]["seq"], 3);
    assert!(out(Role::Display, &event(Some("gabcd1234-r1"))).is_none());
}

#[test]
fn other_devices_op_finished_dropped() {
    assert!(out(Role::Phone, &event(Some("gffff0000-r1"))).is_none());
    assert!(out(Role::Phone, &event(Some("tdo-1"))).is_none());
    assert!(out(Role::Phone, &event(None)).is_none());
}

#[test]
fn job_and_sync_events_dropped() {
    for ev in [
        json!({"seq": 1, "type": "sync.started", "reason": "manual"}),
        json!({"seq": 1, "type": "job.finished", "job_id": "j", "state": {"state": "done"}}),
        json!({"seq": 1, "type": "something.new"}),
    ] {
        let n = Notification::Event(serde_json::from_value(ev).unwrap());
        assert!(out(Role::Phone, &n).is_none());
    }
    for ev in [
        json!({"seq": 1, "type": "status.changed", "status_rev": 2}),
        json!({"seq": 1, "type": "data.changed", "revision": 2, "cause": "write", "uids": []}),
    ] {
        let n = Notification::Event(serde_json::from_value(ev).unwrap());
        let v = out(Role::Display, &n).unwrap();
        assert_eq!(v["method"], "event");
    }
    let m = out(
        Role::Display,
        &Notification::Missed(Missed {
            seq: 1,
            revision: 2,
            status_rev: 3,
        }),
    )
    .unwrap();
    assert_eq!(m["method"], "missed");
    let h = out(
        Role::Display,
        &Notification::Heartbeat(Heartbeat { seq: 4 }),
    )
    .unwrap();
    assert_eq!(h["params"]["seq"], 4);
}

#[test]
fn smuggled_fields_are_stripped() {
    let mut s = submit(task(json!({}))).unwrap();
    s["extra"] = json!("x");
    s["op"]["Create"]["bonus"] = json!(1);
    let (_, p) = run(Role::Phone, "op.submit", Some(s)).unwrap();
    assert!(p.get("extra").is_none());
    assert!(p["op"]["Create"].get("bonus").is_none());
    let (_, p) = run(
        Role::Phone,
        "op.undo",
        Some(json!({"request_id": "u", "extra": 1, "session": "s"})),
    )
    .unwrap();
    assert_eq!(p.as_object().unwrap().len(), 2);
    let (_, p) = run(
        Role::Phone,
        "op.submit",
        submit(json!({"Complete": {"uid": "u", "extra": 1}})),
    )
    .unwrap();
    assert_eq!(p["op"], json!({"Complete": {"uid": "u"}}));
    assert!(
        p.as_object()
            .unwrap()
            .keys()
            .all(|k| { ["session", "request_id", "based_on", "op"].contains(&k.as_str()) })
    );
    let (_, p) = run(
        Role::Phone,
        "views.get",
        Some(json!({"view": "inbox", "x": 1})),
    )
    .unwrap();
    assert_eq!(p, json!({"view": "inbox"}));
    let (_, p) = run(Role::Phone, "events.subscribe", Some(json!({"x": 1}))).unwrap();
    assert_eq!(p, json!({}));
    let (_, p) = run(Role::Display, "events.unsubscribe", None).unwrap();
    assert_eq!(p, json!({}));
}

#[test]
fn malformed_params_and_method_tricks_refused() {
    for m in ["op.submit", "op.undo"] {
        assert_refused(run(Role::Phone, m, None));
        assert_refused(run(Role::Phone, m, Some(Value::Null)));
        assert_refused(run(Role::Phone, m, Some(json!([1]))));
        assert_refused(run(Role::Phone, m, Some(json!("s"))));
    }
    assert_refused(run(Role::Phone, "views.get", None));
    assert_refused(run(Role::Phone, "views.get", Some(json!({"view": "Nope"}))));
    for rid in [json!(5), json!(null), json!(["a"])] {
        let mut s = submit(task(json!({}))).unwrap();
        s["request_id"] = rid;
        assert_refused(run(Role::Phone, "op.submit", Some(s)));
    }
    for m in [
        "views.get ",
        "VIEWS.GET",
        " views.get",
        "views.get\n",
        "op.submit\0",
    ] {
        assert_refused(run(Role::Phone, m, Some(json!({"view": "inbox"}))));
    }
}

#[test]
fn prefix_is_fixed_width_hex_so_it_names_one_device() {
    let p = request_prefix(&dev(Role::Phone));
    let id = &p[1..p.len() - 1];
    assert_eq!(id.len(), 8);
    assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
    assert!(p.starts_with('g') && p.ends_with('-'));
}
