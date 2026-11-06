//! The on-disk write queue: `<state>/queue/<id>.json` items waiting for the vdir lock.
//! See the write-queue design (`docs/superpowers/specs/2026-11-06-write-queue-design.md`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::fsio::{remove_durable, sync_dir, write_atomic};
use super::journal::{Entry, FamilyGuard, Write};
use crate::edit::Op;

/// Current queue file format.
pub const FORMAT: u32 = 1;

/// Who queued an item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    /// `"tui"` or `"op"`.
    pub tool: String,
    pub pid: u32,
    /// Process start (first use of [`origin`]), to tell a recycled pid from the original.
    pub started: DateTime<Utc>,
}

/// The origin of this process for `tool`.
pub fn origin(tool: &str) -> Origin {
    static STARTED: OnceLock<DateTime<Utc>> = OnceLock::new();
    Origin {
        tool: tool.to_string(),
        pid: std::process::id(),
        started: *STARTED.get_or_init(Utc::now),
    }
}

/// An entry planned against the caller's overlaid view; `entry.id` is replaced at commit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prepared {
    pub entry: Entry,
    pub holds: Vec<(String, String)>,
    pub family: Option<FamilyGuard>,
}

/// What a queued item does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
// `Op` carries whole tasks; items are built one at a time, never in bulk.
#[allow(clippy::large_enum_variant)]
pub enum Kind {
    Op {
        op: Op,
        prepared: Prepared,
    },
    /// An undo or redo of journal entry `entry`; `writes` are the swapped writes (for the overlay).
    Revert {
        entry: String,
        writes: Vec<Write>,
    },
}

/// One queued write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub format: u32,
    pub id: String,
    pub created: DateTime<Utc>,
    pub origin: Origin,
    pub label: String,
    #[serde(flatten)]
    pub kind: Kind,
}

impl Item {
    /// The writes this item will commit, for the overlay.
    pub fn writes(&self) -> &[Write] {
        match &self.kind {
            Kind::Op { prepared, .. } => &prepared.entry.writes,
            Kind::Revert { writes, .. } => writes,
        }
    }
}

/// A refused item, kept until the user has been told.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Refusal {
    pub format: u32,
    pub item: Item,
    pub reason: String,
    pub at: DateTime<Utc>,
    pub shown: bool,
}

/// A queue id: `<%Y%m%dT%H%M%S%6fZ>-<seq:08>-<pid:010>`. Fixed widths, so string order is
/// creation order and the id passes the journal's id charset.
pub fn new_id(now: DateTime<Utc>) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!(
        "{}-{:08}-{:010}",
        now.format("%Y%m%dT%H%M%S%6fZ"),
        seq,
        std::process::id()
    )
}

/// The queue directory, `<state>/queue/`.
#[derive(Debug, Clone)]
pub struct Queue {
    dir: PathBuf,
}

fn valid_id(id: &str) -> io::Result<()> {
    if id.is_empty()
        || id.starts_with('.')
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid queue item id: {id:?}"),
        ));
    }
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

impl Queue {
    /// Opens the queue under `state`, creating `queue/` and `queue/refused/`.
    pub fn open(state: &Path) -> io::Result<Queue> {
        let dir = state.join("queue");
        if !dir.join("refused").is_dir() {
            fs::create_dir_all(dir.join("refused"))?;
            sync_dir(&dir)?;
            sync_dir(state)?;
        }
        Ok(Queue { dir })
    }

    /// The queue if its directory exists; creates nothing.
    pub fn existing(state: &Path) -> Option<Queue> {
        let dir = state.join("queue");
        dir.is_dir().then_some(Queue { dir })
    }

    fn json(&self, id: &str) -> io::Result<PathBuf> {
        valid_id(id)?;
        Ok(self.dir.join(format!("{id}.json")))
    }

    fn claimed(&self, id: &str) -> io::Result<PathBuf> {
        valid_id(id)?;
        Ok(self.dir.join(format!("{id}.claim")))
    }

    fn refused(&self) -> PathBuf {
        self.dir.join("refused")
    }

    /// Writes `item` durably.
    pub fn push(&self, item: &Item) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(item).map_err(io::Error::other)?;
        write_atomic(&self.json(&item.id)?, &bytes)
    }

    /// Queued and claimed items, sorted by id. Unreadable files are skipped.
    pub fn list(&self) -> io::Result<Vec<Item>> {
        let mut out = Vec::new();
        for dent in fs::read_dir(&self.dir)? {
            let dent = dent?;
            let name = dent.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.') || !(name.ends_with(".json") || name.ends_with(".claim")) {
                continue;
            }
            if let Some(item) = read_json::<Item>(&dent.path()) {
                out.push(item);
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    /// Number of listed items.
    pub fn len(&self) -> io::Result<usize> {
        Ok(self.list()?.len())
    }

    pub fn is_empty(&self) -> io::Result<bool> {
        Ok(self.len()? == 0)
    }

    /// Takes item `id` for committing. `false` if it is gone (cancelled or claimed away);
    /// `true` if now (or already) claimed.
    pub fn claim(&self, id: &str) -> io::Result<bool> {
        match fs::rename(self.json(id)?, self.claimed(id)?) {
            Ok(()) => {
                sync_dir(&self.dir)?;
                Ok(true)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(self.claimed(id)?.exists()),
            Err(e) => Err(e),
        }
    }

    /// Withdraws an unclaimed item. `false` if it is gone or already claimed.
    pub fn cancel(&self, id: &str) -> io::Result<bool> {
        match fs::remove_file(self.json(id)?) {
            Ok(()) => {
                sync_dir(&self.dir)?;
                Ok(true)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Removes a claimed item once it is committed or dealt with.
    pub fn done(&self, id: &str) -> io::Result<()> {
        remove_durable(&self.claimed(id)?)
    }

    /// Records why `item` was refused, then removes its claim.
    pub fn refuse(&self, item: &Item, reason: &str, at: DateTime<Utc>) -> io::Result<()> {
        valid_id(&item.id)?;
        let r = Refusal {
            format: FORMAT,
            item: item.clone(),
            reason: reason.to_string(),
            at,
            shown: false,
        };
        let bytes = serde_json::to_vec_pretty(&r).map_err(io::Error::other)?;
        write_atomic(&self.refused().join(format!("{}.json", item.id)), &bytes)?;
        self.done(&item.id)
    }

    fn refusals(&self) -> io::Result<Vec<Refusal>> {
        let mut out = Vec::new();
        for dent in fs::read_dir(self.refused())? {
            let dent = dent?;
            let name = dent.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.') || !name.ends_with(".json") {
                continue;
            }
            if let Some(r) = read_json::<Refusal>(&dent.path()) {
                out.push(r);
            }
        }
        out.sort_by(|a, b| a.item.id.cmp(&b.item.id));
        Ok(out)
    }

    /// Refusals the user has not been told about yet, sorted by item id.
    pub fn unshown(&self) -> io::Result<Vec<Refusal>> {
        Ok(self.refusals()?.into_iter().filter(|r| !r.shown).collect())
    }

    /// Marks the refusal of item `id` as shown. A missing refusal is `Ok`.
    pub fn mark_shown(&self, id: &str) -> io::Result<()> {
        valid_id(id)?;
        let path = self.refused().join(format!("{id}.json"));
        let Some(mut r) = read_json::<Refusal>(&path) else {
            return Ok(());
        };
        r.shown = true;
        let bytes = serde_json::to_vec_pretty(&r).map_err(io::Error::other)?;
        write_atomic(&path, &bytes)
    }

    /// Removes refusals recorded before `before`; returns how many.
    pub fn prune(&self, before: DateTime<Utc>) -> io::Result<usize> {
        let mut n = 0;
        for r in self.refusals()? {
            if r.at < before {
                remove_durable(&self.refused().join(format!("{}.json", r.item.id)))?;
                n += 1;
            }
        }
        Ok(n)
    }
}
