//! The JSON form of a [`Snapshot`] on the socket: tasks without file bytes, paths relative
//! to the vdir, and the loaded configuration.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use twydo_core::config::Config;
use twydo_core::model::Task;
use twydo_core::vdir::{Collection, LoadReport};
use twydo_core::views::Snapshot;
pub use twydo_core::views::rev_of;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WireSnapshot {
    pub revision: u64,
    pub config: Config,
    pub config_warnings: Vec<String>,
    pub collections: Vec<WireCollection>,
    pub tasks: Vec<WireTask>,
    pub errors: Vec<WireError>,
    /// `daemon.unit_warning` of the configuration, which is not serialised inside `config`
    /// (an older client would reject the key). Absent means `true`.
    #[serde(default = "yes")]
    pub unit_warning: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WireTask {
    /// `path` is relative to the vdir.
    #[serde(flatten)]
    pub task: Task,
    pub rev: String,
    pub pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WireCollection {
    pub id: String,
    pub display_name: Option<String>,
    /// `#rrggbb`.
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WireError {
    /// Relative to the vdir.
    pub path: String,
    pub message: String,
}

fn relative(vdir: &std::path::Path, p: &std::path::Path) -> String {
    p.strip_prefix(vdir)
        .unwrap_or(p)
        .to_string_lossy()
        .into_owned()
}

pub fn to_wire(snap: &Snapshot, revision: u64) -> WireSnapshot {
    let vdir = &snap.config.vdir;
    let tasks = snap
        .tasks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let mut task = t.clone();
            task.path = PathBuf::from(relative(vdir, &t.path));
            WireTask {
                task,
                rev: snap.rev(i).unwrap_or_default().to_string(),
                pending: snap.is_pending(i),
            }
        })
        .collect();
    WireSnapshot {
        revision,
        config: snap.config.clone(),
        config_warnings: snap.config.warnings(),
        collections: snap
            .collections
            .iter()
            .map(|c| WireCollection {
                id: c.id.clone(),
                display_name: c.display_name.clone(),
                color: c.color.map(|(r, g, b)| format!("#{r:02x}{g:02x}{b:02x}")),
            })
            .collect(),
        tasks,
        errors: snap
            .errors
            .iter()
            .map(|(p, m)| WireError {
                path: relative(vdir, p),
                message: m.clone(),
            })
            .collect(),
        unit_warning: snap.config.daemon.unit_warning,
    }
}

/// Rebuilds a snapshot on the client: paths are joined with the configured vdir, pending
/// flags and revisions restored. It has no file bytes (no sources).
pub fn from_wire(w: WireSnapshot) -> Snapshot {
    let mut config = w.config;
    config.daemon.unit_warning = w.unit_warning;
    let vdir = config.vdir.clone();
    let mut revs = HashMap::new();
    let mut pending = HashSet::new();
    let mut tasks = Vec::with_capacity(w.tasks.len());
    for wt in w.tasks {
        let mut t = wt.task;
        t.path = vdir.join(&t.path);
        // An empty `rev` is how the wire says "bytes unknown": keep that as no revision.
        if !wt.rev.is_empty() {
            revs.insert(t.uid.clone(), wt.rev);
        }
        if wt.pending {
            pending.insert(t.uid.clone());
        }
        tasks.push(t);
    }
    let collections = w
        .collections
        .into_iter()
        .map(|c| Collection {
            path: vdir.join(&c.id),
            color: c.color.as_deref().and_then(twydo_core::vdir::parse_color),
            display_name: c.display_name,
            id: c.id,
        })
        .collect();
    let report = LoadReport {
        collections,
        tasks,
        errors: w
            .errors
            .into_iter()
            .map(|e| (vdir.join(e.path), e.message))
            .collect(),
        sources: HashMap::new(),
    };
    Snapshot::new(report, config)
        .with_pending(pending)
        .with_revs(revs)
}
