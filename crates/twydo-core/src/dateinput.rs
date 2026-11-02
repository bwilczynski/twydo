//! Typed due and start dates: `today`, `3d`, `2w`, `1m`, `15th`, `fri 5pm`, `2026-11-20 14:30`.
//!
//! The grammar follows 2Do's date-field shortcuts (`docs/reference/2do/01-features.md` §16),
//! with one change: a bare number such as `9` is rejected rather than read as a time.

use chrono::{Datelike, Days, Months, NaiveDate, NaiveTime, Weekday};
use chrono_tz::Tz;

use crate::edit::Due;
use crate::ical::value::{IcalDateTime, Resolved};

/// A parsed date with an optional time of day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateInput {
    pub date: NaiveDate,
    pub time: Option<NaiveTime>,
}

impl DateInput {
    /// The due date this input sets: timed when a time was given.
    pub fn due(&self) -> Due {
        match self.time {
            Some(t) => Due::Time(self.date.and_time(t)),
            None => Due::Date(self.date),
        }
    }
}

/// Parse `input` relative to `today`. Empty or whitespace-only input is `Ok(None)`: the
/// field is to be cleared.
pub fn parse(input: &str, today: NaiveDate) -> Result<Option<DateInput>, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let fail = || format!("can't read date \"{input}\"");
    let lower = trimmed.to_lowercase();
    let tokens: Vec<&str> = lower.split_whitespace().collect();

    // `YYYY-MM-DD` or any other single-token date part, then an optional time.
    let (date, rest) = match date_part(tokens[0], today) {
        Some(d) => (d, &tokens[1..]),
        None => (today, &tokens[..]),
    };
    let time = if rest.is_empty() {
        None
    } else {
        Some(time_part(&rest.concat()).ok_or_else(fail)?)
    };
    Ok(Some(DateInput { date, time }))
}

fn date_part(tok: &str, today: NaiveDate) -> Option<NaiveDate> {
    match tok {
        "today" | "tod" => return Some(today),
        "tomorrow" | "tom" => return today.checked_add_days(Days::new(1)),
        _ => {}
    }
    if let Ok(d) = NaiveDate::parse_from_str(tok, "%Y-%m-%d") {
        return Some(d);
    }
    if let Some(w) = weekday(tok) {
        return (1..=7)
            .filter_map(|n| today.checked_add_days(Days::new(n)))
            .find(|d| d.weekday() == w);
    }
    let split = tok.find(|c: char| !c.is_ascii_digit())?;
    if split == 0 {
        return None;
    }
    let n: u32 = tok[..split].parse().ok()?;
    match &tok[split..] {
        "st" | "nd" | "rd" | "th" => ordinal(n, today),
        unit if n > 0 => relative(n, unit, today),
        _ => None,
    }
}

fn relative(n: u32, unit: &str, today: NaiveDate) -> Option<NaiveDate> {
    match unit {
        "d" | "day" | "days" => today.checked_add_days(Days::new(n.into())),
        "w" | "week" | "weeks" => today.checked_add_days(Days::new(u64::from(n) * 7)),
        "m" | "month" | "months" => today.checked_add_months(Months::new(n)),
        "y" | "year" | "years" => today.checked_add_months(Months::new(n.checked_mul(12)?)),
        _ => None,
    }
}

/// The first date after `today` whose day of month is `n`.
fn ordinal(n: u32, today: NaiveDate) -> Option<NaiveDate> {
    if !(1..=31).contains(&n) {
        return None;
    }
    let first = today.with_day(1)?;
    (0..=12)
        .filter_map(|m| first.checked_add_months(Months::new(m)))
        .filter_map(|month| month.with_day(n))
        .find(|d| *d > today)
}

fn weekday(tok: &str) -> Option<Weekday> {
    Some(match tok {
        "sun" | "sunday" => Weekday::Sun,
        "mon" | "monday" => Weekday::Mon,
        "tue" | "tues" | "tuesday" => Weekday::Tue,
        "wed" | "wednesday" => Weekday::Wed,
        "thu" | "thur" | "thurs" | "thursday" => Weekday::Thu,
        "fri" | "friday" => Weekday::Fri,
        "sat" | "saturday" => Weekday::Sat,
        _ => return None,
    })
}

/// `midnight`, `noon`, `H:MM` (24-hour) or `H[[:]MM](a|am|p|pm)` (12-hour).
fn time_part(s: &str) -> Option<NaiveTime> {
    match s {
        "midnight" => return NaiveTime::from_hms_opt(0, 0, 0),
        "noon" => return NaiveTime::from_hms_opt(12, 0, 0),
        _ => {}
    }
    let (digits, pm) = if let Some(d) = s.strip_suffix("am").or_else(|| s.strip_suffix('a')) {
        (d, Some(false))
    } else if let Some(d) = s.strip_suffix("pm").or_else(|| s.strip_suffix('p')) {
        (d, Some(true))
    } else {
        (s, None)
    };
    // Only ASCII digits and one colon may remain; this also keeps byte splits on char
    // boundaries.
    if !digits.bytes().all(|b| b.is_ascii_digit() || b == b':') {
        return None;
    }
    let (h, m) = match digits.split_once(':') {
        Some((h, m)) if m.len() == 2 => (h, m),
        Some(_) => return None,
        None if pm.is_none() => return None,
        None if digits.len() > 2 => digits.split_at(digits.len() - 2),
        None => (digits, "00"),
    };
    if h.is_empty()
        || !h.bytes().all(|b| b.is_ascii_digit())
        || !m.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    let h = match pm {
        None => h,
        Some(_) if !(1..=12).contains(&h) => return None,
        Some(false) => h % 12,
        Some(true) => h % 12 + 12,
    };
    NaiveTime::from_hms_opt(h, m, 0)
}

/// `Fri 6 Nov`, `Fri 6 Nov 09:00`; the year is added when it is not `today`'s.
pub fn describe(d: &DateInput, today: NaiveDate) -> String {
    let mut s = d.date.format("%a %-d %b").to_string();
    if d.date.year() != today.year() {
        s.push_str(&format!(" {}", d.date.year()));
    }
    if let Some(t) = d.time {
        s.push_str(&t.format(" %H:%M").to_string());
    }
    s
}

/// The local date and optional wall time of `v` in `tz`. Date-only, floating, and values
/// zoned in `tz` itself are taken as written: resolving them first would move a time that
/// falls in a DST gap.
fn local_parts(v: &IcalDateTime, tz: Tz) -> (NaiveDate, Option<NaiveTime>) {
    match v {
        IcalDateTime::Date(d) => return (*d, None),
        IcalDateTime::Floating(n) => return (n.date(), Some(n.time())),
        IcalDateTime::Zoned { local, tzid } if tzid.parse::<Tz>() == Ok(tz) => {
            return (local.date(), Some(local.time()));
        }
        _ => {}
    }
    match v.resolve(tz).0 {
        Resolved::Date(d) => (d, None),
        Resolved::Instant(t) => {
            let t = t.with_timezone(&tz).naive_local();
            (t.date(), Some(t.time()))
        }
    }
}

/// The form text for an existing value: `2026-11-06` or `2026-11-06 09:00`, in `tz` local
/// time. [`parse`] reads it back to the same date and time.
pub fn input_text(v: &IcalDateTime, tz: Tz) -> String {
    match local_parts(v, tz) {
        (d, None) => d.format("%Y-%m-%d").to_string(),
        (d, Some(t)) => format!("{} {}", d.format("%Y-%m-%d"), t.format("%H:%M")),
    }
}

/// A due-date shortcut key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DueShift {
    Today,
    Tomorrow,
    Days(i64),
}

/// The due date after a shortcut. `Today`/`Tomorrow` set the date; `Days(n)` moves the current
/// local date (today when there is none). A timed due keeps its local wall time as floating
/// time, the way 2Do writes it.
pub fn shift_due(current: Option<&IcalDateTime>, tz: Tz, today: NaiveDate, s: DueShift) -> Due {
    let (base, time) = match current {
        Some(v) => local_parts(v, tz),
        None => (today, None),
    };
    let date = match s {
        DueShift::Today => today,
        DueShift::Tomorrow => today + chrono::Duration::days(1),
        DueShift::Days(n) => base + chrono::Duration::days(n),
    };
    DateInput { date, time }.due()
}
