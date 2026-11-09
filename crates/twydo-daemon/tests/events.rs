//! Subscriptions: one event per published revision, no event for a no-op reload, slow
//! subscribers, `missed` and heartbeats. Synthetic fixture content.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use common::{Fixture, TestDaemon, empty, todo_ics};
use serde_json::Value;
use twydo_daemon::client::{Client, Notification};
use twydo_daemon::protocol::events::{Cause, Event};
use twydo_daemon::protocol::methods::SubscribeResult;

const WAIT: Duration = Duration::from_secs(10);

fn fx() -> Fixture {
    Fixture::new(&[("inbox", "uid-a", "Alpha", None)])
}

fn subscribed(td: &TestDaemon) -> (Client, SubscribeResult) {
    let c = td.client();
    let r: SubscribeResult = c.call("events.subscribe", empty()).unwrap();
    (c, r)
}

/// The next `data.changed`, skipping everything else.
fn next_data(c: &Client) -> (u64, u64) {
    let deadline = Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if let Notification::Event(e) = c.notifications().recv_timeout(left).expect("no event")
            && let Event::DataChanged { revision, .. } = e.event
        {
            return (e.seq, revision);
        }
    }
}

#[test]
fn one_data_changed_per_published_revision() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let (c, sub) = subscribed(&td);
    let r1 = td.touch_outside("inbox/uid-b.ics", &todo_ics("uid-b", "Beta", None));
    assert_eq!(r1, sub.revision + 1);
    // Reloading without a change publishes nothing.
    assert_eq!(td.handle().reload_for_test(Cause::Outside).unwrap(), r1);
    let r2 = td.touch_outside("inbox/uid-c.ics", &todo_ics("uid-c", "Gamma", None));
    assert_eq!(r2, r1 + 1);
    let (s1, v1) = next_data(&c);
    let (s2, v2) = next_data(&c);
    assert_eq!((v1, v2), (r1, r2));
    assert!(s2 > s1 && s1 > sub.seq);
    // Nothing else is pending: exactly one data.changed per revision.
    while let Ok(n) = c.notifications().recv_timeout(Duration::from_millis(200)) {
        assert!(
            !matches!(n, Notification::Event(e) if matches!(e.event, Event::DataChanged { .. })),
            "extra data.changed"
        );
    }
}

#[test]
fn a_reload_without_changes_publishes_nothing() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let (c, sub) = subscribed(&td);
    let r = td.handle().reload_for_test(Cause::Outside).unwrap();
    assert_eq!(r, sub.revision);
    let snap: Value = c.call("snapshot.get", empty()).unwrap();
    assert_eq!(snap["revision"].as_u64().unwrap(), sub.revision);
    assert!(
        c.notifications()
            .recv_timeout(Duration::from_millis(200))
            .is_err()
    );
}

#[test]
fn unsubscribe_stops_events() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let (c, _) = subscribed(&td);
    let _: Value = c.call("events.unsubscribe", empty()).unwrap();
    td.touch_outside("inbox/uid-b.ics", &todo_ics("uid-b", "Beta", None));
    assert!(
        c.notifications()
            .recv_timeout(Duration::from_millis(200))
            .is_err()
    );
}

/// A subscriber on a raw stream that is not read until told to.
fn raw_subscriber(td: &TestDaemon) -> BufReader<UnixStream> {
    let mut s = UnixStream::connect(td.socket()).unwrap();
    let hello =
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"hello","params":td.hello_params()});
    let sub = serde_json::json!({"jsonrpc":"2.0","id":2,"method":"events.subscribe","params":{}});
    writeln!(s, "{hello}\n{sub}").unwrap();
    let mut r = BufReader::new(s);
    for _ in 0..2 {
        let mut l = String::new();
        r.read_line(&mut l).unwrap();
        assert!(l.contains("\"result\""), "{l}");
    }
    r
}

fn big(n: usize) -> Vec<String> {
    // Large uid lists make each event big enough to fill the socket buffer.
    vec!["x".repeat(64 * 1024); n]
}

#[test]
fn stalled_subscriber_does_not_block_others() {
    let fx = fx();
    let td = TestDaemon::start_with(&fx, |o| {
        o.event_buffer = 4;
        o.heartbeat = Duration::from_millis(50);
    });
    let mut a = raw_subscriber(&td);
    let (b, _) = subscribed(&td);

    let mut last = 0;
    let mut revs = Vec::new();
    let mut last_seq = 0;
    for i in 0..20 {
        let uid = format!("uid-{i}");
        let rel = format!("inbox/{uid}.ics");
        std::fs::write(fx.vdir().join(&rel), todo_ics(&uid, "Item", None)).unwrap();
        last = i + 1;
        td.handle()
            .reload_for_test_with(Cause::Outside, big(1))
            .unwrap();
        // B keeps up: it sees each revision before the next is published, so a slow machine
        // cannot push it behind its own small buffer.
        let (seq, rev) = next_data(&b);
        last_seq = seq;
        revs.push(rev);
    }
    assert_eq!(revs.len(), last);
    assert!(revs.windows(2).all(|w| w[1] == w[0] + 1), "{revs:?}");
    // The daemon still answers while A is stalled.
    let _: Value = b.call("daemon.info", empty()).unwrap();

    // A now reads: some events, then one `missed` covering everything emitted.
    let deadline = Instant::now() + WAIT;
    let mut missed_seq = None;
    while missed_seq.is_none() {
        assert!(Instant::now() < deadline, "no missed");
        let mut l = String::new();
        a.get_mut().set_read_timeout(Some(WAIT)).unwrap();
        a.read_line(&mut l).unwrap();
        let v: Value = serde_json::from_str(&l).unwrap();
        if v["method"] == "missed" {
            missed_seq = v["params"]["seq"].as_u64();
        }
    }
    assert!(
        missed_seq.unwrap() >= last_seq,
        "{missed_seq:?} < {last_seq}"
    );
}

#[test]
fn events_after_missed_continue_in_order() {
    let fx = fx();
    let td = TestDaemon::start_with(&fx, |o| {
        o.event_buffer = 4;
        o.heartbeat = Duration::from_millis(50);
    });
    let mut a = raw_subscriber(&td);
    for i in 0..20 {
        let uid = format!("uid-{i}");
        std::fs::write(
            fx.vdir().join(format!("inbox/{uid}.ics")),
            todo_ics(&uid, "Item", None),
        )
        .unwrap();
        td.handle()
            .reload_for_test_with(Cause::Outside, big(1))
            .unwrap();
    }
    a.get_mut().set_read_timeout(Some(WAIT)).unwrap();
    let mut missed_seq;
    loop {
        let mut l = String::new();
        a.read_line(&mut l).unwrap();
        let v: Value = serde_json::from_str(&l).unwrap();
        if v["method"] == "missed" {
            missed_seq = v["params"]["seq"].as_u64().unwrap();
            break;
        }
    }
    // Fresh events follow with consecutive seq, none repeating the missed ones.
    for i in 20..23 {
        let uid = format!("uid-{i}");
        std::fs::write(
            fx.vdir().join(format!("inbox/{uid}.ics")),
            todo_ics(&uid, "Item", None),
        )
        .unwrap();
        td.handle().reload_for_test(Cause::Outside).unwrap();
    }
    let mut seen = Vec::new();
    while seen.len() < 3 {
        let mut l = String::new();
        a.read_line(&mut l).unwrap();
        let v: Value = serde_json::from_str(&l).unwrap();
        if v["method"] == "event" {
            let seq = v["params"]["seq"].as_u64().unwrap();
            if seq > missed_seq {
                seen.push(seq);
            }
        } else if v["method"] == "missed" {
            missed_seq = v["params"]["seq"].as_u64().unwrap();
        }
    }
    assert!(seen.windows(2).all(|w| w[1] == w[0] + 1), "{seen:?}");
}

#[test]
fn heartbeat_on_idle() {
    let fx = fx();
    let td = TestDaemon::start_with(&fx, |o| o.heartbeat = Duration::from_millis(50));
    let (c, sub) = subscribed(&td);
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match c.notifications().recv_timeout(left) {
            Ok(Notification::Heartbeat(h)) => {
                assert_eq!(h.seq, sub.seq);
                return;
            }
            Ok(_) => {}
            Err(_) => panic!("no heartbeat within 500 ms"),
        }
    }
}

#[test]
fn shutdown_stops_the_heartbeat_thread() {
    let fx = fx();
    let td = TestDaemon::start_with(&fx, |o| o.heartbeat = Duration::from_millis(10));
    let (_c, _) = subscribed(&td);
    let t = Instant::now();
    td.handle().shutdown();
    assert!(t.elapsed() < Duration::from_secs(5));
}

#[test]
fn only_published_revisions_are_kept() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let snap: Value = td.client().call("snapshot.get", empty()).unwrap();
    assert_eq!(snap["revision"], 1);
    let p = td.handle().publisher_for_test();
    assert!(p.at(0).is_none());
    assert!(p.at(1).is_some());
}

/// The next `data.changed` within `limit`.
fn data_within(c: &Client, limit: Duration) -> Option<(u64, Cause, Vec<String>)> {
    let deadline = Instant::now() + limit;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match c.notifications().recv_timeout(left) {
            Ok(Notification::Event(e)) => {
                if let Event::DataChanged {
                    revision,
                    cause,
                    uids,
                } = e.event
                {
                    return Some((revision, cause, uids));
                }
            }
            Ok(_) => {}
            Err(_) => return None,
        }
    }
}

#[test]
fn a_hand_edit_is_published_with_cause_outside() {
    let fx = Fixture::new(&[("inbox", "u1", "Alpha", None)]);
    let td = TestDaemon::start(&fx);
    let (c, sub) = subscribed(&td);
    fx.write("inbox/u1.ics", &todo_ics("u1", "Alpha edited", None));
    let (rev, cause, uids) = data_within(&c, Duration::from_secs(2)).expect("no data.changed");
    assert_eq!(rev, sub.revision + 1);
    assert_eq!(cause, Cause::Outside);
    assert_eq!(uids, ["u1"]);
}

#[test]
fn a_change_during_a_sync_is_labelled_sync() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let (c, _) = subscribed(&td);
    let _guard = td
        .handle()
        .hold_long(twydo_daemon::server::lane::LongKind::Sync);
    fx.write("inbox/uid-n.ics", &todo_ics("uid-n", "New", None));
    let (_, cause, uids) = data_within(&c, Duration::from_secs(3)).expect("no data.changed");
    assert_eq!(cause, Cause::Sync);
    assert_eq!(uids, ["uid-n"]);
}

#[test]
fn own_write_publishes_one_revision() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let w = td.client();
    let (c, _) = subscribed(&td);
    let r = common::submit(&w, common::edit_title("uid-a", "Alpha 2")).unwrap();
    let (rev, cause, _) = data_within(&c, Duration::from_secs(2)).expect("no data.changed");
    assert_eq!((rev, cause), (r.revision, Cause::Op));
    // The watcher sees the daemon's own file change after its debounce and publishes nothing.
    assert!(
        data_within(&c, Duration::from_secs(1)).is_none(),
        "a second data.changed for one write"
    );
}
