//! List workers use the same service in the real loop and the integration tests.
use crate::app::{ListAction, ListCompletion};
use std::path::Path;
use std::sync::mpsc;
use twydo_core::{
    config::Config,
    lists::{ListError, ListResult, ListService, operation},
};

pub fn perform_list_action(
    cfg: &Config,
    config_path: Option<&Path>,
    action: ListAction,
) -> ListCompletion {
    let result = (|| -> Result<ListResult, ListError> {
        let mut service = ListService::open(cfg, config_path)?;
        match &action {
            ListAction::ExportBackup {
                backup,
                destination,
            } => {
                twydo_core::lists::backup::export(cfg, backup, destination)?;
                Ok(ListResult::Exported {
                    backup: backup.clone(),
                    destination: destination.clone(),
                })
            }
            ListAction::PreviewDelete { id } => {
                service.preview_delete(id).map(ListResult::DeletePreview)
            }
            ListAction::Delete(preview) => service.delete_list(preview),
            ListAction::Save(request) => service.execute(request.clone()),
            ListAction::Archive { expected, archived } => service
                .set_archived(&expected.id, *archived, expected)
                .map(|state| ListResult::Saved {
                    operation: String::new(),
                    state,
                }),
            ListAction::Reconcile {
                operation,
                keep_server,
            } => {
                if *keep_server {
                    service.keep_server(operation)
                } else {
                    service.reconcile(operation)
                }
            }
        }
    })()
    .map_err(|e| e.to_string());
    let active = operation::active(cfg).ok().flatten();
    let server_values = active
        .as_ref()
        .and_then(twydo_core::lists::operation::observed);
    let pending = active.and_then(|r| r.request.collection.map(|id| (r.id, id)));
    ListCompletion {
        action,
        result,
        pending,
        server_values,
    }
}

pub(super) fn start(
    cfg: &Config,
    path: Option<&Path>,
    action: ListAction,
    tx: &mpsc::Sender<ListCompletion>,
) {
    let cfg = cfg.clone();
    let path = path.map(Path::to_path_buf);
    let tx = tx.clone();
    std::thread::spawn(move || {
        let _ = tx.send(perform_list_action(&cfg, path.as_deref(), action));
    });
}
