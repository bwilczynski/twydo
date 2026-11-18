//! The daemon's sync log lines: what is logged, and when it is logged again.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use twydo_core::sync::run::{HeldEmptied, HeldReason, RunReport, Skip};
use twydo_core::sync::status::{CollState, SyncStatus};
use twydo_daemon::server::synclog::SyncLog;

fn now() -> DateTime<Utc> {
    "2026-11-18T10:00:00Z".parse().unwrap()
}

fn ok_status(lists: usize) -> SyncStatus {
    SyncStatus {
        last_success: Some(now()),
        collections: (0..lists)
            .map(|i| (format!("c{i}"), CollState::Ok))
            .collect(),
        ..SyncStatus::default()
    }
}

fn failing(error: &str, failures: u32) -> SyncStatus {
    SyncStatus {
        last_error: Some(error.into()),
        failures,
        next_attempt: Some(now() + Duration::seconds(120)),
        ..SyncStatus::default()
    }
}

#[test]
fn first_success_logs_list_count() {
    let mut log = SyncLog::default();
    let s = ok_status(3);
    assert_eq!(log.lines(None, None, &s, now()), ["sync ok (3 lists)"]);
    assert!(log.lines(None, None, &s, now()).is_empty());
}

#[test]
fn failure_logs_once_per_error_text() {
    let mut log = SyncLog::default();
    assert_eq!(
        log.lines(None, None, &failing("boom", 2), now()),
        ["sync failed (attempt 2, next in 120s): boom"]
    );
    assert!(log.lines(None, None, &failing("boom", 3), now()).is_empty());
    assert_eq!(
        log.lines(None, None, &failing("other", 4), now()),
        ["sync failed (attempt 4, next in 120s): other"]
    );
    let mut s = failing("x", 2);
    s.next_attempt = None;
    assert_eq!(
        SyncLog::default().lines(None, None, &s, now()),
        ["sync failed (attempt 2): x"]
    );
}

#[test]
fn recovery_logs_failure_count() {
    let mut log = SyncLog::default();
    log.lines(None, None, &failing("boom", 4), now());
    assert_eq!(
        log.lines(None, None, &ok_status(1), now()),
        ["sync ok again after 4 failures"]
    );
    assert!(log.lines(None, None, &ok_status(1), now()).is_empty());
}

#[test]
fn fresh_logger_reports_current_failure() {
    let mut log = SyncLog::default();
    assert_eq!(log.lines(None, None, &failing("boom", 1), now()).len(), 1);
}

#[test]
fn discovery_error_logged_on_change() {
    let mut log = SyncLog::default();
    let s = SyncStatus {
        discovery_error: Some("x".into()),
        ..SyncStatus::default()
    };
    assert_eq!(
        log.lines(None, None, &s, now()),
        ["list discovery failed: x"]
    );
    assert!(log.lines(None, None, &s, now()).is_empty());
}

#[test]
fn applied_emptyings_logged_every_time() {
    let mut log = SyncLog::default();
    let report = RunReport {
        auto_forced: vec!["a".into(), "b".into()],
        auto_forced_remote: vec!["a".into()],
        ..RunReport::default()
    };
    let s = SyncStatus::default();
    let want = [
        "applied list a emptied on the server",
        "applied list b emptied here on the server",
    ];
    assert_eq!(log.lines(Some(&report), None, &s, now()), want);
    assert_eq!(log.lines(Some(&report), None, &s, now()), want);
}

#[test]
fn held_emptying_logged_when_new() {
    let mut log = SyncLog::default();
    let s = SyncStatus::default();
    let held = |remote, reason| {
        let mut m = BTreeMap::new();
        m.insert("a".to_string(), HeldEmptied { remote, reason });
        RunReport {
            held_emptied: m,
            ..RunReport::default()
        }
    };
    let remote = held(true, HeldReason::QueuedEdits);
    assert_eq!(
        log.lines(Some(&remote), None, &s, now()),
        ["list a emptied on the server; not applied (edits here not yet synced)"]
    );
    assert!(log.lines(Some(&remote), None, &s, now()).is_empty());
    assert!(
        log.lines(Some(&RunReport::default()), None, &s, now())
            .is_empty()
    );
    assert_eq!(log.lines(Some(&remote), None, &s, now()).len(), 1);
    let local = held(false, HeldReason::NotEmptiedByTwydo);
    assert_eq!(
        SyncLog::default().lines(Some(&local), None, &s, now()),
        ["list a emptied here; not applied (not emptied by Twydo)"]
    );
}

#[test]
fn skipped_run_logs_nothing() {
    let mut log = SyncLog::default();
    let report = RunReport {
        skipped: Some(Skip::Busy),
        auto_forced: vec!["a".into()],
        ..RunReport::default()
    };
    assert!(
        log.lines(Some(&report), None, &failing("boom", 1), now())
            .is_empty()
    );
    // State unchanged: the failure is still new.
    assert_eq!(log.lines(None, None, &failing("boom", 1), now()).len(), 1);
}

#[test]
fn failed_run_with_stale_ok_status_logs_failure_not_ok() {
    let mut log = SyncLog::default();
    let stale = ok_status(2);
    let lines = log.lines(None, Some("journal is locked"), &stale, now());
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with("sync failed"), "{lines:?}");
    assert!(lines[0].contains("journal is locked"), "{lines:?}");
    assert!(!lines[0].contains("sync ok"));
    // The same text again is quiet; a later success logs the recovery.
    assert!(
        log.lines(None, Some("journal is locked"), &stale, now())
            .is_empty()
    );
    let again = log.lines(None, None, &stale, now());
    assert_eq!(again.len(), 1);
    assert!(again[0].starts_with("sync ok again after"), "{again:?}");
}

#[test]
fn report_error_is_logged_as_failure() {
    let mut log = SyncLog::default();
    let report = RunReport {
        error: Some("vdirsyncer missing".into()),
        ..RunReport::default()
    };
    let lines = log.lines(Some(&report), None, &ok_status(1), now());
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with("sync failed"), "{lines:?}");
    assert!(lines[0].contains("vdirsyncer missing"));
    assert!(
        log.lines(Some(&report), None, &ok_status(1), now())
            .is_empty()
    );
}
