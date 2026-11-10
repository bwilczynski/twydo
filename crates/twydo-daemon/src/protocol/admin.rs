//! Params and results of the conflict methods (`conflicts.list`, `conflicts.resolve`) and the
//! journal methods (`journal.list`, `journal.recover`, `journal.revert`, `journal.abandon`).

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use twydo_core::conflicts::{Conflict, ConflictKind, ConflictState, Side};
use twydo_core::store::journal::{Entry, RecoveryReport, Stage};
use twydo_core::sync::resolve::Resolution;

/// What kind of disagreement a conflict is (mirrors the core's `ConflictKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = super::open_enum)]
pub enum WireConflictKind {
    BothChanged,
    LocalDeleteRemoteEdit,
    RemoteDeleteLocalEdit,
    Journal,
    /// A value this client does not know: a newer daemon sent it. Never sent by the daemon
    /// this client was built with; see "Unknown values" in the daemon spec §6.2.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

/// Where a conflict is in its life (mirrors the core's `ConflictState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = super::open_enum)]
pub enum WireConflictState {
    Open,
    Chosen,
    Resolved,
    Superseded,
    /// A value this client does not know: a newer daemon sent it. Never sent by the daemon
    /// this client was built with; see "Unknown values" in the daemon spec §6.2.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

/// Which copy of a task (mirrors the core's `Side`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = super::open_enum)]
pub enum WireSide {
    Local,
    Remote,
    /// A value this client does not know: a newer daemon sent it. Never sent by the daemon
    /// this client was built with; see "Unknown values" in the daemon spec §6.2.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

impl From<ConflictKind> for WireConflictKind {
    fn from(k: ConflictKind) -> Self {
        match k {
            ConflictKind::BothChanged => WireConflictKind::BothChanged,
            ConflictKind::LocalDeleteRemoteEdit => WireConflictKind::LocalDeleteRemoteEdit,
            ConflictKind::RemoteDeleteLocalEdit => WireConflictKind::RemoteDeleteLocalEdit,
            ConflictKind::Journal => WireConflictKind::Journal,
        }
    }
}

impl From<ConflictState> for WireConflictState {
    fn from(s: ConflictState) -> Self {
        match s {
            ConflictState::Open => WireConflictState::Open,
            ConflictState::Chosen => WireConflictState::Chosen,
            ConflictState::Resolved => WireConflictState::Resolved,
            ConflictState::Superseded => WireConflictState::Superseded,
        }
    }
}

impl From<Side> for WireSide {
    fn from(s: Side) -> Self {
        match s {
            Side::Local => WireSide::Local,
            Side::Remote => WireSide::Remote,
        }
    }
}

/// One unresolved conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConflictView {
    pub id: String,
    pub uid: String,
    pub collection: String,
    /// Vdir-relative path of the task's file.
    pub path: String,
    /// The task's `SUMMARY` from either version (local first); absent when neither has one.
    pub title: Option<String>,
    pub kind: WireConflictKind,
    /// True while vdirsyncer still keeps the two sides apart: only local or remote can be kept.
    pub held: bool,
    pub state: WireConflictState,
    /// The side chosen, once a held conflict has one (applied on the next sync).
    pub choice: Option<WireSide>,
    pub detected: DateTime<Utc>,
}

/// Result of `conflicts.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConflictList {
    pub conflicts: Vec<ConflictView>,
}

impl ConflictView {
    pub fn of(c: &Conflict) -> ConflictView {
        ConflictView {
            id: c.id.clone(),
            uid: c.uid.clone(),
            collection: c.collection.clone(),
            path: c.path.clone(),
            title: c.title_opt(),
            kind: c.kind.into(),
            held: c.held,
            state: c.state.into(),
            choice: c.choice.map(Into::into),
            detected: c.detected,
        }
    }
}

/// Which version to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WireKeep {
    Local,
    Remote,
    /// The task as the vdir holds it now (not for a held conflict).
    Current,
}

/// Params of `conflicts.resolve`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConflictsResolveParams {
    pub request_id: String,
    pub id: String,
    pub keep: WireKeep,
}

/// What a resolution did (mirrors the core's `Resolution`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = super::open_enum)]
pub enum WireResolution {
    /// A held conflict: the choice is recorded and the next sync applies it.
    Recorded,
    /// The vdir already held the kept version.
    AlreadyInPlace,
    /// The kept version was written to the vdir (`op_id` names the journal entry).
    Written,
    /// The conflict is closed and the vdir file left as it is.
    KeptCurrent,
    /// A value this client does not know: a newer daemon sent it. Never sent by the daemon
    /// this client was built with; see "Unknown values" in the daemon spec §6.2.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

/// Result of `conflicts.resolve`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResolveResult {
    pub resolution: WireResolution,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op_id: Option<String>,
    /// The confirmation, in the words `tdo conflicts resolve` uses.
    pub message: String,
}

impl ResolveResult {
    pub fn of(r: &Resolution) -> ResolveResult {
        let (resolution, op_id) = match r {
            Resolution::Recorded => (WireResolution::Recorded, None),
            Resolution::AlreadyInPlace => (WireResolution::AlreadyInPlace, None),
            Resolution::Written(op) => (WireResolution::Written, Some(op.clone())),
            Resolution::KeptCurrent => (WireResolution::KeptCurrent, None),
        };
        ResolveResult {
            resolution,
            op_id,
            message: r.message().to_string(),
        }
    }
}

/// Where a journal entry is in its life (mirrors the core's `Stage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = super::open_enum)]
pub enum WireStage {
    Prepared,
    Applied,
    Abandoned,
    /// A value this client does not know: a newer daemon sent it. Never sent by the daemon
    /// this client was built with; see "Unknown values" in the daemon spec §6.2.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

impl From<Stage> for WireStage {
    fn from(s: Stage) -> Self {
        match s {
            Stage::Prepared => WireStage::Prepared,
            Stage::Applied => WireStage::Applied,
            Stage::Abandoned => WireStage::Abandoned,
        }
    }
}

/// One journal entry, as `tdo op journal` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JournalEntryView {
    pub id: String,
    pub created: DateTime<Utc>,
    pub kind: String,
    pub stage: WireStage,
    pub uids: Vec<String>,
    /// When a sync confirmed the server holds the entry's result.
    pub synced: Option<DateTime<Utc>>,
    /// The client request behind the entry, if one made it.
    pub request_id: Option<String>,
}

impl JournalEntryView {
    pub fn of(e: &Entry) -> JournalEntryView {
        JournalEntryView {
            id: e.id.clone(),
            created: e.created,
            kind: e.kind.clone(),
            stage: e.stage.into(),
            uids: e.uids.clone(),
            synced: e.synced,
            request_id: e.request.as_ref().map(|r| r.request_id.clone()),
        }
    }
}

/// An entry recovery could not finish.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StuckView {
    pub id: String,
    pub reason: String,
}

/// Result of `journal.recover`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecoverResult {
    /// Ids of the entries rolled forward to applied.
    pub resumed: Vec<String>,
    pub stuck: Vec<StuckView>,
}

impl RecoverResult {
    pub fn of(r: &RecoveryReport) -> RecoverResult {
        RecoverResult {
            resumed: r.resumed.clone(),
            stuck: r
                .stuck
                .iter()
                .map(|(id, reason)| StuckView {
                    id: id.clone(),
                    reason: reason.clone(),
                })
                .collect(),
        }
    }
}

/// Params of `journal.revert`: undoes the applied entry `op_id`, whoever wrote it. The revert
/// is not on any session's undo stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JournalRevertParams {
    pub session: String,
    pub request_id: String,
    pub op_id: String,
}

/// Params of `journal.abandon`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JournalAbandonParams {
    pub request_id: String,
    pub op_id: String,
}

/// Result of `journal.abandon`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AbandonResult {
    /// False when the entry was already abandoned.
    pub abandoned: bool,
}
