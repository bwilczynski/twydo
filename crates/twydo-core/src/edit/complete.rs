//! Completion and reopen, as the 2Do iPhone app writes them.
//!
//! - One-time complete (H3, `plan4-list1/03-complete` A): `STATUS:COMPLETED`,
//!   `PERCENT-COMPLETE:100` and `COMPLETED:<UTC>` after `STATUS`.
//! - Reopen (H4 as captured, `plan4-list1/04-reopen` A and E's history copy):
//!   `STATUS:NEEDS-ACTION`, `COMPLETED` removed, `PERCENT-COMPLETE:0`.
//! - Repeat complete (H11, `rule-months-day-1-28/02-complete`, `rule-years/02-complete`):
//!   copy-and-advance. The live task's due date moves to the next occurrence, and a completed
//!   history copy with a new UID records the occurrence that was done.
//!
//! Only the captured repeat shape is supported: a date-only or floating-time `DUE` (the time
//! is kept), `RRULE` `FREQ=MONTHLY` or `FREQ=YEARLY` with an optional `INTERVAL`, repeating
//! from the due date. A JSON `StartDate` moves by as many days as the due date (H11, gap
//! session round 04), and a JSON `uid`
//! naming the task (a project) is rewritten to the copy's UID. Anything else is
//! refused rather than guessed, so a repeating obligation is never advanced wrongly.

use chrono::{DateTime, Datelike, Months, NaiveDate, NaiveTime, Utc};

use super::{
    EditError, Meta, has_own_dtstart, meta_of, set_date_prop, the_todo, touch, unsupported,
    write_meta,
};
use crate::ical::value::{IcalDateTime, format_date_time, parse_date_time};
use crate::ical::{Calendar, Component, Property};
use crate::model::meta_edit::MetaJson;
use crate::model::repeat::{End, Freq, RRule, parse_rrule};

/// 2Do's `RecurrenceType` codes for the verified frequencies (reference 01 §6).
const TYPE_MONTHLY: i64 = 0x102;
const TYPE_YEARLY: i64 = 0x103;
/// `RecurrenceFrom` of a task that repeats from its due date (the only value captured).
const FROM_DUE: i64 = 1;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum State {
    Open,
    Completed,
    Cancelled,
}

/// The task's state, read the way the model reads it: an explicit `STATUS` wins, and
/// `COMPLETED` alone means completed only without `STATUS`.
pub(super) fn state(todo: &Component) -> State {
    let status = todo
        .prop("STATUS")
        .map(|p| p.value.trim().to_ascii_uppercase());
    match status.as_deref() {
        Some("COMPLETED") => State::Completed,
        Some("CANCELLED") => State::Cancelled,
        Some(_) => State::Open,
        None if todo.prop("COMPLETED").is_some() => State::Completed,
        None => State::Open,
    }
}

/// Set a plain-valued property, keeping its bytes when the value is already right. A missing
/// one goes after the first existing property in `after`.
fn set_plain(todo: &mut Component, name: &str, value: &str, after: &[&str]) {
    match todo.prop_mut(name) {
        Some(p) => {
            if p.value != value {
                p.set_value(value);
            }
        }
        None => {
            let after = after.iter().copied().find(|n| todo.prop(n).is_some());
            todo.insert_prop(Property::new(name, Vec::new(), value), after);
        }
    }
}

fn set_status(todo: &mut Component, value: &str) {
    set_plain(todo, "STATUS", value, &["DESCRIPTION", "SUMMARY", "UID"]);
}

/// `STATUS:COMPLETED`, `PERCENT-COMPLETE:100` and `COMPLETED:<now UTC>` after `STATUS`.
pub(super) fn mark_completed(todo: &mut Component, now: DateTime<Utc>) {
    set_status(todo, "COMPLETED");
    set_plain(todo, "PERCENT-COMPLETE", "100", &["STATUS"]);
    let at = IcalDateTime::Utc(now);
    let (_, stamp) = format_date_time(&at);
    if let Some(p) = todo.prop_mut("COMPLETED")
        && !p.params.is_empty()
    {
        p.set_params(Vec::new());
    }
    set_plain(todo, "COMPLETED", &stamp, &["PERCENT-COMPLETE", "STATUS"]);
}

/// `STATUS:NEEDS-ACTION`, `COMPLETED` removed, `PERCENT-COMPLETE:0` (after `STATUS` if absent).
pub(super) fn mark_open(todo: &mut Component) {
    set_status(todo, "NEEDS-ACTION");
    todo.remove_props("COMPLETED");
    set_plain(todo, "PERCENT-COMPLETE", "0", &["STATUS"]);
}

/// Complete a task that does not repeat (H3). All or nothing.
pub fn complete_one_time(cal: &mut Calendar, now: DateTime<Utc>) -> Result<(), EditError> {
    let mut work = cal.clone();
    let todo = the_todo(&mut work)?;
    if todo.prop("RRULE").is_some() {
        return Err(unsupported("use complete_recurring for repeating tasks"));
    }
    match state(todo) {
        State::Open => {}
        State::Completed => return Err(unsupported("task is already completed")),
        State::Cancelled => {
            return Err(unsupported("completing a cancelled task is not supported"));
        }
    }
    mark_completed(todo, now);
    touch(todo, now);
    *cal = work;
    Ok(())
}

/// Reopen a completed task that does not repeat (H4 as captured). Also reopens a repeat's
/// history copy, which has no `RRULE`. All or nothing.
pub fn reopen_one_time(cal: &mut Calendar, now: DateTime<Utc>) -> Result<(), EditError> {
    let mut work = cal.clone();
    let todo = the_todo(&mut work)?;
    if todo.prop("RRULE").is_some() {
        return Err(unsupported("reopening a repeating task is not supported"));
    }
    match state(todo) {
        State::Completed => {}
        State::Open => return Err(unsupported("task is not completed")),
        State::Cancelled => return Err(unsupported("reopening a cancelled task is not supported")),
    }
    mark_open(todo);
    touch(todo, now);
    *cal = work;
    Ok(())
}

/// The rule as RRULE text, for messages.
fn rule_text(r: &RRule) -> String {
    let join = |v: Vec<String>| v.join(",");
    let mut parts = vec![format!(
        "FREQ={}",
        match r.freq {
            Freq::Daily => "DAILY",
            Freq::Weekly => "WEEKLY",
            Freq::Monthly => "MONTHLY",
            Freq::Yearly => "YEARLY",
        }
    )];
    if r.interval != 1 {
        parts.push(format!("INTERVAL={}", r.interval));
    }
    if !r.by_day.is_empty() {
        let days = r.by_day.iter().map(|(n, w)| {
            let code = w.to_string().to_ascii_uppercase();
            format!(
                "{}{}",
                n.map(|n| n.to_string()).unwrap_or_default(),
                &code[..2]
            )
        });
        parts.push(format!("BYDAY={}", join(days.collect())));
    }
    let nums = |v: &[i8]| join(v.iter().map(|n| n.to_string()).collect());
    if !r.by_month_day.is_empty() {
        parts.push(format!("BYMONTHDAY={}", nums(&r.by_month_day)));
    }
    if !r.by_month.is_empty() {
        let months = r.by_month.iter().map(|n| n.to_string()).collect();
        parts.push(format!("BYMONTH={}", join(months)));
    }
    if !r.by_set_pos.is_empty() {
        parts.push(format!("BYSETPOS={}", nums(&r.by_set_pos)));
    }
    match &r.end {
        End::Never => {}
        End::Count(n) => parts.push(format!("COUNT={n}")),
        End::Until(u) => parts.push(format!("UNTIL={}", format_date_time(u).1)),
    }
    for (k, v) in &r.unknown {
        parts.push(format!("{k}={v}"));
    }
    parts.join(";")
}

/// `d` moved forward by `months`. A result on another day of the month (day 29-31 or Feb 29,
/// which the gate excludes) is refused rather than clamped.
pub fn shift_months(d: NaiveDate, months: u32) -> Result<NaiveDate, EditError> {
    let next = d
        .checked_add_months(Months::new(months))
        .ok_or_else(|| unsupported("date is out of range"))?;
    if next.day() != d.day() {
        return Err(unsupported(format!(
            "the next date would not fall on day {} of the month",
            d.day()
        )));
    }
    Ok(next)
}

/// [`shift_months`] with the refusal worded for `what` ("due" or "start").
fn shift_named(d: NaiveDate, months: u32, what: &str) -> Result<NaiveDate, EditError> {
    shift_months(d, months).map_err(|e| match e {
        EditError::Unsupported(m) if m.starts_with("the next date") => unsupported(format!(
            "next {what} date after {d} would not fall on day {} of the month",
            d.day()
        )),
        EditError::Unsupported(_) => unsupported(format!("next {what} date is out of range")),
        e => e,
    })
}

/// The months one repeat interval spans. Verified shape only: `FREQ=MONTHLY|YEARLY` with an
/// optional `INTERVAL` and nothing else.
fn rule_months(rule: &RRule) -> Result<u32, EditError> {
    let plain = rule.by_day.is_empty()
        && rule.by_month_day.is_empty()
        && rule.by_month.is_empty()
        && rule.by_set_pos.is_empty()
        && rule.end == End::Never
        && rule.unknown.is_empty();
    let months = match rule.freq {
        Freq::Monthly if plain => Some(rule.interval),
        Freq::Yearly if plain => rule.interval.checked_mul(12),
        _ => None,
    };
    months.filter(|&m| m >= 1).ok_or_else(|| {
        unsupported(format!(
            "repeat rule shape not verified: {}",
            rule_text(rule)
        ))
    })
}

/// The next due date of a date-only repeat (see [`rule_months`] and [`shift_months`]).
pub fn next_due(due: NaiveDate, rule: &RRule) -> Result<NaiveDate, EditError> {
    shift_named(due, rule_months(rule)?, "due")
}

/// The JSON repeat fields must describe the same verified rule as `RRULE`: repeat from the
/// due date, matching type and interval, no end.
fn check_repeat_meta(m: &MetaJson, rule: &RRule) -> Result<(), EditError> {
    let want_type = match rule.freq {
        Freq::Monthly => TYPE_MONTHLY,
        Freq::Yearly => TYPE_YEARLY,
        _ => return Err(unsupported("repeat rule shape not verified")),
    };
    let checks = [
        ("RecurrenceFrom", Some(FROM_DUE)),
        ("RecurrenceType", Some(want_type)),
        ("RecurrenceValue", Some(i64::from(rule.interval))),
    ];
    for (key, want) in checks {
        let got = m.get_i64(key);
        if got != want {
            return Err(unsupported(format!(
                "2Do repeat metadata not verified for completion: {key} = {}",
                got.map_or("missing".to_string(), |v| v.to_string())
            )));
        }
    }
    if let Some(t) = m.get_i64("RecurrenceEndType")
        && t != 0
    {
        return Err(unsupported(format!(
            "2Do repeat metadata not verified for completion: RecurrenceEndType = {t}"
        )));
    }
    Ok(())
}

/// Complete a repeating task the way 2Do does. Returns (the advanced live resource, the
/// completed history copy with UID `copy_uid`); `cal` is not changed.
///
/// - Live: `DUE` and its `DTSTART` mirror move by one interval (a floating time keeps its
///   time), as does JSON `StartDate`; stamps touched.
/// - Copy: cloned from `cal`; `UID` = `copy_uid`, stamps touched, completed (`STATUS`,
///   `PERCENT-COMPLETE`, `COMPLETED`), `RRULE` removed, JSON `RecurrenceValue`,
///   `RecurrenceType`, `RecurrenceFrom` = 0, `RUID` kept (set to the original UID if missing
///   or empty).
pub fn complete_recurring(
    cal: &Calendar,
    now: DateTime<Utc>,
    copy_uid: &str,
) -> Result<(Calendar, Calendar), EditError> {
    let mut live = cal.clone();
    let todo = the_todo(&mut live)?;
    let uid = todo
        .prop("UID")
        .map(|p| p.value.clone())
        .filter(|u| !u.is_empty())
        .ok_or_else(|| unsupported("task has no UID"))?;
    if copy_uid.is_empty() || copy_uid == uid {
        return Err(unsupported("history copy needs a new UID"));
    }
    let raw = match todo.props_named("RRULE").collect::<Vec<_>>().as_slice() {
        [] => return Err(unsupported("task does not repeat")),
        [p] => p.value.clone(),
        _ => return Err(unsupported("several RRULEs are not supported")),
    };
    if state(todo) != State::Open {
        return Err(unsupported("only an open repeating task can be completed"));
    }
    let rule =
        parse_rrule(&raw).map_err(|e| unsupported(format!("unreadable repeat rule: {e}")))?;
    let due_prop = todo
        .prop("DUE")
        .ok_or_else(|| unsupported("completing a repeat without a due date is not verified"))?;
    let due = match parse_date_time(due_prop) {
        Ok(v @ (IcalDateTime::Date(_) | IcalDateTime::Floating(_))) => v,
        Ok(_) => {
            return Err(unsupported(
                "completing a repeat with a zoned due time is not verified",
            ));
        }
        Err(e) => return Err(unsupported(format!("unreadable due date: {e}"))),
    };
    let months = rule_months(&rule).map_err(|e| match e {
        EditError::Unsupported(m) if m.starts_with("repeat rule shape") => {
            unsupported(format!("repeat rule shape not verified: {raw}"))
        }
        e => e,
    })?;
    let next = match &due {
        IcalDateTime::Floating(t) => {
            IcalDateTime::Floating(shift_named(t.date(), months, "due")?.and_time(t.time()))
        }
        IcalDateTime::Date(d) => IcalDateTime::Date(shift_named(*d, months, "due")?),
        _ => unreachable!("zoned due times are refused above"),
    };
    let meta = match meta_of(todo) {
        Meta::Json(m) => m,
        _ => {
            return Err(unsupported(
                "completing a repeat without 2Do JSON metadata is not verified",
            ));
        }
    };
    check_repeat_meta(&meta, &rule)?;
    let mirror = todo.prop("DTSTART").is_some();
    if mirror && has_own_dtstart(todo) {
        return Err(unsupported(
            "completing a repeat with its own start (DTSTART) is not verified",
        ));
    }

    // JSON `StartDate` (UTC midnight, H9) moves by as many days as the due date did, not by
    // one interval: a yearly due 2027-03-01 -> 2028-03-01 (366 days) took its start
    // 2027-02-13 -> 2028-02-14 (`gap-list1/04-phone-complete`).
    let (due_date, next_date) = match (&due, &next) {
        (IcalDateTime::Floating(a), IcalDateTime::Floating(b)) => (a.date(), b.date()),
        (IcalDateTime::Date(a), IcalDateTime::Date(b)) => (*a, *b),
        _ => unreachable!("the next due keeps the due's form"),
    };
    let mut live_meta = meta.clone();
    if let Some(secs) = meta.get_i64("StartDate") {
        if secs <= 0 || secs % 86_400 != 0 {
            return Err(unsupported(
                "completing a repeat with an unusual start date is not verified",
            ));
        }
        let start = DateTime::from_timestamp(secs, 0)
            .map(|t| t.date_naive())
            .ok_or_else(|| unsupported("2Do start date is out of range"))?;
        let next_start = start
            .checked_add_signed(next_date - due_date)
            .ok_or_else(|| unsupported("next start date is out of range"))?;
        live_meta.set_i64(
            "StartDate",
            next_start.and_time(NaiveTime::MIN).and_utc().timestamp(),
        );
    }

    if mirror {
        set_date_prop(todo, "DTSTART", &next, &[]);
    }
    set_date_prop(todo, "DUE", &next, &[]);
    write_meta(todo, Some(&meta), &live_meta);
    touch(todo, now);

    let mut copy = cal.clone();
    let t = the_todo(&mut copy)?;
    if let Some(p) = t.prop_mut("UID") {
        p.set_value(copy_uid);
    }
    touch(t, now);
    mark_completed(t, now);
    t.remove_props("RRULE");
    let mut m = meta.clone();
    for key in ["RecurrenceValue", "RecurrenceType", "RecurrenceFrom"] {
        m.set_i64(key, 0);
    }
    if m.get_str("uid") == Some(uid.as_str()) {
        m.set_str("uid", copy_uid);
    }
    if m.get_str("RUID").is_none_or(str::is_empty) {
        m.set_str("RUID", &uid);
    }
    write_meta(t, Some(&meta), &m);
    Ok((live, copy))
}
