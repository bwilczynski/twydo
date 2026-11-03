//! Crash-safe storage: atomic file writes, the vdir lock, the operation journal with
//! recovery (see `docs/architecture.md`), and the [`Store`] that applies operations through them.
//!
//! An operation runs only when the gate allows it and every file it touches still holds
//! the bytes the caller loaded. It is recorded in the journal before the vdir is touched,
//! so a crash leaves it to be rolled forward by recovery, never half applied.

mod family;
pub mod fsio;
pub mod journal;

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use crate::compat::{Matrix, Verdict};
use crate::config::{Config, WriteMode};
use crate::edit::{
    Change, EditError, Op, apply_changes, complete_one_time, complete_recurring, file_name,
    moved_copy, new_task, new_uid, reopen_one_time,
};
use crate::gate::{self, Blocked};
use crate::ical::{self, Calendar};
use crate::views::Snapshot;
use fsio::VdirLock;
use journal::{
    ApplyError, Entry, FORMAT, Journal, Stage, Write, apply_entry, new_op_id, recover_locked,
    try_recover,
};

pub use journal::RecoveryReport;

/// How often a write with a lock wait retries the vdir lock.
const LOCK_RETRY: Duration = Duration::from_millis(25);

/// The edge case that must pass before a repeating completion can be undone in verified mode.
const UNDO_REPEAT_EDGE: &str = "reopen-undo-before-after-sync";

#[derive(thiserror::Error, Debug)]
pub enum WriteError {
    #[error("{0}")]
    Blocked(Blocked),
    #[error("{0}")]
    Edit(#[from] EditError),
    #[error("{} changed on disk; reload and try again", .0.display())]
    Stale(PathBuf),
    #[error("journal has unfinished operations: {0}")]
    Stuck(String),
    #[error("nothing to undo")]
    NothingToUndo,
    #[error("nothing to redo")]
    NothingToRedo,
    /// The vdir lock is held (a sync is running) and the store was told not to wait.
    #[error("a sync is running; try again in a moment")]
    Busy,
    #[error(transparent)]
    Io(#[from] io::Error),
}

fn io_error(kind: io::ErrorKind, msg: impl Into<String>) -> WriteError {
    WriteError::Io(io::Error::new(kind, msg.into()))
}

/// Name of the identity file in a per-vdir state directory: the canonical vdir path.
pub const VDIR_ID_FILE: &str = "vdir";

/// FNV-1a 64 of `bytes`.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn canonical_vdir(vdir: &Path) -> io::Result<PathBuf> {
    fs::canonicalize(vdir).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("cannot resolve vdir {}: {e}", vdir.display()),
        )
    })
}

fn state_dir_for(state_dir: &Path, canonical: &Path) -> PathBuf {
    let id = fnv1a64(canonical.as_os_str().as_encoded_bytes());
    state_dir.join("vdirs").join(format!("{id:016x}"))
}

fn identity_bytes(canonical: &Path) -> Vec<u8> {
    let mut b = canonical.as_os_str().as_encoded_bytes().to_vec();
    b.push(b'\n');
    b
}

/// Checks the identity file in `dir` names `canonical`. A missing file is accepted.
/// Returns whether the file exists.
fn check_identity(dir: &Path, canonical: &Path) -> io::Result<bool> {
    let found = match fs::read(dir.join(VDIR_ID_FILE)) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    if found == identity_bytes(canonical) {
        return Ok(true);
    }
    let named = String::from_utf8_lossy(&found);
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "state dir {} belongs to vdir {}, not {}; refusing to use its journal",
            dir.display(),
            named.trim_end_matches('\n'),
            canonical.display()
        ),
    ))
}

/// The state directory for `vdir` under `state_dir`: `<state_dir>/vdirs/<id>`, where `<id>`
/// is the 16-hex-digit FNV-1a 64 hash of the canonical vdir path. The journal and the vdir
/// lock live there, so each vdir has its own. Creates nothing; fails if `vdir` cannot be
/// canonicalized.
pub fn vdir_state_dir(state_dir: &Path, vdir: &Path) -> io::Result<PathBuf> {
    Ok(state_dir_for(state_dir, &canonical_vdir(vdir)?))
}

/// For writers: [`vdir_state_dir`], created, with its identity file written if missing.
/// Fails with `InvalidData` if the identity file names another vdir (a hash collision or
/// manual tampering): its journal must not be replayed into this one.
pub fn bind_vdir_state(state_dir: &Path, vdir: &Path) -> io::Result<PathBuf> {
    let canonical = canonical_vdir(vdir)?;
    let dir = state_dir_for(state_dir, &canonical);
    if !check_identity(&dir, &canonical)? {
        fs::create_dir_all(&dir)?;
        fsio::write_atomic(&dir.join(VDIR_ID_FILE), &identity_bytes(&canonical))?;
    }
    Ok(dir)
}

/// For readers: [`vdir_state_dir`] if it exists, else `None`. A vdir that does not exist has
/// no state either (`None`), so the caller reports the unreadable vdir itself. Creates
/// nothing. Fails with `InvalidData` if its identity file names another vdir.
pub fn existing_vdir_state(state_dir: &Path, vdir: &Path) -> io::Result<Option<PathBuf>> {
    let canonical = match canonical_vdir(vdir) {
        Ok(c) => c,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let dir = state_dir_for(state_dir, &canonical);
    if !dir.is_dir() {
        return Ok(None);
    }
    check_identity(&dir, &canonical)?;
    Ok(Some(dir))
}

/// Writes operations to the vdir through the journal, with a session undo/redo stack.
#[derive(Debug)]
pub struct Store {
    vdir: PathBuf,
    state: PathBuf,
    mode: WriteMode,
    matrix: Matrix,
    journal: Journal,
    /// Entries applied (or redone) this session, oldest first; `u` reverts the last.
    done: Vec<String>,
    /// Undo entries of this session; `Ctrl-r` reverts the last. A new apply clears it.
    undone: Vec<String>,
    /// Refuse with [`WriteError::Busy`] instead of waiting for the vdir lock.
    lock_wait: Option<Duration>,
}

/// The parts of a journal entry an operation produces.
struct Plan {
    kind: &'static str,
    uids: Vec<String>,
    writes: Vec<Write>,
    /// Loaded files the op reads but does not write (`rel`, loaded bytes): a family member
    /// left as it is. Each must still hold its bytes when the entry is applied.
    holds: Vec<(String, String)>,
}

/// The loaded resource an operation targets.
struct Target {
    collection: String,
    rel: String,
    source: String,
    cal: Calendar,
    repeats: bool,
}

impl Target {
    fn update(self, cal: &Calendar) -> Result<Write, WriteError> {
        Ok(Write {
            path: self.rel,
            before: Some(self.source),
            after: Some(text(cal)?),
        })
    }

    fn remove(self) -> Write {
        Write {
            path: self.rel,
            before: Some(self.source),
            after: None,
        }
    }

    /// The file is left as it is, but must still hold the loaded bytes.
    fn hold(self) -> (String, String) {
        (self.rel, self.source)
    }
}

fn text(cal: &Calendar) -> Result<String, WriteError> {
    String::from_utf8(ical::serialize(cal))
        .map_err(|_| io_error(io::ErrorKind::InvalidData, "resource is not UTF-8"))
}

fn create(collection: &str, uid: &str, cal: &Calendar) -> Result<Write, WriteError> {
    Ok(Write {
        path: format!("{collection}/{}", file_name(uid)),
        before: None,
        after: Some(text(cal)?),
    })
}

/// A completion of a repeating task or project: history copies are created and the live
/// task advanced.
fn is_repeat_completion(e: &Entry) -> bool {
    e.kind == "complete" && e.writes.iter().any(|w| w.before.is_none())
}

fn no_entry(id: &str) -> WriteError {
    io_error(io::ErrorKind::NotFound, format!("no journal entry {id}"))
}

/// The JSON `uid` in the metadata of the task with iCalendar UID `parent`, if it has one.
/// A new item's `parentuid` is this value, else the parent's UID.
pub fn parent_json_uid(snap: &Snapshot, parent: &str) -> Option<String> {
    let raw = snap.tasks[snap.find(parent)?].meta.raw.as_deref()?;
    let meta = crate::model::meta_edit::MetaJson::parse(raw)?;
    meta.get_str("uid").map(str::to_string)
}

impl Store {
    /// [`Store::open_with`] the embedded compatibility matrix.
    pub fn open(cfg: &Config) -> Result<(Store, RecoveryReport), WriteError> {
        Store::open_with(cfg, Matrix::embedded().clone())
    }

    /// Runs recovery, then opens the journal. While any entry is stuck, writes return
    /// [`WriteError::Stuck`] until it is [abandoned](Store::abandon).
    pub fn open_with(cfg: &Config, matrix: Matrix) -> Result<(Store, RecoveryReport), WriteError> {
        let vdir = cfg.vdir.clone();
        let state = bind_vdir_state(&cfg.state_dir(), &vdir)?;
        // Recovery takes the lock itself; the store holds none between operations (R2). A sync
        // holding the vdir defers it to the first write, which recovers under the lock anyway.
        let report = try_recover(&vdir, &state)?.unwrap_or_default();
        let journal = Journal::open(&state)?;
        let store = Store {
            vdir,
            state,
            mode: cfg.writes.mode,
            matrix,
            journal,
            done: Vec::new(),
            undone: Vec::new(),
            lock_wait: None,
        };
        Ok((store, report))
    }

    /// How long a write waits for the vdir lock. `None` (the default) blocks until it is free
    /// (the CLI). `Some(d)` retries every 25 ms for up to `d`, then returns
    /// [`WriteError::Busy`] (`Some(ZERO)`: no wait). For the TUI, which must not freeze for
    /// the length of a sync.
    pub fn set_lock_wait(&mut self, wait: Option<Duration>) {
        self.lock_wait = wait;
    }

    fn lock(&self) -> Result<VdirLock, WriteError> {
        let Some(wait) = self.lock_wait else {
            return Ok(VdirLock::exclusive(&self.state)?);
        };
        let deadline = Instant::now() + wait;
        loop {
            if let Some(lock) = VdirLock::try_exclusive(&self.state)? {
                return Ok(lock);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(WriteError::Busy);
            }
            std::thread::sleep(left.min(LOCK_RETRY));
        }
    }

    /// The per-vdir state directory holding this store's journal and lock.
    pub fn state_dir(&self) -> &Path {
        &self.state
    }

    /// Apply `op` to the tasks loaded in `snap`. Refused unless the gate allows it and every
    /// file it touches still holds the loaded bytes (a new file must not exist yet).
    /// Returns the journal entry id.
    pub fn apply(
        &mut self,
        snap: &Snapshot,
        op: &Op,
        now: DateTime<Utc>,
    ) -> Result<String, WriteError> {
        let today = now.with_timezone(&snap.tz()).date_naive();
        gate::check(snap, &self.matrix, self.mode, op, today).map_err(WriteError::Blocked)?;
        let plan = self.plan(snap, op, now)?;
        let holds = plan.holds;
        let family = self.family_of(snap, op)?;
        let entry = Entry {
            format: FORMAT,
            id: new_op_id(now),
            created: now,
            kind: plan.kind.to_string(),
            uids: plan.uids,
            stage: Stage::Prepared,
            inverse_of: None,
            synced: None,
            writes: plan.writes,
        };

        let _lock = self.lock()?;
        self.check_not_stuck()?;
        for w in &entry.writes {
            self.check_holds(&w.path, w.before.as_deref())?;
        }
        for (rel, source) in &holds {
            self.check_holds(rel, Some(source))?;
        }
        if let Some(f) = &family {
            self.check_family(f)?;
        }
        let id = self.run(entry)?;
        self.done.push(id.clone());
        self.undone.clear();
        Ok(id)
    }

    /// Writes one file through the journal (kind "resolve") without the gate: it restores a
    /// version that 2Do or Twydo already wrote. `path` is vdir-relative (`<collection>/<file>`).
    /// Refused with [`WriteError::Stale`] unless the file holds `expect` (`None`: must not
    /// exist). `content` `None` deletes the file. Returns the journal entry id.
    pub fn restore(
        &mut self,
        path: &str,
        expect: Option<&str>,
        content: Option<&str>,
        uids: Vec<String>,
        now: DateTime<Utc>,
    ) -> Result<String, WriteError> {
        let plain = Path::new(path).components().count() == 2
            && Path::new(path)
                .components()
                .all(|c| matches!(c, Component::Normal(_)));
        if !plain {
            return Err(io_error(
                io::ErrorKind::InvalidInput,
                format!("{path} is not a <collection>/<file> path"),
            ));
        }
        let entry = Entry {
            format: FORMAT,
            id: new_op_id(now),
            created: now,
            kind: "resolve".to_string(),
            uids,
            stage: Stage::Prepared,
            inverse_of: None,
            synced: None,
            writes: vec![Write {
                path: path.to_string(),
                before: expect.map(str::to_string),
                after: content.map(str::to_string),
            }],
        };
        let _lock = self.lock()?;
        self.check_not_stuck()?;
        self.check_holds(path, expect)?;
        self.run(entry)
    }

    /// Revert entry `id` with a new entry (writes swapped and reversed, `inverse_of` = `id`).
    /// Every file must still hold what `id` wrote. Returns the new entry id.
    pub fn undo(&mut self, id: &str, now: DateTime<Utc>) -> Result<String, WriteError> {
        let new = self.revert(id, now)?;
        self.done.retain(|d| d != id);
        Ok(new)
    }

    /// Undo the last operation applied (or redone) this session.
    pub fn undo_last(&mut self, now: DateTime<Utc>) -> Result<String, WriteError> {
        let id = self.done.last().cloned().ok_or(WriteError::NothingToUndo)?;
        let new = self.revert(&id, now)?;
        self.done.pop();
        self.undone.push(new.clone());
        Ok(new)
    }

    /// Redo the last undo of this session by reverting it.
    pub fn redo_last(&mut self, now: DateTime<Utc>) -> Result<String, WriteError> {
        let id = self
            .undone
            .last()
            .cloned()
            .ok_or(WriteError::NothingToRedo)?;
        let new = self.revert(&id, now)?;
        self.undone.pop();
        self.done.push(new.clone());
        Ok(new)
    }

    /// Mark the unfinished entry `id` Abandoned so writes can continue. Its files are left
    /// as they are; the entry stays in the journal for manual recovery.
    pub fn abandon(&mut self, id: &str) -> Result<(), WriteError> {
        let _lock = self.lock()?;
        let e = self.journal.get(id)?.ok_or_else(|| no_entry(id))?;
        if e.stage != Stage::Prepared {
            return Err(io_error(
                io::ErrorKind::InvalidInput,
                format!("operation {id} is not unfinished"),
            ));
        }
        self.journal.set_stage(id, Stage::Abandoned)?;
        Ok(())
    }

    fn revert(&self, id: &str, now: DateTime<Utc>) -> Result<String, WriteError> {
        let _lock = self.lock()?;
        self.check_not_stuck()?;
        let e = self.journal.get(id)?.ok_or_else(|| no_entry(id))?;
        if e.stage != Stage::Applied {
            return Err(io_error(
                io::ErrorKind::InvalidInput,
                format!("operation {id} was not applied"),
            ));
        }
        if self.mode == WriteMode::Verified && is_repeat_completion(&e) {
            let ok = self
                .matrix
                .edge_case(UNDO_REPEAT_EDGE)
                .is_some_and(|c| c.verdict == Verdict::Pass);
            if !ok {
                return Err(WriteError::Blocked(Blocked(format!(
                    "undoing the completion of a repeating task is not verified yet \
                     ({UNDO_REPEAT_EDGE})"
                ))));
            }
        }
        for w in &e.writes {
            self.check_holds(&w.path, w.after.as_deref())?;
        }
        let writes = e
            .writes
            .iter()
            .rev()
            .map(|w| Write {
                path: w.path.clone(),
                before: w.after.clone(),
                after: w.before.clone(),
            })
            .collect();
        self.run(Entry {
            format: FORMAT,
            id: new_op_id(now),
            created: now,
            kind: e.kind,
            uids: e.uids,
            stage: Stage::Prepared,
            inverse_of: Some(id.to_string()),
            synced: None,
            writes,
        })
    }

    /// The journal protocol (the caller holds the lock): prepare, apply, mark applied. A
    /// conflict leaves the entry Prepared, which blocks later writes until it is abandoned.
    fn run(&self, e: Entry) -> Result<String, WriteError> {
        self.journal.prepare(&e)?;
        match apply_entry(&self.vdir, &e) {
            Ok(()) => {}
            Err(ApplyError::Io(err)) => return Err(err.into()),
            Err(err) => return Err(WriteError::Stuck(format!("{} ({err})", e.id))),
        }
        self.journal.set_stage(&e.id, Stage::Applied)?;
        Ok(e.id)
    }

    /// Recovery under the held lock; any entry it cannot finish blocks the write.
    fn check_not_stuck(&self) -> Result<(), WriteError> {
        let report = recover_locked(&self.vdir, &self.journal)?;
        if report.stuck.is_empty() {
            return Ok(());
        }
        let list: Vec<String> = report
            .stuck
            .iter()
            .map(|(id, why)| format!("{id} ({why})"))
            .collect();
        Err(WriteError::Stuck(list.join("; ")))
    }

    /// The file at vdir-relative `rel` must hold `want` (`None`: must not exist).
    fn check_holds(&self, rel: &str, want: Option<&str>) -> Result<(), WriteError> {
        let path = self.vdir.join(rel);
        let current = match fs::read(&path) {
            Ok(b) => Some(b),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        if current.as_deref() == want.map(str::as_bytes) {
            Ok(())
        } else {
            Err(WriteError::Stale(path))
        }
    }

    /// `<collection>/<file>` of a loaded task's absolute path.
    fn rel(&self, path: &Path) -> Result<String, WriteError> {
        let outside = || {
            io_error(
                io::ErrorKind::InvalidInput,
                format!("{} is not in the vdir", path.display()),
            )
        };
        let rest = path.strip_prefix(&self.vdir).map_err(|_| outside())?;
        let parts = rest
            .components()
            .map(|c| match c {
                Component::Normal(s) => s.to_str().ok_or_else(outside),
                _ => Err(outside()),
            })
            .collect::<Result<Vec<&str>, _>>()?;
        if parts.len() != 2 {
            return Err(outside());
        }
        Ok(parts.join("/"))
    }

    fn target(&self, snap: &Snapshot, uid: &str) -> Result<Target, WriteError> {
        let i = snap
            .find(uid)
            .ok_or_else(|| WriteError::Blocked(Blocked(format!("no task with UID {uid}"))))?;
        let t = &snap.tasks[i];
        let bytes = snap
            .source(&t.path)
            .ok_or_else(|| WriteError::Stale(t.path.clone()))?;
        let bad = |why: String| io_error(io::ErrorKind::InvalidData, why);
        let source = String::from_utf8(bytes.to_vec())
            .map_err(|_| bad(format!("{} is not UTF-8", t.path.display())))?;
        let cal = ical::parse(bytes).map_err(|e| bad(format!("{}: {e}", t.path.display())))?;
        Ok(Target {
            collection: t.collection.clone(),
            rel: self.rel(&t.path)?,
            source,
            cal,
            repeats: t.rrule.is_some(),
        })
    }

    /// A save (kind "save"): the edit's writes, then the move's, planned on the edited files.
    /// A path written twice keeps the first `before` and the last `after`, at its later place
    /// (a move creates the copy before it removes the source). Either part may be empty.
    fn plan_save(
        &self,
        snap: &Snapshot,
        uid: &str,
        changes: &[Change],
        to: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Plan, WriteError> {
        let mut plan = Plan {
            kind: "save",
            uids: Vec::new(),
            writes: Vec::new(),
            holds: Vec::new(),
        };
        let mut edited = snap.clone();
        if !changes.is_empty() || to.is_none() {
            let edit = Op::Edit {
                uid: uid.to_string(),
                changes: changes.to_vec(),
            };
            let part = self.plan(snap, &edit, now)?;
            for w in &part.writes {
                if let Some(after) = &w.after {
                    edited.set_source(&self.vdir.join(&w.path), after.clone().into_bytes());
                }
            }
            plan.uids = part.uids;
            plan.writes = part.writes;
            plan.holds = part.holds;
        }
        let Some(to) = to else {
            return Ok(plan);
        };
        let mv = Op::Move {
            uid: uid.to_string(),
            to: to.to_string(),
        };
        let part = self.plan(&edited, &mv, now)?;
        for w in part.writes {
            match plan.writes.iter().position(|e| e.path == w.path) {
                Some(k) => {
                    let first = plan.writes.remove(k);
                    plan.writes.push(Write {
                        before: first.before,
                        ..w
                    });
                }
                None => plan.writes.push(w),
            }
        }
        for u in part.uids {
            if !plan.uids.contains(&u) {
                plan.uids.push(u);
            }
        }
        plan.holds.extend(part.holds);
        let writes = &plan.writes;
        plan.holds
            .retain(|(rel, _)| writes.iter().all(|w| &w.path != rel));
        Ok(plan)
    }

    fn plan(&self, snap: &Snapshot, op: &Op, now: DateTime<Utc>) -> Result<Plan, WriteError> {
        let one = |kind, uid: &String, w| Plan {
            kind,
            uids: vec![uid.clone()],
            writes: vec![w],
            holds: Vec::new(),
        };
        // A checklist or project with children: the op acts on the whole family.
        let parent = |uid: &str| snap.find(uid).filter(|&i| !snap.children(i).is_empty());
        Ok(match op {
            Op::Create(n) => {
                let uid = new_uid();
                let parent_json_uid = n.parent.as_deref().and_then(|p| parent_json_uid(snap, p));
                let cal = new_task(n, &uid, parent_json_uid.as_deref(), now)?;
                let w = create(&n.collection, &uid, &cal)?;
                one("create", &uid, w)
            }
            Op::Edit { uid, changes }
                if let Some(i) = parent(uid)
                    && crate::gate::checklist_date_change(snap, i, op) =>
            {
                self.plan_checklist_date(snap, i, self.target(snap, uid)?, changes, now)?
            }
            Op::Edit { uid, changes } => {
                let t = self.target(snap, uid)?;
                let mut cal = t.cal.clone();
                apply_changes(&mut cal, changes, now)?;
                one("edit", uid, t.update(&cal)?)
            }
            Op::Complete { uid } if let Some(i) = parent(uid) => {
                self.plan_family_complete(snap, i, self.target(snap, uid)?, now)?
            }
            Op::Reopen { uid } if let Some(i) = parent(uid) => {
                self.plan_family_reopen(snap, i, self.target(snap, uid)?, now)?
            }
            Op::Delete { uid } if let Some(i) = parent(uid) => {
                self.plan_family_delete(snap, i, self.target(snap, uid)?)?
            }
            Op::Complete { uid } => {
                let t = self.target(snap, uid)?;
                if t.repeats {
                    // 2Do's copy-and-advance: the history copy first, then the live task.
                    let copy_uid = new_uid();
                    let (live, copy) = complete_recurring(&t.cal, now, &copy_uid)?;
                    let copy = create(&t.collection, &copy_uid, &copy)?;
                    Plan {
                        kind: "complete",
                        uids: vec![uid.clone(), copy_uid],
                        writes: vec![copy, t.update(&live)?],
                        holds: Vec::new(),
                    }
                } else {
                    let mut cal = t.cal.clone();
                    complete_one_time(&mut cal, now)?;
                    one("complete", uid, t.update(&cal)?)
                }
            }
            Op::Reopen { uid } => {
                let t = self.target(snap, uid)?;
                let mut cal = t.cal.clone();
                reopen_one_time(&mut cal, now)?;
                one("reopen", uid, t.update(&cal)?)
            }
            Op::Move { uid, to } => {
                // H5 as captured: a new resource under a new UID, then the source removed.
                let t = self.target(snap, uid)?;
                if &t.collection == to {
                    return Err(EditError::Unsupported(format!("task is already in {to}")).into());
                }
                if let Some(i) = parent(uid) {
                    return self.plan_family_move(snap, i, t, to, now);
                }
                let new = new_uid();
                let copy = create(to, &new, &moved_copy(&t.cal, &new, None, now)?)?;
                Plan {
                    kind: "move",
                    uids: vec![uid.clone(), new],
                    writes: vec![copy, t.remove()],
                    holds: Vec::new(),
                }
            }
            Op::Save { uid, changes, to } => {
                self.plan_save(snap, uid, changes, to.as_deref(), now)?
            }
            Op::Delete { uid } => {
                let t = self.target(snap, uid)?;
                one("delete", uid, t.remove())
            }
        })
    }
}
