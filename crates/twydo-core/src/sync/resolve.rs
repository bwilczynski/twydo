//! Resolving a conflict by keeping one of its two versions (`tdo conflicts resolve`).
//!
//! A held conflict (vdirsyncer still keeps the two sides apart) only records the choice: the
//! conflict hook applies it on the next sync and the runner then closes the conflict. For any
//! other conflict the vdir already holds one version; keeping it closes the conflict once the
//! file is checked to still hold it, and keeping the other writes it through
//! [`Store::restore`] (which the next sync propagates as an ordinary change), then closes the
//! conflict. A file that no longer holds the version it held then is refused. Lock order:
//! the vdir lock, then the conflicts lock (inside `mark_resolved`); `restore` takes and
//! releases the exclusive vdir lock itself, before `mark_resolved`.

use std::fs;
use std::io;

use chrono::{DateTime, Utc};

use crate::config::Config;
use crate::conflicts::{Conflict, ConflictState, ConflictStore, Side};
use crate::store::fsio::VdirLock;
use crate::store::{Store, WriteError, existing_vdir_state};

/// What `resolve` did.
#[derive(Debug, PartialEq, Eq)]
pub enum Resolution {
    /// A held conflict: the choice is recorded and the next sync applies it.
    Recorded,
    /// The vdir already holds the kept version; the conflict is closed.
    AlreadyInPlace,
    /// The kept version was written to the vdir (journal entry id); the conflict is closed.
    Written(String),
}

#[derive(thiserror::Error, Debug)]
pub enum ResolveError {
    #[error("no conflict {0}")]
    NotFound(String),
    #[error("conflict {0} is already resolved or superseded")]
    NotUnresolved(String),
    #[error("{0} changed since the conflict was recorded; run tdo sync and check tdo conflicts")]
    Changed(String),
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
    id: &str,
    keep: Side,
    now: DateTime<Utc>,
) -> Result<Resolution, ResolveError> {
    let not_found = || ResolveError::NotFound(id.to_string());
    let state = existing_vdir_state(&cfg.state_dir(), &cfg.vdir)?.ok_or_else(not_found)?;
    let store = ConflictStore::existing(&state)?.ok_or_else(not_found)?;
    let c = store.get(id)?.ok_or_else(not_found)?;
    if !matches!(c.state, ConflictState::Open | ConflictState::Chosen) {
        return Err(ResolveError::NotUnresolved(id.to_string()));
    }
    if c.held {
        store.choose(id, keep)?;
        return Ok(Resolution::Recorded);
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
        store.mark_resolved(id, now)?;
        return Ok(Resolution::AlreadyInPlace);
    }
    // The file no longer holds the version it held then (a later sync changed or deleted
    // it): closing the conflict now would drop the kept version unseen.
    if keep == c.vdir_holds {
        return Err(ResolveError::Changed(c.path));
    }
    drop(shared);
    // A file that differs from the recorded version only in line endings still holds it; hand
    // `restore` the bytes actually on disk.
    let expect = if same_text(actual.as_deref(), holds) {
        actual.as_deref()
    } else {
        holds
    };
    let (mut writer, _) = Store::open(cfg)?;
    let op = writer.restore(&c.path, expect, kept, vec![c.uid.clone()], now)?;
    store.mark_resolved(id, now)?;
    Ok(Resolution::Written(op))
}
