//! Params and results of the sync methods, and what a sync reports.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Params of `sync.run`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SyncRunParams {
    pub request_id: String,
    /// Lists to sync with `--force-delete` (directory or display names), as
    /// `tdo sync --force-delete`.
    #[serde(default)]
    pub force_delete: Vec<String>,
    /// Look for lists created on the server before syncing (`:discover` and `tdo sync --discover`
    /// ask for it; a daemon does it by itself at startup). Absent means no.
    #[serde(default)]
    pub discover: bool,
}

/// Why a sync ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = super::open_enum)]
pub enum SyncReason {
    Manual,
    Startup,
    Periodic,
    Batch,
    Resolve,
    SwitchOn,
    List,
    /// A value this client does not know: a newer daemon sent it. Never sent by the daemon
    /// this client was built with; see "Unknown values" in the daemon spec §6.2.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = super::open_enum)]
pub enum SyncOutcome {
    Ok,
    Failed,
    Stopped,
    /// The run did nothing (another runner held the run lock, sync is not configured, ...).
    Skipped,
    /// A value this client does not know: a newer daemon sent it. Never sent by the daemon
    /// this client was built with; see "Unknown values" in the daemon spec §6.2.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

/// What a sync did, in the words `tdo sync` uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SyncResult {
    pub outcome: SyncOutcome,
    /// What the user should act on (`tdo sync` prints them after `tdo: `).
    pub problems: Vec<String>,
    /// What did not fail the run (`tdo sync` prints them after `tdo: warning: `).
    pub warnings: Vec<String>,
    /// What the runner did on its own.
    pub notes: Vec<String>,
    pub new_conflicts: usize,
    /// Queued edits the sync dealt with (each also gets its `op.finished`).
    pub drained: usize,
}

/// Result of `sync.stop`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SyncStopResult {
    /// False when no sync ran in this daemon, so there was nothing to stop.
    pub stopping: bool,
}

/// Params of `sync.set_auto`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SyncSetAutoParams {
    pub request_id: String,
    pub on: bool,
}

/// Result of `sync.set_auto`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SyncSetAutoResult {
    /// The switch the call set: `true` is automatic sync on. It repeats the request's `on`;
    /// it is not read back from the switch file.
    pub auto: bool,
    /// The confirmation, in the words `tdo sync on|off` uses.
    pub message: String,
}
