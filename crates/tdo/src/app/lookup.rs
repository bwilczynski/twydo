//! The reads the panels make: what forgetting a list keeps, the preserved lists and their
//! tasks, the backups deletions kept, and the unfinished list save. The terminal reads them
//! from the daemon ([`DaemonLookup`]); the tests read the files ([`FileLookup`]).
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use twydo_core::config::Config;
use twydo_core::views::Snapshot;
use twydo_daemon::client::{Caller, ClientError};
use twydo_daemon::protocol::lists::{
    BackupPreviewParams, Backups, ForgetPreview, ListPreviewForgetParams, PendingListOp,
    RetiredBackupView, WireDeletePreview,
};
use twydo_daemon::protocol::wire::{WireSnapshot, from_wire};

/// How long a panel read waits for the daemon.
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

pub trait Lookup {
    fn forget_preview(&self, name: &str) -> Result<ForgetPreview, String>;
    fn retired(&self) -> Result<Vec<RetiredBackupView>, String>;
    fn retired_preview(&self, id: &str) -> Result<Snapshot, String>;
    fn deleted_backups(&self) -> Result<Vec<WireDeletePreview>, String>;
    fn pending_list_op(&self) -> Result<Option<PendingListOp>, String>;
}

/// Reads the files directly. Only the tests use it, and `App::new` starts with it until the
/// terminal swaps in [`DaemonLookup`].
#[doc(hidden)]
pub struct FileLookup(pub Config);

impl Lookup for FileLookup {
    fn forget_preview(&self, name: &str) -> Result<ForgetPreview, String> {
        twydo_core::retired::preview(&self.0, name)
            .map(|p| ForgetPreview::of(&p))
            .map_err(|e| e.to_string())
    }
    fn retired(&self) -> Result<Vec<RetiredBackupView>, String> {
        twydo_core::retired::list(&self.0)
            .map(|v| v.iter().map(RetiredBackupView::of).collect())
            .map_err(|e| e.to_string())
    }
    fn retired_preview(&self, id: &str) -> Result<Snapshot, String> {
        twydo_core::retired::snapshot(&self.0, id).map_err(|e| e.to_string())
    }
    fn deleted_backups(&self) -> Result<Vec<WireDeletePreview>, String> {
        twydo_core::lists::backup::list(&self.0)
            .map(|v| v.iter().map(WireDeletePreview::of).collect())
            .map_err(|e| e.to_string())
    }
    fn pending_list_op(&self) -> Result<Option<PendingListOp>, String> {
        twydo_core::lists::operation::active(&self.0)
            .map(|r| r.as_ref().map(PendingListOp::of))
            .map_err(|e| e.to_string())
    }
}

/// Reads through the daemon. It asks `source` for the connection on each read, so it follows
/// reconnects; while the link is down every read says so.
pub struct DaemonLookup(pub Arc<dyn Fn() -> Option<Caller> + Send + Sync>);

impl DaemonLookup {
    fn call<P: serde::Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<R, String> {
        let caller = (self.0)().ok_or("daemon not running; try again")?;
        caller
            .call_timeout(method, params, CALL_TIMEOUT)
            .map_err(|e| match e {
                ClientError::Rpc(e) => e.message,
                ClientError::TimedOut(_) => "daemon did not answer; try again".into(),
                e => format!("daemon not running: {e}"),
            })
    }
}

impl Lookup for DaemonLookup {
    fn forget_preview(&self, name: &str) -> Result<ForgetPreview, String> {
        self.call(
            "lists.preview_forget",
            ListPreviewForgetParams { name: name.into() },
        )
    }
    fn retired(&self) -> Result<Vec<RetiredBackupView>, String> {
        let b: Backups = self.call("lists.backups", serde_json::json!({}))?;
        Ok(b.retired)
    }
    fn retired_preview(&self, id: &str) -> Result<Snapshot, String> {
        let w: WireSnapshot = self.call(
            "lists.backup_preview",
            BackupPreviewParams { id: id.into() },
        )?;
        Ok(from_wire(w))
    }
    fn deleted_backups(&self) -> Result<Vec<WireDeletePreview>, String> {
        let b: Backups = self.call("lists.backups", serde_json::json!({}))?;
        Ok(b.deleted
            .into_iter()
            .map(|d| WireDeletePreview {
                backup: d.id,
                collection: d.collection,
                name: d.name,
                resources: d.resources,
                local_files: d.local_files,
            })
            .collect())
    }
    fn pending_list_op(&self) -> Result<Option<PendingListOp>, String> {
        self.call("lists.pending", serde_json::json!({}))
    }
}
