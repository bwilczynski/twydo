//! Admission of start times (`set-start.time`, `complete.start-time`) on the start-time round's
//! phone captures (`fixtures/set-start/phone-encodings`).

mod common;

use std::fs;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use twydo_core::compat::{ContractId, Matrix, Verdict};
use twydo_core::config::WriteMode;
use twydo_core::edit::{Change, Due, NewTask, Op};
use twydo_core::gate::check;

const BATCH1: &str = "set-start/phone-encodings/phone-a-batch1/list1";
const BATCH2: &str = "set-start/phone-encodings/phone-a-batch2/list1";
const S1: &str = "0f8195e0cd3947e4809d1b5d5015525a";
const S2: &str = "d9ae60e7b31b4ff8b4d4249b5499cb59";
const S3: &str = "0907b521c5fd473193319a18076365c2";
const S7: &str = "b42a79b674184d66bded898223236ed1";

const SET_TIME: &str = "setting a start time is not verified yet";
const COMPLETE_TIME: &str = "completing a repeating task with a start time is not verified yet";

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 21).unwrap()
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-21T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn passing() -> Matrix {
    let mut m = Matrix::embedded().clone();
    common::set_contract(&mut m, ContractId::SetStartTime, Verdict::Pass);
    common::set_contract(&mut m, ContractId::CompleteStartTime, Verdict::Pass);
    m
}

fn timed(s: &str) -> Option<Due> {
    Some(Due::Time(
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap(),
    ))
}

fn start(uid: &str, to: Option<Due>) -> Op {
    Op::Edit {
        uid: uid.to_string(),
        changes: vec![Change::Start(to)],
    }
}

fn reason(r: Result<(), twydo_core::gate::Blocked>) -> String {
    r.expect_err("refused").to_string()
}

#[test]
fn set_start_time_needs_its_contract() {
    let env = common::Env::new(WriteMode::Verified, BATCH2);
    let s = env.snap();
    let shipping = Matrix::embedded();
    let go = |m: &Matrix, op: &Op| check(&s, m, WriteMode::Verified, op, today());

    // A new time on a task without a start (S3), and a changed time (S1).
    for op in [
        start(S3, timed("2026-11-25 10:00")),
        start(S1, timed("2026-11-25 16:00")),
    ] {
        assert!(reason(go(shipping, &op)).starts_with(SET_TIME), "{op:?}");
        assert!(go(&passing(), &op).is_ok(), "{op:?}");
    }
    // Dropping the time or removing a timed start rewrites a timed value too.
    let day = NaiveDate::from_ymd_opt(2026, 11, 25).unwrap();
    for op in [start(S1, Some(Due::Date(day))), start(S1, None)] {
        assert!(reason(go(shipping, &op)).starts_with(SET_TIME), "{op:?}");
        assert!(go(&passing(), &op).is_ok(), "{op:?}");
    }
    // A date-only start on a date-only task is admitted as before.
    assert!(go(shipping, &start(S2, Some(Due::Date(day)))).is_ok());

    let mut create = NewTask::plain(&s.tasks[s.find(S1).unwrap()].collection, "S new start");
    create.start = timed("2026-11-25 10:00");
    let op = Op::Create(create);
    assert!(reason(go(shipping, &op)).starts_with(SET_TIME));
    assert!(go(&passing(), &op).is_ok());
}

/// Review focus 4: undo restores the exact bytes of a timed-start edit.
#[test]
fn undo_set_start_time_restores_bytes() {
    let env = common::Env::new(WriteMode::Verified, BATCH2);
    let mut store = env.store(passing());
    let before = env.files();
    let s = env.snap();
    store
        .apply(&s, &start(S3, timed("2026-11-25 10:00")), now())
        .unwrap();
    let after = env.files();
    assert_ne!(after, before);
    let later = now() + chrono::Duration::minutes(1);
    store.undo_last(later).unwrap();
    assert_eq!(env.files(), before);
    store.redo_last(later).unwrap();
    assert_eq!(env.files(), after);
}

/// Completion-anchored repeats were captured with date-only starts only, and
/// their later date edits require one. A start time there is refused up front instead of
/// leaving a task whose start and due can no longer be edited.
#[test]
fn start_time_on_a_from_completion_repeat_is_refused() {
    let env = common::Env::new(
        WriteMode::Verified,
        "set-due/from-completion/seeded-after-upload/list1",
    );
    let s = env.snap();
    let uid = s
        .tasks
        .iter()
        .find(|t| t.from_completion && t.due.is_some() && t.parent.is_none())
        .map(|t| t.uid.clone())
        .expect("a from-completion task");
    let refused = reason(check(
        &s,
        &passing(),
        WriteMode::Verified,
        &start(&uid, timed("2026-11-25 10:00")),
        today(),
    ));
    assert!(refused.starts_with("from-completion edits"), "{refused}");
}

#[test]
fn complete_timed_start_needs_its_contract() {
    let env = common::Env::new(WriteMode::Verified, BATCH1);
    let s = env.snap();
    let op = Op::Complete { uid: S7.into() };
    assert_eq!(
        reason(check(
            &s,
            Matrix::embedded(),
            WriteMode::Verified,
            &op,
            today()
        )),
        COMPLETE_TIME
    );
    assert!(check(&s, &passing(), WriteMode::Verified, &op, today()).is_ok());
}

#[test]
fn non_zero_start_day_delay_stays_refused() {
    let env = common::Env::new(WriteMode::Verified, BATCH1);
    let p = env.file(S7);
    let text = fs::read_to_string(&p).unwrap();
    let changed = text.replace("StartDayDelay%22%3A0", "StartDayDelay%22%3A1");
    assert_ne!(text, changed);
    fs::write(&p, changed).unwrap();
    let s = env.snap();
    assert_eq!(
        reason(check(
            &s,
            &passing(),
            WriteMode::Verified,
            &Op::Complete { uid: S7.into() },
            today()
        )),
        "completing a repeating task with a start-day delay is not verified yet"
    );
}
