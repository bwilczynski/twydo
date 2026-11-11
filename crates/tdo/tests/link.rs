//! The TUI link against an in-process daemon. Every test has its own socket; content is
//! synthetic and no real vdirsyncer runs.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{TestEnv, link_to, list_env};
use serde_json::json;
use tdo::app::{ListAction, ListCompletion, SYNC_DONE};
use tdo::term::link::{Command, Connector, Link, Update, WriteRequest, sync_message};
use tdo::write::Request;
use twydo_core::config::Config;
use twydo_core::edit::{Change, Op};
use twydo_core::lists::{ListRequest, ListResult};
use twydo_daemon::protocol::events::Cause;
use twydo_daemon::protocol::sync::{SyncOutcome, SyncResult};
use twydo_daemon::server::{Daemon, DaemonOptions};

const BOUND: Duration = Duration::from_secs(20);

fn link_for(env: &TestEnv) -> Link {
    link_to(env.socket(), env.instance())
}

/// Updates until `pick` accepts one, within [`BOUND`].
fn until<T>(link: &Link, mut pick: impl FnMut(Update) -> Option<T>) -> T {
    let end = Instant::now() + BOUND;
    loop {
        let left = end.saturating_duration_since(Instant::now());
        let Some(u) = link.next_update(left) else {
            panic!("no matching update within {BOUND:?}");
        };
        if let Some(t) = pick(u) {
            return t;
        }
    }
}

fn task_ics(uid: &str, summary: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\nUID:{uid}\r\n\
         SUMMARY:{summary}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
    )
}

#[test]
fn link_delivers_the_first_snapshot_and_status() {
    let env = TestEnv::new("");
    fs::write(
        env.vdir().join("inbox/a.ics"),
        task_ics("uid-a", "Synthetic"),
    )
    .unwrap();
    let _daemon = env.in_process_daemon(None);
    let link = link_for(&env);
    let mut seen = Vec::new();
    let (mut snapshot, mut status) = (None, None);
    until(&link, |u| {
        match u {
            Update::Connected { restarted, .. } => {
                assert!(!restarted);
                seen.push("connected");
            }
            Update::Snapshot {
                revision,
                snapshot: s,
                ..
            } => {
                seen.push("snapshot");
                snapshot = Some((revision, s.tasks.len()));
            }
            Update::Status(s) => {
                seen.push("status");
                status = Some(s);
            }
            other => panic!("unexpected {other:?}"),
        }
        (snapshot.is_some() && status.is_some()).then_some(())
    });
    assert_eq!(seen[0], "connected");
    assert_eq!(snapshot.unwrap().1, 1);
    // No [sync]: the status has no sync part.
    assert!(status.unwrap().sync.is_none());
    let based_on = link.based_on().unwrap();
    assert_eq!(based_on.revision, snapshot.unwrap().0);
    assert!(link.caller().is_some());
}

#[test]
fn an_outside_change_arrives_as_a_snapshot_with_cause_outside() {
    let env = TestEnv::new("");
    let daemon = env.in_process_daemon(None);
    let link = link_for(&env);
    until(&link, |u| matches!(u, Update::Status(_)).then_some(()));
    fs::write(
        env.vdir().join("inbox/b.ics"),
        task_ics("uid-b", "From outside"),
    )
    .unwrap();
    daemon
        .reload_for_test_with(Cause::Outside, vec!["uid-b".into()])
        .unwrap();
    let (cause, uids, tasks) = until(&link, |u| match u {
        Update::Snapshot {
            cause,
            uids,
            snapshot,
            ..
        } if cause == Cause::Outside => Some((cause, uids, snapshot.tasks.len())),
        _ => None,
    });
    assert_eq!(cause, Cause::Outside);
    assert_eq!(uids, vec!["uid-b".to_string()]);
    assert_eq!(tasks, 1);
}

#[test]
fn link_reconnects_and_reports_a_restart() {
    let env = TestEnv::new("");
    let first = env.in_process_daemon(None);
    let link = link_for(&env);
    until(&link, |u| matches!(u, Update::Status(_)).then_some(()));
    first.shutdown();
    drop(first);
    let begun = Instant::now();
    let down = until(&link, |u| match u {
        Update::Down(t) => Some(t),
        _ => None,
    });
    assert!(
        begun.elapsed() < Duration::from_secs(2),
        "{:?}",
        begun.elapsed()
    );
    assert!(down.contains("daemon not running"), "{down}");
    assert!(down.ends_with("retrying"), "{down}");
    assert!(link.caller().is_none());
    let _second = env.in_process_daemon(None);
    let restarted = until(&link, |u| match u {
        Update::Connected { restarted, .. } => Some(restarted),
        _ => None,
    });
    assert!(restarted);
    until(&link, |u| {
        matches!(u, Update::Snapshot { .. }).then_some(())
    });
    assert!(link.caller().is_some());
}

#[test]
fn missed_events_refetch_snapshot_and_status() {
    let env = TestEnv::new("");
    let cfg = Config::load(&env.config_path()).unwrap();
    let mut opts = DaemonOptions::new(env.socket());
    opts.schedule = false;
    opts.event_buffer = 1;
    let daemon = Daemon::start(cfg, env.instance(), opts).unwrap();
    assert!(daemon.wait_ready(Duration::from_secs(10)));
    let link = link_for(&env);
    until(&link, |u| matches!(u, Update::Status(_)).then_some(()));
    let mut last = 0;
    for i in 0..12 {
        let uid = format!("uid-{i}");
        fs::write(
            env.vdir().join(format!("inbox/{uid}.ics")),
            task_ics(&uid, "Burst"),
        )
        .unwrap();
        last = daemon.reload_for_test(Cause::Outside).unwrap();
    }
    let tasks = until(&link, |u| match u {
        Update::Snapshot {
            revision, snapshot, ..
        } if revision == last => Some(snapshot.tasks.len()),
        _ => None,
    });
    assert_eq!(tasks, 12);
    assert_eq!(link.based_on().unwrap().revision, last);
}

#[test]
fn a_write_through_the_link_is_answered_and_refetched() {
    let env = TestEnv::new("");
    fs::write(env.vdir().join("inbox/a.ics"), task_ics("uid-a", "Before")).unwrap();
    let _daemon = env.in_process_daemon(None);
    let link = link_for(&env);
    until(&link, |u| matches!(u, Update::Status(_)).then_some(()));
    let based_on = link.based_on().unwrap();
    link.send(Command::Write {
        seq: 7,
        write: WriteRequest {
            request: Request::Apply(Op::Edit {
                uid: "uid-a".into(),
                changes: vec![Change::Summary("After".into())],
            }),
            based_on,
        },
    });
    // The answer comes after the snapshot it refers to (cause `op`, with the uids when the
    // daemon's event came first).
    let mut snapshot = None;
    let (seq, written) = until(&link, |u| match u {
        Update::Snapshot {
            snapshot: s, cause, ..
        } => {
            snapshot = Some((s.tasks[0].summary.clone(), cause));
            None
        }
        Update::Written { seq, result } => Some((seq, result)),
        _ => None,
    });
    assert_eq!(seq, 7);
    assert!(written.is_ok(), "{written:?}");
    let (title, cause) = snapshot.expect("the snapshot came before the answer");
    assert_eq!(title, "After");
    assert_eq!(cause, Cause::Op);
}

#[test]
fn a_write_resent_after_a_reconnect_applies_once() {
    let env = TestEnv::new("");
    fs::write(env.vdir().join("inbox/a.ics"), task_ics("uid-a", "Before")).unwrap();
    let _daemon = env.in_process_daemon(None);
    let (proxy, submitted) =
        common::lossy_proxy(env.socket(), env.sock_dir.path(), common::Lose::First);
    let link = link_to(proxy, env.instance());
    until(&link, |u| matches!(u, Update::Status(_)).then_some(()));
    let based_on = link.based_on().unwrap();
    link.send(Command::Write {
        seq: 1,
        write: WriteRequest {
            request: Request::Apply(Op::Edit {
                uid: "uid-a".into(),
                changes: vec![Change::Summary("After".into())],
            }),
            based_on,
        },
    });
    // The first answer is lost with the connection; the link reconnects and asks again.
    let (seq, result) = until(&link, |u| match u {
        Update::Written { seq, result } => Some((seq, result)),
        _ => None,
    });
    assert_eq!(seq, 1);
    assert!(result.is_ok(), "{result:?}");
    // Exactly one reply, however long we wait.
    while let Some(u) = link.next_update(Duration::from_millis(500)) {
        assert!(
            !matches!(u, Update::Written { .. }),
            "a second reply: {u:?}"
        );
    }
    let ids = submitted.lock().unwrap().clone();
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert_eq!(ids[0], ids[1], "the resend must reuse the request id");
    // Applied once: one journal entry for it, and the task changed.
    let c = env.connect(&env.socket());
    let entries: Vec<twydo_daemon::protocol::admin::JournalEntryView> =
        c.call("journal.list", json!({})).unwrap();
    let ours: Vec<_> = entries
        .iter()
        .filter(|e| e.request_id.as_deref() == Some(ids[0].as_str()))
        .collect();
    assert_eq!(ours.len(), 1, "{entries:?}");
    assert_eq!(entries.len(), 1, "{entries:?}");
    let text = fs::read_to_string(env.vdir().join("inbox/a.ics")).unwrap();
    assert!(text.contains("SUMMARY:After"), "{text}");
}

/// A proxy to `target` that answers the first `op.submit` itself with `unavailable` (the
/// daemon's "still being applied" refusal) without passing it on; it records the request id
/// of every `op.submit` it sees.
fn busy_proxy(
    target: PathBuf,
    dir: &Path,
) -> (PathBuf, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    let path = dir.join("busy.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let refused = Arc::new(AtomicBool::new(false));
    let record = Arc::clone(&seen);
    std::thread::spawn(move || {
        for client in listener.incoming() {
            let Ok(client) = client else { return };
            let Ok(daemon) = UnixStream::connect(&target) else {
                continue;
            };
            let (mut to_daemon, from_client) =
                (daemon.try_clone().unwrap(), client.try_clone().unwrap());
            let mut to_client_early = client.try_clone().unwrap();
            let record = Arc::clone(&record);
            let refused = Arc::clone(&refused);
            std::thread::spawn(move || {
                for line in BufReader::new(from_client).lines() {
                    let Ok(line) = line else { break };
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line)
                        && v["method"] == "op.submit"
                    {
                        record
                            .lock()
                            .unwrap()
                            .push(v["params"]["request_id"].as_str().unwrap().to_string());
                        if !refused.swap(true, Ordering::SeqCst) {
                            let reply = json!({
                                "jsonrpc": "2.0",
                                "id": v["id"],
                                "error": {
                                    "code": -32000,
                                    "message": "the write is still being applied; try again",
                                    "data": {"kind": "unavailable"}
                                }
                            });
                            if writeln!(to_client_early, "{reply}").is_err() {
                                break;
                            }
                            continue;
                        }
                    }
                    if writeln!(to_daemon, "{line}").is_err() {
                        break;
                    }
                }
            });
            std::thread::spawn(move || {
                let mut to_client = client;
                for line in BufReader::new(daemon).lines() {
                    let Ok(line) = line else { break };
                    if writeln!(to_client, "{line}").is_err() {
                        break;
                    }
                }
            });
        }
    });
    (path, seen)
}

#[test]
fn a_mid_commit_refusal_is_retried_not_reported() {
    let env = TestEnv::new("");
    fs::write(env.vdir().join("inbox/a.ics"), task_ics("uid-a", "Before")).unwrap();
    let _daemon = env.in_process_daemon(None);
    let (proxy, submitted) = busy_proxy(env.socket(), env.sock_dir.path());
    let link = link_to(proxy, env.instance());
    until(&link, |u| matches!(u, Update::Status(_)).then_some(()));
    let based_on = link.based_on().unwrap();
    link.send(Command::Write {
        seq: 1,
        write: WriteRequest {
            request: Request::Apply(Op::Edit {
                uid: "uid-a".into(),
                changes: vec![Change::Summary("After".into())],
            }),
            based_on,
        },
    });
    let (seq, result) = until(&link, |u| match u {
        Update::Written { seq, result } => Some((seq, result)),
        _ => None,
    });
    assert_eq!(seq, 1);
    assert!(result.is_ok(), "{result:?}");
    while let Some(u) = link.next_update(Duration::from_millis(500)) {
        assert!(
            !matches!(u, Update::Written { .. }),
            "a second reply: {u:?}"
        );
    }
    let ids = submitted.lock().unwrap().clone();
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert_eq!(ids[0], ids[1], "the resend must reuse the request id");
    let c = env.connect(&env.socket());
    let entries: Vec<twydo_daemon::protocol::admin::JournalEntryView> =
        c.call("journal.list", json!({})).unwrap();
    assert_eq!(entries.len(), 1, "{entries:?}");
    let text = fs::read_to_string(env.vdir().join("inbox/a.ics")).unwrap();
    assert!(text.contains("SUMMARY:After"), "{text}");
}

#[test]
fn sync_message_matches_tdo_sync_wording() {
    let r = |outcome, problems: &[&str], warnings: &[&str]| SyncResult {
        outcome,
        problems: problems.iter().map(|s| s.to_string()).collect(),
        warnings: warnings.iter().map(|s| s.to_string()).collect(),
        notes: Vec::new(),
        new_conflicts: 0,
        drained: 0,
    };
    assert_eq!(
        sync_message(&r(SyncOutcome::Failed, &["one", "two"], &["careful"])),
        "one · two · warning: careful"
    );
    assert_eq!(sync_message(&r(SyncOutcome::Ok, &[], &[])), SYNC_DONE);
    assert_eq!(
        sync_message(&r(SyncOutcome::Failed, &[], &[])),
        "sync failed"
    );
    assert_eq!(
        sync_message(&r(SyncOutcome::Stopped, &[], &[])),
        "sync stopped"
    );
    assert_eq!(
        sync_message(&r(SyncOutcome::Unknown, &[], &[])),
        "sync stopped"
    );
    assert_eq!(sync_message(&r(SyncOutcome::Ok, &[], &["w"])), "warning: w");
}

#[test]
fn a_sync_through_the_link_reports_its_message() {
    let env = TestEnv::with_sync("exit 0");
    let _daemon = env.in_process_daemon(None);
    // The daemon's startup discovery marker would make this first sync discover (and this
    // environment has no list helper): the message under test is the plain sync's.
    std::fs::remove_file(env.state().join("sync/discover")).unwrap();
    let link = link_for(&env);
    until(&link, |u| matches!(u, Update::Status(_)).then_some(()));
    link.send(Command::Sync {
        force_delete: Vec::new(),
    });
    let done = until(&link, |u| match u {
        Update::SyncDone(m) => Some(m),
        _ => None,
    });
    assert_eq!(done, SYNC_DONE);
}

#[test]
fn a_list_save_through_the_link_reports_its_result() {
    let e = list_env();
    let _ = &e.daemon;
    let link = link_to(e.socket.clone(), e.instance.clone());
    until(&link, |u| matches!(u, Update::Status(_)).then_some(()));
    let action = ListAction::Save(ListRequest::Create {
        id: "c3".into(),
        name: "Synthetic three".into(),
        color: (0x37, 0x94, 0x17),
    });
    link.send(Command::List(action.clone()));
    let done: ListCompletion = until(&link, |u| match u {
        Update::ListDone(c) => Some(c),
        _ => None,
    });
    assert_eq!(done.action, action);
    match done.result {
        Ok(ListResult::Saved { state, .. }) => {
            assert_eq!(state.id, "c3");
            assert_eq!(state.name, "Synthetic three");
            assert_eq!(state.color, Some((0x37, 0x94, 0x17)));
        }
        other => panic!("expected a saved list: {other:?}"),
    }
    assert_eq!(done.pending, None);
}

#[test]
fn a_failed_list_job_reports_its_message() {
    let e = list_env();
    let link = link_to(e.socket.clone(), e.instance.clone());
    until(&link, |u| matches!(u, Update::Status(_)).then_some(()));
    // `c1` exists already: creating it is refused.
    link.send(Command::List(ListAction::Save(ListRequest::Create {
        id: "c1".into(),
        name: "Again".into(),
        color: (1, 2, 3),
    })));
    let done = until(&link, |u| match u {
        Update::ListDone(c) => Some(c),
        _ => None,
    });
    assert!(done.result.is_err(), "{done:?}");
}

#[test]
fn forget_through_the_link_reports_the_backup() {
    let e = list_env();
    let link = link_to(e.socket.clone(), e.instance.clone());
    until(&link, |u| matches!(u, Update::Status(_)).then_some(()));
    link.send(Command::Forget {
        name: "c1".into(),
        collection: "c1".into(),
    });
    let message = until(&link, |u| match u {
        Update::Forgot(m) => Some(m),
        _ => None,
    });
    assert!(message.starts_with("preserved "), "{message}");
    assert!(message.contains("recover with :recover"), "{message}");
}

#[test]
fn a_final_connect_error_stops_the_link_without_retrying() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let tries = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&tries);
    let connect: Connector = Box::new(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
        Err(tdo::client::ConnectError::TooOld {
            client: "1".into(),
            daemon: "2".into(),
        })
    });
    let link = Link::start_with(connect);
    let Some(Update::Down(m)) = link.next_update(Duration::from_secs(5)) else {
        panic!("no Down update");
    };
    assert_eq!(m, "tdo was upgraded; restart it");
    // A write is refused with that message, and nothing connects again.
    link.send(Command::Write {
        seq: 1,
        write: WriteRequest {
            request: Request::Undo,
            based_on: twydo_daemon::protocol::methods::BasedOn {
                epoch: "e".into(),
                revision: 0,
            },
        },
    });
    let Some(Update::Written { result, .. }) = link.next_update(Duration::from_secs(5)) else {
        panic!("no answer");
    };
    assert_eq!(result.unwrap_err(), m);
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(tries.load(Ordering::SeqCst), 1);
    assert!(link.next_update(Duration::from_millis(10)).is_none());
}

#[test]
fn a_daemon_that_never_answers_hello_shows_down_and_retries() {
    let env = TestEnv::new("");
    let listener = std::os::unix::net::UnixListener::bind(env.socket()).unwrap();
    let held = std::thread::spawn(move || {
        let mut keep = Vec::new();
        while let Ok((s, _)) = listener.accept() {
            keep.push(s);
        }
    });
    let link = link_for(&env);
    let begun = Instant::now();
    let Some(Update::Down(m)) = link.next_update(Duration::from_secs(10)) else {
        panic!("no Down update within 10 s");
    };
    assert!(m.contains("did not answer"), "{m}");
    assert!(m.ends_with("retrying"), "{m}");
    assert!(begun.elapsed() < Duration::from_secs(10));
    // It tries again (a second connect is another hello the fake never answers), and the
    // thread is not stuck: it takes Quit.
    link.send(Command::Quit);
    // Nothing answers there for the teardown to stop.
    let _ = fs::remove_file(env.socket());
    drop(held);
}
