//! `views.get`: the Inbox and Today views, finished (daemon spec §6, phone companion design
//! §6). The rows come from the same planner the TUI uses, on the published snapshot with the
//! queued edits laid over it, at the daemon's clock in the configured timezone.

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use serde_json::Value;
use twydo_core::ical::value::{IcalDateTime, Resolved};
use twydo_core::model::{Status, Task};
use twydo_core::views::{
    DateGroup, Expansion, Row, SidebarEntry, Snapshot, View, build_view, counts, date_group,
    default_list, group_label, inbox_list, sidebar, view_name,
};

use super::Shared;
use super::admin::unresolved_conflict_uids;
use super::syncsummary::summary;
use crate::protocol::rpc::{ErrorKind, RpcError};
use crate::protocol::views::{
    CaptureTarget, RowList, RowParent, SectionKey, ViewName, ViewRow, ViewSection, ViewsEntry,
    ViewsGetParams, ViewsGetResult, ViewsListItem, ViewsListResult,
};

/// Check the params and map them to a core view.
pub(crate) fn resolve(snap: &Snapshot, p: &ViewsGetParams) -> Result<View, RpcError> {
    let invalid = |m: &str| RpcError::refused(ErrorKind::Invalid, m.to_string());
    if p.list.is_some() && p.view != ViewName::List {
        return Err(invalid("list is only for the list view"));
    }
    Ok(match p.view {
        ViewName::Inbox => View::List(inbox_list(snap).unwrap_or_else(|| default_list(snap))),
        ViewName::Today => View::Today,
        ViewName::All => View::All,
        ViewName::Scheduled => View::Scheduled,
        ViewName::Done => View::Done,
        ViewName::Next3Days => View::Next3Days,
        ViewName::List => {
            let id = p
                .list
                .as_deref()
                .ok_or_else(|| invalid("list needs a list id"))?;
            if !snap.collections.iter().any(|c| c.id == id) {
                return Err(invalid("no such list"));
            }
            View::List(id.to_string())
        }
        ViewName::Unknown => return Err(invalid("unknown view")),
    })
}

fn internal(e: impl std::fmt::Display) -> RpcError {
    RpcError::refused(ErrorKind::Internal, e.to_string())
}

pub fn get(shared: &Arc<Shared>, p: ViewsGetParams) -> Result<Value, RpcError> {
    let (revision, published) = shared.publisher.current();
    let snap = &published.snap;
    let tz = snap.tz();
    let at = shared.clock.now();
    let now = at.with_timezone(&tz);
    let today = now.date_naive();
    let conflicts = unresolved_conflict_uids(shared).map_err(internal)?;

    let list_name = |id: &str| list_name(snap, id);
    let view = resolve(snap, &p)?;
    let default_capture = |due| {
        let id = default_list(snap);
        (!id.is_empty()).then(|| CaptureTarget {
            name: list_name(&id),
            collection: id,
            due,
        })
    };
    let (title, capture) = match p.view {
        ViewName::Today => ("Today".to_string(), default_capture(Some(today))),
        ViewName::Inbox => match &view {
            View::List(id) if !id.is_empty() => {
                let name = list_name(id);
                let capture = CaptureTarget {
                    collection: id.clone(),
                    name: name.clone(),
                    due: None,
                };
                (name, Some(capture))
            }
            _ => ("Inbox".to_string(), None),
        },
        ViewName::List => match &view {
            View::List(id) => (
                view_name(snap, &view),
                Some(CaptureTarget {
                    collection: id.clone(),
                    name: list_name(id),
                    due: None,
                }),
            ),
            _ => unreachable!("resolve maps list to View::List"),
        },
        _ => (view_name(snap, &view), default_capture(None)),
    };

    let built = build_view(snap, &view, now, &Expansion::default());
    let sections = built
        .groups
        .iter()
        .map(|(group, rows)| ViewSection {
            group: SectionKey::from(*group),
            date: match group {
                DateGroup::CompletedOn(d) => Some(*d),
                _ => None,
            },
            label: group_label(*group, today),
            rows: rows
                .iter()
                .map(|r| row(snap, r, at, &conflicts, &list_name))
                .collect(),
        })
        .collect();

    let result = ViewsGetResult {
        epoch: shared.epoch.clone(),
        revision,
        now: now.fixed_offset(),
        today,
        title,
        open_count: counts(snap, &view, now).total,
        capture,
        sections,
        sync: shared.publisher.status().and_then(|s| {
            let conflicts = s.conflicts;
            s.sync
                .map(|v| summary(&v, conflicts, now.fixed_offset(), &list_name))
        }),
    };
    serde_json::to_value(result).map_err(internal)
}

/// A list's display name, falling back to its id.
pub(crate) fn list_name(snap: &Snapshot, id: &str) -> String {
    snap.collections
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.display_name.clone().unwrap_or_else(|| c.id.clone()))
        .unwrap_or_else(|| id.to_string())
}

/// A list's colour as `#rrggbb`.
pub(crate) fn list_color(snap: &Snapshot, id: &str) -> Option<String> {
    snap.collections
        .iter()
        .find(|c| c.id == id)
        .and_then(|c| c.color)
        .map(|(r, g, b)| format!("#{r:02x}{g:02x}{b:02x}"))
}

/// The sidebar, finished: names and counts at the daemon's clock, each view as the params
/// that open it.
pub fn list(shared: &Arc<Shared>) -> Result<Value, RpcError> {
    let (revision, published) = shared.publisher.current();
    let snap = &published.snap;
    let now = shared.clock.now().with_timezone(&snap.tz());
    let inbox = inbox_list(snap);
    let entries = sidebar(snap)
        .into_iter()
        .map(|e| match e {
            SidebarEntry::Heading(heading) => ViewsEntry::Heading { heading },
            SidebarEntry::Item(v) => {
                let c = counts(snap, &v, now);
                let (params, color) = match &v {
                    View::List(id) if Some(id) == inbox.as_ref() => (
                        ViewsGetParams {
                            view: ViewName::Inbox,
                            list: None,
                        },
                        list_color(snap, id),
                    ),
                    View::List(id) => (
                        ViewsGetParams {
                            view: ViewName::List,
                            list: Some(id.clone()),
                        },
                        list_color(snap, id),
                    ),
                    View::Today => (
                        ViewsGetParams {
                            view: ViewName::Today,
                            list: None,
                        },
                        None,
                    ),
                    View::All => (
                        ViewsGetParams {
                            view: ViewName::All,
                            list: None,
                        },
                        None,
                    ),
                    View::Scheduled => (
                        ViewsGetParams {
                            view: ViewName::Scheduled,
                            list: None,
                        },
                        None,
                    ),
                    View::Done => (
                        ViewsGetParams {
                            view: ViewName::Done,
                            list: None,
                        },
                        None,
                    ),
                    View::Next3Days => (
                        ViewsGetParams {
                            view: ViewName::Next3Days,
                            list: None,
                        },
                        None,
                    ),
                };
                ViewsEntry::View(ViewsListItem {
                    view: params,
                    name: view_name(snap, &v),
                    color,
                    open_count: c.total,
                    due_count: c.due_or_overdue,
                })
            }
        })
        .collect();
    serde_json::to_value(ViewsListResult {
        epoch: shared.epoch.clone(),
        revision,
        now: now.fixed_offset(),
        today: now.date_naive(),
        entries,
    })
    .map_err(internal)
}

/// The calendar day of a date value in the clock's timezone.
fn day_in(d: &IcalDateTime, now: DateTime<Tz>) -> NaiveDate {
    match d.resolve(now.timezone()).0 {
        Resolved::Date(day) => day,
        Resolved::Instant(i) => i.with_timezone(&now.timezone()).date_naive(),
    }
}

/// `(overdue, due_today)` for task `idx` at `now`.
pub(crate) fn due_flags(snap: &Snapshot, idx: usize, now: DateTime<Tz>) -> (bool, bool) {
    let t = &snap.tasks[idx];
    let today = now.date_naive();
    (
        t.status == Status::Open && date_group(snap, idx, now, false) == DateGroup::Overdue,
        t.due.as_ref().is_some_and(|d| day_in(d, now) == today),
    )
}

fn row(
    snap: &Snapshot,
    r: &Row,
    at: DateTime<Utc>,
    conflicts: &std::collections::HashSet<String>,
    list_name: &dyn Fn(&str) -> String,
) -> ViewRow {
    let t: &Task = &snap.tasks[r.task];
    let now = at.with_timezone(&snap.tz());
    let today = now.date_naive();
    let (overdue, due_today) = due_flags(snap, r.task, now);
    let day_of = |d: &IcalDateTime| day_in(d, now);
    let color = list_color(snap, &t.collection);
    ViewRow {
        uid: t.uid.clone(),
        title: t.summary.clone(),
        depth: r.depth,
        parent: r.parent_hint.map(|p| RowParent {
            uid: snap.tasks[p].uid.clone(),
            title: snap.tasks[p].summary.clone(),
        }),
        due: t
            .due
            .as_ref()
            .map(|d| twydo_core::format::relative_date(d, now)),
        start: t
            .start
            .as_ref()
            .filter(|s| day_of(s) > today)
            .map(|s| twydo_core::format::relative_date(s, now)),
        overdue,
        due_today,
        completed: t.status != Status::Open,
        note: t.description.clone(),
        kind: t.kind,
        repeats: t.rrule.is_some(),
        list: RowList {
            id: t.collection.clone(),
            name: list_name(&t.collection),
            color,
        },
        pending: snap.is_pending(r.task),
        conflict: conflicts.contains(&t.uid),
    }
}
