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
//! Every repeat kind Twydo can create completes (see [`next_due`]): a date-only or
//! floating-time `DUE` (the time is kept), from the due date or from completion, with an
//! optional `COUNT` or `UNTIL` end (session B, `plan7-twydo-list1/03-phone-act`); each kind's
//! result equals the phone's own completion (session C, `plan7-twydo-list2`). A JSON
//! `StartDate` moves by as many days as the due date (H11, gap session round 04), and a JSON
//! `uid` naming the task (a project) is rewritten to the copy's UID. Anything else is
//! refused rather than guessed, so a repeating obligation is never advanced wrongly.

use chrono::{DateTime, Datelike, Days, Months, NaiveDate, NaiveTime, Utc, Weekday};

use super::repeat::twodo_codes;
use super::{
    EditError, Meta, has_own_dtstart, meta_of, set_date_prop, the_todo, touch, unsupported,
    write_meta,
};
use crate::ical::value::{IcalDateTime, format_date_time, parse_date_time};
use crate::ical::{Calendar, Component, Property};
use crate::model::meta_edit::MetaJson;
use crate::model::repeat::{End, Freq, RRule, parse_rrule};

/// `RecurrenceFrom` of a task that repeats from its due date, and from completion (01 §6).
const FROM_DUE: i64 = 1;
const FROM_COMPLETION: i64 = 2;

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

/// Why a completed repeating task (the last occurrence of an ended series, completed in
/// place) is not reopened: no capture shows what 2Do does with it.
pub const REOPEN_ENDED: &str =
    "reopening the last occurrence of an ended repeat is not supported; use undo";

/// Reopen a completed task that does not repeat (H4 as captured). Also reopens a repeat's
/// history copy, which has no `RRULE`. All or nothing.
pub fn reopen_one_time(cal: &mut Calendar, now: DateTime<Utc>) -> Result<(), EditError> {
    let mut work = cal.clone();
    let todo = the_todo(&mut work)?;
    if todo.prop("RRULE").is_some() {
        return Err(unsupported(if state(todo) == State::Completed {
            REOPEN_ENDED
        } else {
            "reopening a repeating task is not supported"
        }));
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
pub(super) fn rule_text(r: &RRule) -> String {
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

/// The repeat kinds Twydo completes: everything it can create (`repeatinput`). The kinds
/// 2Do 3.91 on the iPhone cannot show (last day, Nth workday, Nth weekend day; session B) and
/// shapes Twydo never writes are refused.
enum Step {
    /// `FREQ=DAILY` (N days) and plain `FREQ=WEEKLY` (7N days).
    Days(u64),
    /// Plain `FREQ=MONTHLY` (N months) and `FREQ=YEARLY` (12N months).
    Months(u32),
    /// `FREQ=WEEKLY;BYDAY=<set>`, 2Do's weekday bitmask (no interval).
    Weekdays(Vec<Weekday>),
    /// `FREQ=MONTHLY;BYDAY=<n><day>`, n 1..=5 or -1 (last).
    NthWeekday(i8, Weekday),
}

fn step_of(rule: &RRule) -> Result<Step, EditError> {
    let plain = rule.by_month_day.is_empty()
        && rule.by_month.is_empty()
        && rule.by_set_pos.is_empty()
        && rule.unknown.is_empty()
        && rule.interval >= 1;
    let n = rule.interval;
    let step = match (rule.freq, rule.by_day.as_slice()) {
        _ if !plain => None,
        (Freq::Daily, []) => Some(Step::Days(u64::from(n))),
        (Freq::Weekly, []) => Some(Step::Days(u64::from(n) * 7)),
        (Freq::Weekly, days) if n == 1 && days.iter().all(|(o, _)| o.is_none()) => {
            Some(Step::Weekdays(days.iter().map(|(_, w)| *w).collect()))
        }
        (Freq::Monthly, []) => Some(Step::Months(n)),
        (Freq::Monthly, [(Some(o), w)]) if n == 1 && matches!(o, 1..=5 | -1) => {
            Some(Step::NthWeekday(*o, *w))
        }
        (Freq::Yearly, []) => n.checked_mul(12).map(Step::Months),
        _ => None,
    };
    step.ok_or_else(|| {
        unsupported(format!(
            "repeat rule shape not verified: {}",
            rule_text(rule)
        ))
    })
}

/// The `n`th (`-1` = last) `w` of the month.
fn nth_weekday(year: i32, month: u32, n: i8, w: Weekday) -> Option<NaiveDate> {
    if n > 0 {
        return NaiveDate::from_weekday_of_month_opt(year, month, w, n.unsigned_abs());
    }
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let last = first.checked_add_months(Months::new(1))?.pred_opt()?;
    let back = (7 + last.weekday().num_days_from_monday() - w.num_days_from_monday()) % 7;
    last.checked_sub_days(Days::new(u64::from(back)))
}

/// The first `n`th `w` of a month after `base`. A month without one (a 5th Tuesday) is
/// skipped, as RFC 5545 does.
fn nth_weekday_after(base: NaiveDate, n: i8, w: Weekday) -> Option<NaiveDate> {
    let mut month = base.with_day(1)?;
    for _ in 0..24 {
        if let Some(d) = nth_weekday(month.year(), month.month(), n, w)
            && d > base
        {
            return Some(d);
        }
        month = month.checked_add_months(Months::new(1))?;
    }
    None
}

/// The date part of an `UNTIL`.
fn until_date(u: &IcalDateTime) -> NaiveDate {
    match u {
        IcalDateTime::Date(d) => *d,
        IcalDateTime::Floating(n) => n.date(),
        IcalDateTime::Utc(t) => t.date_naive(),
        IcalDateTime::Zoned { local, .. } => local.date(),
    }
}

/// The next due date of a repeat whose current due date is `due`. `from` is the day a
/// from-completion rule counts from: the completion day, whatever the task's start date
/// (session C, `FS`: due Dec 7 with start Dec 20, completed Nov 4 -> Dec 4); `None` counts
/// from `due`.
/// `Ok(None)`: the rule has ended (`COUNT=1`, or the next date is after `UNTIL`), so
/// completing finishes the series.
///
/// Stateless, from the one date given (2Do keeps no other day): every N days or weeks; a
/// weekday set's next day in the set; every N calendar months or years, a day the shorter
/// month lacks becoming its last day (Jan 31 -> Feb 28, Feb 29 -> Feb 28; then the 28th);
/// the Nth or last weekday of the next month that has one.
pub fn next_due(
    due: NaiveDate,
    rule: &RRule,
    from: Option<NaiveDate>,
) -> Result<Option<NaiveDate>, EditError> {
    let step = step_of(rule)?;
    let base = from.unwrap_or(due);
    let next = match step {
        Step::Days(n) => base.checked_add_days(Days::new(n)),
        Step::Months(n) => base.checked_add_months(Months::new(n)),
        Step::Weekdays(set) => (1..=7)
            .filter_map(|i| base.checked_add_days(Days::new(i)))
            .find(|d| set.contains(&d.weekday())),
        Step::NthWeekday(n, w) => nth_weekday_after(base, n, w),
    }
    .ok_or_else(|| unsupported("next due date is out of range"))?;
    let ended = match &rule.end {
        End::Never => false,
        End::Count(n) => *n <= 1,
        End::Until(u) => next > until_date(u),
    };
    Ok((!ended).then_some(next))
}

/// What Twydo wrote for "last" in the Nth-weekday class before session C showed the phone's
/// own 6 ([`super::repeat::LAST_ORDINAL`]); files written then still carry it.
const OLD_LAST_ORDINAL: i64 = -1;

/// Whether `rule` is the last-weekday-of-the-month shape (`BYDAY=-1<day>`).
fn is_last_weekday(rule: &RRule) -> bool {
    rule.freq == Freq::Monthly && matches!(rule.by_day.as_slice(), [(Some(-1), _)])
}

/// The JSON repeat fields must describe the same rule as `RRULE`: the codes Twydo and the
/// phone write for it ([`twodo_codes`]; for "last" in the Nth-weekday class also the -1 older
/// Twydo files carry), from the due date (1) or from completion (2), and the matching end: an `UNTIL`'s
/// day must be the UTC day of JSON `RecurrenceEndDate`. Returns whether the rule repeats
/// from completion.
fn check_repeat_meta(m: &MetaJson, rule: &RRule) -> Result<bool, EditError> {
    let refuse = |key: &str, got: Option<i64>| {
        unsupported(format!(
            "2Do repeat metadata not verified for completion: {key} = {}",
            got.map_or("missing".to_string(), |v| v.to_string())
        ))
    };
    let from = m.get_i64("RecurrenceFrom");
    let from_completion = match from {
        Some(FROM_DUE) => false,
        Some(FROM_COMPLETION) => true,
        _ => return Err(refuse("RecurrenceFrom", from)),
    };
    let (kind, value) = twodo_codes(rule);
    let kind_got = m.get_i64("RecurrenceType");
    if kind_got != Some(kind) {
        return Err(refuse("RecurrenceType", kind_got));
    }
    let value_got = m.get_i64("RecurrenceValue");
    let old_last = is_last_weekday(rule) && value_got == Some(OLD_LAST_ORDINAL);
    if value_got != Some(value) && !old_last {
        return Err(refuse("RecurrenceValue", value_got));
    }
    let end = m.get_i64("RecurrenceEndType");
    let end_ok = match &rule.end {
        End::Never => end.is_none_or(|t| t == 0),
        End::Count(n) => {
            end == Some(2) && m.get_i64("RecurrenceEndRepetitions") == Some(i64::from(*n))
        }
        End::Until(_) => end == Some(1),
    };
    if !end_ok {
        return Err(refuse("RecurrenceEndType", end));
    }
    if let End::Until(u) = &rule.end {
        let secs = m.get_i64("RecurrenceEndDate");
        let day = secs
            .and_then(|s| DateTime::from_timestamp(s, 0))
            .map(|t| t.date_naive());
        if day != Some(until_date(u)) {
            return Err(refuse("RecurrenceEndDate", secs));
        }
    }
    Ok(from_completion)
}

/// `raw` (an RRULE value) with its `COUNT` part set to `n`, every other part as written.
fn with_count(raw: &str, n: u32) -> String {
    raw.split(';')
        .map(|part| match part.split_once('=') {
            Some((k, _)) if k.trim().eq_ignore_ascii_case("COUNT") => format!("{k}={n}"),
            _ => part.to_string(),
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Complete a repeating task the way 2Do does. Returns the live resource and, when the series
/// goes on, the completed history copy with UID `copy_uid`; `cal` is not changed. `today` is
/// the completion day, which a from-completion rule counts from.
///
/// - Live: `DUE` and its `DTSTART` mirror move to [`next_due`] (a floating time keeps its
///   time), JSON `StartDate` by as many days; a `COUNT` and JSON `RecurrenceEndRepetitions`
///   count down by one (`CT`, session B); `RecurrenceValue` is written as [`twodo_codes`]
///   gives it (an older -1 for "last" becomes 6, `LF`, session C); stamps touched.
/// - Copy: cloned from `cal`; `UID` = `copy_uid`, stamps touched, completed (`STATUS`,
///   `PERCENT-COMPLETE`, `COMPLETED`), `RRULE` removed, JSON `RecurrenceValue`,
///   `RecurrenceType`, `RecurrenceFrom` and the end keys = 0 (`RecurrenceEndDate` removed),
///   `RUID` kept (set to the original UID if missing or empty).
/// - Last occurrence (`COUNT=1`, or the next date after `UNTIL`): no copy; the live task is
///   completed in place, its `RRULE`, due and metadata kept.
pub fn complete_recurring(
    cal: &Calendar,
    now: DateTime<Utc>,
    today: NaiveDate,
    copy_uid: &str,
) -> Result<(Calendar, Option<Calendar>), EditError> {
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
    let due_date = match &due {
        IcalDateTime::Floating(t) => t.date(),
        IcalDateTime::Date(d) => *d,
        _ => unreachable!("zoned due times are refused above"),
    };
    step_of(&rule).map_err(|_| unsupported(format!("repeat rule shape not verified: {raw}")))?;
    let meta = match meta_of(todo) {
        Meta::Json(m) => m,
        _ => {
            return Err(unsupported(
                "completing a repeat without 2Do JSON metadata is not verified",
            ));
        }
    };
    let from_completion = check_repeat_meta(&meta, &rule)?;
    let mirror = todo.prop("DTSTART").is_some();
    if mirror && has_own_dtstart(todo) {
        return Err(unsupported(
            "completing a repeat with its own start (DTSTART) is not verified",
        ));
    }
    let start = match meta.get_i64("StartDate") {
        None => None,
        Some(secs) if secs <= 0 || secs % 86_400 != 0 => {
            return Err(unsupported(
                "completing a repeat with an unusual start date is not verified",
            ));
        }
        Some(secs) => Some(
            DateTime::from_timestamp(secs, 0)
                .map(|t| t.date_naive())
                .ok_or_else(|| unsupported("2Do start date is out of range"))?,
        ),
    };
    let from = from_completion.then_some(today);

    let Some(next_date) = next_due(due_date, &rule, from)? else {
        // The series ends with this occurrence: the task itself is done.
        mark_completed(todo, now);
        touch(todo, now);
        return Ok((live, None));
    };
    let next = match &due {
        IcalDateTime::Floating(t) => IcalDateTime::Floating(next_date.and_time(t.time())),
        _ => IcalDateTime::Date(next_date),
    };

    // JSON `StartDate` (UTC midnight, H9) moves by as many days as the due date did, not by
    // one interval: a yearly due 2027-03-01 -> 2028-03-01 (366 days) took its start
    // 2027-02-13 -> 2028-02-14 (`gap-list1/04-phone-complete`).
    let mut live_meta = meta.clone();
    if let Some(start) = start {
        let next_start = start
            .checked_add_signed(next_date - due_date)
            .ok_or_else(|| unsupported("next start date is out of range"))?;
        live_meta.set_i64(
            "StartDate",
            next_start.and_time(NaiveTime::MIN).and_utc().timestamp(),
        );
    }
    // The codes as 2Do writes them: the phone rewrites an older -1 for "last" to 6 when it
    // completes (session C, `LF`); every other code is already this one.
    live_meta.set_i64("RecurrenceValue", twodo_codes(&rule).1);
    if let End::Count(n) = rule.end {
        let left = n - 1;
        if let Some(p) = todo.prop_mut("RRULE") {
            p.set_value(&with_count(&raw, left));
        }
        live_meta.set_i64("RecurrenceEndRepetitions", i64::from(left));
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
    // `CT`'s copy (session B): the end goes too.
    for key in [
        "RecurrenceEndType",
        "RecurrenceEndRepetitions",
        "RecurrenceEndRepetitionsOrig",
    ] {
        if m.get_i64(key).is_some() {
            m.set_i64(key, 0);
        }
    }
    m.remove("RecurrenceEndDate");
    if m.get_str("uid") == Some(uid.as_str()) {
        m.set_str("uid", copy_uid);
    }
    if m.get_str("RUID").is_none_or(str::is_empty) {
        m.set_str("RUID", &uid);
    }
    write_meta(t, Some(&meta), &m);
    Ok((live, Some(copy)))
}
