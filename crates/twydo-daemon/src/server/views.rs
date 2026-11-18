//! `views.get`: the Inbox and Today views, finished (daemon spec §6, phone companion design
//! §6). The rows come from the same planner the TUI uses, on the published snapshot with the
//! queued edits laid over it, at the daemon's clock in the configured timezone.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::Value;
use twydo_core::ical::value::{IcalDateTime, Resolved};
use twydo_core::model::{Status, Task};
use twydo_core::views::{
    DateGroup, Expansion, Row, Snapshot, View, build_view, counts, date_group, default_list,
    group_label, inbox_list,
};

use super::Shared;
use super::admin::unresolved_conflict_uids;
use super::syncsummary::summary;
use crate::protocol::rpc::{ErrorKind, RpcError};
use crate::protocol::views::{
    CaptureTarget, RowList, RowParent, SectionKey, ViewName, ViewRow, ViewSection, ViewsGetParams,
    ViewsGetResult,
};

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

    let list_name = |id: &str| {
        snap.collections
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.display_name.clone().unwrap_or_else(|| c.id.clone()))
            .unwrap_or_else(|| id.to_string())
    };
    let (view, title, capture) = match p.view {
        ViewName::Today => {
            let id = default_list(snap);
            let capture = (!id.is_empty()).then(|| CaptureTarget {
                name: list_name(&id),
                collection: id,
                due: Some(today),
            });
            (View::Today, "Today".to_string(), capture)
        }
        ViewName::Inbox => {
            let id = inbox_list(snap).unwrap_or_else(|| default_list(snap));
            if id.is_empty() {
                (View::List(id), "Inbox".to_string(), None)
            } else {
                let name = list_name(&id);
                let capture = CaptureTarget {
                    collection: id.clone(),
                    name: name.clone(),
                    due: None,
                };
                (View::List(id), name, Some(capture))
            }
        }
    };

    let built = build_view(snap, &view, now, &Expansion::default());
    let sections = built
        .groups
        .iter()
        .map(|(group, rows)| ViewSection {
            group: SectionKey::from(*group),
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
    let day_of = |d: &IcalDateTime| match d.resolve(now.timezone()).0 {
        Resolved::Date(day) => day,
        Resolved::Instant(i) => i.with_timezone(&now.timezone()).date_naive(),
    };
    let color = snap
        .collections
        .iter()
        .find(|c| c.id == t.collection)
        .and_then(|c| c.color)
        .map(|(r, g, b)| format!("#{r:02x}{g:02x}{b:02x}"));
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
        overdue: t.status == Status::Open
            && date_group(snap, r.task, now, false) == DateGroup::Overdue,
        due_today: t.due.as_ref().is_some_and(|d| day_of(d) == today),
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
