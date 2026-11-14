//! Typed operation admission. Structural and scheduling invariants live here;
//! compatibility observations are supplied by operation-scoped contracts.

mod checklist_edit;
mod history_reopen;
mod relocate;
mod reparent;
mod text_edit;

use std::fmt;

use chrono::{Datelike, NaiveDate};

use crate::compat::{ContractId, Matrix, OperationId};
use crate::config::WriteMode;
use crate::edit::{
    Change, Due, NTH_WEEKDAY_INTERVAL, NewTask, Op, REOPEN_ENDED, interval_refusal, next_due,
};
use crate::ical::value::IcalDateTime;
use crate::model::meta_edit::MetaJson;
use crate::model::repeat::{End, Freq, RRule, parse_rrule, rule_class};
use crate::model::{MetaFormat, Status, Task, TaskKind, Trigger};
use crate::repeatinput::RepeatSpec;
use crate::views::{Snapshot, declared_kind, task_rule_class};

mod delete;
pub(crate) mod relationships;

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
const CHILD_RECURRENCE: &str = "a repeating task inside a repeating project is not verified yet \
                                (project-child-own-recurrence)";
const NO_DUE: &str = "a repeating task needs a due date";
const PLAIN_PARENT: &str = "subtasks of a plain task are not verified yet";
const PROJECT_END: &str =
    "completing a recurring project that ends or repeats from completion is not verified yet";
const START_OFFSETS: &str =
    "completing a repeating task with a start date is not verified yet (start-due-alarm-offsets)";

/// Reuse admission facts across many operations on the same immutable snapshot.
/// The borrow prevents edits while facts are reused; a reloaded/edited snapshot
/// gets a fresh checker. The writer's commit and replay guards still check live files.
pub struct Checker<'a> {
    snap: &'a Snapshot,
    matrix: &'a Matrix,
    mode: WriteMode,
    today: NaiveDate,
    relationships: std::sync::OnceLock<relationships::Index>,
}

impl<'a> Checker<'a> {
    pub fn new(snap: &'a Snapshot, matrix: &'a Matrix, mode: WriteMode, today: NaiveDate) -> Self {
        Self {
            snap,
            matrix,
            mode,
            today,
            relationships: Default::default(),
        }
    }

    pub fn snapshot(&self) -> &Snapshot {
        self.snap
    }

    pub fn check(&self, op: &Op) -> Result<(), Blocked> {
        check_with(self, op)
    }

    fn relationships(&self) -> &relationships::Index {
        self.relationships
            .get_or_init(|| relationships::Index::new(self.snap))
    }
}

/// Allow `op` or say why not. `today` is the local date the caller considers current.
pub fn check(
    snap: &Snapshot,
    m: &Matrix,
    mode: WriteMode,
    op: &Op,
    today: NaiveDate,
) -> Result<(), Blocked> {
    Checker::new(snap, m, mode, today).check(op)
}

fn check_with(checker: &Checker<'_>, op: &Op) -> Result<(), Blocked> {
    let (snap, m, mode, today) = (checker.snap, checker.matrix, checker.mode, checker.today);
    if let Op::Reparent {
        uid,
        collection,
        parent,
    } = op
    {
        return reparent::check(
            snap,
            m,
            mode,
            uid,
            collection,
            parent.as_deref(),
            checker.relationships(),
        );
    }
    if matches!(op, Op::Reorder { .. } | Op::CreateAt { .. }) {
        return check_order(snap, m, mode, op, today);
    }

    // Rule 0: a save is its edit, then its move, each checked against the loaded tasks.
    if let Op::Save { uid, changes, to } = op {
        if !changes.is_empty() || to.is_none() {
            let changes = changes.clone();
            checker.check(&Op::Edit {
                uid: uid.clone(),
                changes,
            })?;
        }
        if let Some(to) = to {
            // Both the loaded and the edited structure must be movable, as in 0.1.9.
            // The pre-edit check is not redundant: a shape the edit changes would
            // otherwise be judged only by its result, widening admission.
            checker.check(&Op::Move {
                uid: uid.clone(),
                to: to.clone(),
            })?;
            {
                let after = relocate::after_edit(snap, uid, changes, today)?;
                return relocate::check(
                    &after,
                    m,
                    mode,
                    lookup(&after, uid)?,
                    to,
                    &relationships::Index::new(&after),
                );
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
        Op::Reparent { .. } | Op::Save { .. } | Op::Reorder { .. } | Op::CreateAt { .. } => {
            unreachable!("handled above")
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

    if let (Some(i), Op::Edit { changes, .. }) = (target, op)
        && let Some(result) = text_edit::check(snap, m, mode, i, changes, checker)
    {
        return result;
    }

    if let (Some(i), Op::Move { to, .. }) = (target, op) {
        return relocate::check(snap, m, mode, i, to, checker.relationships());
    }

    // Individual deletion removes a file without editing or executing schedules.
    // Its dedicated evidence therefore precedes the ordinary task/family cases.
    if let (Some(i), Op::Delete { .. }) = (target, op)
        && let Some(p) = snap.parent(i)
    {
        let case = match declared_kind(&snap.tasks[p]) {
            TaskKind::Checklist => Some("checklist-item"),
            TaskKind::Project => Some("project-item"),
            TaskKind::Task => None,
        };
        if let Some(case) = case {
            return check_item_delete(snap, m, mode, p, case);
        }
    }

    // A top-level delete removes one resource or one flat family. Its dedicated rows
    // authorize removal without recurrence-execution or field-edit requirements.
    if let (Some(i), Op::Delete { .. }) = (target, op)
        && snap.parent(i).is_none()
    {
        return delete::check(snap, m, mode, i, checker.relationships());
    }

    if let (Some(i), Op::Reopen { .. }) = (target, op)
        && let Some(result) = history_reopen::check(snap, m, mode, i, checker)
    {
        return result;
    }

    // Rule 1b: family operations.
    if let Some(i) = target {
        family_op(snap, op, i)?;
        // Twydo's version of 2Do's item-date removal is not checked on the phone yet.
        if mode == WriteMode::Verified
            && checklist_date_change(snap, i, op)
            && m.require(ContractId::SetDueItemCleanup, mode).is_err()
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

    // Rule 5b before the rows: the last occurrence of an ended series, completed in place,
    // keeps its RRULE. Only the captured top-level UNTIL shapes may be reopened, and they
    // need a dedicated row rather than a broad daily/monthly reopen verdict.
    if let (Some(i), Op::Reopen { .. }) = (target, op)
        && snap.tasks[i].status == Status::Completed
        && snap.tasks[i].rrule.is_some()
    {
        let task = &snap.tasks[i];
        let class = task_rule_class(task, snap);
        let case = match class.as_deref() {
            Some("days+until") => "ended-until-daily",
            Some("months-day-1-28+until") => "ended-until-monthly",
            _ => return blocked(REOPEN_ENDED),
        };
        let ended = match (
            &task.due,
            task.rrule.as_deref().and_then(|r| parse_rrule(r).ok()),
        ) {
            (Some(IcalDateTime::Date(due)), Some(rule)) => {
                matches!(rule.end, End::Until(IcalDateTime::Date(_)))
                    && next_due(*due, &rule, None).is_ok_and(|n| n.is_none())
            }
            _ => false,
        };
        if task.kind != TaskKind::Task
            || snap.parent(i).is_some()
            || !snap.children(i).is_empty()
            || !ended
            || !captured_until(snap, i)
        {
            return blocked(REOPEN_ENDED);
        }
        if !preserved_schedule(class.as_deref().unwrap()) {
            return blocked("the ended repeat's preservation is not verified yet");
        }
        return require(
            m,
            mode,
            if case == "ended-until-daily" {
                ContractId::ReopenEndedDaily
            } else {
                ContractId::ReopenEndedMonthly
            },
        );
    }

    // In every mode: no interval Twydo could not complete or 2Do could not keep.
    written_interval(snap, target, op)?;

    // Rule 4 first, so a rule 2Do cannot show says so instead of citing a missing row.
    written_rule(snap, m, target, op)?;

    if let (Some(i), Op::Edit { changes, .. }) = (target, op)
        && let Some(result) = checklist_edit::check(snap, m, mode, i, changes, checker)
    {
        result?;
        // Scheduling evidence authorizes only scheduling fields. Run any text
        // subset through the real gate as its own contract, keeping custom
        // matrices and future text capabilities authoritative.
        let text: Vec<_> = changes
            .iter()
            .filter(|c| matches!(c, Change::Summary(_) | Change::Description(_)))
            .cloned()
            .collect();
        return if text.is_empty() {
            Ok(())
        } else {
            checker.check(&Op::Edit {
                uid: snap.tasks[i].uid.clone(),
                changes: text,
            })
        };
    }

    let reshaped = matches!((target, op), (Some(i), Op::Edit { changes, .. }) if reshapes_repeat(snap, i, changes));

    if let (Some(i), Op::Edit { changes, .. }) = (target, op)
        && new_repeat(changes).is_some()
        && (snap.tasks[i].kind == TaskKind::Checklist
            || after.is_some_and(|a| matches!(a.family, Some("checklist" | "recurring-checklist"))))
    {
        checklist_repeat_shape(&snap.tasks[i], changes)?;
    }
    if let (Some(i), Op::Edit { changes, .. }) = (target, op)
        && reshaped
        && (matches!(cases.own, "daily" | "yearly")
            || after.is_some_and(|a| matches!(a.own, "daily" | "yearly")))
    {
        plain_repeat_transition(&snap.tasks[i], changes)?;
    }

    let repeat_only = reshaped
        && matches!(op, Op::Edit { changes, .. } if changes.iter().all(|c| matches!(c, Change::Repeat(_))));
    operation_contracts(m, mode, cases, after, op, reshaped)?;
    if let (Some(i), Op::Edit { changes, .. }) = (target, op) {
        if changes.is_empty() {
            // The writer still touches stamps: missing evidence cannot authorize it.
            require(m, mode, ContractId::EditTitleResource)?;
        } else if changes.iter().any(|c| matches!(c, Change::Kind(_)))
            && convert_case(snap, i, changes).is_none()
        {
            require(m, mode, ContractId::ConvertSameKind)?;
        }
    }
    if let (Some(i), Op::Edit { changes, .. }) = (target, op)
        && let Some(case) = convert_case(snap, i, changes)
    {
        let id = match case {
            "task-to-checklist" => ContractId::ConvertTaskChecklist,
            "task-to-project" => ContractId::ConvertTaskProject,
            "checklist-to-project" => ContractId::ConvertChecklistProject,
            "project-to-checklist" => ContractId::ConvertProjectChecklist,
            "empty-project-to-checklist" => ContractId::ConvertEmptyProjectChecklist,
            "parent-with-items-to-task" => ContractId::ConvertParentTask,
            "empty-parent-to-task" => ContractId::ConvertEmptyParentTask,
            _ => unreachable!(),
        };
        require(m, mode, id)?;
    }

    if cases.family == Some("recurring-checklist")
        || after.is_some_and(|a| a.family == Some("recurring-checklist"))
    {
        captured_checklist_write(snap, target, op, repeat_only)?;
    }

    if let (Some(i), Op::Edit { changes, .. }) = (target, op)
        && ((snap.tasks[i].rrule.is_some() && snap.tasks[i].from_completion)
            || new_repeat(changes)
                .flatten()
                .is_some_and(|r| r.from_completion))
    {
        captured_from_completion_edit(snap, i, changes)?;
    }

    let Some(i) = target else {
        return Ok(());
    };
    let task = &snap.tasks[i];

    // Rules 4 and 5: repeating tasks.
    if let Some(class) = task_rule_class(task, snap) {
        let not_verified = || blocked(format!("{class} repeats are not verified yet"));
        if !preserved_schedule(&class) {
            return not_verified();
        }
        if let Op::Edit { changes, .. } = op {
            let keeps_rule = !matches!(new_repeat(changes), Some(None));
            if keeps_rule && changes.iter().any(|c| matches!(c, Change::Due(None))) {
                return blocked("removing the due date of a repeating task is not verified yet");
            }
            if schedule_contracts(m, OperationId::SetRepeat, &class).is_err()
                && changes
                    .iter()
                    .any(|c| matches!(c, Change::RepeatInterval(_)))
            {
                return not_verified();
            }
        }
        if let Op::Complete { .. } = op {
            // A recurring project's completion was captured for a plain rule only.
            let ends = task
                .rrule
                .as_deref()
                .and_then(|r| parse_rrule(r).ok())
                .is_some_and(|r| r.end != End::Never);
            if matches!(
                cases.family,
                Some("recurring-project" | "recurring-checklist")
            ) && task.rrule.is_some()
                && (ends || task.from_completion)
            {
                return blocked(if cases.family == Some("recurring-checklist") {
                    "completing a recurring checklist that ends or repeats from completion is not verified yet"
                } else {
                    PROJECT_END
                });
            }
            if cases.family == Some("recurring-checklist")
                && snap
                    .children(i)
                    .iter()
                    .any(|&c| snap.tasks[c].status == Status::Cancelled)
            {
                return blocked(
                    "completing a recurring checklist with cancelled items is not verified yet",
                );
            }
            if cases.family == Some("recurring-checklist")
                && (!captured_schedule(task)
                    || !task
                        .rrule
                        .as_deref()
                        .and_then(|r| parse_rrule(r).ok())
                        .is_some_and(|r| {
                            plain_rule(&r, &[Freq::Monthly])
                                && r.end == End::Never
                                && task
                                    .due
                                    .as_ref()
                                    .is_some_and(|d| d.local_date(snap.tz()).day() <= 28)
                        }))
            {
                return blocked(
                    "only plain monthly, date-only checklist completion is verified yet",
                );
            }
            if task
                .rrule
                .as_deref()
                .and_then(|r| parse_rrule(r).ok())
                .is_some_and(|r| matches!(r.end, End::Until(_)))
                && !captured_until(snap, i)
            {
                return blocked(
                    "until completion is verified only for top-level date-only daily/plain-monthly tasks without starts or alerts",
                );
            }
            schedule_contracts(m, OperationId::Complete, &class)?;
            recurring_complete_shape(task, m, today)?;
        }
    }

    Ok(())
}

/// Structural categories of a task: its own case and, for a member of a family (a checklist, a
/// project, or one of their items), the family's case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cases {
    /// `one-time`, `daily`, `weekly`, `monthly`, `yearly` or `from-completion`.
    pub own: &'static str,
    /// `checklist`, `recurring-checklist`, `project`, `recurring-project` or `project-recurring-child`.
    pub family: Option<&'static str>,
}

/// The matrix cases of the task at `idx`, or why it has none. Only the shapes the Plan 6
/// iPhone sessions captured have cases: one level of children under a typed checklist or
/// project, recurring checklists included; no independently repeating checklist item or
/// repeating child of a repeating project.
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
        TaskKind::Checklist if item && own_repeats => return blocked(ITEM_DATES),
        TaskKind::Checklist if head_repeats => "recurring-checklist",
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

/// Whether `changes` give the task's repeat a new interval: a `RepeatInterval`, or a
/// `Change::Repeat` that keeps the rule but for another interval.
fn changes_interval(snap: &Snapshot, idx: usize, changes: &[Change]) -> bool {
    if changes
        .iter()
        .any(|c| matches!(c, Change::RepeatInterval(_)))
    {
        return true;
    }
    let old = snap.tasks[idx]
        .rrule
        .as_deref()
        .and_then(|r| parse_rrule(r).ok());
    match (new_repeat(changes), old) {
        (Some(Some(new)), Some(old)) => {
            !reshapes_repeat(snap, idx, changes) && new.rule.interval != old.interval
        }
        _ => false,
    }
}

/// The interval of a rule Twydo writes. An Nth-weekday repeat keeps its interval (2Do stores
/// its ordinal where the interval would go), and a written rule must not get an interval its
/// shape cannot be completed with ([`interval_refusal`]: a weekday set repeats every week
/// only), whether by `RepeatInterval`, by `Change::Repeat` or on a create.
fn written_interval(snap: &Snapshot, target: Option<usize>, op: &Op) -> Result<(), Blocked> {
    let rule = match (target, op) {
        (None, Op::Create(n)) => n.repeat.as_ref().map(|r| r.rule.clone()),
        (Some(i), Op::Edit { changes, .. }) => {
            let writes = changes
                .iter()
                .any(|c| matches!(c, Change::Repeat(_) | Change::RepeatInterval(_)));
            if !writes {
                return Ok(());
            }
            let old = snap.tasks[i]
                .rrule
                .as_deref()
                .and_then(|r| parse_rrule(r).ok());
            if old
                .as_ref()
                .is_some_and(|r| r.by_day.iter().any(|(n, _)| n.is_some()))
                && changes_interval(snap, i, changes)
            {
                return blocked(NTH_WEEKDAY_INTERVAL);
            }
            // The rule the changes leave, in order.
            let mut rule = old;
            for c in changes {
                match c {
                    Change::Repeat(r) => rule = r.as_ref().map(|r| r.rule.clone()),
                    Change::RepeatInterval(n) => {
                        if let Some(r) = rule.as_mut() {
                            r.interval = *n;
                        }
                    }
                    _ => {}
                }
            }
            rule
        }
        _ => None,
    };
    match rule.as_ref().and_then(interval_refusal) {
        Some(why) => blocked(why),
        None => Ok(()),
    }
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

/// A written or removed rule needs the operation-local encoding profiles for both
/// source and destination; unchanged schedules do not ask for execution evidence.
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
    for class in old.into_iter().chain(new) {
        schedule_contracts(m, OperationId::SetRepeat, &class)?;
    }

    Ok(())
}

/// A readable JSON `TaskType` must be 0, 1 or 2 and match the declared kind (0 task,
/// 1 checklist, 2 project). Without one there is nothing to check.
fn known_task_type(t: &Task) -> bool {
    if t.warnings
        .iter()
        .any(|w| w.starts_with("unrecognised @tasktype value:"))
    {
        return false;
    }
    let Some(meta) = json_meta(t) else {
        return true;
    };
    if !meta.has("TaskType") {
        return true;
    }
    let Some(n) = meta.get_integer("TaskType") else {
        return false;
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
/// (`create/family-roundtrip/initial-phone-edit/list1`), so the op writes the family.
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

/// A dedicated item-delete verdict authorizes only removal of the selected
/// resource. All family members must have unambiguous identities and form one
/// recognized, same-list, flat family. Their schedules/statuses are held byte for
/// byte by the writer; field-edit and recurrence-execution verdicts do not apply.
fn check_item_delete(
    snap: &Snapshot,
    m: &Matrix,
    mode: WriteMode,
    parent: usize,
    case: &str,
) -> Result<(), Blocked> {
    let head = &snap.tasks[parent];
    if head.parent.is_some() {
        return blocked(NESTED);
    }
    for i in std::iter::once(parent).chain(snap.children(parent).iter().copied()) {
        lookup(snap, &snap.tasks[i].uid)?;
    }
    for i in std::iter::once(parent).chain(snap.children(parent).iter().copied()) {
        let t = &snap.tasks[i];
        if !known_task_type(t) {
            return blocked("this 2Do task type is not verified yet");
        }
        if t.meta.unreadable {
            return blocked("item deletion needs readable task metadata");
        }
        if t.collection != head.collection {
            return blocked("an item must be in the list of its checklist or project");
        }
        if i != parent && (declared_kind(t) != TaskKind::Task || !snap.children(i).is_empty()) {
            return blocked(NESTED);
        }
        if task_rule_class(t, snap)
            .is_some_and(|class| class.split('+').next() == Some("unrecognized"))
        {
            return blocked("item deletion needs recognized repeat rules");
        }
    }
    require(
        m,
        mode,
        if case == "checklist-item" {
            ContractId::DeleteChecklistItem
        } else {
            ContractId::DeleteProjectItem
        },
    )
}

/// Parent writes validate every touched child and its held structural schedule. Individual deletion is scoped to the C29 shape; moving an item or reopening one
/// under a completed/cancelled parent remains unverified, as do independent checklist-item dates.
fn family_op(snap: &Snapshot, op: &Op, idx: usize) -> Result<(), Blocked> {
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
                if snap.tasks[p].kind != TaskKind::Checklist || snap.tasks[p].rrule.is_none() {
                    return blocked(
                        "deleting one item of a project or checklist is not verified yet",
                    );
                }
                if !captured_monthly_checklist(snap, p)
                    || snap.children(p).len() < 2
                    || snap
                        .children(p)
                        .iter()
                        .any(|&c| !captured_checklist_item(snap, c, p))
                {
                    return blocked(
                        "individual deletion is verified only for an open item with open siblings under a plain monthly date-only checklist",
                    );
                }
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
                && !preserved_schedule(&class)
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

fn require(m: &Matrix, mode: WriteMode, id: ContractId) -> Result<(), Blocked> {
    m.require(id, mode)
        .map_err(|error| Blocked(error.to_string()))
}

/// All requested field contracts must pass. Shape interaction guards run on the
/// combined request; successful individual fields never bypass those guards.
fn operation_contracts(
    m: &Matrix,
    mode: WriteMode,
    before: Cases,
    after: Option<Cases>,
    op: &Op,
    reshaped: bool,
) -> Result<(), Blocked> {
    use ContractId as C;
    match op {
        Op::Create(_) => {
            let id = match before.family {
                Some("checklist") => C::CreateChecklist,
                Some("recurring-checklist") => C::CreateRecurringChecklist,
                Some("project") => C::CreateProject,
                Some("recurring-project") => C::CreateRecurringProject,
                Some("project-recurring-child") => C::CreateRecurringProjectItem,
                _ => C::CreateTask,
            };
            require(m, mode, id)
        }
        Op::Complete { .. } => require(
            m,
            mode,
            match before.family {
                Some("recurring-checklist") => C::CompleteRecurringChecklist,
                Some("recurring-project") => C::CompleteRecurringProject,
                Some("checklist") => C::CompleteChecklist,
                Some("project" | "project-recurring-child") => C::CompleteProject,
                _ => C::CompleteTask,
            },
        ),
        Op::Reopen { .. } => require(
            m,
            mode,
            match (before.family, before.own) {
                (Some("recurring-checklist"), _) => C::ReopenRecurringChecklist,
                (_, "monthly" | "yearly") => C::ReopenRepeatingCalendar,
                (_, "daily" | "weekly") => C::ReopenRepeatingOther,
                (_, "from-completion") => C::ReopenFromCompletion,
                _ => C::ReopenResource,
            },
        ),
        Op::Edit { changes, .. } => {
            for change in changes {
                let id = match change {
                    Change::Summary(_) => C::EditTitleResource,
                    Change::Description(_) => C::EditNotesResource,
                    Change::Due(_) => C::SetDueTask,
                    Change::Start(_) => C::SetStartTask,
                    Change::RepeatInterval(_) => C::SetRepeatInterval,
                    Change::Repeat(_) if !reshaped => C::SetRepeatInterval,
                    Change::Repeat(_) => {
                        for shape in std::iter::once(before).chain(after) {
                            require(
                                m,
                                mode,
                                match shape.family {
                                    Some("checklist" | "recurring-checklist") => {
                                        C::SetRepeatChecklist
                                    }
                                    Some("project" | "recurring-project") => C::SetRepeatProject,
                                    Some("project-recurring-child") => C::SetRepeatProjectItem,
                                    _ => C::SetRepeatTask,
                                },
                            )?;
                        }
                        continue;
                    }
                    Change::Kind(_) => continue, // exact conversion contract below
                };
                require(m, mode, id)?;
            }
            Ok(())
        }
        _ => unreachable!("resource, history, ordering and membership contracts dispatch earlier"),
    }
}

/// Recognized held schedules: unchanged fields need structural readability,
/// not permission to execute or rewrite their recurrence.
fn preserved_schedule(class: &str) -> bool {
    let mut parts = class.split('+');
    matches!(
        parts.next(),
        Some(
            "days"
                | "weeks"
                | "weekdays-set"
                | "months-day-1-28"
                | "months-day-29-31"
                | "months-nth-weekday"
                | "years"
                | "years-feb-29"
        )
    ) && parts.all(|p| matches!(p, "from-completion" | "count" | "until"))
}

/// Only writers/executors ask for recurrence evidence. Profiles are scoped to
/// their operation, never a global recurrence permission table.
fn schedule_contracts(m: &Matrix, operation: OperationId, class: &str) -> Result<(), Blocked> {
    for part in class.split('+') {
        let id = ContractId::schedule(operation, part)
            .ok_or_else(|| Blocked(format!("{class} repeats are not verified yet")))?;
        if operation == OperationId::SetRepeat
            && m.get(id)
                .is_some_and(|r| r.iphone_baseline == crate::compat::Verdict::Fail)
        {
            return blocked(format!(
                "{class} repeats are not supported by 2Do on the iPhone"
            ));
        }
        require(m, WriteMode::Verified, id)?;
    }
    Ok(())
}

/// Scheduling context observed in Plan 8 batch 1. Offset evidence from other task/family
/// shapes must not widen the newly enabled checklist/UNTIL operations.
fn captured_schedule(t: &Task) -> bool {
    matches!(t.due, Some(IcalDateTime::Date(_)))
        && t.start.is_none()
        && t.alarms.is_empty()
        && !t.from_completion
        && json_meta(t).is_some_and(|m| {
            m.get_i64("StartDate").is_none()
                && m.get_i64("duetime").is_none()
                && m.get_i64("StartDayDelay").unwrap_or(0) == 0
        })
}

fn plain_rule(r: &RRule, freqs: &[Freq]) -> bool {
    freqs.contains(&r.freq)
        && r.interval == 1
        && r.by_day.is_empty()
        && r.by_month_day.is_empty()
        && r.by_month.is_empty()
        && r.by_set_pos.is_empty()
        && r.unknown.is_empty()
}

fn captured_until(snap: &Snapshot, i: usize) -> bool {
    let t = &snap.tasks[i];
    t.kind == TaskKind::Task
        && snap.parent(i).is_none()
        && snap.children(i).is_empty()
        && captured_schedule(t)
        && matches!(
            task_rule_class(t, snap).as_deref(),
            Some("days+until" | "months-day-1-28+until")
        )
        && t.rrule
            .as_deref()
            .and_then(|r| parse_rrule(r).ok())
            .is_some_and(|r| {
                plain_rule(&r, &[Freq::Daily, Freq::Monthly])
                    && matches!(r.end, End::Until(IcalDateTime::Date(_)))
            })
}

/// Daily/yearly repeat-change evidence composes with the existing plain weekly/monthly
/// writer. It does not establish changing endings, completion anchors or weekday shapes.
fn plain_repeat_transition(t: &Task, changes: &[Change]) -> Result<(), Blocked> {
    let valid = |r: &RRule, from_completion: bool| {
        let mut shape = r.clone();
        shape.interval = 1; // Existing interval encoding is separately verified.
        !from_completion
            && r.end == End::Never
            && plain_rule(
                &shape,
                &[Freq::Daily, Freq::Weekly, Freq::Monthly, Freq::Yearly],
            )
    };
    if t.status != Status::Open
        || !t
            .rrule
            .as_deref()
            .is_none_or(|raw| parse_rrule(raw).is_ok_and(|r| valid(&r, t.from_completion)))
        || !new_repeat(changes)
            .flatten()
            .is_none_or(|r| valid(&r.rule, r.from_completion))
    {
        return blocked(
            "daily/yearly repeat changes are verified only between plain due-anchored rules without an end",
        );
    }
    Ok(())
}

/// Inspect the original component: the parsed alarm model does not distinguish a
/// custom zero-offset alarm from 2Do's automatic due-time alarm.
fn captured_automatic_alarm(snap: &Snapshot, t: &Task) -> bool {
    let Some(cal) = snap
        .source(&t.path)
        .and_then(|b| crate::ical::parse(b).ok())
    else {
        return false;
    };
    let Some(todo) = cal.root.children_named("VTODO").next() else {
        return false;
    };
    if let Some(start) = todo.prop("DTSTART")
        && !todo
            .prop("DUE")
            .is_some_and(|due| start.params == due.params && start.value == due.value)
    {
        return false;
    }
    let alarms: Vec<_> = todo.children_named("VALARM").collect();
    alarms.len() == t.alarms.len()
        && alarms.iter().all(|a| {
            a.children.is_empty()
                && a.props.len() == 6
                && [
                    "TRIGGER",
                    "ACTION",
                    "DESCRIPTION",
                    "X-2DOAPP-SOUND",
                    "X-2DOAPP-ISAUTO",
                    "X-2DOAPP-RELDUETIME",
                ]
                .iter()
                .all(|name| a.props.iter().filter(|p| p.name == *name).count() == 1)
                && a.props.iter().all(|p| match p.name.as_str() {
                    "TRIGGER" => p.value == "PT0S" && p.params.is_empty(),
                    "ACTION" => p.value == "AUDIO" && p.params.is_empty(),
                    "DESCRIPTION" => p.params.is_empty(),
                    "X-2DOAPP-SOUND" | "X-2DOAPP-ISAUTO" | "X-2DOAPP-RELDUETIME" => {
                        let expected = if p.name == "X-2DOAPP-SOUND" {
                            "Basso"
                        } else {
                            "1"
                        };
                        p.value == expected
                            && p.params.len() == 1
                            && p.params[0].name == "SHARE-SCOPE"
                            && p.params[0].values == ["GLOBAL"]
                    }
                    _ => false,
                })
        })
}

/// RF/RA evidence covers ordinary monthly edits and plain monthly/weekly anchor
/// transitions separately. Their shared operation row must not enable other classes,
/// families, endings or combinations that were never captured.
fn captured_from_completion_edit(
    snap: &Snapshot,
    i: usize,
    changes: &[Change],
) -> Result<(), Blocked> {
    const REASON: &str = "from-completion edits are verified only for captured standalone monthly fields, interval 1 to 2, or plain monthly/weekly anchor changes without an end";
    let t = &snap.tasks[i];
    let refuse = || blocked(REASON);
    let old = t
        .rrule
        .as_deref()
        .map(parse_rrule)
        .transpose()
        .map_err(|_| Blocked(REASON.into()))?;
    let plain = |r: &RRule| r.end == End::Never && plain_rule(r, &[Freq::Monthly, Freq::Weekly]);
    let due_ok = |d: &Due| match d {
        Due::Date(d) => d.day() <= 28,
        Due::Time(d) => d.day() <= 28,
    };
    let Some(meta) = json_meta(t) else {
        return refuse();
    };
    // A plain task retaining a foreign series id may be reopened history. Setting
    // recurrence there would create a second live series with the old link. Live
    // imported recurring tasks can legitimately have opaque foreign RUIDs.
    if old.is_none()
        && meta
            .get_str("RUID")
            .is_some_and(|r| !r.is_empty() && r != t.uid)
    {
        return blocked(
            "setting completion recurrence on a plain task linked to another series is not verified yet",
        );
    }
    if t.kind != TaskKind::Task
        || t.status != Status::Open
        || snap.parent(i).is_some()
        || !snap.children(i).is_empty()
        || !match t.due {
            Some(IcalDateTime::Date(d)) => d.day() <= 28,
            Some(IcalDateTime::Floating(d)) => d.day() <= 28,
            _ => false,
        }
        || !captured_automatic_alarm(snap, t)
        || !due_after(snap, i, changes).as_ref().is_some_and(due_ok)
        || meta.get_i64("StartDayDelay").unwrap_or(0) != 0
        || meta
            .get_i64("StartDate")
            .is_some_and(|n| n <= 0 || n % 86_400 != 0)
        || !t
            .start
            .as_ref()
            .is_none_or(|s| matches!(s, IcalDateTime::Date(_)))
        || t.alarms.len() > 1
        || t.alarms
            .iter()
            .any(|a| !matches!(a.trigger, Trigger::Relative { offset, .. } if offset.is_zero()))
    {
        return refuse();
    }
    let date_only = matches!(t.due, Some(IcalDateTime::Date(_)))
        && t.start.is_none()
        && t.alarms.is_empty()
        && meta.get_i64("StartDate").is_none()
        && meta.get_i64("duetime").is_none();
    if changes.iter().all(|c| matches!(c, Change::Repeat(_))) {
        // The edit card represents an interval-only change with a full Repeat spec.
        // The CLI uses RepeatInterval; both write the same captured recurrence fields.
        let interval_two = old
            .as_ref()
            .is_some_and(|r| r.freq == Freq::Monthly && plain(r))
            && t.from_completion
            && new_repeat(changes).flatten().is_some_and(|r| {
                let mut rule = r.rule.clone();
                rule.interval = 1;
                r.from_completion
                    && r.rule.interval == 2
                    && rule.freq == Freq::Monthly
                    && plain(&rule)
            });
        if date_only && interval_two {
            return Ok(());
        }
        if !date_only
            || !old.as_ref().is_none_or(plain)
            || !new_repeat(changes).flatten().is_none_or(|r| plain(&r.rule))
        {
            return refuse();
        }
        // Each change involving completion anchoring needs its own phone twin.
        // The direct task covers setting and removing monthly completion recurrence;
        // the anchor task covers monthly due -> completion -> weekly completion -> due.
        let before = old.as_ref().map(|r| (r.freq, t.from_completion));
        let after = new_repeat(changes)
            .flatten()
            .map(|r| (r.rule.freq, r.from_completion));
        return if before == after
            || matches!(
                (before, after),
                (None, Some((Freq::Monthly, true)))
                    | (Some((Freq::Monthly, false)), Some((Freq::Monthly, true)))
                    | (Some((Freq::Monthly, true)), Some((Freq::Weekly, true)))
                    | (Some((Freq::Weekly, true)), Some((Freq::Weekly, false)))
                    | (Some((Freq::Monthly, true)), None)
            ) {
            Ok(())
        } else {
            refuse()
        };
    }
    let Some(old) = old else {
        return refuse();
    };
    if !t.from_completion || old.freq != Freq::Monthly || !plain(&old) {
        return refuse();
    }
    if changes
        .iter()
        .any(|c| matches!(c, Change::RepeatInterval(_)))
    {
        return if date_only
            && changes
                .iter()
                .all(|c| matches!(c, Change::RepeatInterval(2)))
        {
            Ok(())
        } else {
            refuse()
        };
    }
    if changes
        .iter()
        .any(|c| matches!(c, Change::Repeat(_) | Change::Kind(_)))
    {
        return refuse();
    }
    Ok(())
}

/// C29 creation/text/item operations have their own shape boundary. The existing
/// repeat-only path retains its independently verified monthly/weekly transitions.
fn captured_monthly_schedule(t: &Task) -> bool {
    t.status == Status::Open
        && captured_schedule(t)
        && matches!(t.due, Some(IcalDateTime::Date(d)) if d.day() <= 28)
        && t.rrule.as_deref().is_some_and(|raw| {
            parse_rrule(raw).is_ok_and(|r| plain_rule(&r, &[Freq::Monthly]) && r.end == End::Never)
        })
}

fn captured_monthly_checklist(snap: &Snapshot, i: usize) -> bool {
    snap.tasks[i].kind == TaskKind::Checklist
        && known_task_type(&snap.tasks[i])
        && snap.tasks[i].blocked.is_none()
        && snap.parent(i).is_none()
        && captured_monthly_schedule(&snap.tasks[i])
}

fn captured_checklist_item(snap: &Snapshot, i: usize, parent: usize) -> bool {
    let t = &snap.tasks[i];
    t.kind == TaskKind::Task
        && known_task_type(t)
        && t.blocked.is_none()
        && t.status == Status::Open
        && t.rrule.is_none()
        && !t.from_completion
        && t.start.is_none()
        && t.alarms.is_empty()
        && snap.children(i).is_empty()
        && (t.due.is_none() || t.due == snap.tasks[parent].due)
        && json_meta(t).is_some_and(|m| {
            m.get_i64("StartDate").is_none()
                && m.get_i64("duetime").is_none()
                && m.get_i64("StartDayDelay").unwrap_or(0) == 0
        })
}

fn captured_checklist_write(
    snap: &Snapshot,
    target: Option<usize>,
    op: &Op,
    repeat_only: bool,
) -> Result<(), Blocked> {
    let allowed = match op {
        Op::Create(n) => match n.parent.as_deref() {
            Some(uid) => snap
                .find(uid)
                .is_some_and(|p| captured_monthly_checklist(snap, p)),
            None => {
                n.kind == TaskKind::Checklist
                    && n.start.is_none()
                    && matches!(n.due, Some(Due::Date(d)) if d.day() <= 28)
                    && n.repeat.as_ref().is_some_and(|r| {
                        !r.from_completion
                            && plain_rule(&r.rule, &[Freq::Monthly])
                            && r.rule.end == End::Never
                    })
            }
        },
        Op::Edit { changes, .. } if !repeat_only => {
            let i = target.expect("an edit has a target");
            let t = &snap.tasks[i];
            let shape = if let Some(p) = snap.parent(i) {
                captured_monthly_checklist(snap, p) && captured_checklist_item(snap, i, p)
            } else {
                captured_monthly_schedule(t)
                    && (t.kind == TaskKind::Checklist
                        || (t.kind == TaskKind::Task && snap.children(i).is_empty()))
            };
            shape
                && changes.iter().all(|c| match c {
                    Change::Summary(_) | Change::Description(_) => true,
                    Change::Kind(TaskKind::Checklist) => {
                        t.kind == TaskKind::Task && snap.parent(i).is_none()
                    }
                    _ => false,
                })
        }
        _ => true,
    };
    if allowed {
        Ok(())
    } else {
        blocked(
            "checklist creation and text editing are verified only for open plain monthly date-only checklists; conversion is from a top-level recurring task",
        )
    }
}

fn checklist_repeat_shape(t: &Task, changes: &[Change]) -> Result<(), Blocked> {
    let valid = |r: &RRule, from_completion: bool| {
        !from_completion
            && plain_rule(r, &[Freq::Weekly, Freq::Monthly])
            && r.end == End::Never
            && (r.freq != Freq::Monthly
                || matches!(t.due, Some(IcalDateTime::Date(d)) if d.day() <= 28))
    };
    let old_ok = t
        .rrule
        .as_deref()
        .is_none_or(|raw| parse_rrule(raw).is_ok_and(|r| valid(&r, t.from_completion)));
    let new_ok = new_repeat(changes)
        .flatten()
        .is_none_or(|r| valid(&r.rule, r.from_completion));
    if t.kind != TaskKind::Checklist
        || t.status != Status::Open
        || !captured_schedule(t)
        || !old_ok
        || !new_ok
        || changes
            .iter()
            .any(|c| matches!(c, Change::Kind(k) if *k != TaskKind::Checklist))
    {
        return blocked(
            "checklist repeat changes are verified only for plain date-only monthly/weekly rules without an end",
        );
    }
    Ok(())
}

/// Rule 5: the repeating completions `complete_recurring` was built for: a date-only or
/// floating due, a rule [`next_due`] accepts (every kind Twydo creates; the rule's `execute`
/// verdict, checked before, still decides), a JSON `StartDate` (if any) on a UTC midnight
/// after 1970, no absolute alert and no start-day delay. A due
/// time, start date or relative alerts need the `start-due-alarm-offsets` edge case to pass;
/// a next due on or before `today` (a missed occurrence) needs `monthly-15th-three-months-late`.
fn recurring_complete_shape(t: &Task, m: &Matrix, today: NaiveDate) -> Result<(), Blocked> {
    let passes = |id| m.require(id, WriteMode::Verified).is_ok();
    let offsets = passes(ContractId::CompleteStartOffsets);
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
    let meta = json_meta(t);
    let from = t.from_completion.then_some(today);
    let next = next_due(due, &rule, from).or_else(|e| blocked(e.to_string()))?;
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
    // The last occurrence of an ending series has no next date.
    if next.is_some_and(|n| n <= today) && !passes(ContractId::CompleteLateMonthly) {
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

/// Ordering originates in Twydo: 2Do has no phone reorder control. Its evidence is
/// display of Twydo-written positions and preservation through an unrelated phone edit.
fn check_order(
    snap: &Snapshot,
    m: &Matrix,
    mode: WriteMode,
    op: &Op,
    today: NaiveDate,
) -> Result<(), Blocked> {
    let (anchor, name) = match op {
        Op::Reorder { anchor, .. } => (anchor, "reorder"),
        Op::CreateAt { anchor, .. } => (anchor, "insert"),
        _ => unreachable!(),
    };
    let a = lookup(snap, anchor)?;
    let p = snap.parent(a).ok_or_else(|| {
        Blocked("ordering is available only for items within a checklist or project".into())
    })?;
    let head = &snap.tasks[p];
    if head.parent.is_some() {
        return blocked(NESTED);
    }
    if declared_kind(head) == TaskKind::Task {
        return blocked(PLAIN_PARENT);
    }
    match op {
        Op::Reorder { uid, .. } => {
            let i = lookup(snap, uid)?;
            if i == a || snap.parent(i) != Some(p) {
                return blocked("ordering needs two different items in the same parent");
            }
        }
        Op::CreateAt { task, .. } => {
            if task.parent.as_deref() != Some(snap.tasks[p].uid.as_str()) {
                return blocked("insertion anchor must belong to the destination parent");
            }
            check(snap, m, mode, &Op::Create(task.clone()), today)?;
        }
        _ => unreachable!(),
    }
    for i in std::iter::once(p).chain(snap.children(p).iter().copied()) {
        let t = &snap.tasks[lookup(snap, &snap.tasks[i].uid)?];
        if !known_task_type(t) {
            return blocked("this 2Do task type is not verified yet");
        }
        if t.collection != head.collection || json_meta(t).is_none() {
            return blocked("item ordering needs same-list editable JSON metadata");
        }
        if i != p && (declared_kind(t) != TaskKind::Task || !snap.children(i).is_empty()) {
            return blocked(NESTED);
        }
        // Only DisplayOrder and modification stamps change. Recurrence/date/status
        // classifications belong to operations that edit or execute those fields,
        // not to this metadata-only write. CreateAt already passed the Create gate.
    }
    require(
        m,
        mode,
        if name == "reorder" {
            ContractId::ReorderItems
        } else {
            ContractId::InsertItems
        },
    )
}
