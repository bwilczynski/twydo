//! The on-disk write queue: `<state>/queue/<id>.json` items waiting for the vdir lock.
//! See the write-queue design (`docs/superpowers/specs/2026-11-06-write-queue-design.md`).

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::fsio::{VdirLock, remove_durable, sync_dir, write_atomic};
use super::journal::{Entry, FamilyGuard, Journal, RequestRef, Write};
use super::{Drained, Store, WriteError, existing_vdir_state};
use crate::compat::Matrix;
use crate::config::Config;
use crate::edit::Op;
use crate::vdir::LoadReport;
use crate::views::Snapshot;

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
    /// Additional membership dependencies, including both sides of relocation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub families: Vec<FamilyGuard>,
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
    /// The client request that queued this item; its drained entry carries it too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<RequestRef>,
    #[serde(flatten)]
    pub kind: Kind,
}

/// Where a queued item stands, as found by id or request.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Queued {
    /// Waiting for the next lock holder.
    Pending(Item),
    /// A drainer took it and has not finished.
    Claimed(Item),
    /// A drain refused it.
    Refused(Refusal),
    /// An undo withdrew it before any drain took it.
    Cancelled(Cancellation),
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
    /// The refusal was [`WriteError::Stale`]: a file changed under the queued edit.
    #[serde(default)]
    pub stale: bool,
}

/// A queued item an undo withdrew, kept so a resent request (the one that queued the item,
/// or the undo that withdrew it) is answered from disk after a restart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cancellation {
    pub format: u32,
    pub item: Item,
    /// The request that withdrew the item, if a client named one.
    pub by: Option<RequestRef>,
    pub at: DateTime<Utc>,
}

/// The line that tells the user a queued edit was not saved.
pub fn refusal_message(r: &Refusal) -> String {
    if r.stale {
        format!("Not saved: \"{}\" changed on another device", r.item.label)
    } else {
        format!("Not saved: \"{}\": {}", r.item.label, r.reason)
    }
}

/// Drains the queue for a caller that holds the vdir lock (the sync runner), with the
/// compatibility matrix `matrix`.
pub fn drain_with_lock(
    cfg: &Config,
    matrix: &Matrix,
    held: &VdirLock,
    now: DateTime<Utc>,
) -> Result<Vec<Drained>, WriteError> {
    Store::without_recovery(cfg, matrix.clone())?.drain_locked(held, now)
}

/// Whether every file `item` was planned against still holds what it read: its writes'
/// `before` and, for an op, its holds. Otherwise the drain will refuse it.
fn base_matches(item: &Item, report: &LoadReport, vdir: &Path) -> bool {
    let holds: &[(String, String)] = match &item.kind {
        Kind::Op { prepared, .. } => &prepared.holds,
        Kind::Revert { .. } => &[],
    };
    let source = |rel: &str| report.sources.get(&vdir.join(rel)).map(Vec::as_slice);
    item.writes()
        .iter()
        .all(|w| source(&w.path) == w.before.as_deref().map(str::as_bytes))
        && holds
            .iter()
            .all(|(rel, bytes)| source(rel) == Some(bytes.as_bytes()))
}

/// The snapshot of `report` with the queued items laid over it, so a queued edit shows as if
/// applied (spec §6.5), and the tasks they touch marked pending. An item is laid over only
/// while every file it was planned against is as it expected: otherwise the drain will refuse
/// it and the row shows the file. Without state or a readable queue this is a plain
/// [`Snapshot::new`]. A reverted entry whose journal record cannot be read is not marked.
pub fn view(mut report: LoadReport, cfg: Config) -> Snapshot {
    let layers = (|| -> Option<(Vec<Item>, Vec<Entry>)> {
        let state = existing_vdir_state(&cfg.state_dir(), &cfg.vdir).ok()??;
        let queue = Queue::existing(&state)?;
        let items: Vec<Item> = queue
            .list()
            .ok()?
            .into_iter()
            .filter(|i| !queue.is_refused(&i.id).unwrap_or(false))
            .collect();
        if items.is_empty() {
            return None;
        }
        // Listed once for all items; an unreadable journal only loses the applied-claim
        // check and the reverted entries' pending marks.
        let entries = if state.join("journal").is_dir() {
            Journal::open(&state)
                .and_then(|j| j.list())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        Some((items, entries))
    })();
    let Some((items, entries)) = layers else {
        return Snapshot::new(report, cfg);
    };
    let drained: HashSet<&str> = entries
        .iter()
        .filter_map(|e| e.from_queue.as_ref().map(|q| q.id.as_str()))
        .collect();
    let mut pending = HashSet::new();
    for item in items {
        // Applied but not yet removed (a drain is between its commit and `done`).
        if drained.contains(item.id.as_str()) {
            continue;
        }
        if !base_matches(&item, &report, &cfg.vdir) {
            continue;
        }
        report.overlay_writes(&cfg.vdir, item.writes());
        match &item.kind {
            Kind::Op { prepared, .. } => pending.extend(prepared.entry.uids.iter().cloned()),
            Kind::Revert { entry, .. } => {
                if let Some(e) = entries.iter().find(|e| e.id == *entry) {
                    pending.extend(e.uids.iter().cloned());
                }
            }
        }
    }
    Snapshot::new(report, cfg).with_pending(pending)
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

/// The item file at `path`: `None` if it does not exist, an `InvalidData` error naming the
/// path if it exists and does not parse.
fn read_item(path: &Path) -> io::Result<Option<Item>> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unreadable queue item {}: {e}", path.display()),
        )
    })
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

    fn cancelled(&self) -> PathBuf {
        self.dir.join("cancelled")
    }

    /// Writes `item` durably. The collection fence is not checked here: an edit saved while
    /// a list operation runs is queued, and the drain refuses it if the operation is left
    /// unfinished ([`crate::lists::operation::ensure_idle`]). The coordination lock still
    /// orders the push against a list operation checking the queue as it begins.
    pub fn push(&self, item: &Item) -> io::Result<()> {
        let state = self.dir.parent().expect("queue state directory");
        let _coord = crate::lists::operation::coordinate(state)?;
        let bytes = serde_json::to_vec_pretty(item).map_err(io::Error::other)?;
        write_atomic(&self.json(&item.id)?, &bytes)
    }

    /// Queued and claimed items, sorted by id. Unreadable files are skipped.
    pub fn list(&self) -> io::Result<Vec<Item>> {
        self.list_impl(false)
    }

    /// Collection mutation cannot ignore an unreadable task edit whose target is unknown.
    pub(crate) fn list_strict(&self) -> io::Result<Vec<Item>> {
        self.list_impl(true)
    }

    fn list_impl(&self, strict: bool) -> io::Result<Vec<Item>> {
        let mut out = Vec::new();
        for dent in fs::read_dir(&self.dir)? {
            let dent = dent?;
            let name = dent.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.') || !(name.ends_with(".json") || name.ends_with(".claim")) {
                continue;
            }
            if let Some(item) = read_json::<Item>(&dent.path()) {
                if strict && item.format != FORMAT {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unsupported queued task operation; reconcile it before managing lists",
                    ));
                }
                out.push(item);
            } else if strict {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unreadable queued task operation; reconcile it before managing lists",
                ));
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    /// The pending and claimed item files that do not parse. [`Queue::list`] skips them and
    /// [`Queue::state`] answers an error for their id, so the caller can report them.
    pub fn unreadable(&self) -> io::Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        for dent in fs::read_dir(&self.dir)? {
            let dent = dent?;
            let name = dent.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.') || !(name.ends_with(".json") || name.ends_with(".claim")) {
                continue;
            }
            if read_item(&dent.path()).is_err() {
                out.push(dent.path());
            }
        }
        out.sort();
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

    /// Withdraws unclaimed `item` on behalf of request `by`, recording the withdrawal first
    /// (`cancelled/<id>.json`) so it outlives a restart. `false` if the item is gone or
    /// already claimed; the record is then removed again.
    ///
    /// A crash between the record and the removal leaves both: the item still drains, and
    /// [`Queue::state`] reports it pending, so the record alone never means "cancelled".
    pub fn cancel_by(
        &self,
        item: &Item,
        by: Option<RequestRef>,
        at: DateTime<Utc>,
    ) -> io::Result<bool> {
        let pending = self.json(&item.id)?;
        if !pending.exists() {
            return Ok(false);
        }
        let record = self.cancelled().join(format!("{}.json", item.id));
        if !self.cancelled().is_dir() {
            fs::create_dir_all(self.cancelled())?;
            sync_dir(&self.dir)?;
        }
        let c = Cancellation {
            format: FORMAT,
            item: item.clone(),
            by,
            at,
        };
        let bytes = serde_json::to_vec_pretty(&c).map_err(io::Error::other)?;
        write_atomic(&record, &bytes)?;
        if self.cancel(&item.id)? {
            return Ok(true);
        }
        remove_durable(&record)?;
        Ok(false)
    }

    /// The recorded withdrawal of item `id` ([`Queue::state`] checks the item is gone first).
    fn cancellation(&self, id: &str) -> Option<Cancellation> {
        read_json::<Cancellation>(&self.cancelled().join(format!("{id}.json")))
    }

    fn cancellations(&self) -> io::Result<Vec<Cancellation>> {
        let mut out = Vec::new();
        let dir = match fs::read_dir(self.cancelled()) {
            Ok(d) => d,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e),
        };
        for dent in dir {
            let dent = dent?;
            let name = dent.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.') || !name.ends_with(".json") {
                continue;
            }
            if let Some(c) = read_json::<Cancellation>(&dent.path()) {
                out.push(c);
            }
        }
        out.sort_by(|a, b| a.item.id.cmp(&b.item.id));
        Ok(out)
    }

    /// The withdrawal recorded for undo request `request_id`, wherever its item stands now
    /// (check [`Queue::state`] of the item: only `Cancelled` means the undo took effect).
    pub fn cancelled_by(&self, request_id: &str) -> io::Result<Option<Cancellation>> {
        Ok(self
            .cancellations()?
            .into_iter()
            .find(|c| c.by.as_ref().is_some_and(|r| r.request_id == request_id)))
    }

    /// Removes a claimed item once it is committed or dealt with.
    pub fn done(&self, id: &str) -> io::Result<()> {
        remove_durable(&self.claimed(id)?)
    }

    /// Where item `id` stands: pending, claimed, refused or cancelled (checked in that
    /// order), else `None` (never queued, or committed and gone). An error of kind
    /// `InvalidData` if the item's own file does not parse: it is not "gone".
    pub fn state(&self, id: &str) -> io::Result<Option<Queued>> {
        if let Some(i) = read_item(&self.json(id)?)? {
            return Ok(Some(Queued::Pending(i)));
        }
        if let Some(i) = read_item(&self.claimed(id)?)? {
            return Ok(Some(Queued::Claimed(i)));
        }
        valid_id(id)?;
        if let Some(r) = read_json::<Refusal>(&self.refused().join(format!("{id}.json"))) {
            return Ok(Some(Queued::Refused(r)));
        }
        Ok(self.cancellation(id).map(Queued::Cancelled))
    }

    /// The item queued by client request `request_id`, wherever it stands now. Request ids
    /// are unique by the daemon's contract; the first match is returned.
    pub fn by_request(&self, request_id: &str) -> io::Result<Option<Queued>> {
        let mut names = Vec::new();
        for dent in fs::read_dir(&self.dir)? {
            if let Some(name) = dent?.file_name().to_str() {
                names.push(name.to_string());
            }
        }
        self.scan_request(&names, request_id)
    }

    /// [`Queue::by_request`] over the listed file `names`. A listed pending file a drainer
    /// claimed before it was read is looked up again by id, so a live item is never missed.
    fn scan_request(&self, names: &[String], request_id: &str) -> io::Result<Option<Queued>> {
        let is = |r: &Option<RequestRef>| r.as_ref().is_some_and(|r| r.request_id == request_id);
        for name in names {
            let pending = name.ends_with(".json");
            if name.starts_with('.') || !(pending || name.ends_with(".claim")) {
                continue;
            }
            let path = self.dir.join(name);
            match read_json::<Item>(&path) {
                Some(item) if is(&item.request) => {
                    return Ok(Some(if pending {
                        Queued::Pending(item)
                    } else {
                        Queued::Claimed(item)
                    }));
                }
                Some(_) => {}
                None if pending && !path.exists() => {
                    let id = name.trim_end_matches(".json");
                    // An item that cannot be read cannot be the one asked for.
                    let found = match self.state(id) {
                        Err(e) if e.kind() == io::ErrorKind::InvalidData => None,
                        other => other?,
                    };
                    let hit = match &found {
                        Some(Queued::Pending(i) | Queued::Claimed(i)) => is(&i.request),
                        Some(Queued::Refused(r)) => is(&r.item.request),
                        Some(Queued::Cancelled(c)) => is(&c.item.request),
                        None => false,
                    };
                    if hit {
                        return Ok(found);
                    }
                }
                None => {}
            }
        }
        if let Some(r) = self.refusals()?.into_iter().find(|r| is(&r.item.request)) {
            return Ok(Some(Queued::Refused(r)));
        }
        // A record whose item is still listed is a cancel cut short: the item was found above.
        Ok(self
            .cancellations()?
            .into_iter()
            .find(|c| is(&c.item.request))
            .map(Queued::Cancelled))
    }

    /// Whether a refusal of item `id` is recorded.
    pub fn is_refused(&self, id: &str) -> io::Result<bool> {
        valid_id(id)?;
        Ok(self.refused().join(format!("{id}.json")).exists())
    }

    /// Records why `item` was refused, then removes its claim.
    pub fn refuse(
        &self,
        item: &Item,
        reason: &str,
        stale: bool,
        at: DateTime<Utc>,
    ) -> io::Result<()> {
        valid_id(&item.id)?;
        let r = Refusal {
            format: FORMAT,
            item: item.clone(),
            reason: reason.to_string(),
            at,
            shown: false,
            stale,
        };
        let bytes = serde_json::to_vec_pretty(&r).map_err(io::Error::other)?;
        write_atomic(&self.refused().join(format!("{}.json", item.id)), &bytes)?;
        self.done(&item.id)
    }

    fn refusals(&self) -> io::Result<Vec<Refusal>> {
        let mut out = Vec::new();
        let dir = match fs::read_dir(self.refused()) {
            Ok(d) => d,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e),
        };
        for dent in dir {
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

    /// Removes refusals and cancellations recorded before `before`; returns how many.
    pub fn prune(&self, before: DateTime<Utc>) -> io::Result<usize> {
        let mut n = 0;
        for r in self.refusals()? {
            if r.at < before {
                remove_durable(&self.refused().join(format!("{}.json", r.item.id)))?;
                n += 1;
            }
        }
        for c in self.cancellations()? {
            if c.at < before {
                remove_durable(&self.cancelled().join(format!("{}.json", c.item.id)))?;
                n += 1;
            }
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn a_pending_file_claimed_after_listing_is_still_found() {
        let tmp = tempfile::tempdir().unwrap();
        let q = Queue::open(tmp.path()).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 11, 4, 15, 0, 0).unwrap();
        let item = Item {
            format: FORMAT,
            id: new_id(now),
            created: now,
            origin: origin("op"),
            label: "x".into(),
            request: Some(RequestRef {
                session: "s".into(),
                request_id: "r".into(),
            }),
            kind: Kind::Revert {
                entry: "e".into(),
                writes: Vec::new(),
            },
        };
        q.push(&item).unwrap();
        // The listing saw `<id>.json`; a drainer claims it before the read.
        let listed = vec![format!("{}.json", item.id)];
        assert!(q.claim(&item.id).unwrap());
        assert!(matches!(
            q.scan_request(&listed, "r").unwrap(),
            Some(Queued::Claimed(i)) if i.id == item.id
        ));
        assert!(q.scan_request(&listed, "other").unwrap().is_none());
    }
}
