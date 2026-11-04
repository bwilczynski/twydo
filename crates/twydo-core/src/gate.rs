//! The write gate: an operation runs only when the compatibility matrix and the verified
//! shapes allow it (spec §6). Otherwise [`check`] returns the exact reason.
//!
//! The edit functions trust the gate to exclude the cases they were not built for, so the
//! rules here are the only guard. Rules are applied in order and the first failure wins:
//!
//! 0. a save ([`Op::Save`]) is its edit, then its move, each checked by these rules as its
//!    own op against the loaded tasks; the first refusal wins (an empty edit is skipped).
//!    When the edit changes the type or repeat rule, the move also needs the `move` rows of
//!    the cases the edit leaves;
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
//! 2. [`task_cases`]: the task's own case (`one-time`, `daily`, `weekly`, `monthly`,
//!    `yearly`, or `from-completion` for any rule repeated from completion) and family case
//!    (`checklist`, `project`, `recurring-project`, `project-recurring-child`), or why it has
//!    none: a JSON `TaskType` (when the JSON metadata is readable) that is not 0, 1 or 2 or
//!    does not match the task's kind (0 task, 1 checklist, 2 project; a parent without
//!    `@tasktype` counts as a task) refuses with "this 2Do task type is not verified yet";
//!    then a broken parent link ("broken parent link"), nested families, repeating
//!    checklists, a repeating child of a repeating project, children of a plain task, or the
//!    repeat refusals of the own case.
//!
//!    A create has the cases of the new task: its own case from its repeat, and the family
//!    it starts (a checklist or project) or joins. A repeating task needs a due date. An
//!    item's parent must exist, be open, be a declared checklist or project with cases of its
//!    own, not be an item, and be in the item's list; the item is a task, and a checklist
//!    item takes no due, start or repeat.
//!
//!    An edit that changes the type ([`Change::Kind`]) or the repeat rule
//!    ([`Change::Repeat`]) must also leave a task with cases and a repeat with a due date
//!    (so a repeating task cannot
//!    become a checklist, an item cannot become a parent, and a repeating project cannot
//!    keep a repeating item);
//! 3. the `[[operation]]` rows of every case the op touches: own, family, and the due/start
//!    attribute cases (with rule 6, the only rules the write mode changes); for a type or
//!    repeat change, the `edit` rows of the cases it leaves too; for a `Change::Repeat`
//!    that sets, removes or reshapes the repeat (another class, from completion or end:
//!    anything but a new interval of the same rule, which Plan 4 verified as an edit), the
//!    `repeat` rows of the own and family cases before and after; and for a type change the
//!    `convert` row of its before and after types (none when the type stays). A row whose
//!    `iphone_baseline` is `unsupported` (2Do does not offer the op) reads "not supported";
//! 4. the `[[rule]]` verdicts of a repeating task; a rule Twydo writes (a create's, or a
//!    `Change::Repeat`) needs its class's preserve and edit verdicts, and changing or
//!    removing a rule needs the old class's edit verdict (on top of rule 3's `repeat` rows);
//! 5. the verified shape of a repeating completion;
//! 6. reopening a repeat's history copy: the monthly or yearly `reopen` row, checked as in
//!    rule 3 (so the write mode applies).

use std::fmt;

use chrono::NaiveDate;

use crate::compat::{Matrix, Verdict, combined_verdicts};
use crate::config::WriteMode;
use crate::edit::{Change, Due, NewTask, Op, next_due};
use crate::ical::value::IcalDateTime;
use crate::model::meta_edit::MetaJson;
use crate::model::repeat::{parse_rrule, rule_class};
use crate::model::{MetaFormat, Status, Task, TaskKind, Trigger};
use crate::repeatinput::RepeatSpec;
use crate::views::{Snapshot, declared_kind, task_rule_class};

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
const NO_DUE: &str = "a repeating task needs a due date";
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
    // Rule 0: a save is its edit, then its move, each checked against the loaded tasks.
    if let Op::Save { uid, changes, to } = op {
        if !changes.is_empty() || to.is_none() {
            let changes = changes.clone();
            check(
                snap,
                m,
                mode,
                &Op::Edit {
                    uid: uid.clone(),
                    changes,
                },
                today,
            )?;
        }
        if let Some(to) = to {
            check(
                snap,
                m,
                mode,
                &Op::Move {
                    uid: uid.clone(),
                    to: to.clone(),
                },
                today,
            )?;
            // The move writes the edited task: the cases the edit leaves need `move` rows too.
            if let Some(a) = edited_cases(snap, lookup(snap, uid)?, changes)? {
                for c in std::iter::once(a.own).chain(a.family) {
                    operation_row(m, mode, c, "move")?;
                }
            }
        }
        return Ok(());
    }

    // Rule 1: target lookup.
    let target = match op {
        Op::Create(n) => {
            known_list(snap, &n.collection)?;
            None
        }
        Op::Save { .. } => unreachable!("handled by rule 0"),
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

    // Rule 2: the task's cases, and those an edit of its type or repeat rule leaves.
    let (cases, after) = match (target, op) {
        (Some(i), Op::Edit { changes, .. }) => {
            (task_cases(snap, i)?, edited_cases(snap, i, changes)?)
        }
        (Some(i), _) => (task_cases(snap, i)?, None),
        (None, Op::Create(n)) => (create_cases(snap, n)?, None),
        (None, _) => unreachable!("only a create has no target"),
    };

    // Rule 3: matrix rows for every case the op touches.
    let name = op_name(op);
    for c in op_cases(snap, target, cases, op) {
        operation_row(m, mode, c, name)?;
    }
    if let Some(a) = after {
        for c in std::iter::once(a.own).chain(a.family) {
            operation_row(m, mode, c, "edit")?;
        }
    }
    if let (Some(i), Op::Edit { changes, .. }, Some(a)) = (target, op, after)
        && reshapes_repeat(snap, i, changes)
    {
        let before = std::iter::once(cases.own).chain(cases.family);
        let mut seen = Vec::new();
        for c in before.chain(std::iter::once(a.own)).chain(a.family) {
            if !seen.contains(&c) {
                seen.push(c);
                operation_row(m, mode, c, "repeat")?;
            }
        }
    }
    if let (Some(i), Op::Edit { changes, .. }) = (target, op)
        && let Some(case) = convert_case(snap, i, changes)
    {
        operation_row(m, mode, case, "convert")?;
    }

    // Rule 4: a repeat rule Twydo writes or changes.
    written_rule(snap, m, target, op)?;

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
            let keeps_rule = !matches!(new_repeat(changes), Some(None));
            if keeps_rule && changes.iter().any(|c| matches!(c, Change::Due(None))) {
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
    /// `one-time`, `daily`, `weekly`, `monthly`, `yearly` or `from-completion`.
    pub own: &'static str,
    /// `checklist`, `project`, `recurring-project` or `project-recurring-child`.
    pub family: Option<&'static str>,
}

/// The matrix cases of the task at `idx`, or why it has none. Only the shapes the Plan 6
/// iPhone session captured have cases: one level of children under a typed checklist or
/// project, no repeating checklist or item, no repeating child of a repeating project.
pub fn task_cases(snap: &Snapshot, idx: usize) -> Result<Cases, Blocked> {
    if !known_task_type(&snap.tasks[idx]) {
        return blocked("this 2Do task type is not verified yet");
    }
    cases_as(snap, idx, &Shape::of(snap, idx))
}

/// What the cases of a task depend on: its kind as the snapshot shows it, the kind its file
/// declares, and its repeat class.
#[derive(Debug, Clone)]
struct Shape {
    kind: TaskKind,
    declared: TaskKind,
    class: Option<String>,
}

impl Shape {
    fn of(snap: &Snapshot, idx: usize) -> Shape {
        let t = &snap.tasks[idx];
        Shape {
            kind: t.kind,
            declared: declared_kind(t),
            class: task_rule_class(t, snap),
        }
    }
}

/// [`task_cases`] of the task at `idx` if it had the shape `me` (its parent and items as
/// loaded).
fn cases_as(snap: &Snapshot, idx: usize, me: &Shape) -> Result<Cases, Blocked> {
    let t = &snap.tasks[idx];
    let parent = snap.parent(idx);
    let children = snap.children(idx);
    if t.parent.is_some() && parent.is_none() {
        return blocked("broken parent link");
    }
    let nested = match parent {
        Some(p) => !children.is_empty() || snap.parent(p).is_some() || me.kind != TaskKind::Task,
        None => children.iter().any(|&c| !snap.children(c).is_empty()),
    };
    if nested {
        return blocked(NESTED);
    }
    if parent.is_none() && me.kind == TaskKind::Task {
        return Ok(Cases {
            own: own_case(me.class.as_deref())?,
            family: None,
        });
    }
    // The checklist or project: the task itself or its parent.
    let head = match parent {
        Some(p) => Shape::of(snap, p),
        None => me.clone(),
    };
    // `Snapshot::new` shows a parent without `@tasktype` as a project; it is still a plain task.
    if head.declared == TaskKind::Task {
        return blocked(PLAIN_PARENT);
    }
    let family = family_case(
        head.kind,
        parent.is_some(),
        head.class.is_some(),
        me.class.is_some(),
    )?;
    Ok(Cases {
        own: own_case(me.class.as_deref())?,
        family: Some(family),
    })
}

/// The family case of a checklist or project of kind `head` (an item of it when `item`),
/// from whether the head and the task repeat.
fn family_case(
    head: TaskKind,
    item: bool,
    head_repeats: bool,
    own_repeats: bool,
) -> Result<&'static str, Blocked> {
    Ok(match head {
        TaskKind::Checklist if head_repeats || own_repeats => return blocked(REPEATING_CHECKLIST),
        TaskKind::Checklist => "checklist",
        _ if !item && head_repeats => "recurring-project",
        _ if !item => "project",
        _ if head_repeats && own_repeats => return blocked(CHILD_RECURRENCE),
        _ if head_repeats => "recurring-project",
        _ if own_repeats => "project-recurring-child",
        _ => "project",
    })
}

/// The cases of a new task: its own case from its repeat, and the family it starts (a new
/// checklist or project) or joins (an item of `parent`, which must be a declared checklist
/// or project in the same list, not an item, with cases of its own). A checklist item takes
/// no due, start or repeat.
fn create_cases(snap: &Snapshot, n: &NewTask) -> Result<Cases, Blocked> {
    if n.repeat.is_some() && n.due.is_none() {
        return blocked(NO_DUE);
    }
    let class = n.repeat.as_ref().map(|r| new_class(r, n.due.as_ref()));
    let own = own_case(class.as_deref())?;
    let repeats = class.is_some();
    let Some(parent) = &n.parent else {
        let family = match n.kind {
            TaskKind::Task => None,
            k => Some(family_case(k, false, repeats, repeats)?),
        };
        return Ok(Cases { own, family });
    };
    let p = lookup(snap, parent)?;
    let head = &snap.tasks[p];
    if head.status != Status::Open {
        return blocked(
            "adding an item to a completed or cancelled project or checklist is not verified yet",
        );
    }
    if n.kind != TaskKind::Task || snap.parent(p).is_some() {
        return blocked(NESTED);
    }
    if declared_kind(head) == TaskKind::Task {
        return blocked(PLAIN_PARENT);
    }
    task_cases(snap, p)?;
    if head.collection != n.collection {
        return blocked("an item must be in the list of its checklist or project");
    }
    if head.kind == TaskKind::Checklist && (n.due.is_some() || n.start.is_some() || repeats) {
        return blocked(ITEM_DATES);
    }
    let family = family_case(head.kind, true, head.rrule.is_some(), repeats)?;
    Ok(Cases {
        own,
        family: Some(family),
    })
}

/// The cases the task at `idx` has after `changes`, when they change its type or repeat rule
/// (`None` otherwise). A repeating project cannot keep a repeating item.
fn edited_cases(snap: &Snapshot, idx: usize, changes: &[Change]) -> Result<Option<Cases>, Blocked> {
    let kind = changes.iter().rev().find_map(|c| match c {
        Change::Kind(k) => Some(*k),
        _ => None,
    });
    let repeat = new_repeat(changes);
    if kind.is_none() && repeat.is_none() {
        return Ok(None);
    }
    let mut me = Shape::of(snap, idx);
    if let Some(k) = kind {
        (me.kind, me.declared) = (k, k);
    }
    if let Some(r) = repeat {
        if r.is_some() && due_after(snap, idx, changes).is_none() {
            return blocked(NO_DUE);
        }
        me.class = r.map(|r| new_class(r, due_after(snap, idx, changes).as_ref()));
    }
    let cases = cases_as(snap, idx, &me)?;
    if cases.family == Some("recurring-project")
        && snap
            .children(idx)
            .iter()
            .any(|&c| snap.tasks[c].rrule.is_some())
    {
        return blocked(CHILD_RECURRENCE);
    }
    Ok(Some(cases))
}

/// The last `Change::Repeat` in `changes`: `Some(None)` removes the rule.
fn new_repeat(changes: &[Change]) -> Option<Option<&RepeatSpec>> {
    changes.iter().rev().find_map(|c| match c {
        Change::Repeat(r) => Some(r.as_ref()),
        _ => None,
    })
}

/// The task's due date after `changes`: the last `Change::Due`, else its own.
fn due_after(snap: &Snapshot, idx: usize, changes: &[Change]) -> Option<Due> {
    let changed = changes.iter().rev().find_map(|c| match c {
        Change::Due(d) => Some(*d),
        _ => None,
    });
    changed.unwrap_or_else(|| {
        let d = snap.tasks[idx].due.as_ref()?;
        Some(Due::Date(d.local_date(snap.tz())))
    })
}

/// Whether `changes` set, remove or reshape the task's repeat: anything but a new interval
/// of the same rule (same class, from completion and end). Plan 4 verified interval edits
/// only; everything else needs the `repeat` rows.
fn reshapes_repeat(snap: &Snapshot, idx: usize, changes: &[Change]) -> bool {
    let Some(new) = new_repeat(changes) else {
        return false;
    };
    let task = &snap.tasks[idx];
    let (Some(new), Some(raw)) = (new, task.rrule.as_deref()) else {
        return true;
    };
    let Ok(mut old) = parse_rrule(raw) else {
        return true;
    };
    let due = due_after(snap, idx, changes);
    let same_class = task_rule_class(task, snap).as_deref() == Some(&new_class(new, due.as_ref()));
    old.interval = new.rule.interval;
    !(same_class && old == new.rule)
}

/// The matrix class of a rule Twydo writes, anchored on the due date it will have.
fn new_class(r: &RepeatSpec, due: Option<&Due>) -> String {
    let anchor = due.map(|d| match d {
        Due::Date(d) => *d,
        Due::Time(t) => t.date(),
    });
    rule_class(&r.rule, anchor, r.from_completion)
}

/// The `convert` case of an edit that changes the task's declared kind, if it does.
fn convert_case(snap: &Snapshot, idx: usize, changes: &[Change]) -> Option<&'static str> {
    use TaskKind::{Checklist, Project, Task};
    let to = changes.iter().rev().find_map(|c| match c {
        Change::Kind(k) => Some(*k),
        _ => None,
    })?;
    let items = !snap.children(idx).is_empty();
    Some(match (declared_kind(&snap.tasks[idx]), to) {
        (Task, Task) | (Checklist, Checklist) | (Project, Project) => return None,
        (Task, Checklist) => "task-to-checklist",
        (Task, Project) => "task-to-project",
        (Checklist, Project) => "checklist-to-project",
        (Project, Checklist) if items => "project-to-checklist",
        (Project, Checklist) => "empty-project-to-checklist",
        (_, Task) if items => "parent-with-items-to-task",
        (_, Task) => "empty-parent-to-task",
    })
}

/// How a `convert` case reads in a refusal.
fn convert_label(case: &str) -> &str {
    match case {
        "task-to-checklist" => "a task to a checklist",
        "task-to-project" => "a task to a project",
        "checklist-to-project" => "a checklist to a project",
        "project-to-checklist" => "a project with items to a checklist",
        "empty-project-to-checklist" => "an empty project to a checklist",
        "parent-with-items-to-task" => "a checklist or project with items to a task",
        "empty-parent-to-task" => "an empty checklist or project to a task",
        other => other,
    }
}

/// Rule 4 for a rule Twydo writes: a new rule's class needs its `preserve` and `edit`
/// verdicts; changing or removing a rule needs the old class's `edit` verdict.
fn written_rule(
    snap: &Snapshot,
    m: &Matrix,
    target: Option<usize>,
    op: &Op,
) -> Result<(), Blocked> {
    let (old, new) = match (target, op) {
        (None, Op::Create(n)) => (
            None,
            n.repeat.as_ref().map(|r| new_class(r, n.due.as_ref())),
        ),
        (Some(i), Op::Edit { changes, .. }) => {
            let Some(r) = new_repeat(changes) else {
                return Ok(());
            };
            let due = due_after(snap, i, changes);
            (
                task_rule_class(&snap.tasks[i], snap),
                r.map(|r| new_class(r, due.as_ref())),
            )
        }
        _ => return Ok(()),
    };
    let not_verified = |class: &str| blocked(format!("{class} repeats are not verified yet"));
    if let Some(c) = old
        && combined_verdicts(m, &c).2 != Verdict::Pass
    {
        return not_verified(&c);
    }
    if let Some(c) = new {
        let (preserve, _, edit) = combined_verdicts(m, &c);
        if preserve != Verdict::Pass || edit != Verdict::Pass {
            return not_verified(&c);
        }
    }
    Ok(())
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

/// A task's own case from its repeat class: `one-time`, `daily`, `weekly`, `monthly`,
/// `yearly`, or `from-completion` for any rule repeated from completion.
fn own_case(class: Option<&str>) -> Result<&'static str, Blocked> {
    let Some(class) = class else {
        return Ok("one-time");
    };
    let base = class.split('+').next().unwrap_or(class);
    if class.split('+').any(|x| x == "from-completion") {
        Ok("from-completion")
    } else if base == "days" {
        Ok("daily")
    } else if base == "weeks" || base == "weekdays-set" {
        Ok("weekly")
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
        Op::Save { .. } => "save",
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
    let what = match op {
        "convert" => format!("converting {}", convert_label(case)),
        "repeat" => format!("changing the repeat rule of {case} tasks"),
        _ => format!("{op} for {case} tasks"),
    };
    match columns.into_iter().find(|(_, v)| *v != Verdict::Pass) {
        Some(("iphone_baseline", Verdict::Unsupported)) => {
            blocked(format!("{what} is not supported by 2Do"))
        }
        Some((col, _)) => blocked(format!("{what} is not verified yet ({col})")),
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
