//! Create a task in the layout the 2Do iPhone app writes for a plain task (fixture
//! `plan4-list1/01-create`). The `X-2DOAPP-SYNC-*` / `AUDIO-DURATION` properties are left out
//! (meaning unknown). Due, start, `duetime` and the automatic alert of a timed due go
//! through the same setters as edits.

use chrono::{DateTime, Utc};

use super::{
    EditError, NewTask, set_description, set_due, set_repeat, set_start, set_summary, write_meta,
};
use crate::ical::{Calendar, Component, Property, new_calendar};
use crate::model::TaskKind;
use crate::model::meta_edit::new_task_meta;

/// A fresh UID: UUID v4, 32 lowercase hex digits, no dashes (as 2Do).
pub fn new_uid() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// The resource file name for `uid`.
pub fn file_name(uid: &str) -> String {
    format!("{uid}.ics")
}

fn prop(name: &str, value: &str) -> Property {
    Property::new(name, Vec::new(), value)
}

/// Build the resource for `t` with UID `uid`. `CREATED`, `DTSTAMP` and `LAST-MODIFIED` are `now`.
///
/// `parent_json_uid` is the JSON `uid` of the parent's metadata, when it has one; the item's
/// `parentuid` is that, else `t.parent` (the parent's iCalendar UID). `RELATED-TO` is always
/// `t.parent`. A project or checklist gets its own `uid` in its metadata.
pub fn new_task(
    t: &NewTask,
    uid: &str,
    parent_json_uid: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Calendar, EditError> {
    let stamp = now.format("%Y%m%dT%H%M%SZ").to_string();
    let mut todo = Component::new("VTODO");
    todo.insert_prop(prop("UID", uid), None);
    if let Some(parent) = &t.parent {
        todo.insert_prop(prop("RELATED-TO", parent), None);
    }
    for (n, v) in [
        ("SEQUENCE", "0"),
        ("SUMMARY", "x"),
        ("DESCRIPTION", ""),
        ("STATUS", "NEEDS-ACTION"),
        ("CREATED", stamp.as_str()),
    ] {
        todo.insert_prop(prop(n, v), None);
    }
    set_summary(&mut todo, &t.summary)?;
    set_description(&mut todo, t.description.as_deref());
    let (task_type, own_uid) = match t.kind {
        TaskKind::Task => (0, None),
        TaskKind::Checklist => (1, Some(uid)),
        TaskKind::Project => (2, Some(uid)),
    };
    let parent_uid = t.parent.as_deref().map(|p| parent_json_uid.unwrap_or(p));
    let meta = new_task_meta(task_type, own_uid, parent_uid);
    write_meta(&mut todo, None, &meta);
    if t.start.is_some() {
        set_start(&mut todo, t.start)?;
    }
    if t.due.is_some() {
        set_due(&mut todo, t.due)?;
    }
    if t.repeat.is_some() {
        set_repeat(&mut todo, t.repeat.as_ref())?;
    }

    // DTSTAMP and LAST-MODIFIED come first in 2Do's layout.
    let mut vtodo = Component::new("VTODO");
    vtodo.insert_prop(prop("DTSTAMP", &stamp), None);
    vtodo.insert_prop(prop("LAST-MODIFIED", &stamp), None);
    for p in todo.props {
        vtodo.insert_prop(p, None);
    }
    // 2Do's automatic alert of a timed due.
    for c in todo.children {
        vtodo.push_child(c);
    }

    let mut root = Component::new("VCALENDAR");
    root.insert_prop(prop("VERSION", "2.0"), None);
    root.insert_prop(
        prop(
            "PRODID",
            concat!("-//Twydo//tdo ", env!("CARGO_PKG_VERSION"), "//EN"),
        ),
        None,
    );
    root.insert_prop(prop("CALSCALE", "GREGORIAN"), None);
    root.push_child(vtodo);
    Ok(new_calendar(root))
}
