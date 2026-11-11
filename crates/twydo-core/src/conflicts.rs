//! The conflict store: both versions of a conflicted task, kept outside the vdir.
//!
//! Layout under the per-vdir state directory `<state>`:
//!
//! ```text
//! conflicts/.lock                          conflict store lock
//! conflicts/<id>/conflict.json             pretty JSON record
//! conflicts/<id>/local.ics, remote.ics     the two versions (a missing file = that side deleted it)
//! ```
//!
//! The version files are written first and `conflict.json` last, so a record
//! is visible to readers only once complete. Writers hold `conflicts/.lock`
//! (the last lock in the project-wide order run lock, vdir lock, conflicts
//! lock); readers take no lock and read only whole files written atomically.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::store::fsio::{remove_durable, sync_dir, write_atomic};
use crate::store::journal::{RequestRef, new_op_id};
use crate::sync::resolve::Resolution;

const DIR: &str = "conflicts";
const LOCK: &str = ".lock";
const RECORD: &str = "conflict.json";
const LOCAL: &str = "local.ics";
const REMOTE: &str = "remote.ics";

/// Which copy of a task.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Local,
    Remote,
}

/// What kind of disagreement was recorded.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictKind {
    BothChanged,
    LocalDeleteRemoteEdit,
    RemoteDeleteLocalEdit,
    Journal,
}

impl ConflictKind {
    /// The kind in words, for people.
    pub fn words(self) -> &'static str {
        match self {
            ConflictKind::BothChanged => "changed here and on another device",
            ConflictKind::LocalDeleteRemoteEdit => "deleted here, edited on another device",
            ConflictKind::RemoteDeleteLocalEdit => "edited here, deleted on another device",
            ConflictKind::Journal => "an interrupted change here meets another device's version",
        }
    }
}

/// Where a conflict is in its life.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConflictState {
    Open,
    Chosen,
    Resolved,
    Superseded,
}

/// A conflict to record. `None` for a version means that side deleted the task.
pub struct NewConflict {
    pub uid: String,
    pub collection: String,
    /// Vdir-relative path.
    pub path: String,
    pub kind: ConflictKind,
    /// True when vdirsyncer still keeps the two sides apart.
    pub held: bool,
    pub vdir_holds: Side,
    pub local: Option<String>,
    pub remote: Option<String>,
}

/// The request that resolved a conflict and what it did, so a resend after a restart
/// replays the same answer (the daemon's idempotency record).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ResolvedBy {
    pub request: RequestRef,
    pub resolution: Resolution,
    /// The journal entry a `Written` resolution made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op_id: Option<String>,
}

/// A recorded conflict. The versions are loaded by `get` and `list`, not stored in the JSON.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Conflict {
    pub id: String,
    pub uid: String,
    pub collection: String,
    pub path: String,
    pub kind: ConflictKind,
    pub held: bool,
    pub vdir_holds: Side,
    pub detected: DateTime<Utc>,
    pub state: ConflictState,
    pub choice: Option<Side>,
    pub resolved: Option<DateTime<Utc>>,
    /// The request that resolved or chose last, when one did (older records have none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_by: Option<ResolvedBy>,
    /// The requests that chose before [`Conflict::resolved_by`], oldest first: a held conflict
    /// can be chosen again, and each request keeps its record so its resend replays.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub earlier: Vec<ResolvedBy>,
    #[serde(skip)]
    pub local: Option<String>,
    #[serde(skip)]
    pub remote: Option<String>,
}

impl Conflict {
    /// Makes `by` the latest request record, keeping the one it replaces in `earlier`.
    fn record_request(&mut self, by: Option<ResolvedBy>) {
        let Some(by) = by else { return };
        if let Some(prev) = self.resolved_by.replace(by) {
            self.earlier.push(prev);
        }
    }

    /// The first VTODO property `name` of an `.ics` text, as written.
    pub fn field(text: &str, name: &str) -> Option<String> {
        let cal = crate::ical::parse(text.as_bytes()).ok()?;
        let todo = cal.root.children_named("VTODO").next()?;
        todo.prop(name).map(|p| p.value.clone())
    }

    /// The task's `SUMMARY` from either version (local first).
    pub fn title_opt(&self) -> Option<String> {
        [&self.local, &self.remote]
            .into_iter()
            .flatten()
            .find_map(|t| Self::field(t, "SUMMARY"))
    }

    /// The title from either version (local first), else the UID.
    pub fn title(&self) -> String {
        self.title_opt().unwrap_or_else(|| self.uid.clone())
    }

    fn unresolved(&self) -> bool {
        matches!(self.state, ConflictState::Open | ConflictState::Chosen)
    }
}

/// Exclusive advisory lock on `conflicts/.lock`, released on drop.
struct StoreLock(File);

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// The conflicts of one vdir.
#[derive(Debug, Clone)]
pub struct ConflictStore {
    dir: PathBuf,
}

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg)
}

fn read_opt(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

impl ConflictStore {
    /// Opens the store under `state`, creating `conflicts/`.
    pub fn open(state: &Path) -> io::Result<ConflictStore> {
        let dir = state.join(DIR);
        fs::create_dir_all(&dir)?;
        Ok(ConflictStore { dir })
    }

    /// Opens the store if `conflicts/` exists; creates nothing.
    pub fn existing(state: &Path) -> io::Result<Option<ConflictStore>> {
        let dir = state.join(DIR);
        Ok(dir.is_dir().then_some(ConflictStore { dir }))
    }

    fn lock(&self) -> io::Result<StoreLock> {
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.dir.join(LOCK))?;
        f.lock()?;
        Ok(StoreLock(f))
    }

    fn entry_dir(&self, id: &str) -> Option<PathBuf> {
        let ok = !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        ok.then(|| self.dir.join(id))
    }

    fn load(&self, id: &str) -> io::Result<Option<Conflict>> {
        let Some(dir) = self.entry_dir(id) else {
            return Ok(None);
        };
        let Some(text) = read_opt(&dir.join(RECORD))? else {
            return Ok(None);
        };
        let mut c: Conflict = serde_json::from_str(&text).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {e}", dir.join(RECORD).display()),
            )
        })?;
        c.local = read_opt(&dir.join(LOCAL))?;
        c.remote = read_opt(&dir.join(REMOTE))?;
        Ok(Some(c))
    }

    fn save_record(&self, c: &Conflict) -> io::Result<()> {
        let dir = self.dir.join(&c.id);
        let mut json = serde_json::to_string_pretty(c).map_err(io::Error::other)?;
        json.push('\n');
        write_atomic(&dir.join(RECORD), json.as_bytes())
    }

    /// Records a conflict. Returns an existing unresolved conflict for the same
    /// path with the same kind and bytes; otherwise supersedes any other
    /// unresolved conflict for that path and writes a new one.
    pub fn record(&self, c: NewConflict, now: DateTime<Utc>) -> io::Result<Conflict> {
        let _lock = self.lock()?;
        let older: Vec<Conflict> = self
            .list()?
            .into_iter()
            .filter(|o| o.path == c.path && o.unresolved())
            .collect();
        let keep = older
            .iter()
            .rev()
            .find(|o| o.kind == c.kind && o.local == c.local && o.remote == c.remote)
            .map(|o| o.id.clone());
        // The new conflict is durable before any older one is superseded, so a
        // crash never loses a conflict (it can leave two unresolved ones; the
        // next `record` supersedes the older).
        let rec = match keep {
            Some(id) => older.iter().find(|o| o.id == id).cloned().expect("kept"),
            None => self.write_new(c, now)?,
        };
        for mut old in older {
            if old.id != rec.id {
                old.state = ConflictState::Superseded;
                old.resolved = Some(now);
                self.save_record(&old)?;
            }
        }
        Ok(rec)
    }

    fn write_new(&self, c: NewConflict, now: DateTime<Utc>) -> io::Result<Conflict> {
        let id = new_op_id(now);
        let dir = self.dir.join(&id);
        fs::create_dir(&dir)?;
        sync_dir(&self.dir)?;
        if let Some(l) = &c.local {
            write_atomic(&dir.join(LOCAL), l.as_bytes())?;
        }
        if let Some(r) = &c.remote {
            write_atomic(&dir.join(REMOTE), r.as_bytes())?;
        }
        let rec = Conflict {
            id,
            uid: c.uid,
            collection: c.collection,
            path: c.path,
            kind: c.kind,
            held: c.held,
            vdir_holds: c.vdir_holds,
            detected: now,
            state: ConflictState::Open,
            choice: None,
            resolved: None,
            resolved_by: None,
            earlier: Vec::new(),
            local: c.local,
            remote: c.remote,
        };
        self.save_record(&rec)?;
        Ok(rec)
    }

    /// One conflict with its versions, or `None`.
    pub fn get(&self, id: &str) -> io::Result<Option<Conflict>> {
        self.load(id)
    }

    /// All conflicts in id order, versions loaded.
    pub fn list(&self) -> io::Result<Vec<Conflict>> {
        let mut ids = Vec::new();
        for e in fs::read_dir(&self.dir)? {
            let e = e?;
            if e.file_type()?.is_dir()
                && let Some(name) = e.file_name().to_str()
            {
                ids.push(name.to_owned());
            }
        }
        ids.sort();
        let mut out = Vec::new();
        for id in ids {
            if let Some(c) = self.load(&id)? {
                out.push(c);
            }
        }
        Ok(out)
    }

    /// The conflict a request resolved or chose, and that request's record, by its
    /// `request_id`. Every request that chose counts, not only the last one.
    pub fn by_request(&self, request_id: &str) -> io::Result<Option<(Conflict, ResolvedBy)>> {
        Ok(self.list()?.into_iter().find_map(|c| {
            let by = c
                .resolved_by
                .iter()
                .chain(&c.earlier)
                .find(|r| r.request.request_id == request_id)
                .cloned()?;
            Some((c, by))
        }))
    }

    /// Conflicts that are Open or Chosen.
    pub fn unresolved(&self) -> io::Result<Vec<Conflict>> {
        Ok(self
            .list()?
            .into_iter()
            .filter(Conflict::unresolved)
            .collect())
    }

    fn load_for_update(&self, id: &str) -> io::Result<Conflict> {
        self.load(id)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no conflict {id}")))
    }

    /// Records the user's choice: Open becomes Chosen, and a Chosen conflict takes the new
    /// side (a change of mind before the next sync). Resolved and Superseded ones are errors.
    pub fn choose(&self, id: &str, side: Side) -> io::Result<()> {
        self.choose_by(id, side, None)
    }

    /// [`ConflictStore::choose`], also recording the request that chose.
    pub fn choose_by(&self, id: &str, side: Side, by: Option<ResolvedBy>) -> io::Result<()> {
        let _lock = self.lock()?;
        let mut c = self.load_for_update(id)?;
        if !c.unresolved() {
            return Err(invalid(format!(
                "conflict {id} is {:?}, not unresolved",
                c.state
            )));
        }
        c.state = ConflictState::Chosen;
        c.choice = Some(side);
        c.record_request(by);
        self.save_record(&c)
    }

    /// Marks a conflict Resolved (idempotent). A Superseded one is an error.
    pub fn mark_resolved(&self, id: &str, now: DateTime<Utc>) -> io::Result<()> {
        self.mark_resolved_by(id, now, None)
    }

    /// [`ConflictStore::mark_resolved`], also recording the request that resolved. A conflict
    /// already Resolved keeps its record.
    pub fn mark_resolved_by(
        &self,
        id: &str,
        now: DateTime<Utc>,
        by: Option<ResolvedBy>,
    ) -> io::Result<()> {
        let _lock = self.lock()?;
        let mut c = self.load_for_update(id)?;
        match c.state {
            ConflictState::Resolved => return Ok(()),
            ConflictState::Superseded => {
                return Err(invalid(format!("conflict {id} was superseded")));
            }
            _ => {}
        }
        c.state = ConflictState::Resolved;
        c.resolved = Some(now);
        c.record_request(by);
        self.save_record(&c)
    }

    /// Deletes Resolved and Superseded conflicts resolved before `before`.
    /// Returns the removed ids.
    pub fn prune(&self, before: DateTime<Utc>) -> io::Result<Vec<String>> {
        let _lock = self.lock()?;
        let mut removed = Vec::new();
        for c in self.list()? {
            let done = matches!(c.state, ConflictState::Resolved | ConflictState::Superseded);
            if done && c.resolved.is_some_and(|r| r < before) {
                let dir = self.dir.join(&c.id);
                // The record goes first, so a crash leaves no half-visible entry.
                remove_durable(&dir.join(RECORD))?;
                remove_durable(&dir.join(LOCAL))?;
                remove_durable(&dir.join(REMOTE))?;
                fs::remove_dir(&dir)?;
                sync_dir(&self.dir)?;
                removed.push(c.id);
            }
        }
        Ok(removed)
    }
}
