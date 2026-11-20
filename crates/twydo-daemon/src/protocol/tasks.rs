//! `tasks.get`: one task worded for display, with its children, for a thin client's task
//! screen. Wording is shared with the TUI details pane (`twydo_core::details`).

use chrono::{DateTime, FixedOffset, NaiveDate};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
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
