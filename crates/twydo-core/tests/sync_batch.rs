//! The pure batching rule.

use std::time::Duration as Std;

use chrono::{DateTime, Duration, TimeZone, Utc};
use twydo_core::sync::batch::{Due, QUIET, decide};

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
fn fifth_edit_is_due_after_a_quiet_period() {
    // Five edits within 5 s, the newest a second ago: due 3 s after the newest.
    let e: Vec<_> = (0..5).map(|i| ago(5 - i)).collect();
    assert_eq!(d(&e, None, None), Due::At(ago(1) + QUIET));
    // The newest four seconds ago: the pause is over.
    let e: Vec<_> = (0..5).map(|i| ago(8 - i)).collect();
    assert_eq!(d(&e, None, None), Due::Now);
}

#[test]
fn burst_keeps_pushing_the_size_trigger_until_quiet() {
    let mut e: Vec<_> = (0..5).map(|i| ago(20 - i)).collect();
    for newest in [2, 1, 0] {
        e.push(ago(newest));
        assert_eq!(d(&e, None, None), Due::At(ago(newest) + QUIET));
    }
    // Never later than max_wait after the oldest edit.
    let mut long: Vec<_> = vec![ago(59)];
    long.extend((0..5).map(|_| now()));
    assert_eq!(
        d(&long, None, None),
        Due::At(ago(59) + Duration::seconds(60))
    );
    assert_eq!(
        d(&[ago(70), ago(1), ago(1), ago(1), now()], None, None),
        Due::Now
    );
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
