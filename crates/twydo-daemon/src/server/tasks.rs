//! `tasks.get`: one task and its children, worded for a thin client's task screen, from the
//! published snapshot with queued edits laid over it, at the daemon's clock.

use std::sync::Arc;

use chrono::{NaiveDate, Weekday};
use serde_json::Value;
use twydo_core::details;
use twydo_core::form::{Form, NewTarget};
use twydo_core::format::relative_date;
use twydo_core::model::Status;
use twydo_core::views::Snapshot;

use super::Shared;
use super::admin::unresolved_conflict_uids;
use super::views::{due_flags, list_color, list_name};
use crate::protocol::rpc::{ErrorKind, RpcError};
use crate::protocol::tasks::{
    ChildRow, FieldPreview, FormState, RebasedForm, TaskDetail, TaskParent, TasksFormParams,
    TasksFormResult, TasksGetParams, TasksGetResult,
};
use crate::protocol::views::RowList;

fn internal(e: impl std::fmt::Display) -> RpcError {
    RpcError::refused(ErrorKind::Internal, e.to_string())
}

/// The note text, none when blank.
fn note_of(d: &Option<String>) -> Option<String> {
    d.as_ref().filter(|n| !n.trim().is_empty()).cloned()
}

/// The edit form for the task at `idx`; an item's kind comes from its parent, as in `tdo`.
pub(crate) fn open_form(snap: &Snapshot, idx: usize) -> Form {
    let item_of = snap.parent(idx).map(|p| snap.tasks[p].kind);
    Form::open(&snap.tasks[idx], item_of, snap.tz())
}

pub(crate) fn form_state(f: &Form, today: NaiveDate, work_days: &[Weekday]) -> FormState {
    FormState {
        fields: f.fields(),
        values: f.values(),
        previews: f
            .previews(today, work_days)
            .into_iter()
            .map(|(field, text)| FieldPreview { field, text })
            .collect(),
        alert_editable: f.alert_editable(),
        basis: f.basis(),
    }
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
    let form = form_state(
        &open_form(snap, idx),
        now.date_naive(),
        &snap.config.work_days,
    );
    serde_json::to_value(TasksGetResult {
        epoch: shared.epoch.clone(),
        revision,
        now: now.fixed_offset(),
        today: now.date_naive(),
        task,
        children,
        form,
    })
    .map_err(internal)
}

/// `tasks.form`: checks a typed form against the published snapshot; writes nothing and keeps
/// no state.
pub fn form(shared: &Arc<Shared>, p: TasksFormParams) -> Result<Value, RpcError> {
    let invalid = |m: &str| RpcError::refused(ErrorKind::Invalid, m.to_string());
    let (revision, published) = shared.publisher.current();
    let snap: &Snapshot = &published.snap;
    let tz = snap.tz();
    let today = shared.clock.now().with_timezone(&tz).date_naive();
    let find = |uid: &str| {
        snap.tasks
            .iter()
            .position(|t| t.uid == uid)
            .ok_or_else(|| invalid("no such task"))
    };
    let cur = match (&p.uid, p.new) {
        (Some(uid), None) => open_form(snap, find(uid)?),
        (None, Some(new)) => {
            let parent_kind = match &new.parent {
                Some(uid) => Some(snap.tasks[find(uid)?].kind),
                None => None,
            };
            Form::new_task(
                NewTarget {
                    collection: new.collection,
                    parent: new.parent,
                    parent_kind,
                    due_today: new.due == Some(today),
                },
                String::new(),
                tz,
            )
        }
        _ => return Err(invalid("give uid or new")),
    };
    // Changed since the client opened the form: its starting text, or what the text cannot
    // show (a stored rule or alert, a new item's parent kind). A new task's starting text is
    // the client's own, so only its basis counts.
    let text_changed = cur.uid.is_some() && p.initial.as_ref().is_some_and(|i| i != cur.initial());
    let basis_changed = p.opened.as_ref().is_some_and(|o| *o != cur.basis());
    let (form, changed_since, rebased) = if text_changed || basis_changed {
        let initial = p.initial.unwrap_or_else(|| cur.initial().clone());
        let opened = p.opened.unwrap_or_else(|| cur.basis());
        // The client's typing over what it opened; [`Form::rebase_from`] carries it onto the
        // current form, dropping fields that form no longer has.
        let mut mine = cur.clone().with_initial(initial);
        if let Some(v) = p.values {
            mine.set_values(v);
        }
        let (form, r) = mine.rebase_from(&opened, cur);
        let rebased = RebasedForm {
            initial: form.initial().clone(),
            basis: form.basis(),
            overwritten: r.overlap,
            dropped: r.dropped,
        };
        (form, true, Some(rebased))
    } else {
        let mut cur = cur;
        if let Some(v) = p.values {
            cur.set_values(v);
        }
        (cur, false, None)
    };
    let (op, error) = match form.save(today, &snap.config.work_days) {
        Ok(op) => (Some(op), None),
        Err(e) => (None, Some(e)),
    };
    serde_json::to_value(TasksFormResult {
        epoch: shared.epoch.clone(),
        revision,
        form: form_state(&form, today, &snap.config.work_days),
        op,
        error,
        changed_since,
        rebased,
    })
    .map_err(internal)
}
