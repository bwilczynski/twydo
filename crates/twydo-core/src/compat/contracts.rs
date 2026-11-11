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
    ($( $name:ident => $operation:ident, $id:literal; )*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
        pub enum ContractId { $( #[serde(rename = $id)] $name, )* }
        impl ContractId {
            pub const ALL: &'static [Self] = &[$(Self::$name,)*];
            pub fn operation(self) -> OperationId { match self { $(Self::$name => OperationId::$operation,)* } }
            pub fn as_str(self) -> &'static str { match self { $(Self::$name => $id,)* } }
        }
    }
}
contracts! {
    EditTitleResource => EditTitle, "edit-title.resource";
    EditNotesResource => EditNotes, "edit-notes.resource";
    SetDueTask => SetDue, "set-due.task";
    SetDueChecklist => SetDue, "set-due.recurring-checklist";
    SetDueItemCleanup => SetDue, "set-due.checklist-item-cleanup";
    SetStartTask => SetStart, "set-start.task";
    SetRepeatInterval => SetRepeat, "set-repeat.interval";
    SetRepeatTask => SetRepeat, "set-repeat.task";
    SetRepeatChecklist => SetRepeat, "set-repeat.checklist";
    SetRepeatProject => SetRepeat, "set-repeat.project";
    SetRepeatProjectItem => SetRepeat, "set-repeat.recurring-project-item";
    SetRepeatChecklistInterval => SetRepeat, "set-repeat.checklist-interval";
    CreateTask => Create, "create.task";
    CreateChecklist => Create, "create.checklist";
    CreateProject => Create, "create.project";
    CreateRecurringChecklist => Create, "create.recurring-checklist";
    CreateRecurringProject => Create, "create.recurring-project";
    CreateRecurringProjectItem => Create, "create.recurring-project-item";
    CompleteTask => Complete, "complete.task";
    CompleteChecklist => Complete, "complete.checklist";
    CompleteProject => Complete, "complete.project";
    CompleteRecurringChecklist => Complete, "complete.recurring-checklist";
    CompleteRecurringProject => Complete, "complete.recurring-project";
    ReopenResource => Reopen, "reopen.resource";
    ReopenHistoryTask => Reopen, "reopen.history-task";
    ReopenHistoryFamily => Reopen, "reopen.history-family";
    ReopenEndedDaily => Reopen, "reopen.ended-daily";
    ReopenEndedMonthly => Reopen, "reopen.ended-monthly";
    MoveResource => Move, "move.resource";
    MoveFamily => Move, "move.family";
    DeleteResource => Delete, "delete.resource";
    DeleteFamily => Delete, "delete.family";
    DeleteChecklistItem => Delete, "delete.checklist-item";
    DeleteProjectItem => Delete, "delete.project-item";
    ConvertTaskChecklist => Convert, "convert.task-checklist";
    ConvertTaskProject => Convert, "convert.task-project";
    ConvertChecklistProject => Convert, "convert.checklist-project";
    ConvertProjectChecklist => Convert, "convert.project-checklist";
    ConvertEmptyProjectChecklist => Convert, "convert.empty-project-checklist";
    ConvertParentTask => Convert, "convert.parent-task";
    ConvertEmptyParentTask => Convert, "convert.empty-parent-task";
    ReorderItems => Reorder, "reorder.items";
    InsertItems => Insert, "insert.items";
    ReparentProject => Reparent, "reparent.project";
    ReparentChecklist => Reparent, "reparent.checklist";
    ReparentOutdent => Reparent, "reparent.outdent";
    ListCreate => ListCreate, "list-create.collection";
    ListMetadata => ListMetadata, "list-metadata.collection";
    ListDelete => ListDelete, "list-delete.collection";
    CompleteDays => Complete, "complete.days";
    CompleteWeeks => Complete, "complete.weeks";
    CompleteWeekdays => Complete, "complete.weekdays";
    CompleteMonthly => Complete, "complete.monthly";
    CompleteMonthOverflow => Complete, "complete.month-overflow";
    CompleteNthWeekday => Complete, "complete.nth-weekday";
    CompleteYearly => Complete, "complete.yearly";
    CompleteLeapYear => Complete, "complete.leap-year";
    CompleteFromCompletion => Complete, "complete.from-completion";
    CompleteCount => Complete, "complete.count";
    CompleteUntil => Complete, "complete.until";
    CompleteLateMonthly => Complete, "complete.late-monthly";
    CompleteStartOffsets => Complete, "complete.start-offsets";
    CompleteMissingFifth => Complete, "complete.missing-fifth-weekday";
    CompleteUndo => Complete, "complete.undo";
    SetRepeatDays => SetRepeat, "set-repeat.days";
    SetRepeatWeeks => SetRepeat, "set-repeat.weeks";
    SetRepeatWeekdays => SetRepeat, "set-repeat.weekdays";
    SetRepeatMonthly => SetRepeat, "set-repeat.monthly";
    SetRepeatMonthOverflow => SetRepeat, "set-repeat.month-overflow";
    SetRepeatNthWeekday => SetRepeat, "set-repeat.nth-weekday";
    SetRepeatYearly => SetRepeat, "set-repeat.yearly";
    SetRepeatLeapYear => SetRepeat, "set-repeat.leap-year";
    SetRepeatFromCompletion => SetRepeat, "set-repeat.from-completion";
    SetRepeatCount => SetRepeat, "set-repeat.count";
    SetRepeatUntil => SetRepeat, "set-repeat.until";
    SetRepeatLastDay => SetRepeat, "set-repeat.last-day";
    SetRepeatNthWorkday => SetRepeat, "set-repeat.nth-workday";
    SetRepeatNthWeekend => SetRepeat, "set-repeat.nth-weekend";
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

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{contract} is not verified ({direction}: {verdict})")]
pub struct RequirementFailure {
    pub contract: &'static str,
    pub direction: &'static str,
    pub verdict: &'static str,
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
        let fail = |direction, verdict: Verdict| RequirementFailure {
            contract: id.as_str(),
            direction,
            verdict: verdict.as_str(),
        };
        let Some(record) = record else {
            return Err(fail("missing contract", Verdict::Unverified));
        };
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
