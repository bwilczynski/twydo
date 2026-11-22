//! `views.get`: a finished view for a thin client (a phone, a display), so the client needs
//! no view logic of its own. Daemon time and the configured timezone decide every date.

use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use twydo_core::model::TaskKind;
use twydo_core::views::DateGroup;

/// The views a thin client can ask for. `List` names one list in `ViewsGetParams::list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = super::open_enum)]
pub enum ViewName {
    Inbox,
    Today,
    All,
    Scheduled,
    Done,
    #[serde(rename = "next_3_days")]
    Next3Days,
    List,
    /// A value this client does not know: a newer daemon sent it.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ViewsGetParams {
    pub view: ViewName,
    /// The list's collection id; required for `list`, refused for every other view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<String>,
}

/// `views.list`: the sidebar's entries, in display order, for a thin client's view picker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ViewsListResult {
    pub epoch: String,
    pub revision: u64,
    /// The daemon's clock, in the configured timezone.
    pub now: DateTime<FixedOffset>,
    pub today: NaiveDate,
    pub entries: Vec<ViewsEntry>,
}

/// A heading, or a view to open with `views.get`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ViewsEntry {
    Heading { heading: String },
    View(ViewsListItem),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ViewsListItem {
    /// The params that open this view through `views.get`.
    pub view: ViewsGetParams,
    pub name: String,
    /// `#rrggbb`; a list's colour, none for the built-in views.
    pub color: Option<String>,
    /// The view's open tasks, as the sidebar counts them.
    pub open_count: usize,
    /// The open tasks due today or overdue.
    pub due_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ViewsGetResult {
    pub epoch: String,
    pub revision: u64,
    /// The daemon's clock, in the configured timezone.
    pub now: DateTime<FixedOffset>,
    pub today: NaiveDate,
    /// `Today`, or the Inbox list's display name.
    pub title: String,
    /// The view's open tasks, as the sidebar counts them.
    pub open_count: usize,
    /// Where a task captured in this view goes; none when there is no list at all.
    pub capture: Option<CaptureTarget>,
    pub sections: Vec<ViewSection>,
    /// What sync needs from a person; absent when sync is not configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<SyncSummary>,
    /// Why the notifier cannot deliver reminders; absent while it can.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reminders: Option<RemindersSummary>,
}

/// The notifier's failure, for a phone to show as a warning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RemindersSummary {
    pub error: String,
}

/// A short, phone-sized account of sync, built from the same status the TUI status line reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SyncSummary {
    /// Something needs a person: a failing sync, a refused emptying, conflicts, sync off.
    pub attention: bool,
    pub headline: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub last_success: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CaptureTarget {
    pub collection: String,
    pub name: String,
    pub due: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ViewSection {
    pub group: SectionKey,
    /// The day of a `completed` section, which tells one such section from another.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<NaiveDate>,
    /// The header text; none for an ungrouped view.
    pub label: Option<String>,
    pub rows: Vec<ViewRow>,
}

/// One date group. `CompletedOn(_)` is `Completed`; its date is the section's `date`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = super::open_enum)]
pub enum SectionKey {
    Overdue,
    InProgress,
    LaterToday,
    Today,
    Tomorrow,
    DayAfterTomorrow,
    Next7Days,
    Next2Weeks,
    Next30Days,
    Future,
    Done,
    NoDueDate,
    Scheduled,
    Completed,
    Ungrouped,
    /// A value this client does not know: a newer daemon sent it.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

impl From<DateGroup> for SectionKey {
    fn from(g: DateGroup) -> SectionKey {
        match g {
            DateGroup::Overdue => SectionKey::Overdue,
            DateGroup::InProgress => SectionKey::InProgress,
            DateGroup::LaterToday => SectionKey::LaterToday,
            DateGroup::Today => SectionKey::Today,
            DateGroup::Tomorrow => SectionKey::Tomorrow,
            DateGroup::DayAfterTomorrow => SectionKey::DayAfterTomorrow,
            DateGroup::Next7Days => SectionKey::Next7Days,
            DateGroup::Next2Weeks => SectionKey::Next2Weeks,
            DateGroup::Next30Days => SectionKey::Next30Days,
            DateGroup::Future => SectionKey::Future,
            DateGroup::Done => SectionKey::Done,
            DateGroup::NoDueDate => SectionKey::NoDueDate,
            DateGroup::Scheduled => SectionKey::Scheduled,
            DateGroup::CompletedOn(_) => SectionKey::Completed,
            DateGroup::Ungrouped => SectionKey::Ungrouped,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RowParent {
    pub uid: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RowList {
    pub id: String,
    pub name: String,
    /// `#rrggbb`.
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ViewRow {
    pub uid: String,
    pub title: String,
    pub depth: u8,
    pub parent: Option<RowParent>,
    /// The due value relative to daemon time (`Yesterday`, `Today 17:00`).
    pub due: Option<String>,
    /// The start value, only while it is after today.
    pub start: Option<String>,
    pub overdue: bool,
    pub due_today: bool,
    /// The task is completed or cancelled: shown done, not tappable on a phone.
    pub completed: bool,
    pub note: Option<String>,
    pub kind: TaskKind,
    pub repeats: bool,
    pub list: RowList,
    /// A queued edit is laid over this row.
    pub pending: bool,
    /// The task has an unresolved sync conflict.
    pub conflict: bool,
}
