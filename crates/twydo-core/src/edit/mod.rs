//! Edit operations and the field edits they make.
//!
//! An edit changes a parsed resource the way the 2Do iPhone app does (captured in
//! `fixtures/phase0/v1/plan4-list1` and `rule-*/03-edit`): only the target properties,
//! `DTSTAMP`, `LAST-MODIFIED` and their verified `<2Do Meta>` counterparts change. Every
//! other line keeps its original bytes.

use chrono::{DateTime, NaiveDate, NaiveDateTime, Timelike, Utc};
use thiserror::Error;

use crate::ical::value::{IcalDateTime, escape_text, format_date_time};
use crate::ical::{Calendar, Component, Param, Property};
use crate::model::meta_edit::{MetaJson, default_task_meta};
use crate::model::repeat::{End, parse_rrule};
use crate::repeatinput::RepeatSpec;

pub mod complete;
pub mod create;
pub mod family;
pub mod repeat;

pub use complete::{complete_one_time, complete_recurring, next_due, reopen_one_time};
pub use create::{file_name, new_task, new_uid};
pub use family::{complete_member, drop_member_dates, member_copy, reopen_member};

const META_PROP: &str = "X-2DOAPP-METADATA";
const META_OPEN: &str = "<2Do Meta>";

/// A due date. `Time` is floating local time (`DUE:YYYYMMDDTHHMMSS`, no `TZID`), as 2Do
/// writes it (H8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    Date(NaiveDate),
    Time(NaiveDateTime),
}

/// One field change of an existing task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Summary(String),
    /// `None` clears the notes.
    Description(Option<String>),
    /// `None` removes the due date.
    Due(Option<Due>),
    /// `None` removes the start date.
    Start(Option<NaiveDate>),
    RepeatInterval(u32),
    /// Set the repeat rule, or `None` to remove it. A task without a due date must get one
    /// first (a `Change::Due` earlier in the same list): 2Do makes the due date today.
    Repeat(Option<RepeatSpec>),
}

/// A task to create in `collection`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTask {
    pub collection: String,
    pub summary: String,
    pub description: Option<String>,
    pub due: Option<Due>,
    pub start: Option<NaiveDate>,
}

/// A write operation on the task store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Create(NewTask),
    Edit { uid: String, changes: Vec<Change> },
    Complete { uid: String },
    Reopen { uid: String },
    Move { uid: String, to: String },
    Delete { uid: String },
}

#[derive(Error, Debug)]
pub enum EditError {
    #[error("title must not be empty")]
    EmptyTitle,
    #[error("no VTODO in resource")]
    NoTodo,
    #[error("{0}")]
    Unsupported(String),
}

fn unsupported(msg: impl Into<String>) -> EditError {
    EditError::Unsupported(msg.into())
}

/// Apply `changes` in order to the resource's VTODO, then [`touch`] it once. All or nothing:
/// on error `cal` is left unchanged.
pub fn apply_changes(
    cal: &mut Calendar,
    changes: &[Change],
    now: DateTime<Utc>,
) -> Result<(), EditError> {
    let mut work = cal.clone();
    let todo = the_todo(&mut work)?;
    for c in changes {
        apply_one(todo, c)?;
    }
    touch(todo, now);
    *cal = work;
    Ok(())
}

/// Set `DTSTAMP` and `LAST-MODIFIED` to `now` (UTC). A missing one is inserted: `DTSTAMP`
/// after `CREATED` (or at the default position), `LAST-MODIFIED` after `DTSTAMP`.
pub fn touch(todo: &mut Component, now: DateTime<Utc>) {
    let stamp = now.format("%Y%m%dT%H%M%SZ").to_string();
    for (name, after) in [("DTSTAMP", "CREATED"), ("LAST-MODIFIED", "DTSTAMP")] {
        match todo.prop_mut(name) {
            Some(p) => {
                if !p.params.is_empty() {
                    p.set_params(Vec::new());
                }
                if p.value != stamp {
                    p.set_value(&stamp);
                }
            }
            None => {
                let after = todo.prop(after).is_some().then_some(after);
                todo.insert_prop(Property::new(name, Vec::new(), &stamp), after);
            }
        }
    }
}

/// The resource a move writes to the destination list (H5 as captured): `cal` with
/// `UID` = `new_uid`, stamps touched, and a JSON `uid` that names the old UID (a parent's
/// self-reference) set to `new_uid`.
///
/// - `parent` = `None`: a JSON `RUID` that names the old UID (a live task's self-reference)
///   is set to `new_uid`. Any other `RUID` (a history copy's link to its series) is kept.
/// - `parent` = `Some(p)` (a member moved with its parent): `RELATED-TO` and JSON
///   `parentuid` = `p`, and `RUID` is left alone (`plan6-list2/07-move`: a moved repeating
///   member keeps its old UID there).
///
/// Every other line keeps its bytes; `cal` is not changed.
pub fn moved_copy(
    cal: &Calendar,
    new_uid: &str,
    parent: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Calendar, EditError> {
    let mut work = cal.clone();
    let todo = the_todo(&mut work)?;
    let uid = todo
        .prop_mut("UID")
        .ok_or_else(|| unsupported("task has no UID"))?;
    if new_uid.is_empty() || uid.value == new_uid {
        return Err(unsupported("a moved task needs a new UID"));
    }
    let old_uid = uid.value.clone();
    uid.set_value(new_uid);
    match meta_of(todo) {
        Meta::Json(orig) => {
            let mut m = orig.clone();
            if m.get_str("uid") == Some(old_uid.as_str()) {
                m.set_str("uid", new_uid);
            }
            match parent {
                None => {
                    if m.get_str("RUID") == Some(old_uid.as_str()) {
                        m.set_str("RUID", new_uid);
                    }
                }
                Some(p) => m.set_str("parentuid", p),
            }
            write_meta(todo, Some(&orig), &m);
        }
        Meta::UnreadableJson => return Err(unsupported("this task's 2Do metadata is unreadable")),
        Meta::Tokens if parent.is_some() => {
            return Err(unsupported("this task's 2Do metadata format is read-only"));
        }
        Meta::Absent | Meta::Tokens => {}
    }
    if let Some(p) = parent {
        family::set_related_to(todo, p)?;
    }
    touch(todo, now);
    Ok(work)
}

/// The resource's single VTODO. Recurrence overrides (several VTODOs or a
/// `RECURRENCE-ID`) are not supported.
fn the_todo(cal: &mut Calendar) -> Result<&mut Component, EditError> {
    let overrides = || unsupported("recurrence overrides are not supported");
    match cal.root.children_named("VTODO").count() {
        0 => return Err(EditError::NoTodo),
        1 => {}
        _ => return Err(overrides()),
    }
    let todo = cal
        .root
        .children_named_mut("VTODO")
        .next()
        .ok_or(EditError::NoTodo)?;
    if todo.prop("RECURRENCE-ID").is_some() {
        return Err(overrides());
    }
    Ok(todo)
}

fn apply_one(todo: &mut Component, change: &Change) -> Result<(), EditError> {
    match change {
        Change::Summary(s) => set_summary(todo, s),
        Change::Description(d) => {
            set_description(todo, d.as_deref());
            Ok(())
        }
        Change::Due(d) => set_due(todo, *d),
        Change::Start(d) => set_start(todo, *d),
        Change::RepeatInterval(n) => set_interval(todo, *n),
        Change::Repeat(spec) => set_repeat(todo, spec.as_ref()),
    }
}

/// Set a text property's raw value, keeping its bytes when the value does not change.
fn set_text(todo: &mut Component, name: &str, escaped: &str, after: &str) {
    match todo.prop_mut(name) {
        Some(p) if p.value == escaped => {}
        Some(p) => p.set_value(escaped),
        None => todo.insert_prop(Property::new(name, Vec::new(), escaped), Some(after)),
    }
}

fn set_summary(todo: &mut Component, s: &str) -> Result<(), EditError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(EditError::EmptyTitle);
    }
    set_text(todo, "SUMMARY", &escape_text(s), "UID");
    Ok(())
}

/// `None` empties an existing `DESCRIPTION` (2Do writes `DESCRIPTION:`, H2) and leaves an
/// absent one absent.
fn set_description(todo: &mut Component, d: Option<&str>) {
    match d {
        Some(s) => set_text(todo, "DESCRIPTION", &escape_text(s), "SUMMARY"),
        None => {
            if let Some(p) = todo.prop_mut("DESCRIPTION")
                && !p.value.is_empty()
            {
                p.set_value("");
            }
        }
    }
}

/// The task's `X-2DOAPP-METADATA`, by kind.
enum Meta {
    Absent,
    Json(MetaJson),
    /// `<2Do Meta>` that does not decode.
    UnreadableJson,
    /// The `@key` token format.
    Tokens,
}

fn meta_of(todo: &Component) -> Meta {
    let Some(p) = todo.prop(META_PROP) else {
        return Meta::Absent;
    };
    match MetaJson::parse(&p.value) {
        Some(m) => Meta::Json(m),
        None if p.value.contains(META_OPEN) => Meta::UnreadableJson,
        None => Meta::Tokens,
    }
}

/// Write `meta` back only when it differs from `orig` (an unchanged blob keeps its bytes).
/// A new property goes after `RRULE`, `DUE` or `DTSTART`, as 2Do places it.
fn write_meta(todo: &mut Component, orig: Option<&MetaJson>, meta: &MetaJson) {
    if orig == Some(meta) {
        return;
    }
    let value = meta.encode();
    match todo.prop_mut(META_PROP) {
        Some(p) => p.set_value(&value),
        None => {
            let after = ["RRULE", "DUE", "DTSTART"]
                .into_iter()
                .find(|n| todo.prop(n).is_some());
            let params = vec![Param {
                name: "SHARE-SCOPE".into(),
                values: vec!["GLOBAL".into()],
            }];
            todo.insert_prop(Property::new(META_PROP, params, &value), after);
        }
    }
}

/// Set a DATE / DATE-TIME property. `VALUE` and `TZID` follow the new value; other
/// parameters stay. An unchanged property keeps its bytes. A new one goes after the first
/// existing property in `after`.
fn set_date_prop(todo: &mut Component, name: &str, v: &IcalDateTime, after: &[&str]) {
    let (fresh, value) = format_date_time(v);
    let typed = |p: &Param| p.name == "VALUE" || p.name == "TZID";
    match todo.prop_mut(name) {
        Some(p) => {
            let mut params: Vec<Param> = Vec::new();
            for q in &p.params {
                if !typed(q) {
                    params.push(q.clone());
                } else if let Some(f) = fresh.iter().find(|f| f.name == q.name) {
                    params.push(f.clone());
                }
            }
            for f in &fresh {
                if !params.iter().any(|q| q.name == f.name) {
                    params.push(f.clone());
                }
            }
            if params != p.params {
                p.set_params(params);
            }
            if p.value != value {
                p.set_value(&value);
            }
        }
        None => {
            let after = after.iter().copied().find(|n| todo.prop(n).is_some());
            todo.insert_prop(Property::new(name, fresh, &value), after);
        }
    }
}

/// `DUE`, plus its 2Do counterparts when the task has `<2Do Meta>` JSON: `DTSTART` mirrors
/// `DUE` (H7), `duetime` holds HHMM of a timed due (H8), and a timed due gets 2Do's
/// automatic at-due-time alert ([`auto_alarm`]) unless it already has one, which then stays
/// as is (user decision 2026-11-01, mirroring 2Do). A due without a time drops that alert
/// (`X-2DOAPP-ISAUTO:1`). A due time is written with whole minutes (seconds zeroed), as 2Do
/// writes it.
fn set_due(todo: &mut Component, due: Option<Due>) -> Result<(), EditError> {
    let due = due.map(|d| match d {
        Due::Time(t) => Due::Time(t.date().and_hms_opt(t.hour(), t.minute(), 0).unwrap_or(t)),
        d => d,
    });
    let json = match meta_of(todo) {
        Meta::Json(m) => Some(m),
        Meta::UnreadableJson => return Err(unsupported("this task's 2Do metadata is unreadable")),
        Meta::Absent | Meta::Tokens => None,
    };
    match due {
        Some(d) => {
            let v = match d {
                Due::Date(x) => IcalDateTime::Date(x),
                Due::Time(t) => IcalDateTime::Floating(t),
            };
            if json.is_some() {
                set_date_prop(
                    todo,
                    "DTSTART",
                    &v,
                    &["LAST-MODIFIED", "DTSTAMP", "CREATED"],
                );
            }
            set_date_prop(
                todo,
                "DUE",
                &v,
                &["DTSTART", "LAST-MODIFIED", "DTSTAMP", "CREATED"],
            );
        }
        None => {
            todo.remove_props("DUE");
            if json.is_some() {
                todo.remove_props("DTSTART");
            }
        }
    }
    let timed = matches!(due, Some(Due::Time(_)));
    if let Some(orig) = json {
        let mut m = orig.clone();
        match due {
            Some(Due::Time(t)) => m.set_i64("duetime", i64::from(t.hour() * 100 + t.minute())),
            _ => m.remove("duetime"),
        }
        write_meta(todo, Some(&orig), &m);
        if timed && !todo.children.iter().any(is_auto_alarm) {
            let summary = todo.prop("SUMMARY").map_or("", |p| p.value.as_str());
            let alarm = auto_alarm(summary);
            todo.push_child(alarm);
        }
    }
    if !timed {
        todo.remove_children(is_auto_alarm);
    }
    Ok(())
}

/// 2Do's automatic at-due-time alert, exactly as 2Do writes it (H8, `plan4-list1/01-create`
/// task C). `summary` is the raw (escaped) `SUMMARY` value. A later title edit leaves its
/// `DESCRIPTION` unchanged (Ruling R14; how 2Do treats that is unverified).
fn auto_alarm(summary: &str) -> Component {
    let global = || {
        vec![Param {
            name: "SHARE-SCOPE".into(),
            values: vec!["GLOBAL".into()],
        }]
    };
    let mut a = Component::new("VALARM");
    for (name, params, value) in [
        ("TRIGGER", Vec::new(), "PT0S"),
        ("ACTION", Vec::new(), "AUDIO"),
        ("DESCRIPTION", Vec::new(), summary),
        ("X-2DOAPP-SOUND", global(), "Basso"),
        ("X-2DOAPP-ISAUTO", global(), "1"),
        ("X-2DOAPP-RELDUETIME", global(), "1"),
    ] {
        a.insert_prop(Property::new(name, params, value), None);
    }
    a
}

/// A `DTSTART` that is not a copy of `DUE` (same parameters and value), or one without `DUE`.
fn has_own_dtstart(todo: &Component) -> bool {
    match (todo.prop("DTSTART"), todo.prop("DUE")) {
        (Some(s), Some(d)) => s.params != d.params || s.value != d.value,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

fn is_auto_alarm(c: &Component) -> bool {
    c.name == "VALARM"
        && c.prop("X-2DOAPP-ISAUTO")
            .is_some_and(|p| p.value.trim() == "1")
}

/// The start date lives only in `<2Do Meta>` `StartDate` (epoch seconds of UTC midnight, H9).
/// A task without metadata gets 2Do's default metadata first, unless it has its own
/// `DTSTART` (one that does not mirror `DUE`): with JSON metadata `DTSTART` becomes a mirror,
/// and how 2Do reads that combination is not captured, so it is refused.
fn set_start(todo: &mut Component, start: Option<NaiveDate>) -> Result<(), EditError> {
    let orig = match meta_of(todo) {
        Meta::Json(m) => Some(m),
        Meta::Absent if has_own_dtstart(todo) => {
            return Err(unsupported(
                "setting a start date on a task with its own DTSTART and no 2Do metadata is not supported",
            ));
        }
        Meta::Absent => None,
        Meta::Tokens | Meta::UnreadableJson => {
            return Err(unsupported("this task's 2Do metadata format is read-only"));
        }
    };
    let mut m = orig.clone().unwrap_or_else(default_task_meta);
    match start {
        Some(d) => m.set_i64(
            "StartDate",
            d.and_time(chrono::NaiveTime::MIN).and_utc().timestamp(),
        ),
        None => m.remove("StartDate"),
    }
    write_meta(todo, orig.as_ref(), &m);
    Ok(())
}

/// `RRULE` `INTERVAL=n` (appended, or replaced in place; removed for 1) and JSON
/// `RecurrenceValue` = n (`rule-*/03-edit`).
fn set_interval(todo: &mut Component, n: u32) -> Result<(), EditError> {
    if n == 0 {
        return Err(unsupported("repeat interval must be at least 1"));
    }
    let json = match meta_of(todo) {
        Meta::Json(m) => Some(m),
        Meta::UnreadableJson => return Err(unsupported("this task's 2Do metadata is unreadable")),
        Meta::Absent | Meta::Tokens => None,
    };
    let rrule = todo
        .prop_mut("RRULE")
        .ok_or_else(|| unsupported("task does not repeat"))?;
    parse_rrule(&rrule.value).map_err(|e| unsupported(format!("unreadable repeat rule: {e}")))?;
    let interval = format!("INTERVAL={n}");
    let mut parts: Vec<String> = Vec::new();
    let mut found = false;
    for part in rrule.value.split(';').filter(|s| !s.is_empty()) {
        let key = part.split('=').next().unwrap_or("");
        if key.trim().eq_ignore_ascii_case("INTERVAL") {
            if n > 1 && !found {
                parts.push(interval.clone());
            }
            found = true;
        } else {
            parts.push(part.to_string());
        }
    }
    if n > 1 && !found {
        parts.push(interval);
    }
    let value = parts.join(";");
    if value != rrule.value {
        rrule.set_value(&value);
    }
    if let Some(orig) = json {
        let mut m = orig.clone();
        m.set_i64("RecurrenceValue", i64::from(n));
        write_meta(todo, Some(&orig), &m);
    }
    Ok(())
}

/// The date `RecurrenceEndDate` stores: the end date at 12:00 UTC, as epoch seconds (`plan7-list1`
/// R16: 2027-06-30 is 1814356800).
fn end_date_stamp(u: &IcalDateTime) -> i64 {
    let d = match u {
        IcalDateTime::Date(d) => *d,
        IcalDateTime::Floating(n) => n.date(),
        IcalDateTime::Utc(t) => t.date_naive(),
        IcalDateTime::Zoned { local, .. } => local.date(),
    };
    d.and_hms_opt(12, 0, 0)
        .unwrap_or_default()
        .and_utc()
        .timestamp()
}

/// Set or remove the repeat rule as 2Do's repeat picker does (`plan7-list1/01-phone-repeat`).
///
/// - Set: `RRULE` goes right after `DUE`; JSON `RecurrenceType`/`RecurrenceValue` per
///   [`repeat::twodo_codes`], `RecurrenceFrom` 1 (2 from completion), the end keys, and `RUID`
///   = the task's UID. The phone writes its own internal id into `RUID` for tasks Twydo
///   created; the UID is the nearest value Twydo knows.
/// - Remove (`None`): `RRULE` goes and `RecurrenceType` becomes 0; `RecurrenceValue`,
///   `RecurrenceFrom` and `RUID` stay, as 2Do leaves them (R09).
fn set_repeat(todo: &mut Component, spec: Option<&RepeatSpec>) -> Result<(), EditError> {
    let json = match meta_of(todo) {
        Meta::Json(m) => Some(m),
        Meta::UnreadableJson => return Err(unsupported("this task's 2Do metadata is unreadable")),
        Meta::Absent | Meta::Tokens => None,
    };
    let Some(spec) = spec else {
        todo.remove_props("RRULE");
        if let Some(orig) = json {
            let mut m = orig.clone();
            m.set_i64("RecurrenceType", 0);
            write_meta(todo, Some(&orig), &m);
        }
        return Ok(());
    };
    if todo.prop("DUE").is_none() {
        return Err(unsupported("a repeating task needs a due date"));
    }
    let value = complete::rule_text(&spec.rule);
    match todo.prop_mut("RRULE") {
        Some(p) => {
            if p.value != value {
                p.set_value(&value);
            }
        }
        None => todo.insert_prop(Property::new("RRULE", Vec::new(), &value), Some("DUE")),
    }
    if let Some(orig) = json {
        let uid = todo
            .prop("UID")
            .map(|p| p.value.clone())
            .unwrap_or_default();
        let (kind, val) = repeat::twodo_codes(&spec.rule);
        let mut m = orig.clone();
        m.set_i64("RecurrenceValue", val);
        m.set_i64("RecurrenceType", kind);
        m.set_i64("RecurrenceFrom", if spec.from_completion { 2 } else { 1 });
        m.set_str("RUID", &uid);
        match &spec.rule.end {
            End::Never => {
                m.set_i64("RecurrenceEndType", 0);
                m.set_i64("RecurrenceEndRepetitions", 0);
                m.set_i64("RecurrenceEndRepetitionsOrig", 0);
                m.remove("RecurrenceEndDate");
            }
            End::Count(n) => {
                m.set_i64("RecurrenceEndType", 2);
                m.set_i64("RecurrenceEndRepetitions", i64::from(*n));
                m.set_i64("RecurrenceEndRepetitionsOrig", i64::from(*n));
                m.remove("RecurrenceEndDate");
            }
            End::Until(u) => {
                m.set_i64("RecurrenceEndType", 1);
                m.set_i64("RecurrenceEndRepetitions", 0);
                m.set_i64("RecurrenceEndRepetitionsOrig", 0);
                m.set_i64("RecurrenceEndDate", end_date_stamp(u));
            }
        }
        write_meta(todo, Some(&orig), &m);
    }
    Ok(())
}
