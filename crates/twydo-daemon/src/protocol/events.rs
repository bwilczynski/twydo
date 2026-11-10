//! Server notifications: `event`, `missed` and `heartbeat`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::methods::OpState;

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
