//! Params of the list methods (all jobs, daemon spec §7.4) and the list values their results
//! carry.

use std::path::PathBuf;

use chrono::{DateTime, Utc};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use twydo_core::lists::ListState;
use twydo_core::lists::backup::DeletePreview;

/// A list as the list methods see it: its name, colour and local archive flag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WireListState {
    /// The list's directory name in the vdir.
    pub id: String,
    pub name: String,
    /// `#rrggbb`; absent when the list has no colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub archived: bool,
}

/// `#rrggbb` for a colour.
pub fn color_hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The colour `#rrggbb` (either case) names; an error names the value.
pub fn parse_color(s: &str) -> Result<(u8, u8, u8), String> {
    let hex = s
        .strip_prefix('#')
        .filter(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| format!("invalid colour {s:?}: expected #rrggbb"))?;
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).expect("checked hex digits");
    Ok((byte(0), byte(2), byte(4)))
}

impl From<ListState> for WireListState {
    fn from(s: ListState) -> Self {
        WireListState {
            id: s.id,
            name: s.name,
            color: s.color.map(color_hex),
            archived: s.archived,
        }
    }
}

impl TryFrom<WireListState> for ListState {
    type Error = String;

    fn try_from(w: WireListState) -> Result<Self, String> {
        Ok(ListState {
            id: w.id,
            name: w.name,
            color: w.color.as_deref().map(parse_color).transpose()?,
            archived: w.archived,
        })
    }
}

/// What `lists.preview_delete` saved: the backup that a deletion would keep.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WireDeletePreview {
    pub backup: String,
    pub collection: String,
    pub name: String,
    pub resources: usize,
    pub local_files: usize,
}

impl WireDeletePreview {
    /// The preview a deletion backup `b` stands for.
    pub fn of(b: &twydo_core::lists::backup::Backup) -> WireDeletePreview {
        WireDeletePreview {
            backup: b.id.clone(),
            collection: b.collection.clone(),
            name: b.name.clone(),
            resources: b.remote.resources.len(),
            local_files: b.local.len(),
        }
    }
}

impl From<DeletePreview> for WireDeletePreview {
    fn from(p: DeletePreview) -> Self {
        WireDeletePreview {
            backup: p.backup,
            collection: p.collection,
            name: p.name,
            resources: p.resources,
            local_files: p.local_files,
        }
    }
}

impl From<WireDeletePreview> for DeletePreview {
    fn from(p: WireDeletePreview) -> Self {
        DeletePreview {
            backup: p.backup,
            collection: p.collection,
            name: p.name,
            resources: p.resources,
            local_files: p.local_files,
        }
    }
}

/// Params of `lists.create`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListCreateParams {
    pub request_id: String,
    /// The new list's directory name.
    pub id: String,
    pub name: String,
    /// `#rrggbb`.
    pub color: String,
}

/// Params of `lists.update`: the new name and colour of the list `expected` describes as the
/// client saw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListUpdateParams {
    pub request_id: String,
    pub id: String,
    pub name: String,
    /// `#rrggbb`; absent for no colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub expected: WireListState,
}

/// Params of `lists.archive` and `lists.unarchive`: the list as the client saw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListArchiveParams {
    pub request_id: String,
    pub expected: WireListState,
}

/// Params of `lists.preview_delete`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListPreviewDeleteParams {
    pub request_id: String,
    pub id: String,
}

/// Params of `lists.delete`: the preview the client showed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListDeleteParams {
    pub request_id: String,
    pub preview: WireDeletePreview,
}

/// Params of `lists.reconcile`: finish the unfinished list operation `operation`, or, with
/// `keep_server`, adopt what the server holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListReconcileParams {
    pub request_id: String,
    pub operation: String,
    pub keep_server: bool,
}

/// Params of `lists.export`: copy backup `backup` to the new directory `destination`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListExportParams {
    pub request_id: String,
    pub backup: String,
    pub destination: PathBuf,
}

/// Params of `lists.preview_forget`: the list to forget, by directory or display name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListPreviewForgetParams {
    pub name: String,
}

/// What forgetting a list would preserve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ForgetPreview {
    /// The list's directory name.
    pub collection: String,
    pub name: String,
    pub tasks: usize,
    /// Edits of the list that are not yet synced.
    pub pending: usize,
}

impl ForgetPreview {
    pub fn of(p: &twydo_core::retired::Preview) -> ForgetPreview {
        ForgetPreview {
            collection: p.collection.clone(),
            name: p.name.clone(),
            tasks: p.tasks,
            pending: p.pending,
        }
    }
}

/// Params of `lists.forget`: the list `name` as the preview showed it; the job fails when
/// `name` now resolves to another directory than `collection`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListForgetParams {
    pub request_id: String,
    pub name: String,
    pub collection: String,
}

/// Params of `lists.recover`: copy the tasks of retired-list backup `backup` into the
/// existing list `to`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListRecoverParams {
    pub session: String,
    pub request_id: String,
    pub backup: String,
    pub to: String,
}

/// A preserved (forgotten) list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RetiredBackupView {
    pub id: String,
    pub collection: String,
    pub name: String,
    pub created: DateTime<Utc>,
    pub tasks: usize,
    pub pending: usize,
    pub retired: bool,
}

impl RetiredBackupView {
    pub fn of(b: &twydo_core::retired::Backup) -> RetiredBackupView {
        RetiredBackupView {
            id: b.id.clone(),
            collection: b.collection.clone(),
            name: b.name.clone(),
            created: b.created,
            tasks: b.tasks,
            pending: b.pending,
            retired: b.retired,
        }
    }
}

/// A backup a list deletion kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeletedBackupView {
    pub id: String,
    pub collection: String,
    pub name: String,
    pub created: DateTime<Utc>,
}

/// The answer of `lists.backups`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Backups {
    pub retired: Vec<RetiredBackupView>,
    pub deleted: Vec<DeletedBackupView>,
}

/// Params of `lists.backup_preview`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BackupPreviewParams {
    pub id: String,
}

/// An unfinished list save (`lists.pending`), as the TUI's reconcile card needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PendingListOp {
    /// The operation record's id (`lists.reconcile` takes it).
    pub operation: String,
    /// `create`, `update`, and so on.
    pub action: String,
    pub collection: Option<String>,
    pub name: Option<String>,
    pub color: Option<String>,
    /// The name the server reported, when the record has it.
    pub observed_name: Option<String>,
    /// The colour the server reported, when the record has it.
    pub observed_color: Option<String>,
}

impl PendingListOp {
    pub fn of(record: &twydo_core::lists::operation::ListOperation) -> PendingListOp {
        let (observed_name, observed_color) =
            twydo_core::lists::operation::observed(record).unwrap_or_default();
        PendingListOp {
            operation: record.id.clone(),
            action: record.request.action.clone(),
            collection: record.request.collection.clone(),
            name: record.request.name.clone(),
            color: record.request.color.clone(),
            observed_name,
            observed_color,
        }
    }
}
