//! The operation journal and crash recovery. See `docs/journal.md`.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::fsio::{VdirLock, remove_durable, sync_dir, write_atomic};

/// Current journal entry format.
pub const FORMAT: u32 = 1;

/// Lifecycle stage of a journal entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    /// Recorded before any vdir write; recovery rolls it forward.
    Prepared,
    /// Every write is on disk.
    Applied,
    /// Given up on; recovery ignores it.
    Abandoned,
}

/// One operation: the full before/after content of every file it touches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub format: u32,
    pub id: String,
    pub created: DateTime<Utc>,
    pub kind: String,
    pub uids: Vec<String>,
    pub stage: Stage,
    pub inverse_of: Option<String>,
    /// When a sync confirmed the server holds this entry's result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced: Option<DateTime<Utc>>,
    pub writes: Vec<Write>,
}

/// One file write. `path` is relative to the vdir (`<collection>/<file>.ics`);
/// `None` means the file is absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Write {
    pub path: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// Why an entry could not be applied.
#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    /// The file holds neither the `before` nor the `after` content.
    #[error("conflict: {path} holds neither the expected before nor after content")]
    Conflict { path: String },
    /// The write path is absolute or escapes the vdir.
    #[error("invalid write path: {0}")]
    InvalidPath(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Returns a new operation id, `<YYYYMMDDTHHMMSSZ>-<8 hex>`.
pub fn new_op_id(now: DateTime<Utc>) -> String {
    let rand = uuid::Uuid::new_v4().simple().to_string();
    format!("{}-{}", now.format("%Y%m%dT%H%M%SZ"), &rand[..8])
}

/// The journal directory, `<state>/journal/`, one `<id>.json` per entry.
#[derive(Debug, Clone)]
pub struct Journal {
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
            format!("invalid journal entry id: {id:?}"),
        ));
    }
    Ok(())
}

impl Journal {
    /// Opens the journal under `state_dir`, creating `<state>/journal/` if needed.
    pub fn open(state_dir: &Path) -> io::Result<Journal> {
        let dir = state_dir.join("journal");
        if !dir.is_dir() {
            fs::create_dir_all(&dir)?;
            sync_dir(state_dir)?;
        }
        Ok(Journal { dir })
    }

    fn path(&self, id: &str) -> io::Result<PathBuf> {
        valid_id(id)?;
        Ok(self.dir.join(format!("{id}.json")))
    }

    fn store(&self, e: &Entry) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(e).map_err(io::Error::other)?;
        write_atomic(&self.path(&e.id)?, &bytes)
    }

    /// Records `e` durably. Call before touching the vdir.
    pub fn prepare(&self, e: &Entry) -> io::Result<()> {
        self.store(e)
    }

    /// Rewrites the stage of entry `id`; `NotFound` if there is no such entry.
    pub fn set_stage(&self, id: &str, s: Stage) -> io::Result<()> {
        let mut e = self.get(id)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("no journal entry {id}"))
        })?;
        e.stage = s;
        self.store(&e)
    }

    /// Records that a sync confirmed the server holds entry `id`'s result;
    /// `NotFound` if there is no such entry.
    pub fn mark_synced(&self, id: &str, at: DateTime<Utc>) -> io::Result<()> {
        let mut e = self.get(id)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("no journal entry {id}"))
        })?;
        e.synced = Some(at);
        self.store(&e)
    }

    /// Applied entries not yet confirmed by a sync, sorted by id.
    pub fn pending(&self) -> io::Result<Vec<Entry>> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|e| e.stage == Stage::Applied && e.synced.is_none())
            .collect())
    }

    /// Removes Applied entries confirmed before `before`; returns their ids.
    /// Prepared and Abandoned entries are never pruned.
    pub fn prune(&self, before: DateTime<Utc>) -> io::Result<Vec<String>> {
        let mut gone = Vec::new();
        for e in self.list()? {
            if e.stage == Stage::Applied && e.synced.is_some_and(|t| t < before) {
                remove_durable(&self.path(&e.id)?)?;
                gone.push(e.id);
            }
        }
        Ok(gone)
    }

    /// Reads entry `id`; `None` if absent, `InvalidData` if it does not parse.
    pub fn get(&self, id: &str) -> io::Result<Option<Entry>> {
        match fs::read(self.path(id)?) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Every entry, sorted by id. Fails if any entry does not parse.
    pub fn list(&self) -> io::Result<Vec<Entry>> {
        self.list_raw()?
            .into_iter()
            .map(|(id, r)| {
                r.map_err(|err| io::Error::new(io::ErrorKind::InvalidData, format!("{id}: {err}")))
            })
            .collect()
    }

    /// Every `<id>.json` (temp files skipped), sorted by id, parsed or not.
    fn list_raw(&self) -> io::Result<Vec<(String, Result<Entry, String>)>> {
        let mut out = Vec::new();
        for dent in fs::read_dir(&self.dir)? {
            let dent = dent?;
            let name = dent.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.') {
                continue;
            }
            let Some(id) = name.strip_suffix(".json") else {
                continue;
            };
            let parsed = fs::read(dent.path())
                .map_err(|e| e.to_string())
                .and_then(|b| serde_json::from_slice::<Entry>(&b).map_err(|e| e.to_string()));
            out.push((id.to_string(), parsed));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }
}

/// Resolves a vdir-relative write path, rejecting absolute paths and `..`.
fn resolve(vdir: &Path, rel: &str) -> Result<PathBuf, ApplyError> {
    let p = Path::new(rel);
    let ok = !rel.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)));
    if !ok {
        return Err(ApplyError::InvalidPath(rel.to_string()));
    }
    Ok(vdir.join(p))
}

fn read_opt(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn same(current: &Option<Vec<u8>>, want: &Option<String>) -> bool {
    current.as_deref() == want.as_ref().map(String::as_bytes)
}

/// Applies the writes of `e` to `vdir`, in order. Per write: if the file
/// already equals `after` it is skipped; if it equals `before` it becomes
/// `after` (written atomically, or removed when `after` is `None`); otherwise
/// the entry fails with `Conflict`. Every write is checked before any is
/// made, so a conflict leaves the vdir untouched.
pub fn apply_entry(vdir: &Path, e: &Entry) -> Result<(), ApplyError> {
    // Check pass: simulate the writes in order so repeated paths are handled.
    let mut overlay: HashMap<PathBuf, Option<Vec<u8>>> = HashMap::new();
    let mut todo = Vec::with_capacity(e.writes.len());
    for w in &e.writes {
        let path = resolve(vdir, &w.path)?;
        let current = match overlay.get(&path) {
            Some(c) => c.clone(),
            None => read_opt(&path)?,
        };
        if same(&current, &w.after) {
            todo.push(None);
        } else if same(&current, &w.before) {
            todo.push(Some(path.clone()));
        } else {
            return Err(ApplyError::Conflict {
                path: w.path.clone(),
            });
        }
        overlay.insert(path, w.after.as_ref().map(|s| s.as_bytes().to_vec()));
    }
    for (w, path) in e.writes.iter().zip(todo) {
        let Some(path) = path else { continue };
        match &w.after {
            Some(after) => write_atomic(&path, after.as_bytes())?,
            None => remove_durable(&path)?,
        }
    }
    Ok(())
}

/// Outcome of recovery.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RecoveryReport {
    /// Ids of Prepared entries rolled forward to Applied.
    pub resumed: Vec<String>,
    /// Entries left in place, with the reason (conflict, invalid path, unreadable).
    pub stuck: Vec<(String, String)>,
}

/// Takes the exclusive lock and rolls every Prepared entry forward. A no-op
/// that creates nothing when `<state>/journal` does not exist.
pub fn recover(vdir: &Path, state_dir: &Path) -> io::Result<RecoveryReport> {
    if !state_dir.join("journal").is_dir() {
        return Ok(RecoveryReport::default());
    }
    let _lock = VdirLock::exclusive(state_dir)?;
    let journal = Journal::open(state_dir)?;
    recover_locked(vdir, &journal)
}

/// Like [`recover`], for callers that already hold the exclusive lock.
///
/// Prepared entries are applied in id order; each that applies is marked
/// Applied. A conflicting, invalid or unparseable entry is left as it is and
/// reported as stuck. I/O errors abort recovery.
pub fn recover_locked(vdir: &Path, journal: &Journal) -> io::Result<RecoveryReport> {
    let mut report = RecoveryReport::default();
    for (id, parsed) in journal.list_raw()? {
        let e = match parsed {
            Ok(e) => e,
            Err(err) => {
                report
                    .stuck
                    .push((id, format!("unreadable journal entry: {err}")));
                continue;
            }
        };
        if e.stage != Stage::Prepared {
            continue;
        }
        if e.id != id {
            report.stuck.push((
                id.clone(),
                format!("entry id {:?} does not match its file name", e.id),
            ));
            continue;
        }
        match apply_entry(vdir, &e) {
            Ok(()) => {
                journal.set_stage(&id, Stage::Applied)?;
                report.resumed.push(id);
            }
            Err(ApplyError::Io(err)) => return Err(err),
            Err(err) => report.stuck.push((id, err.to_string())),
        }
    }
    Ok(report)
}
