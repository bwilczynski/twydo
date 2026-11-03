//! The write gate: an operation runs only when the compatibility matrix and the verified
//! shapes allow it (spec §6). Otherwise [`check`] returns the exact reason.
//!
//! The edit functions trust the gate to exclude the cases they were not built for, so the
//! rules here are the only guard. Rules are applied in order and the first failure wins:
//!
//! 1. target lookup (UID, duplicates, `Task::blocked`, destination list);
//!
//!    1b. family ops: completing, reopening, moving or deleting a parent touches every child,
//!    so each child must pass the same lookup, then have cases (rule 2) and, if it repeats,
//!    its class's preserve verdict (rule 4), or the op is refused with the child's reason.
//!    A checklist's date change whose items have dates of their own is such an op too
//!    ([`checklist_date_change`]); it may only set a new date-only due, and in verified
//!    mode needs the `checklist-item-dates` edge case. Moving or deleting
//!    one item alone, reopening an item of a completed or cancelled parent, or giving a
//!    checklist item a due or start date, is refused;
//! 2. [`task_cases`]: the task's own case (`one-time`, `monthly`, `yearly`) and family case
//!    (`checklist`, `project`, `recurring-project`, `project-recurring-child`), or why it has
//!    none: a JSON `TaskType` (when the JSON metadata is readable) that is not 0, 1 or 2 or
//!    does not match the task's kind (0 task, 1 checklist, 2 project; a parent without
//!    `@tasktype` counts as a task) refuses with "this 2Do task type is not verified yet";
//!    then a broken parent link ("broken parent link"), nested families, repeating
//!    checklists, a repeating child of a repeating project, children of a plain task, or the
//!    repeat refusals of the own case;
//! 3. the `[[operation]]` rows of every case the op touches: own, family, and the due/start
//!    attribute cases (with rule 6, the only rules the write mode changes);
//! 4. the `[[rule]]` verdicts of a repeating task;
//! 5. the verified shape of a repeating completion;
//! 6. reopening a repeat's history copy: the monthly or yearly `reopen` row, checked as in
//!    rule 3 (so the write mode applies).

use std::fmt;

use chrono::NaiveDate;

use crate::compat::{Matrix, Verdict, combined_verdicts};
use crate::config::WriteMode;
use crate::edit::{Change, Due, Op, next_due};
use crate::ical::value::IcalDateTime;
use crate::model::meta_edit::MetaJson;
use crate::model::repeat::parse_rrule;
use crate::model::{MetaFormat, Status, Task, TaskKind, Trigger};
use crate::views::{Snapshot, UNTYPED_PARENT, task_rule_class};

/// Why the gate refused an operation. `Display` is the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocked(pub String);

impl fmt::Display for Blocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Blocked {}

fn blocked<T>(reason: impl Into<String>) -> Result<T, Blocked> {
    Err(Blocked(reason.into()))
}

const NESTED: &str = "nested projects and checklists are not verified yet";
const REPEATING_CHECKLIST: &str = "repeating checklists are not verified yet";
const CHILD_RECURRENCE: &str = "a repeating task inside a repeating project is not verified yet \
                                (project-child-own-recurrence)";
const PLAIN_PARENT: &str = "subtasks of a plain task are not verified yet";
const START_OFFSETS: &str =
    "completing a repeating task with a start date is not verified yet (start-due-alarm-offsets)";

/// Allow `op` or say why not. `today` is the local date the caller considers current.
pub fn check(
    snap: &Snapshot,
    m: &Matrix,
    mode: WriteMode,
    op: &Op,
    today: NaiveDate,
) -> Result<(), Blocked> {
    // Rule 1: target lookup.
    let target = match op {
        Op::Create(n) => {
            known_list(snap, &n.collection)?;
            None
        }
        Op::Edit { uid, .. } | Op::Complete { uid } | Op::Reopen { uid } | Op::Delete { uid } => {
            Some(lookup(snap, uid)?)
        }
        Op::Move { uid, to } => {
            let i = lookup(snap, uid)?;
            known_list(snap, to)?;
            Some(i)
        }
    };

    // Rule 1b: family operations.
    if let Some(i) = target {
        family_op(snap, m, op, i)?;
        // Twydo's version of 2Do's item-date removal is not checked on the phone yet.
        if mode == WriteMode::Verified
            && checklist_date_change(snap, i, op)
            && m.edge_case(CHECKLIST_ITEM_DATES)
                .is_none_or(|c| c.verdict != Verdict::Pass)
        {
            return blocked(format!(
                "changing a checklist's date while its items have dates of their own is not \
                 verified yet ({CHECKLIST_ITEM_DATES})"
            ));
        }
    }

    // Setting or removing a repeat rule is not verified yet (Plan 7 Task 7 grants it).
    if let Op::Edit { changes, .. } = op
        && changes.iter().any(|c| matches!(c, Change::Repeat(_)))
    {
        return blocked("changing the repeat rule is not verified yet");
    }

    // Changing a task's type is not verified yet (Plan 7 Task 7 grants it).
    if let Op::Edit { changes, .. } = op
        && changes.iter().any(|c| matches!(c, Change::Kind(_)))
    {
        return blocked("changing a task's type is not verified yet");
    }

    // Creating projects, checklists, items or repeating tasks is not verified yet (Plan 7
    // Task 7 grants it).
    if let Op::Create(n) = op
        && (n.kind != TaskKind::Task || n.parent.is_some() || n.repeat.is_some())
    {
        return blocked(
            "creating projects, checklists, items or repeating tasks is not verified yet",
        );
    }

    // Rule 2: the task's cases.
    let cases = match target {
        Some(i) => task_cases(snap, i)?,
        None => Cases {
            own: "one-time",
            family: None,
        },
    };

    // Rule 3: matrix rows for every case the op touches.
    let name = op_name(op);
    for c in op_cases(snap, target, cases, op) {
        operation_row(m, mode, c, name)?;
    }

    let Some(i) = target else {
        return Ok(());
    };
    let task = &snap.tasks[i];

    // Rules 4 and 5: repeating tasks.
    if let Some(class) = task_rule_class(task, snap) {
        let not_verified = || blocked(format!("{class} repeats are not verified yet"));
        let (preserve, execute, edit) = combined_verdicts(m, &class);
        if preserve != Verdict::Pass {
            return not_verified();
        }
        if let Op::Edit { changes, .. } = op {
            if changes.iter().any(|c| matches!(c, Change::Due(None))) {
                return blocked("removing the due date of a repeating task is not verified yet");
            }
            if edit != Verdict::Pass
                && changes
                    .iter()
                    .any(|c| matches!(c, Change::RepeatInterval(_)))
            {
                return not_verified();
            }
        }
        if let Op::Complete { .. } = op {
            if execute != Verdict::Pass {
                return not_verified();
            }
            recurring_complete_shape(task, m, today)?;
        }
    }

    // Rule 6: a history copy left by completing a repeat.
    if let Op::Reopen { .. } = op
        && is_history_copy(task)
    {
        let reopen_ok = ["monthly", "yearly"]
            .iter()
            .any(|c| operation_row(m, mode, c, "reopen").is_ok());
        if !reopen_ok {
            return blocked("reopening a repeat's history copy is not verified yet");
        }
    }
    Ok(())
}

/// The matrix cases of a task: its own case and, for a member of a family (a checklist, a
/// project, or one of their items), the family's case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cases {
    /// `one-time`, `monthly` or `yearly`.
    pub own: &'static str,
    /// `checklist`, `project`, `recurring-project` or `project-recurring-child`.
    pub family: Option<&'static str>,
}

/// The matrix cases of the task at `idx`, or why it has none. Only the shapes the Plan 6
/// iPhone session captured have cases: one level of children under a typed checklist or
/// project, no repeating checklist or item, no repeating child of a repeating project.
pub fn task_cases(snap: &Snapshot, idx: usize) -> Result<Cases, Blocked> {
    let t = &snap.tasks[idx];
    if !known_task_type(t) {
        return blocked("this 2Do task type is not verified yet");
    }
    let parent = snap.parent(idx);
    let children = snap.children(idx);
    if t.parent.is_some() && parent.is_none() {
        return blocked("broken parent link");
    }
    let nested = match parent {
        Some(p) => !children.is_empty() || snap.parent(p).is_some() || t.kind != TaskKind::Task,
        None => children.iter().any(|&c| !snap.children(c).is_empty()),
    };
    if nested {
        return blocked(NESTED);
    }
    if parent.is_none() && t.kind == TaskKind::Task {
        return Ok(Cases {
            own: own_case(snap, t)?,
            family: None,
        });
    }
    // The checklist or project: the task itself or its parent.
    let head = &snap.tasks[parent.unwrap_or(idx)];
    // `Snapshot::new` shows a parent without `@tasktype` as a project; it is still a plain task.
    if declared_kind(head) == TaskKind::Task {
        return blocked(PLAIN_PARENT);
    }
    let (head_repeats, own_repeats) = (head.rrule.is_some(), t.rrule.is_some());
    let family = match head.kind {
        TaskKind::Checklist if head_repeats || own_repeats => return blocked(REPEATING_CHECKLIST),
        TaskKind::Checklist => "checklist",
        _ if parent.is_none() && head_repeats => "recurring-project",
        _ if parent.is_none() => "project",
        _ if head_repeats && own_repeats => return blocked(CHILD_RECURRENCE),
        _ if head_repeats => "recurring-project",
        _ if own_repeats => "project-recurring-child",
        _ => "project",
    };
    Ok(Cases {
        own: own_case(snap, t)?,
        family: Some(family),
    })
}

/// The kind the task's file declares: `Snapshot::new` shows a parent without `@tasktype` as
/// a project, but it is still a plain task.
fn declared_kind(t: &Task) -> TaskKind {
    if t.warnings.iter().any(|w| w == UNTYPED_PARENT) {
        TaskKind::Task
    } else {
        t.kind
    }
}

/// A readable JSON `TaskType` must be 0, 1 or 2 and match the declared kind (0 task,
/// 1 checklist, 2 project). Without one there is nothing to check.
fn known_task_type(t: &Task) -> bool {
    let Some(n) = json_meta(t).and_then(|m| m.get_i64("TaskType")) else {
        return true;
    };
    let kind = match n {
        0 => TaskKind::Task,
        1 => TaskKind::Checklist,
        2 => TaskKind::Project,
        _ => return false,
    };
    kind == declared_kind(t)
}

/// A task's own case from its repeat rule: `one-time`, `monthly` or `yearly`.
fn own_case(snap: &Snapshot, t: &Task) -> Result<&'static str, Blocked> {
    let Some(class) = task_rule_class(t, snap) else {
        return Ok("one-time");
    };
    let base = class.split('+').next().unwrap_or(&class);
    if class.split('+').any(|x| x == "from-completion") {
        blocked("repeat from completion is not verified yet")
    } else if base.starts_with("months-") {
        Ok("monthly")
    } else if base.starts_with("years") {
        Ok("yearly")
    } else {
        blocked(format!("{class} repeats are not verified yet"))
    }
}

/// The edge case a checklist date change with dated items needs in verified mode.
const CHECKLIST_ITEM_DATES: &str = "checklist-item-dates";
const ITEM_DATES: &str = "checklist items take their date from the checklist";
const DATED_ITEMS: &str = "removing a checklist's date, or giving it a time or a start date, \
                           is not verified yet while its items have dates of their own";

/// Whether `op` changes the due or start date of the checklist at `idx` while some of its
/// items carry a date of their own. 2Do then removes the items' dates
/// (`plan6-twydo-list1/01-phone-edit`), so the op writes the family.
pub fn checklist_date_change(snap: &Snapshot, idx: usize, op: &Op) -> bool {
    let Op::Edit { changes, .. } = op else {
        return false;
    };
    snap.tasks[idx].kind == TaskKind::Checklist
        && changes
            .iter()
            .any(|c| matches!(c, Change::Due(_) | Change::Start(_)))
        && snap
            .children(idx)
            .iter()
            .any(|&c| snap.tasks[c].due.is_some())
}

/// Rule 1b: an op on a parent touches every child, so each must pass [`lookup`], then have
/// cases ([`task_cases`]) and, if it repeats, its class's `[[rule]]` preserve verdict (rule
/// 4); an item moved, deleted, or reopened under a completed or cancelled parent, on its own
/// is not verified, nor is a date on a checklist item.
fn family_op(snap: &Snapshot, m: &Matrix, op: &Op, idx: usize) -> Result<(), Blocked> {
    if let Some(p) = snap.parent(idx) {
        if let Op::Edit { changes, .. } = op
            && snap.tasks[p].kind == TaskKind::Checklist
            && changes
                .iter()
                .any(|c| matches!(c, Change::Due(_) | Change::Start(_)))
        {
            return blocked(ITEM_DATES);
        }
        match op {
            Op::Move { .. } => {
                return blocked(
                    "moving one item out of its project or checklist is not verified yet",
                );
            }
            Op::Delete { .. } => {
                return blocked("deleting one item of a project or checklist is not verified yet");
            }
            Op::Reopen { .. } if snap.tasks[p].status != Status::Open => {
                return blocked(
                    "reopening an item of a completed project or checklist is not verified yet",
                );
            }
            _ => {}
        }
    }
    let date_change = checklist_date_change(snap, idx, op);
    if date_change
        && let Op::Edit { changes, .. } = op
        && !changes.iter().all(|c| match c {
            Change::Due(d) => matches!(d, Some(Due::Date(_))),
            Change::Start(_) => false,
            _ => true,
        })
    {
        return blocked(DATED_ITEMS);
    }
    if date_change
        || matches!(
            op,
            Op::Complete { .. } | Op::Reopen { .. } | Op::Move { .. } | Op::Delete { .. }
        )
    {
        // Two passes: any child's lookup reason wins over any child's shape reason.
        for &c in snap.children(idx) {
            lookup(snap, &snap.tasks[c].uid)?;
        }
        for &c in snap.children(idx) {
            task_cases(snap, c)?;
            if let Some(class) = task_rule_class(&snap.tasks[c], snap)
                && combined_verdicts(m, &class).0 != Verdict::Pass
            {
                return blocked(format!("{class} repeats are not verified yet"));
            }
        }
    }
    Ok(())
}

/// The single task with `uid`, unless it is missing, duplicated or blocked.
fn lookup(snap: &Snapshot, uid: &str) -> Result<usize, Blocked> {
    let mut hits = snap.tasks.iter().enumerate().filter(|(_, t)| t.uid == uid);
    let Some((i, t)) = hits.next() else {
        return blocked(format!("no task with UID {uid}"));
    };
    if hits.next().is_some() {
        return blocked("duplicate UID");
    }
    match &t.blocked {
        Some(reason) => blocked(reason.clone()),
        None => Ok(i),
    }
}

fn known_list(snap: &Snapshot, id: &str) -> Result<(), Blocked> {
    if snap.collections.iter().any(|c| c.id == id) {
        Ok(())
    } else {
        blocked(format!("no list {id}"))
    }
}

fn op_name(op: &Op) -> &'static str {
    match op {
        Op::Create(_) => "create",
        Op::Edit { .. } => "edit",
        Op::Complete { .. } => "complete",
        Op::Reopen { .. } => "reopen",
        Op::Move { .. } => "move",
        Op::Delete { .. } => "delete",
    }
}

fn due_case(timed: bool) -> &'static str {
    if timed { "timed" } else { "date-only" }
}

/// The matrix cases `op` touches: the own and family cases, then the attribute cases of a
/// create (the new task's due and start) or an edit (`Due`: the due before and after; `Start`).
fn op_cases(snap: &Snapshot, target: Option<usize>, cases: Cases, op: &Op) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = std::iter::once(cases.own).chain(cases.family).collect();
    let mut add = |c: &'static str| {
        if !out.contains(&c) {
            out.push(c);
        }
    };
    match op {
        Op::Create(n) => {
            if let Some(d) = n.due {
                add(due_case(matches!(d, Due::Time(_))));
            }
            if n.start.is_some() {
                add("start-date");
            }
        }
        Op::Edit { changes, .. } => {
            let before = target.and_then(|i| snap.tasks[i].due.as_ref());
            for c in changes {
                match c {
                    Change::Due(after) => {
                        if let Some(b) = before {
                            add(due_case(!b.is_date_only()));
                        }
                        if let Some(a) = after {
                            add(due_case(matches!(a, Due::Time(_))));
                        }
                    }
                    Change::Start(_) => add("start-date"),
                    Change::Summary(_)
                    | Change::Description(_)
                    | Change::RepeatInterval(_)
                    | Change::Repeat(_)
                    | Change::Kind(_) => {}
                }
            }
        }
        _ => {}
    }
    out
}

/// Rule 3: the `[[operation]]` row needs an iPhone baseline, and in verified mode both
/// Twydo round trips. A missing row counts as unverified.
fn operation_row(m: &Matrix, mode: WriteMode, case: &str, op: &str) -> Result<(), Blocked> {
    let row = m.operation(case, op);
    let get = |f: fn(&crate::compat::OperationEntry) -> Verdict| row.map_or(Verdict::Unverified, f);
    let mut columns = vec![("iphone_baseline", get(|e| e.iphone_baseline))];
    if mode == WriteMode::Verified {
        columns.push(("twydo_to_iphone", get(|e| e.twydo_to_iphone)));
        columns.push(("iphone_to_twydo", get(|e| e.iphone_to_twydo)));
    }
    match columns.into_iter().find(|(_, v)| *v != Verdict::Pass) {
        Some((col, _)) => blocked(format!("{op} for {case} tasks is not verified yet ({col})")),
        None => Ok(()),
    }
}

/// Rule 5: the repeating completions `complete_recurring` was built for: a date-only or
/// floating due, a rule [`next_due`] accepts, a JSON `StartDate` (if any) on a UTC midnight
/// after 1970, no absolute alert and no start-day delay. A due
/// time, start date or relative alerts need the `start-due-alarm-offsets` edge case to pass;
/// a next due on or before `today` (a missed occurrence) needs `monthly-15th-three-months-late`.
fn recurring_complete_shape(t: &Task, m: &Matrix, today: NaiveDate) -> Result<(), Blocked> {
    let passes = |id: &str| m.edge_case(id).is_some_and(|e| e.verdict == Verdict::Pass);
    let offsets = passes("start-due-alarm-offsets");
    let (due, timed) = match &t.due {
        None => {
            return blocked("completing a repeating task without a due date is not verified yet");
        }
        Some(IcalDateTime::Date(d)) => (*d, false),
        Some(IcalDateTime::Floating(dt)) => (dt.date(), true),
        Some(_) => {
            return blocked(
                "completing a repeating task with a zoned due time is not verified yet",
            );
        }
    };
    let raw = t.rrule.as_deref().unwrap_or_default();
    let rule = parse_rrule(raw).or_else(|e| blocked(format!("unreadable repeat rule: {e}")))?;
    let next = next_due(due, &rule).or_else(|e| blocked(e.to_string()))?;
    let meta = json_meta(t);
    let has_start = t.start.is_some()
        || meta
            .as_ref()
            .is_some_and(|m| m.get_i64("StartDate").is_some());
    if (timed || has_start || !t.alarms.is_empty()) && !offsets {
        return blocked(START_OFFSETS);
    }
    // 2Do writes `StartDate` as a UTC midnight after 1970 (H9).
    if meta
        .as_ref()
        .and_then(|m| m.get_i64("StartDate"))
        .is_some_and(|n| n <= 0 || n % 86_400 != 0)
    {
        return blocked(
            "completing a repeating task with an unusual start date is not verified yet",
        );
    }
    if t.alarms
        .iter()
        .any(|a| matches!(a.trigger, Trigger::Absolute(_)))
    {
        return blocked("completing a repeating task with an absolute alert is not verified yet");
    }
    if meta
        .as_ref()
        .and_then(|m| m.get_i64("StartDayDelay"))
        .is_some_and(|n| n != 0)
    {
        return blocked("completing a repeating task with a start-day delay is not verified yet");
    }
    if next <= today && !passes("monthly-15th-three-months-late") {
        return blocked(
            "completing a repeating task after a missed occurrence is not verified yet \
             (monthly-15th-three-months-late)",
        );
    }
    Ok(())
}

/// The task's `<2Do Meta>` JSON, when it has readable JSON metadata.
fn json_meta(t: &Task) -> Option<MetaJson> {
    if t.meta.format != MetaFormat::Json || t.meta.unreadable {
        return None;
    }
    MetaJson::parse(t.meta.raw.as_deref()?)
}

/// A completed task whose JSON `RUID` names another task: the copy 2Do leaves when a repeat
/// is completed.
fn is_history_copy(t: &Task) -> bool {
    t.status == Status::Completed
        && json_meta(t)
            .and_then(|m| m.get_str("RUID").map(str::to_string))
            .is_some_and(|r| !r.is_empty() && r != t.uid)
}
