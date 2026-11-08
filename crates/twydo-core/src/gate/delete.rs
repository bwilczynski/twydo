//! Resource-scoped deletion: a standalone task or one flat checklist/project family.
//!
//! The dedicated rows admit removal by the resources it deletes, not by recurrence
//! execution or field-edit verdicts. Removal rewrites nothing: every recognized schedule,
//! date, alarm, status and opaque property is either deleted with its resource or left
//! byte-unchanged outside the selected scope.
//!
//! Structural validation is deliberately local and stricter than a parsed label: every
//! member must have one unique identity, readable metadata and a recognized repeat rule;
//! the top-level resource has no parent; children are flat plain tasks in the same list,
//! linked by both `RELATED-TO` and the parent's JSON identity when JSON metadata is present.

use crate::compat::Matrix;
use crate::config::WriteMode;
use crate::model::{Task, TaskKind};
use crate::views::{Snapshot, declared_kind, task_rule_class};

use super::{
    Blocked, NESTED, PLAIN_PARENT, blocked, json_meta, known_task_type, lookup, operation_row,
};

/// A top-level task with no children.
pub(super) const STANDALONE: &str = "standalone-resource";
/// A top-level checklist or project and its flat plain-task children.
pub(super) const FLAT_FAMILY: &str = "flat-family";

/// The dedicated case a top-level delete would use. The caller dispatches only when the
/// matching row exists; a plain task with children is classified as a family so the helper
/// can refuse it rather than silently treating it as standalone.
pub(super) fn case(snap: &Snapshot, idx: usize) -> &'static str {
    let task = &snap.tasks[idx];
    if declared_kind(task) == TaskKind::Task && snap.children(idx).is_empty() {
        STANDALONE
    } else {
        FLAT_FAMILY
    }
}

/// Validate the selected resource (and its family, if any), then check its own delete row.
pub(super) fn check(
    snap: &Snapshot,
    matrix: &Matrix,
    mode: WriteMode,
    idx: usize,
) -> Result<(), Blocked> {
    let head = &snap.tasks[idx];
    lookup(snap, &head.uid)?;
    super::relationships::check(snap, idx)?;
    if head.parent.is_some()
        || json_meta(head)
            .and_then(|meta| meta.get_str("parentuid").map(str::to_string))
            .is_some_and(|uid| !uid.is_empty())
    {
        return blocked("broken parent link");
    }

    let kind = declared_kind(head);
    let children = snap.children(idx);
    if kind == TaskKind::Task {
        if !children.is_empty() {
            return blocked(PLAIN_PARENT);
        }
        validate(snap, idx)?;
        return operation_row(matrix, mode, STANDALONE, "delete");
    }

    // Two passes: any member's identity reason wins over a shape reason.
    for &child in children {
        lookup(snap, &snap.tasks[child].uid)?;
    }
    validate(snap, idx)?;

    let expected_json_parent = json_parent_uid(head);
    for &child in children {
        let task = &snap.tasks[child];
        if declared_kind(task) != TaskKind::Task || !snap.children(child).is_empty() {
            return blocked(NESTED);
        }
        if task.parent.as_deref() != Some(head.uid.as_str()) {
            return blocked("broken parent link");
        }
        if task.collection != head.collection {
            return blocked("an item must be in the list of its checklist or project");
        }
        validate(snap, child)?;
        if let Some(meta) = json_meta(task)
            && let Some(link) = meta.get_str("parentuid")
            && !link.is_empty()
        {
            let expected = expected_json_parent.as_deref().unwrap_or(&head.uid);
            if link != expected {
                return blocked("deletion needs unambiguous parent links");
            }
        }
    }
    operation_row(matrix, mode, FLAT_FAMILY, "delete")
}

fn validate(snap: &Snapshot, idx: usize) -> Result<(), Blocked> {
    let task = &snap.tasks[idx];
    if !known_task_type(task) {
        return blocked("this 2Do task type is not verified yet");
    }
    if task.meta.unreadable {
        return blocked("deletion needs readable task metadata");
    }
    if task_rule_class(task, snap)
        .is_some_and(|class| class.split('+').next() == Some("unrecognized"))
    {
        return blocked("deletion needs recognized repeat rules");
    }
    Ok(())
}

/// The parent's internal JSON identity, when its metadata has one.
fn json_parent_uid(task: &Task) -> Option<String> {
    json_meta(task)
        .and_then(|meta| meta.get_str("uid").map(str::to_string))
        .filter(|uid| !uid.is_empty())
}
