//! The reminder notifier thread: delivery at the trigger, retry, restart, catch-up, clock
//! moves, live configuration, the configured command and shutdown. The clock is a
//! [`FakeClock`] the tests set (2030, UTC, the digest at 23:00 so it never fires); the sink
//! is a [`RecordingSink`] or a harmless script. Synthetic fixture content.

mod common;

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use common::{FakeClock, Fixture, RecordingSink, TestDaemon, empty, snapshot, submit, task};
use twydo_core::config::Config;
use twydo_core::edit::{Due, NewTask, Op};
use twydo_daemon::client::Client;
use twydo_daemon::protocol::events::Event;
use twydo_daemon::protocol::jobs::JobStarted;
use twydo_daemon::protocol::methods::{StatusView, WriteOutcome};
use twydo_daemon::protocol::sync::{SyncReason, SyncRunParams};
use twydo_daemon::server::DaemonOptions;
use twydo_daemon::server::lane::LongKind;

const BOUND: Duration = Duration::from_secs(10);
/// Nothing more happens within this long after a poke: nothing more is coming.
const QUIET: Duration = Duration::from_millis(300);

/// 04:00 UTC; the fixture task is due at 05:00.
fn t(h: u32, m: u32, s: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2030, 1, 10, h, m, s).unwrap()
}

const DUE: &str = "20300110T050000";
const KEY: &str = "t:u1@2030-01-10T05:00:00Z";

fn fx() -> Fixture {
    let fx = Fixture::new(&[]);
    fx.write_reminder("inbox/u1.ics", "u1", "Ping", DUE);
    fx
}

fn config(fx: &Fixture) -> Config {
    let mut c = fx.config();
    c.timezone = Some("UTC".into());
    c.morning_time = chrono::NaiveTime::from_hms_opt(23, 0, 0).unwrap();
    c
}

fn start_cfg(
    fx: &Fixture,
    cfg: Config,
    clock: &Arc<FakeClock>,
    sink: Option<&Arc<RecordingSink>>,
    f: impl FnOnce(&mut DaemonOptions),
) -> TestDaemon {
    let clock = Arc::clone(clock);
    let sink = sink.map(|s| Arc::clone(s) as Arc<dyn twydo_daemon::server::notifier::Sink>);
    TestDaemon::start_cfg(fx, cfg, move |o| {
        o.clock = clock;
        o.notify = sink;
        f(o);
    })
}

fn start(fx: &Fixture, clock: &Arc<FakeClock>, sink: &Arc<RecordingSink>) -> TestDaemon {
    start_cfg(fx, config(fx), clock, Some(sink), |_| {})
}

/// Moves the clock and wakes the notifier.
fn move_to(td: &TestDaemon, clock: &FakeClock, at: DateTime<Utc>) {
    clock.set(at);
    td.handle().poke_notifier();
}

fn count(sink: &RecordingSink) -> usize {
    sink.got.lock().unwrap().len()
}

fn until(what: &str, ok: impl Fn() -> bool) {
    let deadline = Instant::now() + BOUND;
    while !ok() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Waits until the sent-state file names `key`. A delivery is recorded after the sink has
/// it (a crash in between may repeat one, never lose one), so the file can lag the sink.
fn recorded(fx: &Fixture, key: &str) {
    until("the delivery is never recorded", || {
        fs::read_to_string(state_file(fx)).is_ok_and(|s| s.contains(key))
    });
}

fn status(c: &Client) -> StatusView {
    c.call("status.get", empty()).unwrap()
}

fn status_until(c: &Client, what: &str, ok: impl Fn(&StatusView) -> bool) -> StatusView {
    let deadline = Instant::now() + BOUND;
    loop {
        let s = status(c);
        if ok(&s) {
            return s;
        }
        assert!(Instant::now() < deadline, "{what}: {s:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn state_file(fx: &Fixture) -> PathBuf {
    fx.state().join("notifier.json")
}

#[test]
fn delivers_when_the_clock_reaches_the_trigger() {
    let fx = fx();
    let clock = FakeClock::at(t(4, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let td = start(&fx, &clock, &sink);
    td.handle().poke_notifier();
    std::thread::sleep(QUIET);
    assert_eq!(count(&sink), 0, "not yet");
    move_to(&td, &clock, t(5, 0, 0));
    until("never delivered", || count(&sink) == 1);
    {
        let got = sink.got.lock().unwrap();
        assert_eq!(got[0].key, KEY);
        assert_eq!(got[0].title, "Ping");
    }
    recorded(&fx, KEY);
    move_to(&td, &clock, t(5, 1, 0));
    std::thread::sleep(QUIET);
    assert_eq!(count(&sink), 1, "once");
    assert_eq!(status(&td.client()).notifier_error, None);
}

#[test]
fn default_options_deliver_nothing() {
    let fx = fx();
    let clock = FakeClock::at(t(4, 0, 0));
    let td = start_cfg(&fx, config(&fx), &clock, None, |_| {});
    move_to(&td, &clock, t(5, 0, 0));
    std::thread::sleep(QUIET);
    assert_eq!(status(&td.client()).notifier_error, None);
    assert!(!state_file(&fx).exists());
}

#[test]
fn restart_does_not_resend() {
    let fx = fx();
    let clock = FakeClock::at(t(5, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let td = start(&fx, &clock, &sink);
    until("never delivered", || count(&sink) == 1);
    td.handle().shutdown();
    drop(td);
    let again = Arc::new(RecordingSink::default());
    let td = start(&fx, &clock, &again);
    td.handle().poke_notifier();
    std::thread::sleep(QUIET);
    assert_eq!(count(&again), 0);
}

#[test]
fn failed_delivery_is_retried_and_reported() {
    let fx = fx();
    let clock = FakeClock::at(t(4, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    sink.fail.store(true, Ordering::SeqCst);
    let td = start(&fx, &clock, &sink);
    let c = td.client();
    move_to(&td, &clock, t(5, 0, 0));
    let s = status_until(&c, "no delivery error", |s| s.notifier_error.is_some());
    let e = s.notifier_error.unwrap();
    assert!(
        e.contains("cannot deliver reminders") && e.contains("boom"),
        "{e}"
    );
    assert!(!e.contains("Ping"), "the status never names a title: {e}");
    assert_eq!(count(&sink), 0);
    assert!(!state_file(&fx).exists(), "nothing recorded");
    sink.fail.store(false, Ordering::SeqCst);
    move_to(&td, &clock, t(5, 0, 5));
    until("never retried", || count(&sink) == 1);
    status_until(&c, "error never cleared", |s| s.notifier_error.is_none());
    recorded(&fx, KEY);
}

#[test]
fn unsaved_state_does_not_redeliver() {
    let fx = fx();
    let clock = FakeClock::at(t(4, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let td = start(&fx, &clock, &sink);
    let c = td.client();
    fs::create_dir(state_file(&fx)).unwrap();
    move_to(&td, &clock, t(5, 0, 0));
    until("never delivered", || count(&sink) == 1);
    let s = status_until(&c, "no save error", |s| s.notifier_error.is_some());
    let e = s.notifier_error.unwrap();
    assert!(e.contains("cannot save reminder state"), "{e}");
    for m in 1..=3 {
        move_to(&td, &clock, t(5, m, 0));
        std::thread::sleep(QUIET);
    }
    assert_eq!(count(&sink), 1, "the key stays in memory");
}

#[test]
fn sleep_sized_jump_catches_up() {
    let fx = fx();
    let clock = FakeClock::at(t(4, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let td = start(&fx, &clock, &sink);
    move_to(&td, &clock, t(8, 0, 0));
    until("never delivered", || count(&sink) == 1);
    move_to(&td, &clock, t(8, 1, 0));
    std::thread::sleep(QUIET);
    assert_eq!(count(&sink), 1);
}

#[test]
fn clock_rollback_replays_nothing() {
    let fx = fx();
    let clock = FakeClock::at(t(5, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let td = start(&fx, &clock, &sink);
    until("never delivered", || count(&sink) == 1);
    move_to(&td, &clock, t(4, 0, 0));
    std::thread::sleep(QUIET);
    move_to(&td, &clock, t(5, 0, 0));
    std::thread::sleep(QUIET);
    assert_eq!(count(&sink), 1);
}

#[test]
fn a_task_written_while_waiting_is_picked_up() {
    let fx = Fixture::new(&[("inbox", "u0", "Other", None)]);
    let clock = FakeClock::at(t(5, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let td = start(&fx, &clock, &sink);
    std::thread::sleep(QUIET);
    assert_eq!(count(&sink), 0);
    let mut new = NewTask::plain("inbox", "Fresh");
    new.due = Some(Due::Time(
        NaiveDate::from_ymd_opt(2030, 1, 10)
            .unwrap()
            .and_hms_opt(5, 0, 0)
            .unwrap(),
    ));
    let r = submit(&td.client(), Op::Create(new)).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Applied);
    until("never delivered", || count(&sink) == 1);
    assert_eq!(sink.got.lock().unwrap()[0].title, "Fresh");
}

#[test]
fn a_queued_completion_suppresses() {
    let fx = fx();
    let clock = FakeClock::at(t(4, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let td = start(&fx, &clock, &sink);
    let c = td.client();
    let job = td.handle().hold_long(LongKind::Test);
    let r = submit(&c, Op::Complete { uid: "u1".into() }).unwrap();
    assert_eq!(r.outcome, WriteOutcome::Queued);
    until("the queued completion is never shown", || {
        task(&snapshot(&c), "u1").pending
    });
    move_to(&td, &clock, t(5, 0, 0));
    std::thread::sleep(QUIET);
    assert_eq!(count(&sink), 0);
    drop(job);
    move_to(&td, &clock, t(5, 1, 0));
    std::thread::sleep(QUIET);
    assert_eq!(count(&sink), 0);
}

fn write_config(fx: &Fixture, enabled: bool) {
    fs::write(
        fx.instance().config,
        format!(
            "vdir = {:?}\nstate_dir = {:?}\n[writes]\nmode = \"test\"\n[notifications]\nenabled = {enabled}\n",
            fx.vdir(),
            fx.config().state_dir(),
        ),
    )
    .unwrap();
}

#[test]
fn disabled_in_config_delivers_nothing() {
    let fx = fx();
    let clock = FakeClock::at(t(4, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let mut cfg = config(&fx);
    cfg.notifications.enabled = false;
    let td = start_cfg(&fx, cfg, &clock, Some(&sink), |_| {});
    move_to(&td, &clock, t(5, 0, 0));
    std::thread::sleep(QUIET);
    assert_eq!(count(&sink), 0);
    assert_eq!(status(&td.client()).notifier_error, None);
}

#[test]
fn config_reload_enables() {
    let fx = fx();
    let clock = FakeClock::at(t(5, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let mut cfg = config(&fx);
    cfg.notifications.enabled = false;
    let _td = start_cfg(&fx, cfg, &clock, Some(&sink), |_| {});
    std::thread::sleep(QUIET);
    assert_eq!(count(&sink), 0);
    write_config(&fx, true);
    until("the reload never enabled delivery", || count(&sink) == 1);
}

#[test]
fn configured_command_wins() {
    let fx = fx();
    let out = fx.vdir().parent().unwrap().join("out");
    let clock = FakeClock::at(t(4, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let mut cfg = config(&fx);
    cfg.notifications.command = Some(vec![
        "sh".into(),
        "-c".into(),
        format!("printf '%s\\n' \"$@\" > '{}'", out.display()),
        "sh".into(),
    ]);
    let td = start_cfg(&fx, cfg, &clock, Some(&sink), |_| {});
    move_to(&td, &clock, t(5, 0, 0));
    until("the command never ran", || {
        fs::read_to_string(&out).is_ok_and(|s| s.contains("Ping"))
    });
    assert_eq!(count(&sink), 0);
    until("never recorded", || {
        fs::read_to_string(state_file(&fx)).is_ok_and(|s| s.contains(KEY))
    });
}

fn env_probe_command(out: &std::path::Path) -> Vec<String> {
    vec![
        "sh".into(),
        "-c".into(),
        "printf \"%s|%s\" \"$TWYDO_REMINDER_KEY\" \"${TWYDO_REMINDER_UID-unset}\" > \"$0\"".into(),
        out.display().to_string(),
    ]
}

#[test]
fn command_gets_the_reminder_key_and_uid() {
    let fx = fx();
    let out = fx.vdir().parent().unwrap().join("env-out");
    let clock = FakeClock::at(t(4, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let mut cfg = config(&fx);
    cfg.notifications.command = Some(env_probe_command(&out));
    let td = start_cfg(&fx, cfg, &clock, Some(&sink), |_| {});
    move_to(&td, &clock, t(5, 0, 0));
    until("the command never ran", || {
        fs::read_to_string(&out).is_ok_and(|s| !s.is_empty())
    });
    assert_eq!(fs::read_to_string(&out).unwrap(), format!("{KEY}|u1"));
}

#[test]
fn notify_test_passes_the_test_key_without_uid() {
    let fx = fx();
    let out = fx.vdir().parent().unwrap().join("env-out");
    let clock = FakeClock::at(t(4, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let mut cfg = config(&fx);
    cfg.notifications.command = Some(env_probe_command(&out));
    let td = start_cfg(&fx, cfg, &clock, Some(&sink), |_| {});
    notify_test(&td.client()).unwrap();
    assert_eq!(fs::read_to_string(&out).unwrap(), "test|unset");
}

#[test]
fn corrupt_state_file_starts_empty() {
    let fx = fx();
    fs::create_dir_all(fx.state()).unwrap();
    fs::write(state_file(&fx), b"{not json").unwrap();
    let clock = FakeClock::at(t(5, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let td = start(&fx, &clock, &sink);
    until("never delivered", || count(&sink) == 1);
    until("the state file was not rewritten", || {
        fs::read_to_string(state_file(&fx)).is_ok_and(|s| s.contains(KEY))
    });
    assert_eq!(status(&td.client()).notifier_error, None);
}

#[test]
fn a_command_that_echoes_its_arguments_leaks_no_title() {
    let fx = Fixture::new(&[]);
    fx.write_reminder("inbox/u1.ics", "u1", "Zebrafish", DUE);
    let clock = FakeClock::at(t(4, 0, 0));
    let mut cfg = config(&fx);
    cfg.notifications.command = Some(vec![
        "sh".into(),
        "-c".into(),
        "echo \"bad: $@\" >&2; exit 1".into(),
        "sh".into(),
    ]);
    let td = start_cfg(&fx, cfg, &clock, None, |_| {});
    move_to(&td, &clock, t(5, 0, 0));
    let s = status_until(&td.client(), "no delivery error", |s| {
        s.notifier_error.is_some()
    });
    let e = s.notifier_error.unwrap();
    assert!(
        e.contains("cannot deliver reminders") && e.contains("bad"),
        "{e}"
    );
    assert!(!e.contains("Zebrafish"), "the status leaks the title: {e}");
}

#[test]
fn hung_command_does_not_block_shutdown() {
    let fx = fx();
    let clock = FakeClock::at(t(4, 0, 0));
    let mut cfg = config(&fx);
    let marker = fx.vdir().parent().unwrap().join("started");
    cfg.notifications.command = Some(vec![
        "sh".into(),
        "-c".into(),
        format!("touch '{}'; sleep 30", marker.display()),
        "sh".into(),
    ]);
    let td = start_cfg(&fx, cfg, &clock, None, |o| {
        o.notify_timeout = Duration::from_millis(500);
    });
    move_to(&td, &clock, t(5, 0, 0));
    until("the command never started", || marker.exists());
    let t0 = Instant::now();
    td.handle().shutdown();
    assert!(
        t0.elapsed() < Duration::from_secs(5),
        "shutdown took {:?}",
        t0.elapsed()
    );
}

#[test]
fn crash_between_delivery_and_record_duplicates() {
    let fx = fx();
    let clock = FakeClock::at(t(5, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let td = start(&fx, &clock, &sink);
    until("never delivered", || count(&sink) == 1);
    td.handle().shutdown();
    drop(td);
    // As if the record never reached the disk: the delivery is repeated, by design.
    fs::remove_file(state_file(&fx)).unwrap();
    let again = Arc::new(RecordingSink::default());
    let _td = start(&fx, &clock, &again);
    until("never delivered again", || count(&again) == 1);
}

fn notify_test(c: &Client) -> Result<serde_json::Value, twydo_daemon::client::ClientError> {
    c.call("notify.test", empty())
}

fn rpc_message(e: twydo_daemon::client::ClientError) -> String {
    match e {
        twydo_daemon::client::ClientError::Rpc(e) => e.message,
        other => panic!("not a refusal: {other}"),
    }
}

#[test]
fn notify_test_delivers_through_the_sink() {
    let fx = fx();
    let clock = FakeClock::at(t(4, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let td = start(&fx, &clock, &sink);
    notify_test(&td.client()).unwrap();
    let got = sink.got.lock().unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].key, "test");
    assert_eq!(got[0].uid, None);
    assert_eq!(got[0].title, "Twydo");
    assert_eq!(got[0].body, "Notifications work.");
    assert!(!state_file(&fx).exists(), "the test records nothing");
}

#[test]
fn notify_test_works_while_disabled() {
    let fx = fx();
    let clock = FakeClock::at(t(4, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let mut cfg = config(&fx);
    cfg.notifications.enabled = false;
    let td = start_cfg(&fx, cfg, &clock, Some(&sink), |_| {});
    notify_test(&td.client()).unwrap();
    assert_eq!(count(&sink), 1);
}

#[test]
fn notify_test_without_a_sink_is_an_error() {
    let fx = fx();
    let clock = FakeClock::at(t(4, 0, 0));
    let td = start_cfg(&fx, config(&fx), &clock, None, |_| {});
    let e = notify_test(&td.client()).unwrap_err();
    assert_eq!(
        rpc_message(e),
        "notifications are not available in this daemon"
    );
}

#[test]
fn notify_test_reports_the_sink_failure() {
    let fx = fx();
    let clock = FakeClock::at(t(4, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    sink.fail.store(true, Ordering::SeqCst);
    let td = start(&fx, &clock, &sink);
    let e = notify_test(&td.client()).unwrap_err();
    assert!(rpc_message(e).contains("boom"));
    assert_eq!(count(&sink), 0);
}

#[test]
fn an_empty_digest_is_recorded_and_not_sent_later() {
    let fx = Fixture::new(&[]);
    let clock = FakeClock::at(t(9, 30, 0));
    let sink = Arc::new(RecordingSink::default());
    let mut cfg = config(&fx);
    cfg.morning_time = chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap();
    let td = start_cfg(&fx, cfg, &clock, Some(&sink), |_| {});
    td.handle().poke_notifier();
    until("the empty digest is never recorded", || {
        fs::read_to_string(state_file(&fx)).is_ok_and(|s| s.contains("d:2030-01-10"))
    });
    assert_eq!(count(&sink), 0, "an empty digest is not delivered");
    td.touch_outside(
        "inbox/u2.ics",
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\n\
          UID:u2\r\nSUMMARY:Later\r\nDUE:20300110T100000\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\n\
          TRIGGER;RELATED=END:PT0S\r\nEND:VALARM\r\nEND:VTODO\r\nEND:VCALENDAR\r\n",
    );
    move_to(&td, &clock, t(10, 0, 0));
    until("the timed reminder never came", || count(&sink) == 1);
    std::thread::sleep(QUIET);
    assert_eq!(count(&sink), 1, "only the task's own reminder");
    assert_eq!(sink.got.lock().unwrap()[0].key, "t:u2@2030-01-10T10:00:00Z");
    until("the reminder is never recorded", || {
        fs::read_to_string(state_file(&fx))
            .is_ok_and(|s| s.contains("d:2030-01-10") && s.contains("t:u2@2030-01-10T10:00:00Z"))
    });
}

// ---------------------------------------------------------------------------------------------
// The hold: nothing is delivered before the first sync after a start or a wake
// ---------------------------------------------------------------------------------------------

const OK_SYNC: &str = "echo 'Syncing twydo/c1' >&2\nexit 0";

fn root(fx: &Fixture) -> PathBuf {
    fx.vdir().parent().unwrap().to_path_buf()
}

/// A configuration with `[sync]` whose fake vdirsyncer waits for the `gate` file, then runs
/// `body`.
fn sync_cfg(fx: &Fixture, body: &str) -> Config {
    let gate = root(fx).join("gate");
    let script = format!(
        "[ \"$1\" = sync ] || exit 0\nwhile [ ! -e '{}' ]; do sleep 0.01; done\n{body}",
        gate.display()
    );
    let mut c = fx.with_sync(&script);
    c.timezone = Some("UTC".into());
    c.morning_time = chrono::NaiveTime::from_hms_opt(23, 0, 0).unwrap();
    c
}

fn open_gate(fx: &Fixture) {
    fs::write(root(fx).join("gate"), "").unwrap();
}

fn subscribed(td: &TestDaemon) -> Client {
    let c = td.client();
    let _: serde_json::Value = c.call("events.subscribe", empty()).unwrap();
    c
}

/// Starts a manual sync; its job id.
fn sync_run(c: &Client) -> String {
    let started: JobStarted = c
        .call(
            "sync.run",
            SyncRunParams {
                request_id: c.next_request_id(),
                force_delete: Vec::new(),
                discover: false,
            },
        )
        .unwrap();
    started.job_id
}

fn sync_ended(c: &Client, id: &str) {
    common::wait_event(
        c,
        BOUND,
        |e| matches!(e, Event::SyncFinished { job_id: Some(j), reason: SyncReason::Manual, .. } if j == id),
    );
}

#[test]
fn nothing_is_delivered_before_the_first_sync_finishes() {
    let fx = fx();
    let clock = FakeClock::at(t(5, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let td = start_cfg(&fx, sync_cfg(&fx, OK_SYNC), &clock, Some(&sink), |_| {});
    let c = subscribed(&td);
    let id = sync_run(&c);
    std::thread::sleep(QUIET);
    assert_eq!(count(&sink), 0, "held while the sync runs");
    assert!(!state_file(&fx).exists(), "nothing recorded either");
    open_gate(&fx);
    sync_ended(&c, &id);
    until("never delivered after the sync", || count(&sink) == 1);
}

#[test]
fn a_task_arriving_with_the_first_sync_is_in_the_digest() {
    let fx = Fixture::new(&[]);
    let clock = FakeClock::at(t(9, 30, 0));
    let sink = Arc::new(RecordingSink::default());
    let arrives = fx.vdir().join("inbox/u2.ics");
    let body = format!(
        "mkdir -p '{}'\nprintf '%s\\r\\n' BEGIN:VCALENDAR VERSION:2.0 PRODID:-//synthetic//EN \
         BEGIN:VTODO UID:u2 SUMMARY:Phone DUE\\;VALUE=DATE:20300110 END:VTODO END:VCALENDAR > '{}'\n\
         {OK_SYNC}",
        fx.vdir().join("inbox").display(),
        arrives.display()
    );
    let mut cfg = sync_cfg(&fx, &body);
    cfg.morning_time = chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap();
    let td = start_cfg(&fx, cfg, &clock, Some(&sink), |_| {});
    let c = subscribed(&td);
    let id = sync_run(&c);
    std::thread::sleep(QUIET);
    assert_eq!(count(&sink), 0);
    assert!(!state_file(&fx).exists(), "the digest is not decided yet");
    open_gate(&fx);
    sync_ended(&c, &id);
    until("the digest never came", || count(&sink) == 1);
    let got = sink.got.lock().unwrap();
    assert_eq!(got[0].key, "d:2030-01-10");
    assert_eq!(got[0].title, "1 due today");
}

#[test]
fn the_hold_gives_up_after_two_minutes() {
    let fx = fx();
    let clock = FakeClock::at(t(5, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let td = start_cfg(&fx, sync_cfg(&fx, OK_SYNC), &clock, Some(&sink), |_| {});
    let c = subscribed(&td);
    let id = sync_run(&c);
    std::thread::sleep(QUIET);
    // In steps under the wake gap: a bigger jump would restart the hold.
    for at in [t(5, 0, 50), t(5, 1, 40)] {
        move_to(&td, &clock, at);
        std::thread::sleep(QUIET);
    }
    assert_eq!(count(&sink), 0, "still held at 100 s");
    move_to(&td, &clock, t(5, 2, 1));
    until("the hold never gave up", || count(&sink) == 1);
    // Let the blocked sync end so the daemon shuts down at once.
    open_gate(&fx);
    sync_ended(&c, &id);
}

#[test]
fn a_failed_sync_releases_the_hold() {
    // A failed sync also records a backoff, which on its own ends the hold (no sync will come
    // soon); so this test covers the observable behaviour, and the unit test
    // `a_finished_sync_ends_a_hold_even_when_a_sync_will_come` covers the sync count alone.
    let fx = fx();
    let clock = FakeClock::at(t(5, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let td = start_cfg(&fx, sync_cfg(&fx, "exit 1"), &clock, Some(&sink), |_| {});
    let c = subscribed(&td);
    let id = sync_run(&c);
    std::thread::sleep(QUIET);
    assert_eq!(count(&sink), 0);
    open_gate(&fx);
    sync_ended(&c, &id);
    until("a failed sync never released the hold", || {
        count(&sink) == 1
    });
}

#[test]
fn no_hold_without_sync_config() {
    let fx = fx();
    let clock = FakeClock::at(t(5, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let _td = start(&fx, &clock, &sink);
    until("never delivered", || count(&sink) == 1);
}

#[test]
fn no_hold_when_automatic_sync_is_off() {
    let fx = fx();
    let clock = FakeClock::at(t(5, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let cfg = sync_cfg(&fx, OK_SYNC);
    twydo_core::sync::switch::set(&cfg, true).unwrap();
    let _td = start_cfg(&fx, cfg, &clock, Some(&sink), |_| {});
    until("held although automatic sync is off", || count(&sink) == 1);
}

#[test]
fn a_wake_gap_holds_until_the_next_sync() {
    let fx = fx();
    // Due at the start: delivered once the first sync has ended, so the test knows the
    // notifier has passed the first hold before it moves the clock.
    fx.write_reminder("inbox/u0.ics", "u0", "First", "20300110T040000");
    open_gate(&fx);
    let clock = FakeClock::at(t(4, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let td = start_cfg(&fx, sync_cfg(&fx, OK_SYNC), &clock, Some(&sink), |_| {});
    let c = subscribed(&td);
    let id = sync_run(&c);
    sync_ended(&c, &id);
    until("the first sync never released the hold", || {
        count(&sink) == 1
    });
    // The machine sleeps for three hours; the task fell due meanwhile.
    move_to(&td, &clock, t(7, 0, 0));
    std::thread::sleep(QUIET);
    assert_eq!(count(&sink), 1, "held after the wake");
    let id = sync_run(&c);
    sync_ended(&c, &id);
    until("never delivered after the wake sync", || count(&sink) == 2);
    assert_eq!(sink.got.lock().unwrap()[1].key, KEY);
}

#[test]
fn a_backward_clock_move_starts_no_hold() {
    let fx = fx();
    open_gate(&fx);
    let clock = FakeClock::at(t(4, 59, 30));
    let sink = Arc::new(RecordingSink::default());
    let td = start_cfg(&fx, sync_cfg(&fx, OK_SYNC), &clock, Some(&sink), |_| {});
    let c = subscribed(&td);
    let id = sync_run(&c);
    sync_ended(&c, &id);
    // Back, then forward in steps under the wake gap. A hold started by the backward move
    // would still run at 5:00:05 (it lasts 120 s from 4:58:10), so the reminder would stay back.
    for at in [t(4, 58, 10), t(4, 58, 50), t(4, 59, 40), t(5, 0, 5)] {
        move_to(&td, &clock, at);
        std::thread::sleep(QUIET);
    }
    until("a backward move held the notifier", || count(&sink) == 1);
}

// ---------------------------------------------------------------------------------------------
// One failing delivery does not block the others; a panicking pass is survived
// ---------------------------------------------------------------------------------------------

#[test]
fn a_failing_delivery_does_not_block_the_next() {
    let fx = fx();
    fx.write_reminder("inbox/u2.ics", "u2", "Pong", DUE);
    let clock = FakeClock::at(t(4, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    *sink.fail_key.lock().unwrap() = Some(KEY.to_string());
    let td = start(&fx, &clock, &sink);
    let c = td.client();
    move_to(&td, &clock, t(5, 0, 0));
    until("the other key was never delivered", || count(&sink) == 1);
    assert_eq!(sink.got.lock().unwrap()[0].key, "t:u2@2030-01-10T05:00:00Z");
    status_until(&c, "no delivery error", |s| s.notifier_error.is_some());
    // The failing key waits its backoff; a try before 5 s has passed is skipped.
    *sink.fail_key.lock().unwrap() = None;
    move_to(&td, &clock, t(5, 0, 4));
    std::thread::sleep(QUIET);
    assert_eq!(count(&sink), 1, "retried too early");
    move_to(&td, &clock, t(5, 0, 5));
    until("the failing key was never retried", || count(&sink) == 2);
    assert_eq!(sink.got.lock().unwrap()[1].key, KEY);
    status_until(&c, "error never cleared", |s| s.notifier_error.is_none());
}

#[test]
fn a_panicking_pass_is_reported_and_retried() {
    let fx = fx();
    let clock = FakeClock::at(t(5, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    sink.panic_once.store(true, Ordering::SeqCst);
    let td = start(&fx, &clock, &sink);
    let c = td.client();
    let s = status_until(&c, "the panic was never reported", |s| {
        s.notifier_error.is_some()
    });
    assert_eq!(
        s.notifier_error.as_deref(),
        Some("the reminder planner failed; see the daemon log")
    );
    assert_eq!(count(&sink), 0);
    // A poke before the backoff has passed does not retry.
    move_to(&td, &clock, t(5, 0, 2));
    std::thread::sleep(QUIET);
    assert_eq!(count(&sink), 0);
    move_to(&td, &clock, t(5, 0, 5));
    until("never delivered after the panic", || count(&sink) == 1);
    status_until(&c, "error never cleared", |s| s.notifier_error.is_none());
}

#[test]
fn disabling_clears_a_stale_save_error() {
    let fx = fx();
    let clock = FakeClock::at(t(4, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let td = start(&fx, &clock, &sink);
    let c = td.client();
    fs::create_dir(state_file(&fx)).unwrap();
    move_to(&td, &clock, t(5, 0, 0));
    status_until(&c, "no save error", |s| s.notifier_error.is_some());
    write_config(&fx, false);
    status_until(&c, "the save error stayed", |s| s.notifier_error.is_none());
}

#[test]
fn a_sleep_during_a_hold_restarts_the_hold() {
    let fx = fx();
    let clock = FakeClock::at(t(4, 0, 0));
    let sink = Arc::new(RecordingSink::default());
    let td = start_cfg(&fx, sync_cfg(&fx, OK_SYNC), &clock, Some(&sink), |_| {});
    let c = subscribed(&td);
    let id = sync_run(&c);
    std::thread::sleep(QUIET);
    // The machine sleeps for three hours while the first sync is still pending.
    move_to(&td, &clock, t(7, 0, 0));
    std::thread::sleep(QUIET);
    assert_eq!(count(&sink), 0, "the sleep must restart the hold");
    open_gate(&fx);
    sync_ended(&c, &id);
    until("never delivered after the sync", || count(&sink) == 1);
}
