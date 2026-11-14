//! Typed alert offsets: `15m`, `4h`, `1d`, `2w`, `1h30m` (also `15 min`, `1h 30m`, `2 hours`),
//! optionally followed by `before`; empty or `none` removes the alert. `at due` names 2Do's
//! automatic alert, which Twydo shows and removes but does not add. Days and weeks are fixed 24 h / 168 h, as in an
//! iCalendar `TRIGGER`.

use crate::model::AlertOffset;

pub const EXAMPLES: &str = "15m, 4h, 1d, 2w, 1h30m, none";

fn error(input: &str) -> String {
    format!("not an alert: {input:?}; try {EXAMPLES}")
}

/// Unit letters from largest to smallest, each a number of minutes.
const UNITS: [(char, u32); 4] = [('w', 10080), ('d', 1440), ('h', 60), ('m', 1)];

/// How the card shows 2Do's automatic at-due-time alert.
pub const AT_DUE: &str = "at due";

/// Why `at due` can't be typed to add an alert.
pub const AT_DUE_NOT_ADDED: &str =
    "at due is 2Do's automatic alert and comes with a due time; type a time before due, e.g. 15m";

/// Whether `input` names the automatic at-due-time alert: `at due`, `due`, `at due time`.
pub fn is_at_due(input: &str) -> bool {
    let words: Vec<String> = input
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect();
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    matches!(
        words.as_slice(),
        ["at", "due"] | ["due"] | ["at", "due", "time"]
    )
}

/// Parse an alert; `Ok(None)` removes it.
pub fn parse(input: &str) -> Result<Option<AlertOffset>, String> {
    let lower = input.trim().to_ascii_lowercase();
    if lower.is_empty() || lower == "none" {
        return Ok(None);
    }
    if is_at_due(&lower) {
        return Err(AT_DUE_NOT_ADDED.into());
    }
    let mut words: Vec<&str> = lower.split_whitespace().collect();
    if words.last() == Some(&"before") {
        words.pop();
    }
    // `15 min`, `1h 30m`: a space may follow a number or a unit, never sit between digits.
    if words.is_empty()
        || words.windows(2).any(|w| {
            w[0].ends_with(|c: char| c.is_ascii_digit())
                && w[1].starts_with(|c: char| c.is_ascii_digit())
        })
    {
        return Err(error(input));
    }
    let joined = words.concat();
    let term = joined.as_str();
    let mut total: u32 = 0;
    let mut last_unit = usize::MAX;
    let mut rest = term;
    while !rest.is_empty() {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if digits == 0 {
            return Err(error(input));
        }
        let n: u32 = rest[..digits].parse().map_err(|_| error(input))?;
        rest = &rest[digits..];
        let letters = rest.chars().take_while(char::is_ascii_alphabetic).count();
        let word = &rest[..letters];
        rest = &rest[letters..];
        let unit = match word {
            "w" | "week" | "weeks" => 0,
            "d" | "day" | "days" => 1,
            "h" | "hr" | "hrs" | "hour" | "hours" => 2,
            "m" | "min" | "mins" | "minute" | "minutes" => 3,
            _ => return Err(error(input)),
        };
        // Largest unit first, each once: `1h30m`, never `30m1h` or `15m15m`.
        if last_unit != usize::MAX && unit <= last_unit {
            return Err(error(input));
        }
        last_unit = unit;
        total = n
            .checked_mul(UNITS[unit].1)
            .and_then(|v| total.checked_add(v))
            .ok_or_else(|| error(input))?;
    }
    AlertOffset::new(total)
        .map(Some)
        .ok_or_else(|| error(input))
}

/// The largest unit that divides the offset evenly: `15m`, `90m`, `4h`, `1d`, `26h`, `1w`.
pub fn format(offset: AlertOffset) -> String {
    let m = offset.minutes();
    let (letter, size) = UNITS
        .into_iter()
        .find(|(_, size)| m.is_multiple_of(*size))
        .unwrap_or(('m', 1));
    format!("{}{letter}", m / size)
}
