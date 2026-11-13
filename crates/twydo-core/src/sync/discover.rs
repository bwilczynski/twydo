//! Local-only collection discovery through the bundled helper: lists created on the server
//! are adopted locally; a list missing on the server is reported, never created.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::Path;

use crate::config::Config;
use crate::lists::helper::{
    ExchangeError, HelperRequest, Unwatched, Watch, exchange_watched, refusal,
};
use crate::lists::plain_id;
use crate::store::fsio::VdirLock;

/// What a successful discovery changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    /// Collection ids newly created in the local vdir.
    pub adopted: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoverError {
    /// A local list whose collection is gone from the server (`:forget-list` it).
    Missing {
        id: String,
    },
    /// The helper cannot run here (its interpreter does not start, or vdirsyncer is not
    /// importable from it): a setup problem, not a verdict on the lists.
    Unavailable(String),
    /// Killed at its deadline; the marker-driven discovery is retried.
    TimedOut(String),
    /// Stopped on request (`sync/stop`, a daemon shutdown); not a failure.
    Stopped,
    Failed(String),
}

impl DiscoverError {
    /// The one wording of the failure. `name` is the missing list's display name, when known.
    pub fn describe(&self, name: Option<&str>) -> String {
        match self {
            Self::Missing { id } => format!(
                "list discovery stopped: list \"{}\" ({id}) no longer exists on the server; if you deleted it, run :forget-list {id}",
                name.unwrap_or(id)
            ),
            Self::Unavailable(m) => format!("list discovery unavailable: {m}"),
            Self::TimedOut(m) => m.clone(),
            Self::Stopped => "list discovery stopped".into(),
            Self::Failed(m) => format!("list discovery failed: {m}"),
        }
    }
}

impl std::fmt::Display for DiscoverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe(None))
    }
}

impl std::error::Error for DiscoverError {}

/// Refreshes vdirsyncer's collection cache. Callers hold the sync run lock and the shared
/// vdir lock (`state` is the vdir's state directory; the helper takes the shared vdir lock
/// itself too, so it holds it even when this process is killed).
pub fn discover(cfg: &Config, state: &Path) -> Result<Discovered, DiscoverError> {
    discover_watched(cfg, state, &Unwatched)
}

pub(crate) fn discover_watched(
    cfg: &Config,
    state: &Path,
    watch: &dyn Watch,
) -> Result<Discovered, DiscoverError> {
    let failed = |e: crate::lists::ListError| DiscoverError::Failed(e.to_string());
    let request = HelperRequest::discover(cfg, state).map_err(failed)?;
    // A killed helper cannot roll back: vdirsyncer creates a folder for each new collection
    // as it reaches it and saves its cache only at the end. The caller holds the shared vdir
    // lock, so no writer can have put anything into those folders.
    let before = top_level_dirs(&cfg.vdir);
    let lock = VdirLock::shared_for_child(state).map_err(|e| failed(e.into()))?;
    let reply = exchange_watched(cfg, &request, watch, Some(lock)).map_err(|e| {
        // Only a helper that died before it could roll back itself: after an exit 0 (even
        // with a broken reply) its cache lists the new collections, which need their folders.
        if !matches!(e, ExchangeError::Failed(_)) {
            remove_new_empty_dirs(&cfg.vdir, &before);
        }
        match e {
            ExchangeError::Unavailable(m) => DiscoverError::Unavailable(m),
            ExchangeError::Stopped => DiscoverError::Stopped,
            ExchangeError::TimedOut(e) => DiscoverError::TimedOut(e.to_string()),
            ExchangeError::Killed(e) => failed(e),
            ExchangeError::Failed(e) => failed(e),
        }
    })?;
    if !reply.ok {
        return Err(match reply.missing.as_slice() {
            [id] if reply.code == "discovery" && plain_id(id) => {
                DiscoverError::Missing { id: id.clone() }
            }
            _ if reply.code == "dependency" => {
                DiscoverError::Unavailable(refusal(cfg, "dependency"))
            }
            _ => DiscoverError::Failed(refusal(cfg, &reply.code)),
        });
    }
    let adopted = reply
        .result
        .get("adopted")
        .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
        .ok_or_else(|| DiscoverError::Failed(refusal(cfg, "protocol")))?;
    Ok(Discovered { adopted })
}

fn top_level_dirs(vdir: &Path) -> BTreeSet<OsString> {
    std::fs::read_dir(vdir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name())
        .collect()
}

/// `rmdir` (never recursive) of the directories that appeared since `before` and are empty.
fn remove_new_empty_dirs(vdir: &Path, before: &BTreeSet<OsString>) {
    for name in top_level_dirs(vdir).difference(before) {
        let _ = std::fs::remove_dir(vdir.join(name));
    }
}
