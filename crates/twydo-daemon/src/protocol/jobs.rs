//! Jobs: long requests (a sync, list actions) that answer at once with a `job_id` and report
//! their outcome as `job.finished` and through `job.get` (daemon spec §7.4).

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::lists::{WireDeletePreview, WireListState};
use super::rpc::ErrorKind;
use super::sync::SyncResult;

/// The answer of a job method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JobStarted {
    pub job_id: String,
}

/// Params of `job.get`: exactly one of the two.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JobGetParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(transform = super::open_enum)]
pub enum JobKind {
    Sync,
    ListCreate,
    ListUpdate,
    ListArchive,
    ListUnarchive,
    ListPreviewDelete,
    ListDelete,
    ListReconcile,
    ListExport,
    ListForget,
    ListRecover,
    /// A value this client does not know: a newer daemon sent it. Never sent by the daemon
    /// this client was built with; see "Unknown values" in the daemon spec §6.2.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
#[schemars(transform = super::open_enum)]
pub enum JobState {
    Queued,
    Running,
    Finished {
        result: JobResult,
    },
    Failed {
        kind: ErrorKind,
        message: String,
    },
    /// The daemon stopped before the job ended (it was queued or running).
    Interrupted,
    /// A value this client does not know: a newer daemon sent it. Never sent by the daemon
    /// this client was built with; see "Unknown values" in the daemon spec §6.2.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

impl JobState {
    /// Has the job ended (finished, failed or interrupted)?
    pub fn is_final(&self) -> bool {
        !matches!(self, JobState::Queued | JobState::Running)
    }
}

/// What a finished job produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[schemars(transform = super::open_enum)]
pub enum JobResult {
    Sync(SyncResult),
    /// A list was created, changed, archived, unarchived or reconciled; `operation` is the
    /// list operation's id (empty for a local archive change).
    ListSaved {
        operation: String,
        state: WireListState,
    },
    /// `lists.preview_delete` saved a backup.
    DeletePreview(WireDeletePreview),
    /// `lists.export` copied backup `backup` to `destination`.
    Exported {
        backup: String,
        destination: PathBuf,
    },
    /// `lists.forget` preserved list `name` as `backup`; `message` is what the TUI shows
    /// (with the sync's problems appended), `sync` the sync that followed.
    Forgot {
        backup: String,
        name: String,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sync: Option<Box<SyncResult>>,
    },
    /// `lists.recover` copied a backup into a list (`applied`), or found it already copied.
    Recovered {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        op_id: Option<String>,
        applied: bool,
        message: String,
    },
    /// A value this client does not know: a newer daemon sent it. Never sent by the daemon
    /// this client was built with; see "Unknown values" in the daemon spec §6.2.
    #[serde(other)]
    #[schemars(skip)]
    Unknown,
}

/// A job as `job.get` returns it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JobView {
    pub job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub kind: JobKind,
    pub state: JobState,
    pub created: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<DateTime<Utc>>,
}
