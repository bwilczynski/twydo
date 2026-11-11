//! Title/notes edits change text and stamps, never scheduling or lifecycle.
use super::*;

pub(super) fn check(
    snap: &Snapshot,
    matrix: &Matrix,
    mode: WriteMode,
    idx: usize,
    changes: &[Change],
    checker: &Checker<'_>,
) -> Option<Result<(), Blocked>> {
    if changes.is_empty()
        || !changes
            .iter()
            .all(|c| matches!(c, Change::Summary(_) | Change::Description(_)))
    {
        return None;
    }
    Some((|| {
        let head = snap.parent(idx).unwrap_or(idx);
        let parent = &snap.tasks[head];
        checker.relationships().check(snap, head)?;
        if declared_kind(parent) == TaskKind::Task && !snap.children(head).is_empty() {
            return blocked(PLAIN_PARENT);
        }
        // The flat relationship contract is independent of every held schedule,
        // including history, completed items and rules the phone cannot execute.
        for i in std::iter::once(head).chain(snap.children(head).iter().copied()) {
            let task = &snap.tasks[lookup(snap, &snap.tasks[i].uid)?];
            checker.relationships().check_identity(i)?;
            if !known_task_type(task) || task.meta.unreadable {
                return blocked("text editing needs recognized task types and readable metadata");
            }
            if task.collection != parent.collection {
                return blocked("an item must be in the list of its checklist or project");
            }
            if i != head
                && (declared_kind(task) != TaskKind::Task
                    || !snap.children(i).is_empty()
                    || checker.relationships().has_references(i))
            {
                return blocked(NESTED);
            }
            let cal = snap
                .source(&task.path)
                .and_then(|b| crate::ical::parse(b).ok())
                .ok_or_else(|| Blocked("text source is unreadable".into()))?;
            let todos: Vec<_> = cal.root.children_named("VTODO").collect();
            if todos.len() != 1
                || todos[0].prop("RECURRENCE-ID").is_some()
                || todos[0].props_named("UID").count() != 1
                || todos[0].props_named("RELATED-TO").count() > 1
                || todos[0].props_named("X-2DOAPP-METADATA").count() > 1
                || (i == idx
                    && ["SUMMARY", "DESCRIPTION", "DTSTAMP", "LAST-MODIFIED"]
                        .iter()
                        .any(|name| todos[0].props_named(name).count() > 1))
            {
                return blocked(
                    "text editing needs unambiguous resource identity, links and fields",
                );
            }
        }
        changes.iter().try_for_each(|c| {
            require(
                matrix,
                mode,
                if matches!(c, Change::Summary(_)) {
                    ContractId::EditTitleResource
                } else {
                    ContractId::EditNotesResource
                },
            )
        })
    })())
}
