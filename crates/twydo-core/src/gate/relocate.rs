//! Relocation changes identities/links and stamps, never scheduling semantics.
use super::*;

pub(super) fn case(snap: &Snapshot, idx: usize) -> &'static str {
    if declared_kind(&snap.tasks[idx]) == TaskKind::Task && snap.children(idx).is_empty() {
        "standalone-resource"
    } else {
        "flat-family"
    }
}

pub(super) fn check(
    snap: &Snapshot,
    matrix: &Matrix,
    mode: WriteMode,
    idx: usize,
    to: &str,
) -> Result<(), Blocked> {
    known_list(snap, to)?;
    let head = &snap.tasks[idx];
    if head.collection == to {
        return blocked("task is already in the destination list");
    }
    if head.parent.is_some() || snap.parent(idx).is_some() {
        return blocked("moving one item out of its project or checklist is not verified yet");
    }
    let family = case(snap, idx) == "flat-family";
    if family && declared_kind(head) == TaskKind::Task {
        return blocked(PLAIN_PARENT);
    }
    super::relationships::check(snap, idx)?;
    for i in std::iter::once(idx).chain(snap.children(idx).iter().copied()) {
        let t = &snap.tasks[lookup(snap, &snap.tasks[i].uid)?];
        if !known_task_type(t) {
            return blocked("this 2Do task type is not verified yet");
        }
        if t.collection != head.collection {
            return blocked("family members must be in the same list");
        }
        if i != idx && (declared_kind(t) != TaskKind::Task || !snap.children(i).is_empty()) {
            return blocked(NESTED);
        }
        // Validate the exact resource writer as well as the parsed identity. This
        // catches duplicate links/overrides without requiring recurrence execution.
        let cal = snap
            .source(&t.path)
            .and_then(|b| crate::ical::parse(b).ok())
            .ok_or_else(|| Blocked("missing relocation source".into()))?;
        let todos: Vec<_> = cal.root.children_named("VTODO").collect();
        if todos.len() != 1
            || todos[0].props_named("UID").count() != 1
            || todos[0].props_named("RELATED-TO").count() > 1
        {
            return blocked("ambiguous relocation identity or links");
        }
        crate::edit::moved_copy(
            &cal,
            "relocation-contract-probe",
            (i != idx).then_some("relocation-parent-probe"),
            chrono::Utc::now(),
        )
        .map_err(|e| Blocked(e.to_string()))?;
    }
    operation_row(matrix, mode, case(snap, idx), "move")
}

/// Rebuild parsed structure from edited in-memory resources; no speculative disk writes.
pub(super) fn after_edit(
    snap: &Snapshot,
    uid: &str,
    changes: &[Change],
    today: NaiveDate,
) -> Result<Snapshot, Blocked> {
    let mut report = crate::vdir::LoadReport {
        collections: snap.collections.clone(),
        errors: snap.errors.clone(),
        ..Default::default()
    };
    let now = today.and_hms_opt(12, 0, 0).unwrap().and_utc();
    for t in &snap.tasks {
        let mut cal = snap
            .source(&t.path)
            .and_then(|b| crate::ical::parse(b).ok())
            .ok_or_else(|| Blocked("missing relocation source".into()))?;
        let bytes = if t.uid == uid {
            crate::edit::apply_changes(&mut cal, changes, now)
                .map_err(|e| Blocked(e.to_string()))?;
            crate::ical::serialize(&cal)
        } else {
            snap.source(&t.path).unwrap().to_vec()
        };
        report.tasks.extend(crate::model::tasks_from_calendar(
            &cal,
            &t.collection,
            &t.path,
        ));
        report.sources.insert(t.path.clone(), bytes);
    }
    Ok(Snapshot::new(report, snap.config.clone()))
}
