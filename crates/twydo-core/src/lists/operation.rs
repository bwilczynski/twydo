//! Durable collection phases and a fence shared with the task store and sync runner.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::conflicts::{ConflictState, ConflictStore};
use crate::store::bind_vdir_state;
use crate::store::fsio::{VdirLock, remove_durable, sync_dir, write_atomic};
use crate::store::journal::{Journal, new_op_id, recover_locked};
use crate::store::queue::Queue;

use super::{ListError, helper::HelperRequest, invalid, plain_id};
use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    Prepared,
    RemoteStarted,
    RemoteConfirmed,
    LocalConfirmed,
    Complete,
    Uncertain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListOperation {
    pub format: u32,
    pub id: String,
    pub created: DateTime<Utc>,
    pub vdir: PathBuf,
    pub phase: Phase,
    pub request: HelperRequest,
    /// Exact bytes of the held local collection; `None` means it did not exist.
    pub holds: Option<BTreeMap<String, Vec<u8>>>,
    #[serde(default)]
    pub receipt: Option<serde_json::Value>,
    #[serde(default)]
    pub remote_target: Option<String>,
    #[serde(default)]
    pub connection_digest: Option<String>,
    #[serde(default)]
    pub local_base: Option<BTreeMap<String, Vec<u8>>>,
    #[serde(default)]
    pub local_after: Option<BTreeMap<String, Vec<u8>>>,
}

pub(crate) fn private_dir(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.is_symlink() || !m.is_dir()) {
        return Err(invalid("collection state must be a plain directory"));
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// Serializes marker publication with queue insertion. Order: run, vdir, collection fence.
pub(crate) fn coordinate(state: &Path) -> io::Result<File> {
    let dir = state.join("lists");
    private_dir(&dir)?;
    let path = dir.join("coord.lock");
    if fs::symlink_metadata(&path).is_ok_and(|m| m.is_symlink() || !m.is_file()) {
        return Err(invalid("collection coordination lock must be a plain file"));
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.lock()?;
    Ok(file)
}

fn record_path(state: &Path, id: &str) -> io::Result<PathBuf> {
    if !plain_id(id) || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(invalid("invalid collection operation identity"));
    }
    Ok(state.join("lists/operations").join(format!("{id}.json")))
}

pub fn read(state: &Path, id: &str) -> io::Result<ListOperation> {
    let path = record_path(state, id)?;
    if fs::symlink_metadata(&path)?.is_symlink() {
        return Err(invalid("collection operation must be a plain file"));
    }
    let operation: ListOperation =
        serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
    if operation.format != 1 || operation.id != id {
        return Err(invalid("unsupported collection operation"));
    }
    Ok(operation)
}

pub fn ensure_idle(state: &Path) -> io::Result<()> {
    let marker = state.join("lists/active");
    let id = match fs::read_to_string(&marker) {
        Ok(id) => id,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if fs::symlink_metadata(marker)?.is_symlink() {
        return Err(invalid("collection operation marker must be a plain file"));
    }
    let operation = read(state, id.trim())?;
    if operation.phase == Phase::Complete {
        return Ok(());
    }
    Err(invalid(format!(
        "collection operation {} is unfinished; reconcile it before writing or syncing",
        operation.id
    )))
}

/// The pending collection operation, including a create with no visible local list yet.
pub fn active(cfg: &Config) -> io::Result<Option<ListOperation>> {
    let Some(state) = crate::store::existing_vdir_state(&cfg.state_dir(), &cfg.vdir)? else {
        return Ok(None);
    };
    let marker = state.join("lists/active");
    let id = match fs::read_to_string(&marker) {
        Ok(id) => id,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if fs::symlink_metadata(marker)?.is_symlink() {
        return Err(invalid("invalid collection marker"));
    }
    let record = read(&state, id.trim())?;
    if record.vdir != fs::canonicalize(&cfg.vdir)? {
        return Err(invalid("collection operation belongs to another vdir"));
    }
    Ok((record.phase != Phase::Complete).then_some(record))
}

pub fn snapshot(path: &Path) -> io::Result<Option<BTreeMap<String, Vec<u8>>>> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.is_symlink()) {
        return Err(invalid("collection is a symlink"));
    }
    if !path.try_exists()? {
        return Ok(None);
    }
    fn walk(root: &Path, dir: &Path, files: &mut BTreeMap<String, Vec<u8>>) -> io::Result<()> {
        let meta = fs::symlink_metadata(dir)?;
        if meta.is_symlink() || !meta.is_dir() {
            return Err(invalid("collection contains a symlink or special path"));
        }
        for item in fs::read_dir(dir)? {
            let item = item?;
            let path = item.path();
            let meta = fs::symlink_metadata(&path)?;
            if meta.is_symlink() {
                return Err(invalid("collection contains a symlink"));
            }
            if meta.is_dir() {
                walk(root, &path, files)?;
            } else if meta.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .expect("descendant")
                    .to_str()
                    .ok_or_else(|| invalid("non-UTF-8 collection path"))?
                    .to_string();
                files.insert(rel, fs::read(path)?);
            } else {
                return Err(invalid("collection contains a special file"));
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    walk(path, path, &mut files)?;
    Ok(Some(files))
}

pub fn validate_local(operation: &ListOperation) -> io::Result<()> {
    let id = operation
        .request
        .collection
        .as_deref()
        .filter(|id| plain_id(id))
        .ok_or_else(|| invalid("missing collection identity"))?;
    let current = snapshot(&operation.vdir.join(id))?;
    if let Some(after) = &operation.local_after {
        if matches!(operation.phase, Phase::LocalConfirmed | Phase::Complete) {
            return if current.as_ref() == Some(after) {
                Ok(())
            } else {
                Err(invalid("confirmed local collection changed"))
            };
        }
        let before = operation.local_base.as_ref();
        let current = current.as_ref();
        let keys: std::collections::BTreeSet<_> = before
            .into_iter()
            .flat_map(|m| m.keys())
            .chain(after.keys())
            .chain(current.into_iter().flat_map(|m| m.keys()))
            .collect();
        if keys.into_iter().all(|key| {
            let actual = current.and_then(|m| m.get(key));
            actual == before.and_then(|m| m.get(key)) || actual == after.get(key)
        }) {
            return Ok(());
        }
    } else if current == operation.holds {
        return Ok(());
    }
    Err(invalid(
        "collection changed since the operation was prepared",
    ))
}

pub fn save(state: &Path, operation: &ListOperation) -> io::Result<()> {
    private_dir(&state.join("lists/operations"))?;
    write_atomic(
        &record_path(state, &operation.id)?,
        &serde_json::to_vec_pretty(operation).map_err(io::Error::other)?,
    )
}

pub fn prepare(
    cfg: &Config,
    mut request: HelperRequest,
    now: DateTime<Utc>,
) -> Result<ListOperation, ListError> {
    let collection = request
        .collection
        .as_deref()
        .filter(|id| plain_id(id))
        .ok_or_else(|| invalid("missing collection identity"))?
        .to_string();
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir)?;
    private_dir(&state.join("sync"))?;
    let _run = crate::sync::run::run_lock(&state.join("sync"), false)?;
    let _vdir = VdirLock::exclusive(&state)?;
    ensure_idle(&state)?;
    let journal = Journal::open(&state)?;
    if !recover_locked(&cfg.vdir, &journal)?.stuck.is_empty() {
        return Err(invalid("unfinished task recovery must be resolved first").into());
    }
    let _fence = coordinate(&state)?;
    ensure_idle(&state)?;
    let target = |path: &str| {
        Path::new(path)
            .components()
            .next()
            .is_some_and(|c| c.as_os_str() == collection.as_str())
    };
    if journal
        .pending()?
        .iter()
        .any(|e| e.writes.iter().any(|w| target(&w.path)))
    {
        return Err(invalid("list has unsynced task operations; sync them first").into());
    }
    if Queue::existing(&state)
        .map(|q| q.list_strict())
        .transpose()?
        .is_some_and(|items| {
            items
                .iter()
                .any(|i| i.writes().iter().any(|w| target(&w.path)))
        })
    {
        return Err(invalid("list has queued task edits; finish them first").into());
    }
    if ConflictStore::open(&state)?.list()?.iter().any(|c| {
        c.collection == collection && matches!(c.state, ConflictState::Open | ConflictState::Chosen)
    }) {
        return Err(invalid("list has unresolved conflicts").into());
    }
    let root = state.join("lists");
    if fs::canonicalize(&root)?.starts_with(fs::canonicalize(&cfg.vdir)?) {
        return Err(invalid("collection state must be outside the synchronized vdir").into());
    }
    let id = new_op_id(now);
    request.vdir = fs::canonicalize(&cfg.vdir)?;
    request.state = Some(state.clone());
    request.operation = Some(record_path(&state, &id)?);
    let operation = ListOperation {
        format: 1,
        id,
        created: now,
        vdir: request.vdir.clone(),
        phase: Phase::Prepared,
        holds: snapshot(&cfg.vdir.join(&collection))?,
        request,
        receipt: None,
        remote_target: None,
        connection_digest: None,
        local_base: None,
        local_after: None,
    };
    save(&state, &operation)?;
    write_atomic(&root.join("active"), operation.id.as_bytes())?;
    sync_dir(&state)?;
    Ok(operation)
}

pub fn set_phase(
    cfg: &Config,
    operation: &mut ListOperation,
    next: Phase,
) -> Result<(), ListError> {
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir)?;
    let _lock = VdirLock::exclusive(&state)?;
    let saved = read(&state, &operation.id)?;
    let marker = fs::read_to_string(state.join("lists/active"))?;
    if marker.trim() != operation.id
        || operation.phase == Phase::Complete
        || saved.phase != operation.phase
        || saved.vdir != fs::canonicalize(&cfg.vdir)?
    {
        return Err(invalid("collection operation changed; reload before reconciling").into());
    }
    let allowed = matches!(
        (operation.phase, next),
        (Phase::Prepared, Phase::RemoteStarted)
            | (Phase::RemoteStarted, Phase::RemoteConfirmed)
            | (Phase::RemoteConfirmed, Phase::LocalConfirmed)
            | (_, Phase::Uncertain)
    );
    if !allowed {
        return Err(invalid("invalid collection operation transition").into());
    }
    operation.phase = next;
    save(&state, operation)?;
    Ok(())
}

pub fn finish(cfg: &Config, operation: &mut ListOperation) -> Result<(), ListError> {
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir)?;
    private_dir(&state.join("sync"))?;
    let _run = crate::sync::run::run_lock(&state.join("sync"), false)?;
    let _lock = VdirLock::exclusive(&state)?;
    let _fence = coordinate(&state)?;
    let saved = read(&state, &operation.id)?;
    if operation.phase != Phase::LocalConfirmed
        || saved.phase != operation.phase
        || saved.vdir != fs::canonicalize(&cfg.vdir)?
    {
        return Err(
            invalid("collection operation lacks confirmed remote and local results").into(),
        );
    }
    validate_local(operation)?;
    let marker = fs::read_to_string(state.join("lists/active"))?;
    if marker.trim() != operation.id {
        return Err(invalid("another collection operation is active").into());
    }
    operation.phase = Phase::Complete;
    save(&state, operation)?;
    remove_durable(&state.join("lists/active"))?;
    Ok(())
}

/// Plan local metadata before writing it, so a mixed before/after state is recoverable.
pub fn finalize_metadata(cfg: &Config, operation: &mut ListOperation) -> Result<(), ListError> {
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir)?;
    private_dir(&state.join("sync"))?;
    let _run = crate::sync::run::run_lock(&state.join("sync"), false)?;
    let _lock = VdirLock::exclusive(&state)?;
    *operation = read(&state, &operation.id)?;
    if operation.vdir != fs::canonicalize(&cfg.vdir)?
        || operation.phase != Phase::RemoteConfirmed
        || fs::read_to_string(state.join("lists/active"))?.trim() != operation.id
    {
        return Err(invalid("collection metadata lacks its confirmed remote receipt").into());
    }
    validate_local(operation)?;
    let receipt = operation
        .receipt
        .as_ref()
        .ok_or_else(|| invalid("missing collection receipt"))?;
    let mut after = operation.holds.clone().unwrap_or_default();
    for (key, field) in [("displayname", "name"), ("color", "color")] {
        match receipt.get(field) {
            Some(serde_json::Value::String(value)) => {
                after.insert(key.into(), value.as_bytes().to_vec());
            }
            Some(serde_json::Value::Null) => {
                after.remove(key);
            }
            _ => return Err(invalid("invalid collection metadata receipt").into()),
        }
    }
    let id = operation
        .request
        .collection
        .as_deref()
        .filter(|s| plain_id(s))
        .ok_or_else(|| invalid("invalid collection identity"))?;
    let dir = operation.vdir.join(id);
    operation.local_base = snapshot(&dir)?;
    operation.local_after = Some(after.clone());
    save(&state, operation)?;
    if !dir.try_exists()? {
        fs::create_dir(&dir)?;
        sync_dir(&operation.vdir)?;
    }
    for key in ["displayname", "color"] {
        let path = dir.join(key);
        match after.get(key) {
            Some(bytes) if fs::read(&path).ok().as_ref() != Some(bytes) => {
                write_atomic(&path, bytes)?
            }
            None => remove_durable(&path)?,
            _ => {}
        }
    }
    if snapshot(&dir)?.as_ref() != Some(&after) {
        return Err(invalid("collection changed during local finalization").into());
    }
    Ok(())
}

/// Clear a fence only when its persisted phase proves no remote/local mutation occurred.
pub fn cancel_unstarted(cfg: &Config, id: &str) -> Result<(), ListError> {
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir)?;
    private_dir(&state.join("sync"))?;
    let _run = crate::sync::run::run_lock(&state.join("sync"), false)?;
    let _lock = VdirLock::exclusive(&state)?;
    let _fence = coordinate(&state)?;
    let mut record = read(&state, id)?;
    let definitely_unwritten = record.request.action == "preview-delete"
        || record.phase == Phase::Prepared
        || (matches!(record.phase, Phase::RemoteStarted | Phase::Uncertain)
            && record.remote_target.is_none())
        || record
            .receipt
            .as_ref()
            .and_then(|r| r.get("no_remote_write"))
            .and_then(|v| v.as_bool())
            == Some(true);
    if !definitely_unwritten
        || record.local_after.is_some()
        || record.vdir != fs::canonicalize(&cfg.vdir)?
        || fs::read_to_string(state.join("lists/active"))?.trim() != id
    {
        return Err(invalid("collection outcome needs reconciliation").into());
    }
    record.phase = Phase::Complete;
    save(&state, &record)?;
    remove_durable(&state.join("lists/active"))?;
    Ok(())
}
