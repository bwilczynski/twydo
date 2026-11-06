//! Reading VTODO components into [`Task`]s. Read side only: nothing here writes.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};

use super::meta::{MetaFormat, Metadata, parse_metadata};
use crate::ical::value::{IcalDateTime, parse_date_time, parse_duration, unescape_text};
use crate::ical::{Calendar, Component};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TaskKind {
    Task,
    Project,
    Checklist,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Open,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trigger {
    Relative { offset: Duration, related_end: bool },
    Absolute(DateTime<Utc>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alarm {
    pub trigger: Trigger,
    /// Position among the VTODO's VALARMs.
    pub index: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub uid: String,
    pub collection: String,
    pub path: PathBuf,
    pub summary: String,
    pub description: Option<String>,
    pub due: Option<IcalDateTime>,
    pub start: Option<IcalDateTime>,
    pub status: Status,
    pub completed: Option<IcalDateTime>,
    pub created: Option<IcalDateTime>,
    pub last_modified: Option<IcalDateTime>,
    /// Raw RRULE value; parsed elsewhere.
    pub rrule: Option<String>,
    pub parent: Option<String>,
    pub kind: TaskKind,
    pub alarms: Vec<Alarm>,
    pub meta: Metadata,
    pub from_completion: bool,
    pub warnings: Vec<String>,
    /// Reason edits must stay off for this task.
    pub blocked: Option<String>,
}

const METADATA_PROP: &str = "X-2DOAPP-METADATA";

/// Read the task a calendar resource describes. Returns no task when it holds no VTODO.
/// VEVENT/VJOURNAL components are ignored.
pub fn tasks_from_calendar(cal: &Calendar, collection: &str, path: &Path) -> Vec<Task> {
    let todos: Vec<&Component> = cal.root.children_named("VTODO").collect();
    let master = todos
        .iter()
        .find(|t| t.prop("RECURRENCE-ID").is_none())
        .or(todos.first());
    let Some(master) = master else {
        return Vec::new();
    };
    let mut task = read_todo(master, collection, path);
    if todos.len() > 1 || master.prop("RECURRENCE-ID").is_some() {
        task.blocked = Some("recurrence overrides are not supported".to_string());
    }
    vec![task]
}

fn date_field(c: &Component, name: &str, warnings: &mut Vec<String>) -> Option<IcalDateTime> {
    let p = c.prop(name)?;
    match parse_date_time(p) {
        Ok(v) => Some(v),
        Err(e) => {
            warnings.push(format!("invalid {name}: {e}"));
            None
        }
    }
}

fn text_field(c: &Component, name: &str) -> Option<String> {
    c.prop(name).map(|p| unescape_text(&p.value))
}

fn read_todo(c: &Component, collection: &str, path: &Path) -> Task {
    let mut warnings = Vec::new();

    let uid = match c.prop("UID") {
        Some(p) => p.value.trim().to_string(),
        None => {
            warnings.push("missing UID".to_string());
            String::new()
        }
    };
    let summary = text_field(c, "SUMMARY").unwrap_or_else(|| {
        warnings.push("missing SUMMARY".to_string());
        String::new()
    });

    let due = date_field(c, "DUE", &mut warnings);
    let dtstart = date_field(c, "DTSTART", &mut warnings);
    let completed = date_field(c, "COMPLETED", &mut warnings);
    let created = date_field(c, "CREATED", &mut warnings);
    let last_modified = date_field(c, "LAST-MODIFIED", &mut warnings);

    let status_prop = c
        .prop("STATUS")
        .map(|p| p.value.trim().to_ascii_uppercase());
    // An explicit STATUS wins; COMPLETED alone implies completion only when STATUS is absent.
    let has_completed = c.prop("COMPLETED").is_some();
    let status = match status_prop.as_deref() {
        Some("CANCELLED") => Status::Cancelled,
        Some("COMPLETED") => Status::Completed,
        Some(_) => {
            if has_completed {
                warnings.push("STATUS and COMPLETED disagree".to_string());
            }
            Status::Open
        }
        None if has_completed => Status::Completed,
        None => Status::Open,
    };

    let parent = c
        .props_named("RELATED-TO")
        .find(|p| {
            p.param("RELTYPE")
                .is_none_or(|r| r.eq_ignore_ascii_case("PARENT"))
        })
        .map(|p| p.value.trim().to_string());

    let meta = c
        .prop(METADATA_PROP)
        .map(|p| parse_metadata(&unescape_text(&p.value)))
        .unwrap_or_default();
    if meta.unreadable {
        warnings.push("unreadable 2Do metadata".to_string());
    }

    // An unrecognised `@tasktype` reads as a plain task, as if the token were missing.
    let kind = match meta
        .value("tasktype")
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("project") => TaskKind::Project,
        Some("checklist") => TaskKind::Checklist,
        _ => {
            if meta.has("tasktype") {
                let raw = meta.value("tasktype").unwrap_or("");
                warnings.push(format!("unrecognised @tasktype value: {raw}"));
            }
            TaskKind::Task
        }
    };

    let meta_start = meta_start(&meta, &mut warnings);
    // 2Do's CalDAV upload fills DTSTART with a copy of the due date (or drops it), and keeps
    // the real start only in its JSON metadata. When that JSON is present it decides alone.
    let json_meta = meta.format == MetaFormat::Json && !meta.unreadable;
    let start = if json_meta {
        meta_start
    } else {
        match (dtstart, meta_start) {
            (Some(d), Some(m)) => {
                if !start_agrees(&d, &m) {
                    warnings.push("start date: DTSTART and 2Do metadata disagree".to_string());
                }
                Some(d)
            }
            (Some(d), None) => Some(d),
            (None, m) => m,
        }
    };

    let alarms = read_alarms(c, &mut warnings);

    Task {
        uid,
        collection: collection.to_string(),
        path: path.to_path_buf(),
        summary,
        description: text_field(c, "DESCRIPTION"),
        due,
        start,
        status,
        completed,
        created,
        last_modified,
        rrule: c.prop("RRULE").map(|p| p.value.trim().to_string()),
        parent,
        kind,
        alarms,
        from_completion: meta.has("repeatfromcompletion"),
        meta,
        warnings,
        blocked: None,
    }
}

/// `@startdate:YYYY-MM-DD` plus optional `@starttime:HH:MM`, as a Date or Floating value.
/// A value that does not parse is dropped with a warning (an invalid time keeps the date).
fn meta_start(meta: &Metadata, warnings: &mut Vec<String>) -> Option<IcalDateTime> {
    let raw = meta.value("startdate")?;
    let Ok(date) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") else {
        warnings.push(format!("invalid @startdate: {raw}"));
        return None;
    };
    let time = meta.value("starttime").and_then(|t| {
        let parsed = NaiveTime::parse_from_str(t, "%H:%M").ok();
        if parsed.is_none() {
            warnings.push(format!("invalid @starttime: {t}"));
        }
        parsed
    });
    match time {
        Some(t) => Some(IcalDateTime::Floating(date.and_time(t))),
        None => Some(IcalDateTime::Date(date)),
    }
}

/// Whether DTSTART and the metadata start name the same day (and time, when both have one).
/// UTC DTSTART values are compared by date only: the metadata carries no zone.
fn start_agrees(dtstart: &IcalDateTime, meta: &IcalDateTime) -> bool {
    let parts = |v: &IcalDateTime| match v {
        IcalDateTime::Date(d) => (*d, None),
        IcalDateTime::Floating(l) | IcalDateTime::Zoned { local: l, .. } => {
            (l.date(), Some(l.time()))
        }
        IcalDateTime::Utc(u) => (u.date_naive(), None),
    };
    let (dd, dt) = parts(dtstart);
    let (md, mt) = parts(meta);
    dd == md && (dt.is_none() || mt.is_none() || dt == mt)
}

fn read_alarms(c: &Component, warnings: &mut Vec<String>) -> Vec<Alarm> {
    let mut out = Vec::new();
    for (index, a) in c.children_named("VALARM").enumerate() {
        let Some(p) = a.prop("TRIGGER") else {
            warnings.push("invalid TRIGGER: missing".to_string());
            continue;
        };
        let is_dt = p
            .param("VALUE")
            .is_some_and(|v| v.eq_ignore_ascii_case("DATE-TIME"));
        let trigger = if is_dt {
            match parse_date_time(p) {
                Ok(IcalDateTime::Utc(u)) => Ok(Trigger::Absolute(u)),
                Ok(_) => Err(format!("absolute trigger {:?} is not UTC", p.value)),
                Err(e) => Err(e.to_string()),
            }
        } else {
            parse_duration(p.value.trim())
                .map(|offset| Trigger::Relative {
                    offset,
                    related_end: p
                        .param("RELATED")
                        .is_some_and(|r| r.eq_ignore_ascii_case("END")),
                })
                .map_err(|e| e.to_string())
        };
        match trigger {
            Ok(trigger) => out.push(Alarm { trigger, index }),
            Err(e) => warnings.push(format!("invalid TRIGGER: {e}")),
        }
    }
    out
}
