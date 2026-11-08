//! Detached history reopening changes lifecycle and stamps only.
use super::*;

pub(super) fn check(
    snap: &Snapshot,
    matrix: &Matrix,
    mode: WriteMode,
    idx: usize,
) -> Option<Result<(), Blocked>> {
    let task = &snap.tasks[idx];
    if task.status == Status::Completed
        && let Some(parent) = snap.parent(idx)
        && snap.tasks[parent].rrule.is_none()
        && matrix.operation("history-flat-family", "reopen").is_some()
        && json_meta(&snap.tasks[parent])
            .and_then(|m| m.get_str("RUID").map(str::to_owned))
            .is_some_and(|ruid| !ruid.is_empty() && ruid != snap.tasks[parent].uid)
    {
        return Some(blocked(
            "individual history reopening needs open-parent history evidence; use the parent operation for a completed family",
        ));
    }
    if task.rrule.is_some() || !is_history_copy(task) {
        return None;
    }
    // Individual history needs its own evidence: H7 authorizes only the parent operation.
    if snap.parent(idx).is_some() || task.parent.is_some() {
        if matrix.operation("history-task", "reopen").is_none()
            && matrix.operation("history-flat-family", "reopen").is_none()
        {
            return None;
        }
        return Some(blocked(
            "individual history reopening needs open-parent history evidence; use the parent operation for a completed family",
        ));
    }
    let case = match declared_kind(task) {
        TaskKind::Task => "history-task",
        TaskKind::Project | TaskKind::Checklist => "history-flat-family",
    };
    matrix.operation(case, "reopen")?;
    Some(validate(snap, idx).and_then(|()| operation_row(matrix, mode, case, "reopen")))
}

fn validate(snap: &Snapshot, idx: usize) -> Result<(), Blocked> {
    let head = &snap.tasks[idx];
    if declared_kind(head) == TaskKind::Task && !snap.children(idx).is_empty() {
        return blocked(PLAIN_PARENT);
    }
    super::relationships::check(snap, idx)?;
    for i in std::iter::once(idx).chain(snap.children(idx).iter().copied()) {
        let task = &snap.tasks[i];
        lookup(snap, &task.uid)?;
        if !known_task_type(task) || json_meta(task).is_none() {
            return blocked(
                "history reopening needs recognized task types and readable JSON metadata",
            );
        }
        if task.collection != head.collection {
            return blocked("an item must be in the list of its checklist or project");
        }
        if i != idx && (declared_kind(task) != TaskKind::Task || !snap.children(i).is_empty()) {
            return blocked(NESTED);
        }
        if task.rrule.is_some() {
            return blocked("history reopening does not execute or restore recurrence");
        }
        if let Some(meta) = json_meta(task)
            && [
                "RecurrenceType",
                "RecurrenceValue",
                "RecurrenceFrom",
                "RecurrenceEndType",
                "RecurrenceEndRepetitions",
                "RecurrenceEndRepetitionsOrig",
            ]
            .iter()
            .any(|key| meta.get_i64(key) != Some(0))
        {
            return blocked("history reopening needs inactive recurrence metadata");
        }
    }
    Ok(())
}
