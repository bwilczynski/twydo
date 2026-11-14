//! Crash-safe storage: atomic file writes, the vdir lock, the operation journal with
//! recovery (see `docs/architecture.md`), the [`Writer`] that applies operations through them
//! and the per-session [`History`].
//!
//! An operation runs only when the gate allows it and every file it touches still holds
//! the bytes the caller loaded. It is recorded in the journal before the vdir is touched,
//! so a crash leaves it to be rolled forward by recovery, never half applied.

mod family;
pub mod fsio;
pub mod history;
pub mod journal;
mod order;
pub mod plan;
pub mod queue;
mod reparent;
mod retired;
pub mod writer;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::edit::{EditError, Op};
use crate::gate::Blocked;
use crate::ical;
use crate::model::meta_edit::MetaJson;
use crate::views::Snapshot;
use journal::{Entry, FORMAT, Stage, new_op_id};
use queue::Item;

pub use history::{Access, History, Step, StepState};
pub use journal::RecoveryReport;
pub use retired::recover_list;
pub use writer::{Ctx, Fence, Locked, Writer};

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
    /// The vdir lock is held (a sync is running) and the call was told not to wait, or a
    /// drainer is mid-commit while an undo looks for its entry.
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

/// What a write became: applied now, queued, or withdrawn.
#[derive(Debug)]
// Items carry whole tasks; one is built per write, never in bulk.
#[allow(clippy::large_enum_variant)]
pub enum Submitted {
    /// Committed now; the journal entry id.
    Applied(String),
    /// A sync holds the vdir: waiting in the queue for the next lock holder.
    Queued(Item),
    /// An undo removed a queued item before anything committed it.
    Cancelled,
}

/// A queued item a drain dealt with.
#[derive(Debug, Clone)]
pub struct Drained {
    pub item: Item,
    pub outcome: DrainOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrainOutcome {
    /// Committed; the journal entry id.
    Applied(String),
    /// A journal entry already named the item (a crash before its claim was removed).
    AlreadyApplied(String),
    /// Not committed; why.
    Refused(String),
}

/// A drained item and, for a refusal, the error behind it.
pub(crate) struct DrainStep {
    pub(crate) drained: Drained,
    pub(crate) error: Option<WriteError>,
}

impl DrainStep {
    pub(crate) fn new(item: Item, outcome: DrainOutcome) -> DrainStep {
        DrainStep {
            drained: Drained { item, outcome },
            error: None,
        }
    }
}

pub(crate) fn no_entry(id: &str) -> WriteError {
    io_error(io::ErrorKind::NotFound, format!("no journal entry {id}"))
}

/// The JSON `uid` in the metadata of the task with iCalendar UID `parent`, if it has one.
/// A new item's `parentuid` is this value, else the parent's UID.
pub fn parent_json_uid(snap: &Snapshot, parent: &str) -> Option<String> {
    let raw = snap.tasks[snap.find(parent)?].meta.raw.as_deref()?;
    let meta = crate::model::meta_edit::MetaJson::parse(raw)?;
    meta.get_str("uid").map(str::to_string)
}

/// What became of `item` among the `steps` a drain dealt with: committed, refused (the error
/// behind it) or still queued.
pub(crate) fn drained_outcome(steps: Vec<DrainStep>, item: Item) -> Result<Submitted, WriteError> {
    for step in steps {
        if step.drained.item.id != item.id {
            continue;
        }
        return match (step.drained.outcome, step.error) {
            (DrainOutcome::Applied(id) | DrainOutcome::AlreadyApplied(id), _) => {
                Ok(Submitted::Applied(id))
            }
            (DrainOutcome::Refused(reason), error) => Err(match error {
                Some(e) => e,
                None => WriteError::Blocked(Blocked(reason)),
            }),
        };
    }
    Ok(Submitted::Queued(item))
}
