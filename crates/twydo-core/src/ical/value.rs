//! Typed views of iCalendar property values: date-times, durations and text.
//!
//! Parsing never changes the stored [`Property`]; these are read-only interpretations.

use chrono::{
    DateTime, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, Offset, TimeZone, Utc,
};
use chrono_tz::Tz;
use thiserror::Error;

use super::{Param, Property};

/// A DATE / DATE-TIME value as written, before any timezone resolution.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub enum IcalDateTime {
    Date(NaiveDate),
    Floating(NaiveDateTime),
    Utc(DateTime<Utc>),
    Zoned { local: NaiveDateTime, tzid: String },
}

/// A value resolved against a concrete timezone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    Date(NaiveDate),
    Instant(DateTime<Tz>),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValueError {
    #[error("invalid date or date-time value {0:?}")]
    DateTime(String),
    #[error("invalid duration value {0:?}")]
    Duration(String),
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    if s.len() != 8 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    NaiveDate::from_ymd_opt(
        s[0..4].parse().ok()?,
        s[4..6].parse().ok()?,
        s[6..8].parse().ok()?,
    )
}

fn parse_local(s: &str) -> Option<NaiveDateTime> {
    let (d, t) = s.split_once('T')?;
    if t.len() != 6 || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let time = NaiveTime::from_hms_opt(
        t[0..2].parse().ok()?,
        t[2..4].parse().ok()?,
        t[4..6].parse().ok()?,
    )?;
    Some(parse_date(d)?.and_time(time))
}

/// Interpret a DUE/DTSTART/COMPLETED-style property.
///
/// `VALUE=DATE` (or a bare 8-digit value) is a date; a trailing `Z` is UTC (even with a
/// `TZID`); otherwise `TZID` makes it zoned and no `TZID` makes it floating.
pub fn parse_date_time(p: &Property) -> Result<IcalDateTime, ValueError> {
    let v = p.value.trim();
    let err = || ValueError::DateTime(p.value.clone());
    let is_date = p
        .param("VALUE")
        .map(|x| x.eq_ignore_ascii_case("DATE"))
        .unwrap_or(false)
        || (p.param("VALUE").is_none() && v.len() == 8);
    if is_date {
        return parse_date(v).map(IcalDateTime::Date).ok_or_else(err);
    }
    if let Some(body) = v.strip_suffix('Z') {
        let n = parse_local(body).ok_or_else(err)?;
        return Ok(IcalDateTime::Utc(n.and_utc()));
    }
    let local = parse_local(v).ok_or_else(err)?;
    Ok(match p.param("TZID") {
        Some(tzid) => IcalDateTime::Zoned {
            local,
            tzid: tzid.to_string(),
        },
        None => IcalDateTime::Floating(local),
    })
}

/// Resolve a local wall-clock time in `tz`. A DST gap moves forward to the first valid
/// instant; an ambiguous time takes the earlier offset.
fn resolve_in(tz: Tz, local: NaiveDateTime) -> DateTime<Tz> {
    match tz.from_local_datetime(&local) {
        LocalResult::Single(t) => t,
        LocalResult::Ambiguous(earlier, _) => earlier,
        LocalResult::None => {
            // Interpret with the offset in force just before the gap, which lands on the
            // first valid instant after it (e.g. 02:30 EST -> 03:30 EDT).
            let before = match tz.from_local_datetime(&(local - Duration::days(1))) {
                LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => t.offset().fix(),
                LocalResult::None => chrono::FixedOffset::east_opt(0).unwrap(),
            };
            let utc = before
                .from_local_datetime(&local)
                .single()
                .map(|t| t.with_timezone(&Utc))
                .unwrap_or_else(|| local.and_utc());
            utc.with_timezone(&tz)
        }
    }
}

impl IcalDateTime {
    /// Resolve against `local` (the user's zone). Floating values and unknown TZIDs use
    /// `local`; the latter also yields a warning.
    pub fn resolve(&self, local: Tz) -> (Resolved, Option<String>) {
        match self {
            IcalDateTime::Date(d) => (Resolved::Date(*d), None),
            IcalDateTime::Utc(t) => (Resolved::Instant(t.with_timezone(&local)), None),
            IcalDateTime::Floating(n) => (Resolved::Instant(resolve_in(local, *n)), None),
            IcalDateTime::Zoned { local: n, tzid } => match tzid.parse::<Tz>() {
                Ok(tz) => (
                    Resolved::Instant(resolve_in(tz, *n).with_timezone(&tz)),
                    None,
                ),
                Err(_) => (
                    Resolved::Instant(resolve_in(local, *n)),
                    Some(format!(
                        "unknown TZID {tzid:?}; interpreted in the local timezone"
                    )),
                ),
            },
        }
    }

    /// The calendar date in `local`. Date-only values are returned as written.
    pub fn local_date(&self, local: Tz) -> NaiveDate {
        match self.resolve(local).0 {
            Resolved::Date(d) => d,
            Resolved::Instant(t) => t.date_naive(),
        }
    }

    pub fn is_date_only(&self) -> bool {
        matches!(self, IcalDateTime::Date(_))
    }
}

/// RFC 5545 `dur-value`: `[+-]P(nW | nD [T..] | T..)`.
pub fn parse_duration(s: &str) -> Result<Duration, ValueError> {
    let err = || ValueError::Duration(s.to_string());
    let (neg, rest) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let rest = rest.strip_prefix('P').ok_or_else(err)?;
    let mut total = Duration::zero();
    let mut seen_any = false;
    let mut in_time = false;
    let mut num = String::new();
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        if c == 'T' {
            if in_time || !num.is_empty() {
                return Err(err());
            }
            in_time = true;
            if chars.peek().is_none() {
                return Err(err());
            }
            continue;
        }
        let n: i64 = num.parse().map_err(|_| err())?;
        num.clear();
        let unit = match (c, in_time) {
            ('W', false) => Duration::try_weeks(n),
            ('D', false) => Duration::try_days(n),
            ('H', true) => Duration::try_hours(n),
            ('M', true) => Duration::try_minutes(n),
            ('S', true) => Duration::try_seconds(n),
            _ => return Err(err()),
        }
        .ok_or_else(err)?;
        total = total.checked_add(&unit).ok_or_else(err)?;
        seen_any = true;
    }
    if !num.is_empty() || !seen_any {
        return Err(err());
    }
    Ok(if neg { -total } else { total })
}

/// Undo RFC 5545 TEXT escaping: `\n`/`\N` newline, `\,` `\;` `\\`. Unknown escapes and a
/// trailing lone backslash are kept literally.
pub fn unescape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(e @ (',' | ';' | '\\')) => out.push(e),
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Escape TEXT: `\` `;` `,` and newline become `\\` `\;` `\,` `\n`. A `\r\n` pair is one newline.
pub fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\r' => {
                if it.peek() == Some(&'\n') {
                    it.next();
                }
                out.push_str("\\n");
            }
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

/// Parameters and value text for a DATE / DATE-TIME property, as 2Do writes them.
pub fn format_date_time(v: &IcalDateTime) -> (Vec<Param>, String) {
    const DT: &str = "%Y%m%dT%H%M%S";
    match v {
        IcalDateTime::Date(d) => (
            vec![Param {
                name: "VALUE".into(),
                values: vec!["DATE".into()],
            }],
            d.format("%Y%m%d").to_string(),
        ),
        IcalDateTime::Floating(dt) => (vec![], dt.format(DT).to_string()),
        IcalDateTime::Utc(dt) => (vec![], format!("{}Z", dt.format(DT))),
        IcalDateTime::Zoned { local, tzid } => (
            vec![Param {
                name: "TZID".into(),
                values: vec![tzid.clone()],
            }],
            local.format(DT).to_string(),
        ),
    }
}
