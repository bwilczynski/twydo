//! Human-readable text for dates and alerts. Pure functions of their inputs.

use chrono::{DateTime, Datelike, Duration};
use chrono_tz::Tz;

use crate::ical::value::{IcalDateTime, Resolved};
use crate::model::{Alarm, Task, Trigger};

/// A due/start value relative to `now`: `7 days ago`, `Yesterday`, `Today`, `Tomorrow`, a weekday
/// abbreviation for 2-6 days ahead, `6 Nov` within `now`'s year, else `6 Nov 2027`. Timed values
/// (in `now`'s zone) append ` HH:MM`.
pub fn relative_date(d: &IcalDateTime, now: DateTime<Tz>) -> String {
    let (day, time) = match d.resolve(now.timezone()).0 {
        Resolved::Date(day) => (day, None),
        Resolved::Instant(t) => (t.date_naive(), Some(t.format("%H:%M").to_string())),
    };
    let days = (day - now.date_naive()).num_days();
    let text = match days {
        i64::MIN..=-2 => format!("{} days ago", -days),
        -1 => "Yesterday".to_string(),
        0 => "Today".to_string(),
        1 => "Tomorrow".to_string(),
        2..=6 => day.format("%a").to_string(),
        _ if day.year() == now.year() => day.format("%-d %b").to_string(),
        _ => day.format("%-d %b %Y").to_string(),
    };
    match time {
        Some(t) => format!("{text} {t}"),
        None => text,
    }
}

/// One alert in words: `15 min before due`, `at start`, `at 09:00 6 Nov` (absolute, in `tz`).
/// A relative alert is anchored to the end (due) when marked so, or when the task has no start;
/// otherwise to the start.
pub fn describe_alarm(a: &Alarm, task: &Task, tz: Tz) -> String {
    match &a.trigger {
        Trigger::Absolute(t) => t.with_timezone(&tz).format("at %H:%M %-d %b").to_string(),
        Trigger::Relative {
            offset,
            related_end,
        } => {
            let anchor = if *related_end || task.start.is_none() {
                "due"
            } else {
                "start"
            };
            if offset.is_zero() {
                return format!("at {anchor}");
            }
            let dir = if *offset < Duration::zero() {
                "before"
            } else {
                "after"
            };
            format!("{} {dir} {anchor}", span(offset.abs()))
        }
    }
}

/// The largest unit that divides the duration exactly: days, hours, else minutes.
fn span(d: Duration) -> String {
    let mins = d.num_minutes();
    if mins != 0 && mins % 1440 == 0 {
        format!("{} d", mins / 1440)
    } else if mins != 0 && mins % 60 == 0 {
        format!("{} h", mins / 60)
    } else {
        format!("{mins} min")
    }
}
