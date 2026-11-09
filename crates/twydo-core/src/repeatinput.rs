//! Typed repeat rules: `daily`, `2w`, `mon thu`, `workdays`, `last day`, `2nd tue`, with the
//! modifiers `after done`, `until <date>` and `10 times` (spec 2026-11-03 §3.5).
//!
//! `parse` is a pure function of its arguments so the same grammar can serve natural-language
//! input later. The encodings follow what 2Do itself writes (`docs/reference/phase0/plan7-capture.md`)
//! and, where the phone cannot create a shape, the port mapping in `01-features.md` §6.

use std::str::FromStr;

use chrono::{NaiveDate, Weekday};

use crate::dateinput;
use crate::ical::value::IcalDateTime;
use crate::model::repeat::{self, End, Freq, RRule};

/// A repeat rule plus 2Do's "from completion" flag (which is not part of the RRULE).
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct RepeatSpec {
    pub rule: RRule,
    pub from_completion: bool,
}

/// Shown as the hint when the field is empty and focused.
pub const EXAMPLES: &str =
    "daily · weekly · 2w · monthly · mon thu · workdays · last day · 2nd tue · yearly";

const WEEKDAY_ONLY: [Weekday; 5] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
];
const WEEKEND_ONLY: [Weekday; 2] = [Weekday::Sat, Weekday::Sun];

fn rule(freq: Freq, interval: u32) -> RRule {
    RRule {
        freq,
        interval,
        by_day: vec![],
        by_month_day: vec![],
        by_month: vec![],
        by_set_pos: vec![],
        end: End::Never,
        unknown: vec![],
    }
}

fn weekday_name(w: Weekday) -> &'static str {
    ["mon", "tue", "wed", "thu", "fri", "sat", "sun"][w.num_days_from_monday() as usize]
}

fn weekday_of(tok: &str) -> Option<Weekday> {
    // chrono also accepts forms such as "1"; only names are wanted here.
    if tok.chars().all(|c| c.is_ascii_alphabetic()) {
        Weekday::from_str(tok).ok()
    } else {
        None
    }
}

/// `1st`..`5th` and `last` as an RRULE position; `Err` for an ordinal out of range.
fn position_of(tok: &str) -> Option<Result<i8, String>> {
    if tok == "last" {
        return Some(Ok(-1));
    }
    let digits = tok.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let suffix = &tok[digits.len()..];
    let n: u32 = digits.parse().ok()?;
    if !matches!(suffix, "st" | "nd" | "rd" | "th") {
        return None;
    }
    Some(match n {
        1..=5 => Ok(n as i8),
        _ => Err(format!(
            "\"{tok}\" is not a position in the month (use 1st to 5th or last)"
        )),
    })
}

fn position_text(p: i8) -> &'static str {
    match p {
        -1 => "last",
        1 => "1st",
        2 => "2nd",
        3 => "3rd",
        4 => "4th",
        _ => "5th",
    }
}

fn unit_freq(word: &str) -> Option<Freq> {
    match word.trim_end_matches('s') {
        "day" => Some(Freq::Daily),
        "week" => Some(Freq::Weekly),
        "month" => Some(Freq::Monthly),
        "year" => Some(Freq::Yearly),
        _ => None,
    }
}

fn interval_of(num: &str) -> Result<u32, String> {
    match num.parse::<u32>() {
        Ok(n) if n >= 1 => Ok(n),
        Ok(_) => Err("the interval must be at least 1".into()),
        Err(_) => Err(format!("can't read \"{num}\" as a number")),
    }
}

/// A frequency with an interval: `daily`, `biweekly`, `3d`, `every 3 days`, `month`.
/// `Ok(None)` when the tokens are not a period at all.
fn period(tokens: &[&str]) -> Result<Option<(Freq, u32)>, String> {
    Ok(match tokens {
        ["daily"] => Some((Freq::Daily, 1)),
        ["weekly"] => Some((Freq::Weekly, 1)),
        ["biweekly"] => Some((Freq::Weekly, 2)),
        ["monthly"] => Some((Freq::Monthly, 1)),
        ["yearly"] => Some((Freq::Yearly, 1)),
        [unit] if unit_freq(unit).is_some() => Some((unit_freq(unit).unwrap(), 1)),
        // A number and a unit letter: split off the last character, not the last byte, so
        // text in another script (`2週`) is not a period rather than a panic.
        [short]
            if let Some(num) = short.strip_suffix(|c: char| c.is_ascii_alphabetic())
                && num.starts_with(|c: char| c.is_ascii_digit()) =>
        {
            let freq = match &short[num.len()..] {
                "d" => Freq::Daily,
                "w" => Freq::Weekly,
                "m" => Freq::Monthly,
                "y" => Freq::Yearly,
                _ => return Ok(None),
            };
            Some((freq, interval_of(num)?))
        }
        [num, unit] if num.chars().all(|c| c.is_ascii_digit()) && unit_freq(unit).is_some() => {
            Some((unit_freq(unit).unwrap(), interval_of(num)?))
        }
        _ => None,
    })
}

fn weekday_set(tokens: &[&str], work_days: &[Weekday]) -> Option<Vec<Weekday>> {
    let mut set: Vec<Weekday> = vec![];
    let mut any = false;
    for tok in tokens.iter().filter(|t| **t != "and") {
        any = true;
        if *tok == "workdays" {
            set.extend_from_slice(work_days);
        } else {
            set.push(weekday_of(tok)?);
        }
    }
    if !any || set.is_empty() {
        return None;
    }
    set.sort_by_key(|w| w.num_days_from_monday());
    set.dedup();
    Some(set)
}

/// Rules that name a position in the month: `last day`, `2nd tue`, `1st weekday`.
fn monthly_position(tokens: &[&str]) -> Result<Option<RRule>, String> {
    let Some(first) = tokens.first() else {
        return Ok(None);
    };
    let Some(pos) = position_of(first) else {
        return Ok(None);
    };
    let pos = pos?;
    let mut r = rule(Freq::Monthly, 1);
    match &tokens[1..] {
        ["day", rest @ ..] if pos == -1 => {
            r.by_month_day = vec![-1];
            match rest {
                [] => {}
                ["every", spec @ ..] => match period(spec)? {
                    Some((Freq::Monthly, n)) => r.interval = n,
                    _ => return Err("\"last day\" can only repeat every N months".into()),
                },
                _ => return Err("after \"last day\" only \"every 2m\" can follow".into()),
            }
        }
        ["weekday"] => {
            r.by_day = WEEKDAY_ONLY.iter().map(|w| (None, *w)).collect();
            r.by_set_pos = vec![pos];
        }
        ["weekend", "day"] => {
            r.by_day = WEEKEND_ONLY.iter().map(|w| (None, *w)).collect();
            r.by_set_pos = vec![pos];
        }
        [name] => match weekday_of(name) {
            Some(w) => r.by_day = vec![(Some(pos), w)],
            None => return Err(format!("can't read \"{name}\" after the position")),
        },
        _ => return Err("after a position use a weekday, \"weekday\" or \"weekend day\"".into()),
    }
    Ok(Some(r))
}

/// Parse typed repeat text. `Ok(None)` means "no repeat" (empty or `none`).
pub fn parse(
    input: &str,
    today: NaiveDate,
    work_days: &[Weekday],
) -> Result<Option<RepeatSpec>, String> {
    let lower = input.to_lowercase();
    let mut tokens: Vec<&str> = lower
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() || tokens == ["none"] {
        return Ok(None);
    }

    // Modifiers come last; strip them from the end until none is left.
    let mut from_completion = false;
    let mut end = End::Never;
    loop {
        let n = tokens.len();
        if n >= 2 && tokens[n - 2..] == ["after", "done"] {
            from_completion = true;
            tokens.truncate(n - 2);
        } else if n >= 1 && matches!(tokens[n - 1], "times" | "time") {
            let count = n
                .checked_sub(2)
                .and_then(|i| tokens[i].parse::<u32>().ok())
                .filter(|c| *c >= 1)
                .ok_or("\"times\" needs a count of at least 1, like \"10 times\"")?;
            end = End::Count(count);
            tokens.truncate(n - 2);
        } else if let Some(i) = tokens.iter().rposition(|t| *t == "until") {
            let text = tokens[i + 1..].join(" ");
            if text.is_empty() {
                return Err("\"until\" needs a date, like \"until 2027-06-01\"".into());
            }
            let d = dateinput::parse(&text, today)?
                .ok_or_else(|| "\"until\" needs a date".to_string())?;
            end = End::Until(IcalDateTime::Date(d.date));
            tokens.truncate(i);
        } else {
            break;
        }
    }

    let fail = |tokens: &[&str]| {
        let mut msg = format!("can't read repeat \"{}\"", input.trim());
        if tokens.iter().any(|t| position_of(t).is_some()) {
            msg.push_str(" (the repeat takes no day of the month; set it in Due)");
        }
        msg
    };
    if tokens.is_empty() {
        return Err("a repeat modifier needs a rule before it, like \"weekly after done\"".into());
    }
    let body: &[&str] = if tokens[0] == "every" {
        &tokens[1..]
    } else {
        &tokens
    };
    if body.is_empty() {
        return Err("\"every\" needs a rule, like \"every 2 weeks\" or \"every mon\"".into());
    }

    let mut r = if let Some(r) = monthly_position(body)? {
        r
    } else if let Some((freq, n)) = period(body)? {
        rule(freq, n)
    } else if let Some(days) = weekday_set(body, work_days) {
        let mut r = rule(Freq::Weekly, 1);
        r.by_day = days.into_iter().map(|w| (None, w)).collect();
        r
    } else {
        return Err(fail(body));
    };
    r.end = end;
    Ok(Some(RepeatSpec {
        rule: r,
        from_completion,
    }))
}

/// Preview line: 2Do's own wording plus which date the next occurrence follows.
pub fn describe(spec: &RepeatSpec, anchor: Option<NaiveDate>) -> String {
    // `repeat::describe` appends ", from completion" itself; this line says it after the dot.
    let base = repeat::describe(&spec.rule, anchor, false);
    let tail = if spec.from_completion {
        "after done"
    } else {
        "from due date"
    };
    format!("{base} · {tail}")
}

fn period_text(freq: Freq, n: u32) -> String {
    let (word, letter) = match freq {
        Freq::Daily => ("daily", 'd'),
        Freq::Weekly => ("weekly", 'w'),
        Freq::Monthly => ("monthly", 'm'),
        Freq::Yearly => ("yearly", 'y'),
    };
    if n == 1 {
        word.into()
    } else {
        format!("{n}{letter}")
    }
}

fn rule_text(r: &RRule) -> Option<String> {
    let days = |r: &RRule| r.by_day.iter().map(|(_, w)| *w).collect::<Vec<_>>();
    let is_set = |r: &RRule, set: &[Weekday]| {
        r.by_day.len() == set.len() && set.iter().all(|d| days(r).contains(d))
    };
    match (r.freq, r.by_day.as_slice(), r.by_month_day.as_slice()) {
        (Freq::Weekly, [_, ..], []) if r.interval == 1 && r.by_set_pos.is_empty() => Some(
            r.by_day
                .iter()
                .map(|(_, w)| weekday_name(*w))
                .collect::<Vec<_>>()
                .join(" "),
        ),
        (Freq::Monthly, [], [-1]) => Some(if r.interval == 1 {
            "last day".into()
        } else {
            format!("last day every {}m", r.interval)
        }),
        (Freq::Monthly, [(Some(p), w)], []) if r.interval == 1 => {
            Some(format!("{} {}", position_text(*p), weekday_name(*w)))
        }
        (Freq::Monthly, [_, ..], []) if r.interval == 1 && r.by_set_pos.len() == 1 => {
            let p = position_text(r.by_set_pos[0]);
            if is_set(r, &WEEKDAY_ONLY) {
                Some(format!("{p} weekday"))
            } else if is_set(r, &WEEKEND_ONLY) {
                Some(format!("{p} weekend day"))
            } else {
                None
            }
        }
        (_, [], []) => Some(period_text(r.freq, r.interval)),
        _ => None,
    }
}

/// The typed form of a stored rule, to prefill the field; `None` when the syntax cannot say it.
pub fn to_text(rrule: &str, from_completion: bool) -> Option<String> {
    let rule = repeat::parse_rrule(rrule).ok()?;
    let mut text = rule_text(&rule)?;
    match &rule.end {
        End::Never => {}
        End::Count(n) => text.push_str(&format!(" {n} times")),
        End::Until(IcalDateTime::Date(d)) => text.push_str(&format!(" until {d}")),
        End::Until(_) => return None,
    }
    if from_completion {
        text.push_str(" after done");
    }
    // Only offer text that parses back to exactly this rule.
    let back = parse(&text, NaiveDate::from_ymd_opt(2000, 1, 1)?, &WEEKDAY_ONLY).ok()??;
    (back.rule == rule && back.from_completion == from_completion).then_some(text)
}
