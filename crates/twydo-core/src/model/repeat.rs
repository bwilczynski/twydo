//! 2Do repeat rules (RFC 5545 RRULE subset): parsing, display text and a rule class.
//!
//! Nothing here executes a rule; the class is the key of the compatibility matrix.

use chrono::{Datelike, NaiveDate, NaiveDateTime, Weekday};
use thiserror::Error;

use crate::ical::value::IcalDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum End {
    Never,
    Count(u32),
    Until(IcalDateTime),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RRule {
    pub freq: Freq,
    pub interval: u32,
    pub by_day: Vec<(Option<i8>, Weekday)>,
    pub by_month_day: Vec<i8>,
    pub by_month: Vec<u8>,
    pub by_set_pos: Vec<i8>,
    pub end: End,
    /// Parts Twydo does not interpret, kept as written (key uppercased).
    pub unknown: Vec<(String, String)>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RepeatError {
    #[error("repeat rule has no FREQ")]
    MissingFreq,
    #[error("unsupported FREQ {0:?}")]
    UnsupportedFreq(String),
    #[error("invalid {0} value {1:?}")]
    BadValue(&'static str, String),
}

/// Every base class [`rule_class`] can return (before `+` modifiers).
pub const RULE_CLASSES: &[&str] = &[
    "days",
    "weeks",
    "weekdays-set",
    "months-day-1-28",
    "months-day-29-31",
    "months-last-day",
    "months-nth-weekday",
    "months-nth-workday",
    "months-nth-weekend-day",
    "years",
    "years-feb-29",
    "unrecognized",
];

const WEEKDAYS: [(&str, Weekday); 7] = [
    ("MO", Weekday::Mon),
    ("TU", Weekday::Tue),
    ("WE", Weekday::Wed),
    ("TH", Weekday::Thu),
    ("FR", Weekday::Fri),
    ("SA", Weekday::Sat),
    ("SU", Weekday::Sun),
];

fn bad<T>(what: &'static str, v: &str) -> Result<T, RepeatError> {
    Err(RepeatError::BadValue(what, v.to_string()))
}

fn parse_list<T: std::str::FromStr>(what: &'static str, v: &str) -> Result<Vec<T>, RepeatError> {
    v.split(',')
        .map(|x| x.trim().parse::<T>().or_else(|_| bad(what, v)))
        .collect()
}

fn parse_until(v: &str) -> Option<IcalDateTime> {
    let v = v.trim();
    if v.len() == 8 {
        return NaiveDate::parse_from_str(v, "%Y%m%d")
            .ok()
            .map(IcalDateTime::Date);
    }
    if let Some(body) = v.strip_suffix('Z') {
        let n = NaiveDateTime::parse_from_str(body, "%Y%m%dT%H%M%S").ok()?;
        return Some(IcalDateTime::Utc(n.and_utc()));
    }
    NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S")
        .ok()
        .map(IcalDateTime::Floating)
}

fn parse_by_day(v: &str) -> Result<Vec<(Option<i8>, Weekday)>, RepeatError> {
    v.split(',')
        .map(|item| {
            let item = item.trim();
            if item.len() < 2 || !item.is_char_boundary(item.len() - 2) {
                return bad("BYDAY", v);
            }
            let (num, code) = item.split_at(item.len() - 2);
            let wd = WEEKDAYS
                .iter()
                .find(|(c, _)| c.eq_ignore_ascii_case(code))
                .map(|(_, w)| *w);
            let Some(wd) = wd else { return bad("BYDAY", v) };
            let n = if num.is_empty() {
                None
            } else {
                match num.parse::<i8>() {
                    Ok(n) if n != 0 => Some(n),
                    _ => return bad("BYDAY", v),
                }
            };
            Ok((n, wd))
        })
        .collect()
}

/// Parse an RRULE value (an optional `RRULE:` prefix is accepted).
pub fn parse_rrule(s: &str) -> Result<RRule, RepeatError> {
    let s = s.trim();
    let s = s.strip_prefix("RRULE:").unwrap_or(s);
    let mut freq = None;
    let mut r = RRule {
        freq: Freq::Daily,
        interval: 1,
        by_day: vec![],
        by_month_day: vec![],
        by_month: vec![],
        by_set_pos: vec![],
        end: End::Never,
        unknown: vec![],
    };
    for part in s.split(';').filter(|p| !p.trim().is_empty()) {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        let key = k.trim().to_ascii_uppercase();
        let v = v.trim();
        match key.as_str() {
            "FREQ" => {
                freq = Some(match v.to_ascii_uppercase().as_str() {
                    "DAILY" => Freq::Daily,
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    "YEARLY" => Freq::Yearly,
                    _ => return Err(RepeatError::UnsupportedFreq(v.to_string())),
                })
            }
            "INTERVAL" => match v.parse::<u32>() {
                Ok(n) if n >= 1 => r.interval = n,
                _ => return bad("INTERVAL", v),
            },
            "BYDAY" => r.by_day = parse_by_day(v)?,
            "BYMONTHDAY" => r.by_month_day = parse_list("BYMONTHDAY", v)?,
            "BYMONTH" => r.by_month = parse_list("BYMONTH", v)?,
            "BYSETPOS" => r.by_set_pos = parse_list("BYSETPOS", v)?,
            "COUNT" => match v.parse::<u32>() {
                Ok(n) => r.end = End::Count(n),
                Err(_) => return bad("COUNT", v),
            },
            "UNTIL" => match parse_until(v) {
                Some(u) => r.end = End::Until(u),
                None => return bad("UNTIL", v),
            },
            _ => r.unknown.push((key, v.to_string())),
        }
    }
    r.freq = freq.ok_or(RepeatError::MissingFreq)?;
    Ok(r)
}

fn day_name(w: Weekday) -> &'static str {
    ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][w.num_days_from_monday() as usize]
}

fn month_abbrev(m: u32) -> &'static str {
    [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][(m - 1) as usize]
}

fn day_ordinal(n: u32) -> String {
    let suffix = match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

fn ordinal_word(n: i8) -> &'static str {
    match n {
        1 => "first",
        2 => "second",
        3 => "third",
        4 => "fourth",
        5 => "fifth",
        -1 => "last",
        _ => "nth",
    }
}

fn has_all(r: &RRule, days: &[Weekday]) -> bool {
    r.by_day.len() == days.len()
        && days
            .iter()
            .all(|d| r.by_day.iter().any(|(n, w)| n.is_none() && w == d))
}

const WORKDAYS: [Weekday; 5] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
];
const WEEKEND: [Weekday; 2] = [Weekday::Sat, Weekday::Sun];

fn is_every_day(r: &RRule) -> bool {
    WEEKDAYS
        .iter()
        .all(|(_, d)| r.by_day.iter().any(|(_, w)| w == d))
        && r.by_day.len() == 7
}

/// The structural shape of a monthly rule.
enum Monthly {
    Plain,
    LastDay,
    NthWeekday(i8, Weekday),
    SetPos(i8, bool), // true = workday, false = weekend day
}

fn monthly_shape(r: &RRule) -> Option<Monthly> {
    if !r.by_month.is_empty() {
        return None;
    }
    match (
        r.by_day.as_slice(),
        r.by_month_day.as_slice(),
        r.by_set_pos.as_slice(),
    ) {
        ([], [], []) => Some(Monthly::Plain),
        ([], [-1], []) => Some(Monthly::LastDay),
        ([], [d], []) if (1..=31).contains(d) => Some(Monthly::Plain),
        ([(Some(n), w)], [], []) => Some(Monthly::NthWeekday(*n, *w)),
        (_, [], [p]) if has_all(r, &WORKDAYS) => Some(Monthly::SetPos(*p, true)),
        (_, [], [p]) if has_all(r, &WEEKEND) => Some(Monthly::SetPos(*p, false)),
        _ => None,
    }
}

/// Day of month for a plain monthly rule: BYMONTHDAY if given, else the anchor's.
fn month_day(r: &RRule, anchor: Option<NaiveDate>) -> Option<u32> {
    match r.by_month_day.as_slice() {
        [d] if *d > 0 => Some(*d as u32),
        _ => anchor.map(|a| a.day()),
    }
}

/// 2Do's nth-weekday rule covers the 1st..5th and the last (-1) weekday of a month.
fn is_2do_ordinal(n: i8) -> bool {
    matches!(n, 1..=5 | -1)
}

/// A yearly rule repeats on its anchor's day; without an anchor the day is unknown.
fn yearly_supported(r: &RRule, anchor: Option<NaiveDate>) -> bool {
    let Some(a) = anchor else { return false };
    r.by_day.is_empty()
        && r.by_month_day.is_empty()
        && r.by_set_pos.is_empty()
        && match r.by_month.as_slice() {
            [] => true,
            [m] => u32::from(*m) == a.month(),
            _ => false,
        }
}

fn base_class(r: &RRule, anchor: Option<NaiveDate>) -> &'static str {
    if !r.unknown.is_empty() {
        return "unrecognized";
    }
    match r.freq {
        Freq::Daily => {
            if r.by_day.is_empty()
                && r.by_month_day.is_empty()
                && r.by_month.is_empty()
                && r.by_set_pos.is_empty()
            {
                "days"
            } else {
                "unrecognized"
            }
        }
        Freq::Weekly => {
            if !r.by_month_day.is_empty() || !r.by_month.is_empty() || !r.by_set_pos.is_empty() {
                "unrecognized"
            } else if r.by_day.is_empty() {
                "weeks"
            } else if r.by_day.iter().all(|(n, _)| n.is_none()) {
                "weekdays-set"
            } else {
                "unrecognized"
            }
        }
        Freq::Monthly => match monthly_shape(r) {
            None => "unrecognized",
            Some(Monthly::LastDay) => "months-last-day",
            Some(Monthly::NthWeekday(n, _)) if is_2do_ordinal(n) => "months-nth-weekday",
            Some(Monthly::NthWeekday(..)) => "unrecognized",
            Some(Monthly::SetPos(_, true)) => "months-nth-workday",
            Some(Monthly::SetPos(_, false)) => "months-nth-weekend-day",
            // Without BYMONTHDAY or an anchor the day of the month is unknown.
            Some(Monthly::Plain) => match month_day(r, anchor) {
                None => "unrecognized",
                Some(d) if d >= 29 => "months-day-29-31",
                Some(_) => "months-day-1-28",
            },
        },
        Freq::Yearly => {
            if !yearly_supported(r, anchor) {
                "unrecognized"
            } else if anchor.is_some_and(|a| a.month() == 2 && a.day() == 29) {
                "years-feb-29"
            } else {
                "years"
            }
        }
    }
}

/// Compatibility-matrix key: base class plus `+from-completion` and `+count`/`+until`.
/// `unrecognized` carries no modifiers.
pub fn rule_class(r: &RRule, anchor: Option<NaiveDate>, from_completion: bool) -> String {
    let mut c = base_class(r, anchor).to_string();
    if c == "unrecognized" {
        return c;
    }
    if from_completion {
        c.push_str("+from-completion");
    }
    match r.end {
        End::Never => {}
        End::Count(_) => c.push_str("+count"),
        End::Until(_) => c.push_str("+until"),
    }
    c
}

/// Short hint for the task row.
pub fn short_hint(r: &RRule) -> String {
    let n = r.interval;
    let (word, letter) = match r.freq {
        Freq::Daily => ("daily", 'd'),
        Freq::Weekly => ("weekly", 'w'),
        Freq::Monthly => ("monthly", 'm'),
        Freq::Yearly => ("yearly", 'y'),
    };
    if r.freq == Freq::Weekly && n == 1 && is_every_day(r) {
        "daily".into()
    } else if n == 1 {
        word.into()
    } else {
        format!("every {n}{letter}")
    }
}

fn months_label(n: u32) -> String {
    match n {
        1 => "Monthly".into(),
        2 => "Bi-monthly".into(),
        6 => "Semi-yearly".into(),
        _ => format!("Every {n} Months"),
    }
}

fn every_months(n: u32) -> String {
    if n == 1 {
        "each month".into()
    } else {
        format!("every {n} months")
    }
}

fn describe_base(r: &RRule, anchor: Option<NaiveDate>) -> String {
    let n = r.interval;
    match r.freq {
        Freq::Daily => match n {
            1 => "Daily".into(),
            _ => format!("Every {n} Days"),
        },
        Freq::Weekly => {
            if r.by_day.is_empty() {
                match n {
                    1 => "Weekly".into(),
                    2 => "Bi-weekly".into(),
                    _ => format!("Every {n} Weeks"),
                }
            } else if n == 1 && is_every_day(r) {
                "Every Day".into()
            } else {
                let days: Vec<&str> = r.by_day.iter().map(|(_, w)| day_name(*w)).collect();
                if n == 1 {
                    format!("Every {}", days.join(", "))
                } else {
                    format!("Every {n} weeks on {}", days.join(", "))
                }
            }
        }
        Freq::Monthly => match monthly_shape(r) {
            Some(Monthly::LastDay) => {
                format!("On the last day of {}", every_months(n))
            }
            Some(Monthly::NthWeekday(p, w)) => {
                format!(
                    "The {} {} of {}",
                    ordinal_word(p),
                    day_name(w),
                    every_months(n)
                )
            }
            Some(Monthly::SetPos(p, workday)) => {
                let kind = if workday { "weekday" } else { "weekend day" };
                format!("The {} {} of {}", ordinal_word(p), kind, every_months(n))
            }
            Some(Monthly::Plain) | None => match month_day(r, anchor) {
                Some(d) => format!("{} on the {}", months_label(n), day_ordinal(d)),
                None => months_label(n),
            },
        },
        Freq::Yearly => {
            let head = if n == 1 {
                "Yearly".to_string()
            } else {
                format!("Every {n} Years")
            };
            match anchor {
                Some(a) => format!("{head} on {} {}", a.day(), month_abbrev(a.month())),
                None => head,
            }
        }
    }
}

fn until_date(u: &IcalDateTime) -> NaiveDate {
    match u {
        IcalDateTime::Date(d) => *d,
        IcalDateTime::Floating(n) => n.date(),
        IcalDateTime::Utc(t) => t.date_naive(),
        IcalDateTime::Zoned { local, .. } => local.date(),
    }
}

/// Human-readable description for the details pane.
pub fn describe(r: &RRule, anchor: Option<NaiveDate>, from_completion: bool) -> String {
    let mut s = describe_base(r, anchor);
    if !r.unknown.is_empty() {
        let parts: Vec<String> = r.unknown.iter().map(|(k, v)| format!("{k}={v}")).collect();
        s.push_str(&format!(" (unrecognised parts: {})", parts.join(";")));
    }
    if from_completion {
        s.push_str(", from completion");
    }
    match &r.end {
        End::Never => {}
        End::Count(1) => s.push_str(", 1 time"),
        End::Count(c) => s.push_str(&format!(", {c} times")),
        End::Until(u) => {
            let d = until_date(u);
            s.push_str(&format!(
                ", until {} {} {}",
                d.day(),
                month_abbrev(d.month()),
                d.year()
            ));
        }
    }
    s
}
