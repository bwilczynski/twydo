//! Params and results of the report methods (`check.report`).

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Params of `check.report`: the report `tdo check` prints, or with `uid` the permissions of
/// one task (`tdo op check`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CheckReportParams {
    /// Include task titles and parse-error paths.
    pub verbose: bool,
    /// A compatibility matrix file (a test hook); the daemon's own matrix when absent.
    #[serde(default)]
    pub matrix: Option<PathBuf>,
    /// Report the one task with this UID instead of the whole collection.
    #[serde(default)]
    pub uid: Option<String>,
}

/// What `check.report` answers: the lines to print on stdout and the warnings to print on
/// stderr (the client adds `tdo: `).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CheckReport {
    pub lines: Vec<String>,
    pub warnings: Vec<String>,
}

/// What `export` answers: the schema v1 document `tdo export-json` prints, and how many
/// resources the read could not parse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExportResult {
    /// Resources that failed to parse and are missing from `document`.
    pub skipped: usize,
    /// Schema v1, as read by tools/reconcile.py.
    #[schemars(description = "schema v1, as read by tools/reconcile.py")]
    pub document: serde_json::Value,
}
