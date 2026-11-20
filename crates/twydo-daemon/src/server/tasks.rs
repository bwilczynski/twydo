//! `tasks.get`: one task and its children, worded for a thin client's task screen, from the
//! published snapshot with queued edits laid over it, at the daemon's clock.

use std::sync::Arc;

use serde_json::Value;
use twydo_core::details;
use twydo_core::format::relative_date;
use twydo_core::model::Status;
use twydo_core::views::Snapshot;

use super::Shared;
use super::admin::unresolved_conflict_uids;
use super::views::{due_flags, list_color, list_name};
use crate::protocol::rpc::{ErrorKind, RpcError};
use crate::protocol::tasks::{ChildRow, TaskDetail, TaskParent, TasksGetParams, TasksGetResult};
use crate::protocol::views::RowList;

fn internal(e: impl std::fmt::Display) -> RpcError {
    RpcError::refused(ErrorKind::Internal, e.to_string())
}

/// The note text, none when blank.
fn note_of(d: &Option<String>) -> Option<String> {
    d.as_ref().filter(|n| !n.trim().is_empty()).cloned()
}

pub fn get(shared: &Arc<Shared>, p: TasksGetParams) -> Result<Value, RpcError> {
    let (revision, published) = shared.publisher.current();
    let snap: &Snapshot = &published.snap;
    let tz = snap.tz();
    let now = shared.clock.now().with_timezone(&tz);
    let conflicts = unresolved_conflict_uids(shared).map_err(internal)?;
    let idx = snap
        .tasks
        .iter()
        .position(|t| t.uid == p.uid)
        .ok_or_else(|| RpcError::refused(ErrorKind::Invalid, "no such task".to_string()))?;
    let t = &snap.tasks[idx];
    let (overdue, due_today) = due_flags(snap, idx, now);
    let list_name = list_name(snap, &t.collection);
    let task = TaskDetail {
        uid: t.uid.clone(),
        title: t.summary.clone(),
        kind: details::shown_kind(snap, idx),
        list: RowList {
            id: t.collection.clone(),
            name: list_name,
            color: list_color(snap, &t.collection),
        },
        due: t.due.as_ref().map(|d| relative_date(d, now)),
        overdue,
        due_today,
        start: t.start.as_ref().map(|s| relative_date(s, now)),
        repeat: details::repeat_text(t, tz),
        alerts: details::alert_lines(t, tz),
        warnings: t.warnings.clone(),
        blocked: t.blocked.clone(),
        conflict: conflicts.contains(&t.uid),
        note: note_of(&t.description),
        completed: t.status != Status::Open,
        pending: snap.is_pending(idx),
        parent: snap.parent(idx).map(|p| TaskParent {
            uid: snap.tasks[p].uid.clone(),
            title: snap.tasks[p].summary.clone(),
            kind: details::shown_kind(snap, p),
        }),
    };
    let children = details::child_order(snap, tz, idx)
        .into_iter()
        .map(|c| {
            let ct = &snap.tasks[c];
            let (overdue, due_today) = due_flags(snap, c, now);
            ChildRow {
                uid: ct.uid.clone(),
                title: ct.summary.clone(),
                kind: details::shown_kind(snap, c),
                due: ct.due.as_ref().map(|d| relative_date(d, now)),
                overdue,
                due_today,
                completed: ct.status != Status::Open,
                pending: snap.is_pending(c),
                has_note: note_of(&ct.description).is_some(),
                child_count: snap.children(c).len() as u32,
            }
        })
        .collect();
    serde_json::to_value(TasksGetResult {
        epoch: shared.epoch.clone(),
        revision,
        now: now.fixed_offset(),
        today: now.date_naive(),
        task,
        children,
    })
    .map_err(internal)
}
