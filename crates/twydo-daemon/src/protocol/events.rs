//! Server notifications: `event`, `missed` and `heartbeat`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::jobs::JobState;
use super::methods::OpState;
use super::sync::{SyncReason, SyncResult};

/// Params of the `event` notification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EventEnvelope {
    pub seq: u64,
    #[serde(flatten)]
    pub event: Event,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
#[schemars(transform = super::open_enum)]
pub enum Event {
    #[serde(rename = "status.changed")]
    StatusChanged { status_rev: u64 },
    #[serde(rename = "data.changed")]
    DataChanged {
        revision: u64,
        cause: Cause,
        uids: Vec<String>,
    },
    #[serde(rename = "op.finished")]
    OpFinished {
        op_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        state: OpState,
    },
    /// A sync began (a `sync.run` job's, or a background one without a `job_id`).
    #[serde(rename = "sync.started")]
    SyncStarted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        job_id: Option<String>,
        reason: SyncReason,
    },
    /// A sync ended; sent after its changes are published (`status_rev` is the status after
    /// it).
    #[serde(rename = "sync.finished")]
    SyncFinished {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        job_id: Option<String>,
        reason: SyncReason,
        result: SyncResult,
        status_rev: u64,
    },
    /// A job ended; `job.get` returns the same state later.
    #[serde(rename = "job.finished")]
    JobFinished {
        job_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        state: JobState,
    },
    /// A value this client does not know: a newer daemon sent it. Never sent by the daemon
    /// this client was built with; see "Unknown values" in the daemon spec §6.2.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = super::open_enum)]
pub enum Cause {
    Op,
    Sync,
    Outside,
    List,
    Recovery,
    Config,
    /// A value this client does not know: a newer daemon sent it. Never sent by the daemon
    /// this client was built with; see "Unknown values" in the daemon spec §6.2.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

/// Params of the `missed` notification: events were dropped; refetch from these marks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Missed {
    pub seq: u64,
    pub revision: u64,
    pub status_rev: u64,
}

/// Params of the `heartbeat` notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Heartbeat {
    pub seq: u64,
}
