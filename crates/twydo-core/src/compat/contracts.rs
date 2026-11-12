//! Operation-scoped compatibility declarations. Rust owns the finite contract vocabulary;
//! TOML supplies observations, never predicates or an alternate gate implementation.
use crate::{
    compat::{MatrixError, Verdict},
    config::{ConfigError, WriteMode},
};
use serde::Deserialize;
use std::{collections::BTreeSet, path::Path, sync::OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OperationId {
    Create,
    EditTitle,
    EditNotes,
    SetDue,
    SetStart,
    SetRepeat,
    Convert,
    Complete,
    Reopen,
    Move,
    Delete,
    Reorder,
    Insert,
    Reparent,
    ListCreate,
    ListMetadata,
    ListDelete,
}
impl OperationId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::EditTitle => "edit-title",
            Self::EditNotes => "edit-notes",
            Self::SetDue => "set-due",
            Self::SetStart => "set-start",
            Self::SetRepeat => "set-repeat",
            Self::Convert => "convert",
            Self::Complete => "complete",
            Self::Reopen => "reopen",
            Self::Move => "move",
            Self::Delete => "delete",
            Self::Reorder => "reorder",
            Self::Insert => "insert",
            Self::Reparent => "reparent",
            Self::ListCreate => "list-create",
            Self::ListMetadata => "list-metadata",
            Self::ListDelete => "list-delete",
        }
    }
}

macro_rules! contracts {
    ($( $name:ident => $operation:ident, $id:literal, $label:literal; )*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
        pub enum ContractId { $( #[serde(rename = $id)] $name, )* }
        impl ContractId {
            pub const ALL: &'static [Self] = &[$(Self::$name,)*];
            pub fn operation(self) -> OperationId { match self { $(Self::$name => OperationId::$operation,)* } }
            pub fn as_str(self) -> &'static str { match self { $(Self::$name => $id,)* } }
            /// How the contract's write reads in a refusal.
            pub fn label(self) -> &'static str { match self { $(Self::$name => $label,)* } }
        }
    }
}
contracts! {
    EditTitleResource => EditTitle, "edit-title.resource", "editing a title";
    EditNotesResource => EditNotes, "edit-notes.resource", "editing notes";
    SetDueTask => SetDue, "set-due.task", "changing a due date";
    SetDueChecklist => SetDue, "set-due.recurring-checklist", "changing a recurring checklist's due date";
    SetDueItemCleanup => SetDue, "set-due.checklist-item-cleanup", "changing a checklist's date while its items have dates of their own";
    SetStartTask => SetStart, "set-start.task", "changing a start date";
    SetRepeatInterval => SetRepeat, "set-repeat.interval", "changing a repeat interval";
    SetRepeatTask => SetRepeat, "set-repeat.task", "changing a task's repeat rule";
    SetRepeatChecklist => SetRepeat, "set-repeat.checklist", "changing a checklist's repeat rule";
    SetRepeatProject => SetRepeat, "set-repeat.project", "changing a project's repeat rule";
    SetRepeatProjectItem => SetRepeat, "set-repeat.recurring-project-item", "changing the repeat rule of an item in a recurring project";
    SetRepeatChecklistInterval => SetRepeat, "set-repeat.checklist-interval", "changing a recurring checklist's repeat interval";
    CreateTask => Create, "create.task", "creating a task";
    CreateChecklist => Create, "create.checklist", "creating a checklist";
    CreateProject => Create, "create.project", "creating a project";
    CreateRecurringChecklist => Create, "create.recurring-checklist", "creating a recurring checklist";
    CreateRecurringProject => Create, "create.recurring-project", "creating a recurring project";
    CreateRecurringProjectItem => Create, "create.recurring-project-item", "adding an item to a recurring project";
    CompleteTask => Complete, "complete.task", "completing a task";
    CompleteChecklist => Complete, "complete.checklist", "completing a checklist";
    CompleteProject => Complete, "complete.project", "completing a project";
    CompleteRecurringChecklist => Complete, "complete.recurring-checklist", "completing a recurring checklist";
    CompleteRecurringProject => Complete, "complete.recurring-project", "completing a recurring project";
    ReopenResource => Reopen, "reopen.resource", "reopening a task";
    ReopenRecurringChecklist => Reopen, "reopen.recurring-checklist", "reopening a recurring checklist";
    ReopenRepeatingCalendar => Reopen, "reopen.repeating-calendar", "reopening a monthly or yearly repeating task";
    ReopenRepeatingOther => Reopen, "reopen.repeating-other", "reopening a daily or weekly repeating task";
    ReopenFromCompletion => Reopen, "reopen.from-completion", "reopening a task that repeats from completion";
    ReopenHistoryTask => Reopen, "reopen.history-task", "reopening a completed occurrence";
    ReopenHistoryFamily => Reopen, "reopen.history-family", "reopening a completed occurrence of a project or checklist";
    ReopenEndedDaily => Reopen, "reopen.ended-daily", "reopening the last occurrence of an ended daily series";
    ReopenEndedMonthly => Reopen, "reopen.ended-monthly", "reopening the last occurrence of an ended monthly series";
    MoveResource => Move, "move.resource", "moving a task to another list";
    MoveFamily => Move, "move.family", "moving a project or checklist to another list";
    DeleteResource => Delete, "delete.resource", "deleting a task";
    DeleteFamily => Delete, "delete.family", "deleting a project or checklist";
    DeleteChecklistItem => Delete, "delete.checklist-item", "deleting a checklist item";
    DeleteProjectItem => Delete, "delete.project-item", "deleting a project item";
    ConvertSameKind => Convert, "convert.same-kind", "converting a task to the type it already has";
    ConvertTaskChecklist => Convert, "convert.task-checklist", "converting a task to a checklist";
    ConvertTaskProject => Convert, "convert.task-project", "converting a task to a project";
    ConvertChecklistProject => Convert, "convert.checklist-project", "converting a checklist to a project";
    ConvertProjectChecklist => Convert, "convert.project-checklist", "converting a project with items to a checklist";
    ConvertEmptyProjectChecklist => Convert, "convert.empty-project-checklist", "converting an empty project to a checklist";
    ConvertParentTask => Convert, "convert.parent-task", "converting a checklist or project with items to a task";
    ConvertEmptyParentTask => Convert, "convert.empty-parent-task", "converting an empty checklist or project to a task";
    ReorderItems => Reorder, "reorder.items", "reordering items";
    InsertItems => Insert, "insert.items", "inserting an item at a position";
    ReparentProject => Reparent, "reparent.project", "moving a task into a project";
    ReparentChecklist => Reparent, "reparent.checklist", "moving a task into a checklist";
    ReparentOutdent => Reparent, "reparent.outdent", "moving an item out of its parent";
    ListCreate => ListCreate, "list-create.collection", "creating a list";
    ListMetadata => ListMetadata, "list-metadata.collection", "renaming or recoloring a list";
    ListDelete => ListDelete, "list-delete.collection", "deleting a list";
    CompleteDays => Complete, "complete.days", "completing a task that repeats every few days";
    CompleteWeeks => Complete, "complete.weeks", "completing a weekly repeating task";
    CompleteWeekdays => Complete, "complete.weekdays", "completing a task that repeats on set weekdays";
    CompleteMonthly => Complete, "complete.monthly", "completing a monthly repeating task";
    CompleteMonthOverflow => Complete, "complete.month-overflow", "completing a monthly task due on the 29th to 31st";
    CompleteNthWeekday => Complete, "complete.nth-weekday", "completing a task that repeats on the nth weekday of the month";
    CompleteYearly => Complete, "complete.yearly", "completing a yearly repeating task";
    CompleteLeapYear => Complete, "complete.leap-year", "completing a yearly task due on February 29";
    CompleteFromCompletion => Complete, "complete.from-completion", "completing a task that repeats from completion";
    CompleteCount => Complete, "complete.count", "completing a repeating task with a fixed number of occurrences";
    CompleteUntil => Complete, "complete.until", "completing a repeating task with an end date";
    CompleteLateMonthly => Complete, "complete.late-monthly", "completing a monthly task after a missed occurrence";
    CompleteStartOffsets => Complete, "complete.start-offsets", "completing a repeating task with a start date or alerts";
    CompleteUndo => Complete, "complete.undo", "undoing a completion";
    SetRepeatDays => SetRepeat, "set-repeat.days", "setting a repeat every few days";
    SetRepeatWeeks => SetRepeat, "set-repeat.weeks", "setting a weekly repeat";
    SetRepeatWeekdays => SetRepeat, "set-repeat.weekdays", "setting a repeat on set weekdays";
    SetRepeatMonthly => SetRepeat, "set-repeat.monthly", "setting a monthly repeat";
    SetRepeatMonthOverflow => SetRepeat, "set-repeat.month-overflow", "setting a monthly repeat on the 29th to 31st";
    SetRepeatNthWeekday => SetRepeat, "set-repeat.nth-weekday", "setting a repeat on the nth weekday of the month";
    SetRepeatYearly => SetRepeat, "set-repeat.yearly", "setting a yearly repeat";
    SetRepeatLeapYear => SetRepeat, "set-repeat.leap-year", "setting a yearly repeat on February 29";
    SetRepeatFromCompletion => SetRepeat, "set-repeat.from-completion", "setting a repeat from completion";
    SetRepeatCount => SetRepeat, "set-repeat.count", "setting a repeat with a fixed number of occurrences";
    SetRepeatUntil => SetRepeat, "set-repeat.until", "setting a repeat with an end date";
    SetRepeatLastDay => SetRepeat, "set-repeat.last-day", "setting a repeat on the last day of the month";
    SetRepeatNthWorkday => SetRepeat, "set-repeat.nth-workday", "setting a repeat on the nth workday of the month";
    SetRepeatNthWeekend => SetRepeat, "set-repeat.nth-weekend", "setting a repeat on the nth weekend day of the month";
}

/// A risk the user knowingly accepted for a contract that cannot be proven safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AcceptedRisk {
    /// The server offers no protection against concurrent remote edits; the write is
    /// attempted anyway. Only list metadata and list deletion may carry it.
    BestEffort,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contract {
    pub id: ContractId,
    #[serde(default = "unverified")]
    pub iphone_baseline: Verdict,
    #[serde(default = "unverified")]
    pub twydo_to_iphone: Verdict,
    #[serde(default = "unverified")]
    pub iphone_to_twydo: Verdict,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub notes: String,
    /// Collection atomicity is independent of successful client round trips.
    #[serde(default)]
    pub atomicity: Option<Verdict>,
    /// Explicit user acceptance that admits a list write despite failed atomicity.
    #[serde(default)]
    pub accepted_risk: Option<AcceptedRisk>,
}
fn unverified() -> Verdict {
    Verdict::Unverified
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Matrix {
    pub matrix_version: u32,
    #[serde(default)]
    pub contract: Vec<Contract>,
}

/// Why a contract does not authorize a write. A missing contract has no direction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementFailure {
    pub contract: ContractId,
    pub direction: Option<&'static str>,
    pub verdict: Verdict,
}

impl std::error::Error for RequirementFailure {}

/// Says what the write is, then which matrix entry to look at:
/// "converting a project with items to a checklist is not supported by 2Do
/// (convert.project-checklist: iphone_baseline)".
impl std::fmt::Display for RequirementFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let outcome = match self.verdict {
            Verdict::Unsupported => "is not supported by 2Do",
            Verdict::Fail => "fails with 2Do",
            Verdict::Unverified | Verdict::Pass => "is not verified yet",
        };
        let label = self.contract.label();
        let id = self.contract.as_str();
        match self.direction {
            Some(direction) => write!(f, "{label} {outcome} ({id}: {direction})"),
            None => write!(f, "{label} {outcome} ({id}: no matrix entry)"),
        }
    }
}
impl Matrix {
    pub fn from_toml_str(text: &str) -> Result<Self, MatrixError> {
        let value: toml::Value = toml::from_str(text).map_err(MatrixError::Parse)?;
        if value
            .get("matrix_version")
            .and_then(toml::Value::as_integer)
            != Some(2)
        {
            return Err(MatrixError::Invalid(format!(
                "compatibility matrix version {} is not supported; migrate to operation contracts (matrix_version = 2)",
                value
                    .get("matrix_version")
                    .map_or("missing".into(), ToString::to_string)
            )));
        }
        let matrix: Self = value.try_into().map_err(MatrixError::Parse)?;
        let mut seen = BTreeSet::new();
        for contract in &matrix.contract {
            if contract.atomicity.is_some()
                && !matches!(
                    contract.id,
                    ContractId::ListMetadata | ContractId::ListDelete
                )
            {
                return Err(MatrixError::Invalid(format!(
                    "atomicity is not applicable to {}",
                    contract.id.as_str()
                )));
            }
            if contract.accepted_risk.is_some()
                && !matches!(
                    contract.id,
                    ContractId::ListMetadata | ContractId::ListDelete
                )
            {
                return Err(MatrixError::Invalid(format!(
                    "accepted_risk is not applicable to {}",
                    contract.id.as_str()
                )));
            }
            if !seen.insert(contract.id) {
                return Err(MatrixError::Invalid(format!(
                    "duplicate contract {}",
                    contract.id.as_str()
                )));
            }
        }
        Ok(matrix)
    }
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_owned(),
            source,
        })?;
        Self::from_toml_str(&text).map_err(|e| match e {
            MatrixError::Parse(source) => ConfigError::Parse {
                path: path.to_owned(),
                source,
            },
            MatrixError::Invalid(message) => ConfigError::Invalid {
                path: path.to_owned(),
                message,
            },
        })
    }
    pub fn embedded() -> &'static Self {
        static MATRIX: OnceLock<Matrix> = OnceLock::new();
        MATRIX.get_or_init(|| {
            Self::from_toml_str(include_str!("../../../../compat/matrix.toml"))
                .expect("embedded operation contracts must validate")
        })
    }
    pub fn get(&self, id: ContractId) -> Option<&Contract> {
        self.contract.iter().find(|c| c.id == id)
    }
    pub fn require(&self, id: ContractId, mode: WriteMode) -> Result<(), RequirementFailure> {
        let record = self.get(id);
        let fail = |direction, verdict| RequirementFailure {
            contract: id,
            direction: Some(direction),
            verdict,
        };
        let Some(record) = record else {
            return Err(RequirementFailure {
                contract: id,
                direction: None,
                verdict: Verdict::Unverified,
            });
        };
        if id == ContractId::ListCreate {
            for (direction, verdict) in [
                ("twydo_to_iphone", record.twydo_to_iphone),
                ("iphone_to_twydo", record.iphone_to_twydo),
            ] {
                if matches!(verdict, Verdict::Fail | Verdict::Unsupported) {
                    return Err(fail(direction, verdict));
                }
            }
        }
        let list_write = matches!(id, ContractId::ListMetadata | ContractId::ListDelete);
        if list_write && record.accepted_risk == Some(AcceptedRisk::BestEffort) {
            // The user accepted the missing concurrency protection, so atomicity may
            // fail and unverified directions are admitted; a recorded failure or
            // unsupported verdict anywhere still refuses.
            for (direction, verdict) in [
                ("atomicity", record.atomicity.unwrap_or(Verdict::Unverified)),
                ("iphone_baseline", record.iphone_baseline),
                ("twydo_to_iphone", record.twydo_to_iphone),
                ("iphone_to_twydo", record.iphone_to_twydo),
            ] {
                let refused = match direction {
                    "atomicity" => verdict == Verdict::Unsupported,
                    _ => matches!(verdict, Verdict::Fail | Verdict::Unsupported),
                };
                if refused {
                    return Err(fail(direction, verdict));
                }
            }
            return Ok(());
        }
        if list_write {
            let protection = record.atomicity.unwrap_or(Verdict::Unverified);
            if protection != Verdict::Pass {
                return Err(fail("atomicity", protection));
            }
        }
        let ordering = matches!(id, ContractId::ReorderItems | ContractId::InsertItems);
        if record.iphone_baseline != Verdict::Pass
            && !(ordering && record.iphone_baseline == Verdict::Unsupported)
        {
            return Err(fail("iphone_baseline", record.iphone_baseline));
        }
        // These observations authorize only the exact narrowly captured scheduling
        // or atomicity contract, even in test mode.
        let strict = matches!(
            id,
            ContractId::SetDueChecklist
                | ContractId::SetRepeatChecklistInterval
                | ContractId::ListMetadata
                | ContractId::ListDelete
        );
        if mode == WriteMode::Verified || strict {
            for (direction, verdict) in [
                ("twydo_to_iphone", record.twydo_to_iphone),
                ("iphone_to_twydo", record.iphone_to_twydo),
            ] {
                if verdict != Verdict::Pass {
                    return Err(fail(direction, verdict));
                }
            }
        }
        Ok(())
    }
}

impl ContractId {
    pub fn schedule(operation: OperationId, part: &str) -> Option<Self> {
        match (operation, part) {
            (OperationId::Complete, "days") => Some(Self::CompleteDays),
            (OperationId::Complete, "weeks") => Some(Self::CompleteWeeks),
            (OperationId::Complete, "weekdays-set") => Some(Self::CompleteWeekdays),
            (OperationId::Complete, "months-day-1-28") => Some(Self::CompleteMonthly),
            (OperationId::Complete, "months-day-29-31") => Some(Self::CompleteMonthOverflow),
            (OperationId::Complete, "months-nth-weekday") => Some(Self::CompleteNthWeekday),
            (OperationId::Complete, "years") => Some(Self::CompleteYearly),
            (OperationId::Complete, "years-feb-29") => Some(Self::CompleteLeapYear),
            (OperationId::Complete, "from-completion") => Some(Self::CompleteFromCompletion),
            (OperationId::Complete, "count") => Some(Self::CompleteCount),
            (OperationId::Complete, "until") => Some(Self::CompleteUntil),
            (OperationId::SetRepeat, "days") => Some(Self::SetRepeatDays),
            (OperationId::SetRepeat, "weeks") => Some(Self::SetRepeatWeeks),
            (OperationId::SetRepeat, "weekdays-set") => Some(Self::SetRepeatWeekdays),
            (OperationId::SetRepeat, "months-day-1-28") => Some(Self::SetRepeatMonthly),
            (OperationId::SetRepeat, "months-day-29-31") => Some(Self::SetRepeatMonthOverflow),
            (OperationId::SetRepeat, "months-nth-weekday") => Some(Self::SetRepeatNthWeekday),
            (OperationId::SetRepeat, "years") => Some(Self::SetRepeatYearly),
            (OperationId::SetRepeat, "years-feb-29") => Some(Self::SetRepeatLeapYear),
            (OperationId::SetRepeat, "from-completion") => Some(Self::SetRepeatFromCompletion),
            (OperationId::SetRepeat, "count") => Some(Self::SetRepeatCount),
            (OperationId::SetRepeat, "until") => Some(Self::SetRepeatUntil),
            (OperationId::SetRepeat, "months-last-day") => Some(Self::SetRepeatLastDay),
            (OperationId::SetRepeat, "months-nth-workday") => Some(Self::SetRepeatNthWorkday),
            (OperationId::SetRepeat, "months-nth-weekend-day") => Some(Self::SetRepeatNthWeekend),
            _ => None,
        }
    }
}

impl Matrix {
    /// Projection of the same profiles used by write admission, not a separate table.
    pub fn schedule_verdict(&self, operation: OperationId, class: &str) -> Verdict {
        class
            .split('+')
            .map(|part| match ContractId::schedule(operation, part) {
                Some(id) => self
                    .require(id, WriteMode::Verified)
                    .map_or_else(|e| e.verdict, |()| Verdict::Pass),
                None => Verdict::Unverified,
            })
            .fold(Verdict::Pass, Verdict::worst)
    }
}
