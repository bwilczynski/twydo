//! Explicit collection requests and reconciliation; never an automatic write queue.
use std::fs;

use chrono::Utc;
use serde_json::json;

use crate::compat::{Matrix, Verdict};
use crate::config::WriteMode;
use crate::store::bind_vdir_state;

use super::helper::{self, HelperRequest};
use super::operation::{self, ListOperation, Phase};
use super::{ListError, ListService, ListState, plain_id};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListRequest {
    Create {
        id: String,
        name: String,
        color: (u8, u8, u8),
    },
    Edit {
        id: String,
        name: String,
        color: Option<(u8, u8, u8)>,
        expected: ListState,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListResult {
    Saved {
        operation: String,
        state: ListState,
    },
    DeletePreview(super::backup::DeletePreview),
    Exported {
        backup: String,
        destination: std::path::PathBuf,
    },
}

pub(super) fn capability(
    matrix: &Matrix,
    mode: WriteMode,
    case: &str,
    action: &str,
) -> Result<(), ListError> {
    let row = matrix.operation(case, action).ok_or_else(|| {
        ListError::Refused(format!(
            "{action} for lists is not verified (missing capability)"
        ))
    })?;
    let verdicts = [
        row.iphone_baseline,
        row.twydo_to_iphone,
        row.iphone_to_twydo,
    ];
    // The prototype's read/PROPPATCH/read sequence is not atomic. Test mode must
    // not bypass this row: promotion requires a demonstrated server precondition,
    // not only a successful name/color round trip.
    if case == "list-metadata"
        && (verdicts.iter().any(|v| *v != Verdict::Pass)
            || matrix
                .edge_case("list-metadata-atomicity")
                .is_none_or(|e| e.verdict != Verdict::Pass))
    {
        return Err(ListError::Refused(
            "list metadata editing is not verified: server-enforced protection against concurrent name/color edits is required (including in test mode)".into(),
        ));
    }
    if verdicts
        .iter()
        .any(|v| matches!(v, Verdict::Fail | Verdict::Unsupported))
    {
        return Err(ListError::Refused(format!(
            "{action} for lists is refused by its compatibility evidence"
        )));
    }
    if mode == WriteMode::Verified && verdicts.iter().any(|v| *v != Verdict::Pass) {
        return Err(ListError::Refused(format!(
            "{action} for lists is not verified against 2Do yet"
        )));
    }
    Ok(())
}

fn color_hex(color: (u8, u8, u8)) -> String {
    format!("#{:02X}{:02X}{:02X}", color.0, color.1, color.2)
}

fn validate(id: &str, name: &str) -> Result<(), ListError> {
    if !plain_id(id) || id.contains(['%', '?', '#', ':', '\\']) || id.chars().any(char::is_control)
    {
        return Err(ListError::Refused("invalid list identity".into()));
    }
    if name.trim().is_empty() || name.len() > 1024 || name.chars().any(char::is_control) {
        return Err(ListError::Refused(
            "list name must be nonempty, at most 1024 bytes, without control characters".into(),
        ));
    }
    Ok(())
}

impl ListService {
    pub(super) fn validate_binding(&self) -> Result<(), ListError> {
        if let Some(path) = &self.config_path {
            let saved =
                crate::config::Config::load(path).map_err(|e| ListError::Refused(e.to_string()))?;
            if saved.writes != self.config.writes
                || saved.sync != self.config.sync
                || std::path::absolute(saved.state_dir())?
                    != std::path::absolute(self.config.state_dir())?
                || fs::canonicalize(&saved.vdir)? != fs::canonicalize(&self.config.vdir)?
            {
                return Err(ListError::Refused(
                    "list configuration or write policy changed; reopen Twydo before saving".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn execute(&mut self, request: ListRequest) -> Result<ListResult, ListError> {
        self.validate_binding()?;
        let mut helper = HelperRequest::probe(&self.config)?;
        match request {
            ListRequest::Create { id, name, color } => {
                validate(&id, &name)?;
                capability(
                    &self.matrix,
                    self.config.writes.mode,
                    "list-create",
                    "create",
                )?;
                if fs::symlink_metadata(self.config.vdir.join(&id)).is_ok() {
                    return Err(ListError::Refused("list identity already exists".into()));
                }
                helper.action = "create".into();
                helper.collection = Some(id);
                helper.name = Some(name.trim().to_string());
                helper.color = Some(color_hex(color));
            }
            ListRequest::Edit {
                id,
                name,
                color,
                expected,
            } => {
                validate(&id, &name)?;
                capability(
                    &self.matrix,
                    self.config.writes.mode,
                    "list-metadata",
                    "edit",
                )?;
                if self.state(&id)? != expected {
                    return Err(ListError::Refused(
                        "list changed; reload and try again".into(),
                    ));
                }
                if color.is_none() && expected.color.is_some() {
                    return Err(ListError::Refused(
                        "choose a color to replace the existing list color".into(),
                    ));
                }
                helper.action = "edit".into();
                helper.collection = Some(id.clone());
                if name.trim() != expected.name {
                    helper.name = Some(name.trim().to_string());
                }
                if color != expected.color {
                    helper.color = color.map(color_hex);
                }
                if helper.name.is_none() && helper.color.is_none() {
                    return Err(ListError::Refused("no list changes".into()));
                }
                let raw_name = fs::read_to_string(self.config.vdir.join(&id).join("displayname"))
                    .ok()
                    .map(|s| s.trim().to_string());
                helper.expected =
                    Some(json!({"name":raw_name, "color":expected.color.map(color_hex)}));
            }
        }
        let record = operation::prepare(&self.config, helper, Utc::now())?;
        let id = record.id.clone();
        if let Err(error) = helper::invoke(&self.config, &record.request) {
            if operation::cancel_unstarted(&self.config, &id).is_ok() {
                return Err(error);
            }
            return Err(ListError::Refused(format!(
                "list operation {id} needs reconciliation: {error}"
            )));
        }
        self.finalize_remote(&id).map_err(|e| {
            ListError::Refused(format!("list operation {id} needs reconciliation: {e}"))
        })
    }

    fn finalize_remote(&mut self, id: &str) -> Result<ListResult, ListError> {
        self.validate_binding()?;
        let state = bind_vdir_state(&self.config.state_dir(), &self.config.vdir)?;
        let mut record = operation::read(&state, id)?;
        if record.vdir != fs::canonicalize(&self.config.vdir)? {
            return Err(ListError::Refused(
                "collection operation belongs to another vdir".into(),
            ));
        }
        if record.phase == Phase::RemoteConfirmed {
            operation::finalize_metadata(&self.config, &mut record)?;
            let mut request = record.request.clone();
            request.mode = "refresh".into();
            helper::invoke(&self.config, &request).map_err(|e| {
                ListError::Refused(format!("list operation {id} needs reconciliation: {e}"))
            })?;
            record = operation::read(&state, id)?;
        }
        if record.phase != Phase::LocalConfirmed {
            return Err(ListError::Refused(format!(
                "list operation {id} has no confirmed local discovery result"
            )));
        }
        operation::finish(&self.config, &mut record)?;
        self.result(&record)
    }

    fn result(&self, record: &ListOperation) -> Result<ListResult, ListError> {
        let collection = record
            .request
            .collection
            .as_deref()
            .ok_or_else(|| ListError::Refused("missing list identity".into()))?;
        Ok(ListResult::Saved {
            operation: record.id.clone(),
            state: self.state(collection)?,
        })
    }

    pub fn reconcile(&mut self, id: &str) -> Result<ListResult, ListError> {
        self.reconcile_with_server(id, false)
    }

    /// Explicitly accept current server metadata after a partial/conflicting save.
    pub fn keep_server(&mut self, id: &str) -> Result<ListResult, ListError> {
        self.reconcile_with_server(id, true)
    }

    fn reconcile_with_server(
        &mut self,
        id: &str,
        keep_server: bool,
    ) -> Result<ListResult, ListError> {
        self.validate_binding()?;
        let state = bind_vdir_state(&self.config.state_dir(), &self.config.vdir)?;
        let mut record = operation::read(&state, id)?;
        if record.vdir != fs::canonicalize(&self.config.vdir)? {
            return Err(ListError::Refused(
                "collection operation belongs to another vdir".into(),
            ));
        }
        if record.phase == Phase::Complete {
            return self.result(&record);
        }
        if record.request.action == "preview-delete" {
            operation::cancel_unstarted(&self.config, id)?;
            return Err(ListError::Refused(
                "Interrupted read-only deletion preview cleared; request a fresh preview".into(),
            ));
        }
        if record.phase == Phase::LocalConfirmed {
            operation::finish(&self.config, &mut record)?;
            return self.result(&record);
        }
        if operation::cancel_unstarted(&self.config, id).is_ok() {
            return Err(ListError::Refused(
                "the list operation was not sent; retry your changes".into(),
            ));
        }
        let mut request = record.request.clone();
        request.mode = "reconcile".into();
        request.keep_server = keep_server;
        helper::invoke(&self.config, &request).map_err(|e| {
            ListError::Refused(format!("list operation {id} remains unresolved: {e}"))
        })?;
        self.finalize_remote(id)
    }
}
