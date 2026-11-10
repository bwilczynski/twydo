//! Params and results of the plan-1 methods.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use twydo_core::edit::Op;

use super::rpc::ErrorKind;

/// Which configuration and vdir a client means; checked by `hello`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct Instance {
    pub config: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vdir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HelloParams {
    pub client: String,
    pub version: String,
    pub protocol: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub instance: Instance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HelloResult {
    pub daemon: String,
    pub protocol: u32,
    pub epoch: String,
    pub session: String,
    pub vdir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonInfo {
    pub version: String,
    pub protocol: u32,
    pub pid: u32,
    pub uptime_secs: u64,
    pub vdir: PathBuf,
    pub epoch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ShutdownParams {
    pub when_idle: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SubscribeResult {
    pub seq: u64,
    pub revision: u64,
    pub status_rev: u64,
}

/// The snapshot a client planned against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BasedOn {
    pub epoch: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SubmitParams {
    pub session: String,
    pub request_id: String,
    pub based_on: BasedOn,
    pub op: Op,
}

/// Params of `op.undo` and `op.redo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UndoParams {
    pub session: String,
    pub request_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = super::open_enum)]
pub enum WriteOutcome {
    Applied,
    Queued,
    Satisfied,
    /// A value this client does not know: a newer daemon sent it. Never sent by the daemon
    /// this client was built with; see "Unknown values" in the daemon spec §6.2.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Created {
    pub uid: String,
    pub rev: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Undid {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op_id: Option<String>,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WriteResult {
    pub outcome: WriteOutcome,
    pub op_id: String,
    pub revision: u64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<Created>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undid: Option<Undid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OpGetParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
#[schemars(transform = super::open_enum)]
pub enum OpState {
    Pending,
    Applied {
        op_id: String,
    },
    Refused {
        kind: ErrorKind,
        message: String,
    },
    Cancelled,
    Satisfied {
        op_id: String,
    },
    /// A value this client does not know: a newer daemon sent it. Never sent by the daemon
    /// this client was built with; see "Unknown values" in the daemon spec §6.2.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CheckParams {
    pub based_on: BasedOn,
    pub op: Op,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CheckResult {
    pub allowed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AckParams {
    pub request_id: String,
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AckResult {
    pub acknowledged: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StatusView {
    pub status_rev: u64,
    pub sync: Option<SyncView>,
    pub pending_edits: usize,
    pub conflicts: usize,
    pub journal: JournalView,
    pub refusals: Vec<RefusalView>,
    /// Why the last drain of the write queue stopped with items left (a stuck journal, an
    /// I/O error); retried when the next long job ends. `None` when the last drain finished
    /// or only found the vdir held by another process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drain_error: Option<String>,
    /// Queue item files that do not parse (paths): they are not drained and not shown as
    /// pending; the user decides what to do with them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queue_unreadable: Vec<String>,
    /// The state of `config.toml` as the daemon reads it live; omitted while all is well.
    #[serde(default, skip_serializing_if = "ConfigStatus::is_ok")]
    pub config: ConfigStatus,
}

/// How the live configuration stands.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigStatus {
    /// Why the last change to the file was not applied (unreadable, invalid); the last good
    /// configuration stays in force.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Binding settings the file now names differently (`vdir`, `state_dir`, `writes`,
    /// `sync`); they apply only after the daemon restarts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub restart_needed: Vec<String>,
}

impl ConfigStatus {
    pub fn is_ok(&self) -> bool {
        self.error.is_none() && self.restart_needed.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JournalView {
    pub unfinished: usize,
    pub stuck: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RefusalView {
    pub id: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = super::open_enum)]
pub enum SyncState {
    Idle,
    Running,
    /// A value this client does not know: a newer daemon sent it. Never sent by the daemon
    /// this client was built with; see "Unknown values" in the daemon spec §6.2.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SyncView {
    pub state: SyncState,
    pub auto: bool,
    pub last_success: Option<DateTime<Utc>>,
    pub last_attempt: Option<DateTime<Utc>>,
    pub next_attempt: Option<DateTime<Utc>>,
    pub failures: u32,
    pub last_error: Option<String>,
    /// The job of the running sync (a `sync.run`); absent for a background sync.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
}
