//! Resolving a conflict by keeping one of its two versions (`tdo conflicts resolve`).
//!
//! A held conflict (vdirsyncer still keeps the two sides apart) only records the choice: the
//! conflict hook applies it on the next sync and the runner then closes the conflict. For any
//! other conflict the vdir already holds one version; keeping it closes the conflict once the
//! file is checked to still hold it, and keeping the other writes it through
//! [`Locked::restore`](crate::store::Locked::restore) (which the next sync propagates as an
//! ordinary change), then closes the conflict. A file that no longer holds the version it
//! held then is refused; keeping [`Keep::Current`] closes such a conflict and leaves the file
//! as it is. Lock order: the vdir lock, then the conflicts lock (inside `mark_resolved`);
//! `resolve` takes the exclusive vdir lock through the writer where it writes, and `restore`
//! runs under it, before `mark_resolved`.

use std::fs;
use std::io;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::conflicts::{Conflict, ConflictState, ConflictStore, ResolvedBy, Side};
use crate::store::fsio::VdirLock;
use crate::store::journal::RequestRef;
use crate::store::{Ctx, WriteError, Writer, existing_vdir_state};

/// Which version of a conflict to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    Local,
    Remote,
    /// The task as the vdir holds it now, whichever version that is (not for held
    /// conflicts: vdirsyncer still keeps both sides apart there).
    Current,
}

/// What `resolve` did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// A held conflict: the choice is recorded and the next sync applies it.
    Recorded,
    /// The vdir already holds the kept version; the conflict is closed.
    AlreadyInPlace,
    /// The kept version was written to the vdir (journal entry id); the conflict is closed.
    Written(String),
    /// [`Keep::Current`]: the conflict is closed and the vdir file left as it is.
    KeptCurrent,
}

impl Resolution {
    /// What `tdo conflicts resolve` tells the user.
    pub fn message(&self) -> &'static str {
        match self {
            Resolution::Recorded => "recorded; the next sync applies it",
            Resolution::AlreadyInPlace => "already in place; conflict resolved",
            Resolution::Written(_) => "written; conflict resolved",
            Resolution::KeptCurrent => "kept as it is now; conflict resolved",
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ResolveError {
    #[error("no conflict {0}")]
    NotFound(String),
    #[error("conflict {0} is already resolved or superseded")]
    NotUnresolved(String),
    #[error(
        "the task changed since the conflict was recorded ({path} holds neither version); \
         see tdo conflicts show {id}, or keep it as it is now: \
         tdo conflicts resolve {id} --keep current"
    )]
    Changed { id: String, path: String },
    #[error("conflict {0} is held by vdirsyncer until you choose one side: keep local or remote")]
    Held(String),
    #[error(transparent)]
    Write(#[from] WriteError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Text with CRLF line endings read as LF: the conflict hook hands back choices with LF.
fn lf(s: &str) -> String {
    s.replace("\r\n", "\n")
}

fn same_text(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => lf(a) == lf(b),
        (None, None) => true,
        _ => false,
    }
}

fn version(c: &Conflict, side: Side) -> Option<&str> {
    match side {
        Side::Local => c.local.as_deref(),
        Side::Remote => c.remote.as_deref(),
    }
}

/// Keeps the `keep` version of conflict `id`; see the module docs.
pub fn resolve(
    cfg: &Config,
    writer: &Writer,
    id: &str,
    keep: Keep,
    now: DateTime<Utc>,
) -> Result<Resolution, ResolveError> {
    resolve_with(cfg, writer, id, keep, now, None)
}

/// [`resolve`] for a client request: the conflict and, for a written version, the journal
/// entry record `request`, so a resend finds its earlier result ([`ConflictStore::by_request`]).
pub fn resolve_with(
    cfg: &Config,
    writer: &Writer,
    id: &str,
    keep: Keep,
    now: DateTime<Utc>,
    request: Option<RequestRef>,
) -> Result<Resolution, ResolveError> {
    let by = |resolution: &Resolution| {
        request.clone().map(|request| ResolvedBy {
            request,
            op_id: match resolution {
                Resolution::Written(op) => Some(op.clone()),
                _ => None,
            },
            resolution: resolution.clone(),
        })
    };
    let not_found = || ResolveError::NotFound(id.to_string());
    let state = existing_vdir_state(&cfg.state_dir(), &cfg.vdir)?.ok_or_else(not_found)?;
    if crate::retired::archived_conflict(&state, id)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "this conflict belongs to a preserved list; use :recover to restore its local snapshot",
        )
        .into());
    }
    let store = ConflictStore::existing(&state)?.ok_or_else(not_found)?;
    let c = store.get(id)?.ok_or_else(not_found)?;
    if !matches!(c.state, ConflictState::Open | ConflictState::Chosen) {
        return Err(ResolveError::NotUnresolved(id.to_string()));
    }
    let keep = match (keep, c.held) {
        (Keep::Current, true) => return Err(ResolveError::Held(id.to_string())),
        (Keep::Current, false) => {
            // Both versions stay in the conflict's files; the vdir is not touched.
            let r = Resolution::KeptCurrent;
            store.mark_resolved_by(id, now, by(&r))?;
            return Ok(r);
        }
        (Keep::Local, _) => Side::Local,
        (Keep::Remote, _) => Side::Remote,
    };
    if c.held {
        let r = Resolution::Recorded;
        store.choose_by(id, keep, by(&r))?;
        return Ok(r);
    }
    let kept = version(&c, keep);
    let holds = version(&c, c.vdir_holds);
    // `vdir_holds` describes the file when the conflict was recorded; decide on what it holds
    // now. The shared vdir lock keeps writers out between the read and `mark_resolved`.
    let shared = VdirLock::shared(&state)?;
    let actual = match fs::read(cfg.vdir.join(&c.path)) {
        Ok(b) => Some(String::from_utf8(b).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} is not UTF-8", c.path),
            )
        })?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    // Kept version in place, or a retry after a crash between the write and `mark_resolved`.
    if same_text(actual.as_deref(), kept) {
        let r = Resolution::AlreadyInPlace;
        store.mark_resolved_by(id, now, by(&r))?;
        return Ok(r);
    }
    // The file no longer holds the version it held then (a later sync or edit changed or
    // deleted it): keeping either side now would drop the current task, or close the
    // conflict on a version that is gone. Keeping the current one is the user's call.
    if keep == c.vdir_holds || !same_text(actual.as_deref(), holds) {
        return Err(ResolveError::Changed {
            id: id.to_string(),
            path: c.path,
        });
    }
    drop(shared);
    // The file holds `holds`, perhaps with other line endings: hand `restore` the bytes
    // actually on disk (it refuses if they change before it takes the lock).
    let ctx = Ctx {
        now,
        request: request.clone(),
        config: cfg,
    };
    let op = writer.lock_waiting()?.restore(
        &c.path,
        actual.as_deref(),
        kept,
        vec![c.uid.clone()],
        &ctx,
    )?;
    let r = Resolution::Written(op);
    store.mark_resolved_by(id, now, by(&r))?;
    Ok(r)
}
