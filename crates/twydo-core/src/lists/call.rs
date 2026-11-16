//! Refusals of the CalDAV list calls: a stable code, never server output or addresses.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Config;

use super::ListError;

/// A refused or failed call. `sent` is false only when no write request was issued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub code: &'static str,
    pub missing: Vec<String>,
    pub sent: bool,
}

impl Refused {
    pub fn new(code: &'static str) -> Self {
        Refused {
            code,
            missing: Vec::new(),
            sent: true,
        }
    }

    /// A refusal raised before the write it guards was sent.
    pub fn unsent(code: &'static str) -> Self {
        Refused {
            sent: false,
            ..Refused::new(code)
        }
    }

    /// The same refusal, marked as raised once a write may have reached the server.
    pub fn mark_sent(mut self) -> Self {
        self.sent = true;
        self
    }

    /// The same refusal, marked as raised before any write was sent.
    pub fn not_sent(mut self) -> Self {
        self.sent = false;
        self
    }
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(wording(self.code))
    }
}

impl std::error::Error for Refused {}

/// The wording of a refusal code. Unknown codes get the generic failure text.
pub fn wording(code: &str) -> &'static str {
    match code {
        "mapping" => "list operation could not validate this pair and exact list identity",
        "dav" => "list operation could not verify the CalDAV response",
        "connect" => {
            "could not connect to the CalDAV server; check [sync].url, the network and [sync].ca_file"
        }
        "inventory" => {
            "cannot back up the complete remote list: inventory, strong resource ETags or resource reads could not be verified"
        }
        "operation" => "list operation refused stale or mismatched operation state; reconcile it",
        "stale" => "list changed on the server; sync and review your changes before saving",
        "config" => {
            "cannot load the collection pair from the generated vdirsyncer config; run tdo config check"
        }
        "credentials" => "collection credential fetch failed; check [sync].password_command",
        "exists" => "list identity already exists on the server",
        "choice" => {
            "server values differ from the requested save; review them or explicitly keep server values"
        }
        "discovery" => {
            "collection discovery found missing lists; resolve them before retrying (no unrelated list was created)"
        }
        "upgrade" => {
            "this list operation was started by an older Twydo and cannot be resumed; check on the server whether the change happened and make it there if needed"
        }
        "timeout" => "list operation timed out; reconcile its recorded operation before retrying",
        "changed" => "the list changed on the server since the preview; preview the deletion again",
        "present" => "the list still exists on the server",
        "partial" => "the deletion was only partly applied",
        _ => "list operation failed; inspect its operation before retrying",
    }
}

fn execute_mode() -> String {
    "execute".into()
}

fn default_timeout() -> u64 {
    300
}

/// What a list operation asks of the engine: the list request without the vdirsyncer
/// pair, its local storage and its config file. Operation records written by the current
/// version deserialize into it (unknown fields are ignored).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteRequest {
    pub action: String,
    pub vdir: PathBuf,
    pub collection: Option<String>,
    #[serde(default)]
    pub operation: Option<PathBuf>,
    #[serde(default)]
    pub state: Option<PathBuf>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub expected: Option<Value>,
    /// A `delete` request names the preview backup its server state is revalidated against.
    #[serde(default)]
    pub backup: Option<String>,
    #[serde(default = "execute_mode")]
    pub mode: String,
    #[serde(default)]
    pub keep_server: bool,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

impl RemoteRequest {
    /// A request for `action` with nothing else set, for the configured sync pair.
    pub fn new(cfg: &Config, action: &str) -> Result<Self, ListError> {
        let sync = cfg.sync.as_ref().ok_or_else(|| {
            ListError::Refused("list management needs a configured CalDAV sync pair".into())
        })?;
        Ok(Self {
            action: action.into(),
            vdir: std::path::absolute(&cfg.vdir)?,
            collection: None,
            operation: None,
            state: None,
            name: None,
            color: None,
            expected: None,
            backup: None,
            mode: execute_mode(),
            keep_server: false,
            timeout_secs: sync.timeout_secs.max(1),
        })
    }
}
