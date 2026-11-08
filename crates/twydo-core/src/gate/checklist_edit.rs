//! Scheduling contracts for flat recurring checklists. These are edits, not
//! permission to complete the schedule produced by an edit.
use super::*;

pub(super) fn check(
    snap: &Snapshot,
    matrix: &Matrix,
    mode: WriteMode,
    idx: usize,
    changes: &[Change],
) -> Option<Result<(), Blocked>> {
    let t = &snap.tasks[idx];
    if t.kind != TaskKind::Checklist || t.rrule.is_none() {
        return None;
    }
    let old = t.rrule.as_deref().and_then(|r| parse_rrule(r).ok());
    let interval = changes.iter().any(|c| match c {
        Change::RepeatInterval(_) => true,
        Change::Repeat(Some(spec)) => old.as_ref().is_some_and(|old| {
            let mut normalized = spec.rule.clone();
            normalized.interval = old.interval;
            normalized == *old
                && spec.from_completion == t.from_completion
                && spec.rule.interval != old.interval
        }),
        _ => false,
    });
    let due = changes.iter().any(|c| matches!(c, Change::Due(_)));
    if !due && !interval {
        return None;
    }
    Some((|| {
        if !captured_monthly_checklist(snap, idx)
            || !json_meta(t)
                .as_ref()
                .is_some_and(|m| date_only_meta(m) && m.get_i64("TaskType") == Some(1))
        {
            return blocked(
                "checklist scheduling needs an open top-level plain monthly date-only checklist",
            );
        }
        validate_source(snap, idx)?;
        validate_members(snap, idx)?;
        if interval
            && !json_meta(t).is_some_and(|meta| {
                [
                    ("RecurrenceValue", 1),
                    ("RecurrenceType", 258),
                    ("RecurrenceFrom", 1),
                    ("RecurrenceEndType", 0),
                ]
                .iter()
                .all(|(key, value)| meta.get_i64(key) == Some(*value))
            })
        {
            return blocked(
                "checklist interval editing needs recognized monthly recurrence metadata",
            );
        }
        let case = if due {
            "recurring-checklist-due"
        } else {
            "recurring-checklist-interval"
        };
        for change in changes {
            let supported = match change {
                Change::Summary(_) | Change::Description(_) => true,
                Change::Due(Some(Due::Date(d))) => due && !interval && d.day() <= 28,
                Change::RepeatInterval(2) => interval && !due,
                Change::Repeat(Some(spec)) => {
                    interval
                        && !due
                        && spec.rule.interval == 2
                        && !spec.from_completion
                        && old.as_ref().is_some_and(|old| {
                            let mut normalized = spec.rule.clone();
                            normalized.interval = old.interval;
                            normalized == *old
                        })
                }
                _ => false,
            };
            if !supported {
                return blocked("unsupported mixed checklist scheduling edit");
            }
        }
        let class = task_rule_class(t, snap)
            .ok_or_else(|| Blocked("unreadable checklist recurrence".into()))?;
        if combined_verdicts(matrix, &class).0 != Verdict::Pass
            || (interval && combined_verdicts(matrix, &class).2 != Verdict::Pass)
        {
            return blocked("checklist recurrence preservation is not verified yet");
        }
        // Unlike broad edit rows, scheduling rows need affirmative evidence even
        // in test mode: the interval candidate must never silently ship there.
        let row = matrix
            .operation(case, "edit")
            .ok_or_else(|| Blocked(format!("{case} is not verified yet")))?;
        if row.iphone_baseline != Verdict::Pass
            || row.twydo_to_iphone != Verdict::Pass
            || row.iphone_to_twydo != Verdict::Pass
        {
            return blocked(format!("{case} is not verified yet"));
        }
        operation_row(matrix, mode, case, "edit")
    })())
}

fn validate_members(snap: &Snapshot, parent: usize) -> Result<(), Blocked> {
    super::relationships::check(snap, parent)?;
    let head = &snap.tasks[parent];
    let meta = json_meta(head)
        .ok_or_else(|| Blocked("checklist scheduling needs readable JSON".into()))?;
    let alias = meta.get_str("uid").unwrap_or(&head.uid);
    if head.parent.is_some() || meta.get_str("parentuid").is_some_and(|s| !s.is_empty()) {
        return blocked(NESTED);
    }
    // Scan both link encodings, including resources omitted from the indexed
    // family. A JSON-only or conflicting link must not escape validation.
    for (i, t) in snap.tasks.iter().enumerate() {
        let json = json_meta(t);
        let json_parent = json.as_ref().and_then(|m| m.get_str("parentuid"));
        let source = snap
            .source(&t.path)
            .and_then(|b| crate::ical::parse(b).ok());
        let source_linked = source.as_ref().is_some_and(|cal| {
            cal.root.children_named("VTODO").any(|todo| {
                todo.props_named("RELATED-TO")
                    .any(|p| p.value.trim() == head.uid)
            })
        });
        let linked = source_linked
            || snap.parent(i) == Some(parent)
            || t.parent.as_deref() == Some(head.uid.as_str())
            || json_parent.is_some_and(|p| p == head.uid || p == alias);
        if !linked {
            continue;
        }
        lookup(snap, &t.uid)?;
        validate_source(snap, i)?;
        let source_links_ok = source.as_ref().is_some_and(|cal| {
            let todos: Vec<_> = cal.root.children_named("VTODO").collect();
            todos.len() == 1 && {
                let links: Vec<_> = todos[0].props_named("RELATED-TO").collect();
                links.len() == 1 && links[0].value == head.uid
            }
        });
        if snap.parent(i) != Some(parent)
            || !source_links_ok
            || !json_parent.is_some_and(|p| p == alias)
            || t.collection != head.collection
            || !known_task_type(t)
            || t.kind != TaskKind::Task
            || !snap.children(i).is_empty()
            || t.status == Status::Cancelled
            || t.rrule.is_some()
            || t.from_completion
            || t.start.is_some()
            || !t.alarms.is_empty()
            || (t.due.is_some() && t.due != head.due)
            || !json.as_ref().is_some_and(|m| {
                date_only_meta(m) && (!m.has("TaskType") || m.get_i64("TaskType") == Some(0))
            })
        {
            return blocked(
                "checklist scheduling needs plain inherited or undated items with consistent links",
            );
        }
    }
    Ok(())
}

/// Presence with an unreadable value cannot be treated as an absent scheduling
/// field: cleanup must never guess whether a start, time or delay is active.
fn date_only_meta(meta: &MetaJson) -> bool {
    !meta.has("StartDate")
        && !meta.has("duetime")
        && (!meta.has("StartDayDelay") || meta.get_i64("StartDayDelay") == Some(0))
}

/// Parsed task fields cannot prove that ignored/duplicate source fields are safe
/// to rewrite. Check the actual component before changing scheduling fields.
fn validate_source(snap: &Snapshot, idx: usize) -> Result<(), Blocked> {
    let cal = snap
        .source(&snap.tasks[idx].path)
        .and_then(|b| crate::ical::parse(b).ok())
        .ok_or_else(|| Blocked("checklist scheduling needs readable source data".into()))?;
    let todos: Vec<_> = cal.root.children_named("VTODO").collect();
    if todos.len() != 1 {
        return blocked("checklist scheduling needs one unambiguous task component");
    }
    let todo = todos[0];
    if todo.props_named("DUE").count() > 1
        || todo.props_named("DTSTART").count() > 1
        || todo.props_named("RRULE").count() > 1
        || todo.props_named("X-2DOAPP-METADATA").count() != 1
        || todo.children_named("VALARM").next().is_some()
        || todo.prop("DTSTART").is_some_and(|start| {
            !todo
                .prop("DUE")
                .is_some_and(|due| start.value == due.value && start.params == due.params)
        })
    {
        return blocked("checklist scheduling needs unambiguous dates, metadata and no alerts");
    }
    Ok(())
}
