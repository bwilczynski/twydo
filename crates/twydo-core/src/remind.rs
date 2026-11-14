//! Reminder trigger instants and eligibility. Pure: no clock, file or process access.

use std::collections::BTreeMap;

use chrono::{
    DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, SecondsFormat, TimeZone, Utc,
};
use chrono_tz::Tz;

use crate::config::Config;
use crate::format::{describe_alarm, relative_date};
use crate::ical::value::{IcalDateTime, Resolved};
use crate::model::{Status, Task, Trigger};
use crate::views::Snapshot;

/// What fired at an instant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cause {
    Due,
    Alert { index: usize },
}

/// One instant of one task with everything that fires then (never empty), in the order
/// Due first, then alerts by index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timed {
    pub at: DateTime<Utc>,
    pub causes: Vec<Cause>,
}

/// Open, list neither hidden nor archived, not a child of a checklist.
pub fn eligible(snap: &Snapshot, idx: usize) -> bool {
    let prefs = snap.prefs(idx);
    snap.tasks[idx].status == Status::Open && !prefs.hidden && !prefs.archived && !snap.is_item(idx)
}

/// The instant of a local wall-clock time in `tz`: the earlier of an ambiguous time, one hour
/// later when the time does not exist (a DST gap).
fn local_instant(local: NaiveDateTime, tz: Tz) -> DateTime<Utc> {
    tz.from_local_datetime(&local)
        .earliest()
        .or_else(|| {
            local
                .checked_add_signed(Duration::hours(1))
                .and_then(|later| tz.from_local_datetime(&later).earliest())
        })
        // Unreachable in practice: no zone's gap exceeds an hour, so one hour later always
        // exists. UTC is only a total fallback to avoid a panic.
        .map_or_else(|| local.and_utc(), |t| t.with_timezone(&Utc))
}

fn local_midnight(date: NaiveDate, tz: Tz) -> DateTime<Utc> {
    local_instant(date.and_hms_opt(0, 0, 0).expect("midnight exists"), tz)
}

fn instant(value: &IcalDateTime, tz: Tz) -> DateTime<Utc> {
    match value.resolve(tz).0 {
        Resolved::Instant(t) => t.with_timezone(&Utc),
        Resolved::Date(d) => local_midnight(d, tz),
    }
}

/// The task's timed triggers, coalesced by instant, ascending.
pub fn timed(task: &Task, tz: Tz) -> Vec<Timed> {
    let mut found: Vec<(DateTime<Utc>, Cause)> = Vec::new();
    if let Some(
        due @ (IcalDateTime::Floating(_) | IcalDateTime::Utc(_) | IcalDateTime::Zoned { .. }),
    ) = &task.due
    {
        found.push((instant(due, tz), Cause::Due));
    }
    for alarm in &task.alarms {
        let at = match &alarm.trigger {
            Trigger::Absolute(at) => Some(*at),
            Trigger::Relative {
                offset,
                related_end,
            } => task
                .alert_anchor(*related_end)
                .and_then(|a| instant(a, tz).checked_add_signed(*offset)),
        };
        if let Some(at) = at {
            found.push((at, Cause::Alert { index: alarm.index }));
        }
    }
    // Stable sort keeps Due before alerts, and alerts in index order, within an instant.
    found.sort_by_key(|(at, _)| *at);
    let mut out: Vec<Timed> = Vec::new();
    for (at, cause) in found {
        match out.last_mut() {
            Some(last) if last.at == at => last.causes.push(cause),
            _ => out.push(Timed {
                at,
                causes: vec![cause],
            }),
        }
    }
    out
}

/// What the planner needs from the configuration.
#[derive(Debug, Clone)]
pub struct Settings {
    pub tz: Tz,
    pub morning: NaiveTime,
    pub catch_up: Duration,
}

impl Settings {
    pub fn from_config(cfg: &Config) -> Settings {
        Settings {
            tz: cfg.tz(),
            morning: cfg.morning_time,
            catch_up: Duration::hours(i64::from(cfg.notifications.catch_up_hours)),
        }
    }
}

/// Key -> when it was sent.
pub type Sent = BTreeMap<String, DateTime<Utc>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub key: String,
    pub uid: Option<String>,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub due: Vec<Delivery>,
    /// Keys to record as handled without delivering anything (an empty digest).
    pub skip: Vec<String>,
    pub next_wake: DateTime<Utc>,
}

const DIGEST_LINES: usize = 5;

fn title_of(task: &Task) -> String {
    if task.summary.trim().is_empty() {
        "(untitled)".to_string()
    } else {
        task.summary.clone()
    }
}

fn timed_body(snap: &Snapshot, task: &Task, entry: &Timed, now: DateTime<Utc>, tz: Tz) -> String {
    let mut parts = Vec::new();
    let due_at = task.due.as_ref().map(|d| instant(d, tz));
    if due_at.is_some() && !entry.causes.contains(&Cause::Due) && due_at != Some(entry.at) {
        let first = entry.causes.iter().find_map(|c| match c {
            Cause::Alert { index } => task.alarms.iter().find(|a| a.index == *index),
            Cause::Due => None,
        });
        if let Some(alarm) = first {
            parts.push(describe_alarm(alarm, task, tz));
        }
    }
    if let Some(due) = &task.due {
        parts.push(format!(
            "Due {}",
            relative_date(due, now.with_timezone(&tz))
        ));
    }
    let list = snap
        .collections
        .iter()
        .find(|c| c.id == task.collection)
        .and_then(|c| c.display_name.clone())
        .unwrap_or_else(|| task.collection.clone());
    parts.push(list);
    parts.join(" · ")
}

/// The digest for `today`: due today, and overdue (a due date before today, or a due time
/// before `morning`, today's morning instant).
fn digest(snap: &Snapshot, morning: DateTime<Utc>, today: NaiveDate, tz: Tz) -> Option<Delivery> {
    let mut due_today: Vec<(DateTime<Utc>, String)> = Vec::new();
    let mut overdue: Vec<(DateTime<Utc>, String)> = Vec::new();
    for (idx, task) in snap.tasks.iter().enumerate() {
        let Some(due) = &task.due else { continue };
        if !eligible(snap, idx) {
            continue;
        }
        let at = instant(due, tz);
        let entry = (at, title_of(task));
        match due {
            IcalDateTime::Date(d) if *d == today => due_today.push(entry),
            IcalDateTime::Date(d) if *d < today => overdue.push(entry),
            IcalDateTime::Date(_) => {}
            _ if at < morning => overdue.push(entry),
            _ => {}
        }
    }
    if due_today.is_empty() && overdue.is_empty() {
        return None;
    }
    due_today.sort();
    overdue.sort();
    let title = match (due_today.len(), overdue.len()) {
        (n, 0) => format!("{n} due today"),
        (0, m) => format!("{m} overdue"),
        (n, m) => format!("{n} due today, {m} overdue"),
    };
    let mut lines: Vec<String> = due_today
        .into_iter()
        .chain(overdue)
        .map(|(_, t)| t)
        .collect();
    let more = lines.len().saturating_sub(DIGEST_LINES);
    lines.truncate(DIGEST_LINES);
    if more > 0 {
        lines.push(format!("+{more} more"));
    }
    Some(Delivery {
        key: format!("d:{today}"),
        uid: None,
        title,
        body: lines.join("\n"),
    })
}

/// What to deliver now and when to wake next. Pure: `now` and `sent` are inputs.
pub fn plan(snap: &Snapshot, sent: &Sent, now: DateTime<Utc>, s: &Settings) -> Plan {
    let mut triggers: Vec<(DateTime<Utc>, &str, usize, Timed)> = Vec::new();
    for (idx, task) in snap.tasks.iter().enumerate() {
        if !eligible(snap, idx) {
            continue;
        }
        for entry in timed(task, s.tz) {
            triggers.push((entry.at, task.uid.as_str(), idx, entry));
        }
    }
    triggers.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));

    let mut due = Vec::new();
    let mut next_wake: Option<DateTime<Utc>> = None;
    for (at, uid, idx, entry) in &triggers {
        if *at > now {
            next_wake = Some(next_wake.map_or(*at, |n| n.min(*at)));
            continue;
        }
        // An overflow (an absurd `catch_up_hours`) means no lower bound.
        if now
            .checked_sub_signed(s.catch_up)
            .is_some_and(|from| *at <= from)
        {
            continue;
        }
        let key = format!("t:{uid}@{}", at.to_rfc3339_opts(SecondsFormat::Secs, true));
        if sent.contains_key(&key) {
            continue;
        }
        let task = &snap.tasks[*idx];
        due.push(Delivery {
            key,
            uid: Some((*uid).to_string()),
            title: title_of(task),
            body: timed_body(snap, task, entry, now, s.tz),
        });
    }

    let local = now.with_timezone(&s.tz);
    let today = local.date_naive();
    let mut morning = local_instant(today.and_time(s.morning), s.tz);
    // The digest is decided once per local date, at the first plan at or past the morning
    // instant: an empty one is skipped (recorded, not delivered), never sent later that day.
    let mut skip = Vec::new();
    let digest_key = format!("d:{today}");
    if now >= morning && !sent.contains_key(&digest_key) {
        match digest(snap, morning, today, s.tz) {
            Some(d) => due.push(d),
            None => skip.push(digest_key),
        }
    }
    if morning <= now {
        let tomorrow = today.succ_opt().unwrap_or(today);
        morning = local_instant(tomorrow.and_time(s.morning), s.tz);
    }
    let next_wake = next_wake.map_or(morning, |n| n.min(morning));
    Plan {
        due,
        skip,
        next_wake,
    }
}
