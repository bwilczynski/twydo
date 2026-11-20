//! `tasks.get`: one task worded for display, with its children, for a thin client's task
//! screen. Wording is shared with the TUI details pane (`twydo_core::details`).

use chrono::{DateTime, FixedOffset, NaiveDate};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use twydo_core::edit::Op;
use twydo_core::form::{Field, FormBasis, FormValues};
use twydo_core::model::TaskKind;

use super::views::RowList;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TasksGetParams {
    pub uid: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TasksGetResult {
    pub epoch: String,
    pub revision: u64,
    /// The daemon's clock, in the configured timezone.
    pub now: DateTime<FixedOffset>,
    pub today: NaiveDate,
    pub task: TaskDetail,
    /// Every child: open ones in sibling order, then completed ones, newest first.
    pub children: Vec<ChildRow>,
    /// The edit form's starting state.
    pub form: FormState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FieldPreview {
    pub field: Field,
    pub text: String,
}

/// An edit form's state, as the TUI card would show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FormState {
    /// The fields, in display order.
    pub fields: Vec<Field>,
    pub values: FormValues,
    pub previews: Vec<FieldPreview>,
    /// False when the task has alerts Twydo cannot write: Alert is read-only.
    pub alert_editable: bool,
    /// What the values cannot show about where the form started; send it back as `opened`.
    pub basis: FormBasis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskDetail {
    pub uid: String,
    pub title: String,
    /// The shown kind: a plain task with items is a project.
    pub kind: TaskKind,
    pub list: RowList,
    /// The due value relative to daemon time (`Yesterday`, `Today 17:00`).
    pub due: Option<String>,
    pub overdue: bool,
    pub due_today: bool,
    /// The start value, shown whenever it is set.
    pub start: Option<String>,
    /// The repeat rule in words.
    pub repeat: Option<String>,
    /// One line per alert, in order.
    pub alerts: Vec<String>,
    /// Problems found when the task was loaded.
    pub warnings: Vec<String>,
    /// Why edits are off for this task, if they are.
    pub blocked: Option<String>,
    /// The task has an unresolved sync conflict.
    pub conflict: bool,
    pub note: Option<String>,
    /// The task is completed or cancelled.
    pub completed: bool,
    /// A queued edit is laid over this task.
    pub pending: bool,
    pub parent: Option<TaskParent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskParent {
    pub uid: String,
    pub title: String,
    pub kind: TaskKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChildRow {
    pub uid: String,
    pub title: String,
    pub kind: TaskKind,
    pub due: Option<String>,
    pub overdue: bool,
    pub due_today: bool,
    pub completed: bool,
    pub pending: bool,
    pub has_note: bool,
    /// How many children this child has.
    pub child_count: u32,
}

/// Where a new task goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NewTaskTarget {
    pub collection: String,
    /// UID of the checklist or project the task becomes an item of.
    pub parent: Option<String>,
    /// Only today starts the form with a Due; any other date is ignored.
    pub due: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TasksFormParams {
    /// The task being edited; exactly one of `uid` and `new`.
    pub uid: Option<String>,
    pub new: Option<NewTaskTarget>,
    /// The values when the form was opened (or last rebased).
    pub initial: Option<FormValues>,
    /// The form's `basis` when it was opened (or last rebased), unchanged.
    pub opened: Option<FormBasis>,
    /// The typed values; none asks for the starting values.
    pub values: Option<FormValues>,
}

/// What a typed form carried over a newer saved version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RebasedForm {
    /// The new starting values to send as `initial` from now on.
    pub initial: FormValues,
    /// The new basis to send as `opened` from now on.
    pub basis: FormBasis,
    /// Typed fields the saved version changed as well: saving replaces its value.
    pub overwritten: Vec<Field>,
    /// Typed fields the saved version's form cannot take: their typing is lost.
    pub dropped: Vec<Field>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TasksFormResult {
    /// The basis to submit `op` with.
    pub epoch: String,
    pub revision: u64,
    /// The form these values make: on `changed_since`, the rebased one.
    pub form: FormState,
    /// The write, when every field reads and something would change.
    pub op: Option<Op>,
    /// Why `op` is none (`no changes`, `title must not be empty`, ...).
    pub error: Option<String>,
    /// The saved task (or a new item's parent) differs from the `initial` and `opened` the
    /// client sent.
    pub changed_since: bool,
    pub rebased: Option<RebasedForm>,
}
