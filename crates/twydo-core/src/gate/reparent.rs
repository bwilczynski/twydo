//! Membership changes require dedicated evidence; list-move evidence is insufficient.
use super::*;

pub(super) fn check(
    snap: &Snapshot,
    matrix: &Matrix,
    mode: WriteMode,
    uid: &str,
    collection: &str,
    parent: Option<&str>,
) -> Result<(), Blocked> {
    known_list(snap, collection)?;
    let i = lookup(snap, uid)?;
    let task = &snap.tasks[i];
    if task.status != Status::Open
        || declared_kind(task) != TaskKind::Task
        || !known_task_type(task)
        || !snap.children(i).is_empty()
    {
        return blocked("reparent needs one open plain leaf without children");
    }
    let identities = relationships::aliases(task);
    if snap.tasks.iter().enumerate().any(|(n, t)| {
        n != i
            && (!identities.is_disjoint(&relationships::aliases(t))
                || t.parent.as_ref().is_some_and(|p| identities.contains(p))
                || json_meta(t)
                    .and_then(|m| m.get_str("parentuid").map(str::to_owned))
                    .is_some_and(|p| identities.contains(&p)))
    }) {
        return blocked("reparent leaf has ambiguous identity or hidden descendants");
    }
    let source = snap.parent(i);
    if task.parent.is_some() && source.is_none() {
        return blocked("reparent source parent is missing");
    }
    let destination = parent.map(|p| lookup(snap, p)).transpose()?;
    if destination == Some(i) {
        return blocked("reparent cannot create a cycle");
    }
    if source == destination && task.collection == collection {
        return blocked("already in this parent/list; use item ordering");
    }
    for p in source.into_iter().chain(destination) {
        let head = &snap.tasks[p];
        if head.status != Status::Open
            || !matches!(declared_kind(head), TaskKind::Project | TaskKind::Checklist)
        {
            return blocked("reparent needs an open top-level project or checklist");
        }
        relationships::check(snap, p)?;
        let aliases = relationships::aliases(head);
        if snap
            .tasks
            .iter()
            .enumerate()
            .any(|(n, t)| n != p && !aliases.is_disjoint(&relationships::aliases(t)))
        {
            return blocked("ambiguous reparent parent identity");
        }
        for n in std::iter::once(p).chain(snap.children(p).iter().copied()) {
            lookup(snap, &snap.tasks[n].uid)?;
            if !known_task_type(&snap.tasks[n])
                || (n != p
                    && (declared_kind(&snap.tasks[n]) != TaskKind::Task
                        || !snap.children(n).is_empty()))
            {
                return blocked("reparent cannot change nested or unknown families");
            }
            raw(snap, n)?;
        }
    }
    if let Some(p) = destination
        && snap.tasks[p].collection != collection
    {
        return blocked("destination parent must be in the destination list");
    }
    if source.is_none() {
        relationships::check(snap, i)?;
    }
    let clear_dates = destination
        .is_some_and(|p| declared_kind(&snap.tasks[p]) == TaskKind::Checklist)
        || (destination.is_none()
            && source.is_some_and(|p| declared_kind(&snap.tasks[p]) == TaskKind::Checklist));
    crate::edit::reparented_copy(
        &raw(snap, i)?,
        "candidate-reparent-probe",
        parent,
        clear_dates,
        chrono::Utc::now(),
    )
    .map_err(|e| Blocked(e.to_string()))?;
    let case = match destination {
        None => "item-outdent",
        Some(p) if declared_kind(&snap.tasks[p]) == TaskKind::Checklist => "item-to-checklist",
        Some(_) => "item-to-project",
    };
    operation_row(matrix, mode, case, "reparent")
}

fn raw(snap: &Snapshot, i: usize) -> Result<crate::ical::Calendar, Blocked> {
    let cal = snap
        .source(&snap.tasks[i].path)
        .and_then(|b| crate::ical::parse(b).ok())
        .ok_or_else(|| Blocked("unreadable reparent source".into()))?;
    let todos: Vec<_> = cal.root.children_named("VTODO").collect();
    if todos.len() != 1
        || todos[0].props_named("UID").count() != 1
        || todos[0].props_named("RELATED-TO").count() > 1
        || todos[0].props_named("X-2DOAPP-METADATA").count() != 1
    {
        return blocked("ambiguous reparent identity or metadata");
    }
    if todos[0].prop("RECURRENCE-ID").is_some()
        || todos[0].prop("RELATED-TO").is_some_and(|p| {
            p.param("RELTYPE")
                .is_some_and(|r| !r.eq_ignore_ascii_case("PARENT"))
        })
        || json_meta(&snap.tasks[i]).is_none()
    {
        return blocked("unsupported reparent metadata or relationship");
    }
    Ok(cal)
}
