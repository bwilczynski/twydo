//! Runs the TUI's write requests through the store and turns the result into a message.

use chrono::{DateTime, Utc};
use twydo_core::compat::Matrix;
use twydo_core::config::Config;
use twydo_core::edit::Op;
use twydo_core::store::Store;
use twydo_core::views::Snapshot;

/// A write the TUI asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Apply(Op),
    Undo,
    Redo,
}

/// Called after each successful write when `[sync] after_edit` is on.
pub type SyncTrigger = Box<dyn FnMut(&Config)>;

/// The TUI's store for the session, holding its undo/redo stack.
pub struct Writer {
    store: Result<Store, String>,
    config: Config,
    after_write: SyncTrigger,
}

impl Writer {
    /// The embedded matrix; syncs in the background after writes.
    pub fn open(cfg: &Config) -> Writer {
        Writer::open_with(
            cfg,
            Matrix::embedded().clone(),
            Box::new(|c| crate::cli::sync::spawn_background(c, Some(&c.vdir))),
        )
    }

    /// Never fails: an open error is kept and returned by every [`Writer::run`].
    pub fn open_with(cfg: &Config, matrix: Matrix, after_write: SyncTrigger) -> Writer {
        Writer {
            store: Store::open_with(cfg, matrix)
                .map(|(s, _)| s)
                .map_err(|e| format!("cannot write: {e}")),
            config: cfg.clone(),
            after_write,
        }
    }

    /// Ok: the success message. Err: why nothing was written.
    pub fn run(
        &mut self,
        snap: &Snapshot,
        req: &Request,
        now: DateTime<Utc>,
    ) -> Result<String, String> {
        let store = self.store.as_mut().map_err(|e| e.clone())?;
        let done = match req {
            Request::Apply(op) => store.apply(snap, op, now),
            Request::Undo => store.undo_last(now),
            Request::Redo => store.redo_last(now),
        };
        done.map_err(|e| e.to_string())?;
        if self.config.sync.as_ref().is_some_and(|s| s.after_edit) {
            (self.after_write)(&self.config);
        }
        Ok(success(snap, req))
    }
}

fn success(snap: &Snapshot, req: &Request) -> String {
    match req {
        Request::Undo => "undone".into(),
        Request::Redo => "redone".into(),
        Request::Apply(op) => match op {
            Op::Complete { .. } => "done".into(),
            Op::Reopen { .. } => "reopened".into(),
            Op::Delete { .. } => "deleted · u to undo".into(),
            Op::Edit { .. } => "saved".into(),
            Op::Create(_) => "added".into(),
            Op::Move { to, .. } => {
                let name = snap
                    .collections
                    .iter()
                    .find(|c| &c.id == to)
                    .and_then(|c| c.display_name.clone())
                    .unwrap_or_else(|| to.clone());
                format!("moved to {name}")
            }
        },
    }
}
