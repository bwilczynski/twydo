//! The write gate: an operation runs only when the compatibility matrix and the verified
//! shapes allow it (spec §6). Otherwise [`check`] returns the exact reason.
//!
//! The edit functions trust the gate to exclude the cases they were not built for, so the
//! rules here are the only guard. Rules are applied in order and the first failure wins:
//!
//! 1. target lookup (UID, duplicates, `Task::blocked`, destination list);
//! 2. [`task_case`]: the task's matrix case, or why it has none;
//! 3. the `[[operation]]` rows of every case the op touches (with rule 6, the only rules
//!    the write mode changes);
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
use crate::model::{MetaFormat, Status, Task, TaskKind};
use crate::views::{Snapshot, task_rule_class};

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

const PROJECTS: &str = "projects and checklists cannot be edited yet";
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

    // Rule 2: the task's case.
    let case = match target {
        Some(i) => task_case(snap, i)?,
        None => "one-time",
    };

    // Rule 3: matrix rows for every case the op touches.
    let name = op_name(op);
    for c in op_cases(snap, target, case, op) {
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
            recurring_complete_shape(task, today)?;
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

/// The matrix case of the task at `idx`: `one-time`, `monthly` or `yearly`.
pub fn task_case(snap: &Snapshot, idx: usize) -> Result<&'static str, Blocked> {
    let t = &snap.tasks[idx];
    let typed = json_meta(t)
        .and_then(|m| m.get_i64("TaskType"))
        .is_some_and(|n| n != 0);
    if t.kind != TaskKind::Task
        || typed
        || t.parent.is_some()
        || snap.parent(idx).is_some()
        || !snap.children(idx).is_empty()
    {
        return blocked(PROJECTS);
    }
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

/// The matrix cases `op` touches: the task case, then the attribute cases of a create
/// (the new task's due and start) or an edit (`Due`: the due before and after; `Start`).
fn op_cases(
    snap: &Snapshot,
    target: Option<usize>,
    case: &'static str,
    op: &Op,
) -> Vec<&'static str> {
    let mut out = vec![case];
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
                    Change::Summary(_) | Change::Description(_) | Change::RepeatInterval(_) => {}
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

/// Rule 5: the only repeating completion `complete_recurring` was built for: a date-only due,
/// a rule [`next_due`] accepts, no start date, alarms or start-day delay, and an occurrence
/// that is not already missed (the next due lies after `today`).
fn recurring_complete_shape(t: &Task, today: NaiveDate) -> Result<(), Blocked> {
    let due = match &t.due {
        None => {
            return blocked("completing a repeating task without a due date is not verified yet");
        }
        Some(IcalDateTime::Date(d)) => *d,
        Some(_) => {
            return blocked("completing a repeating task with a due time is not verified yet");
        }
    };
    let raw = t.rrule.as_deref().unwrap_or_default();
    let rule = parse_rrule(raw).or_else(|e| blocked(format!("unreadable repeat rule: {e}")))?;
    let next = next_due(due, &rule).or_else(|e| blocked(e.to_string()))?;
    let meta = json_meta(t);
    if t.start.is_some()
        || meta
            .as_ref()
            .is_some_and(|m| m.get_i64("StartDate").is_some())
    {
        return blocked(START_OFFSETS);
    }
    if !t.alarms.is_empty() {
        return blocked(START_OFFSETS);
    }
    if meta
        .as_ref()
        .and_then(|m| m.get_i64("StartDayDelay"))
        .is_some_and(|n| n != 0)
    {
        return blocked("completing a repeating task with a start-day delay is not verified yet");
    }
    if next <= today {
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
