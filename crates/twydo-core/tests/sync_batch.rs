//! The pure batching rule.

use std::time::Duration as Std;

use chrono::{DateTime, Duration, TimeZone, Utc};
use twydo_core::sync::batch::{Due, decide};

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 1, 10, 0, 0).unwrap()
}

fn ago(s: i64) -> DateTime<Utc> {
    now() - Duration::seconds(s)
}

fn d(edits: &[DateTime<Utc>], last: Option<DateTime<Utc>>, next: Option<DateTime<Utc>>) -> Due {
    decide(edits, last, next, 5, Std::from_secs(60), now())
}

#[test]
fn no_edits_is_nothing() {
    assert_eq!(d(&[], None, None), Due::Nothing);
}

#[test]
fn one_edit_waits_until_oldest_plus_max_wait() {
    assert_eq!(
        d(&[ago(10)], None, None),
        Due::At(now() + Duration::seconds(50))
    );
}

#[test]
fn fifth_edit_is_due_now() {
    let e: Vec<_> = (0..5).map(|i| ago(5 - i)).collect();
    assert_eq!(d(&e, None, None), Due::Now);
}

#[test]
fn past_deadline_is_due_now() {
    assert_eq!(d(&[ago(7200)], None, None), Due::Now);
}

#[test]
fn edits_before_the_last_attempt_do_not_count() {
    let mut e = vec![ago(3600); 10];
    let last = Some(ago(1800));
    assert_eq!(d(&e, last, None), Due::Nothing);
    e.push(ago(5));
    assert_eq!(d(&e, last, None), Due::At(now() + Duration::seconds(55)));
}

#[test]
fn backoff_pushes_the_deadline() {
    let next = Some(now() + Duration::minutes(10));
    let want = Due::At(now() + Duration::minutes(10));
    assert_eq!(d(&[ago(120)], None, next), want);
    let six = vec![ago(120); 6];
    assert_eq!(d(&six, None, next), want);
}
