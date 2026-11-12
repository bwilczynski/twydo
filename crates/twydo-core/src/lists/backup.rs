//! Raw collection backups and best-effort list deletion.
//!
//! A preview captures the server's complete collection and the local copy as one backup. A
//! deletion revalidates both against it and then sends one unconditional DELETE: nothing on
//! the server enforces that state at DELETE time, so an edit made by another device in that
//! instant is not protected (the backup remains the recovery path). After a confirmed
//! deletion the local copy is retired through [`crate::retired`] and the backup is kept.
use super::{
    ListError, ListResult, ListService,
    helper::{self, HelperRequest},
    invalid,
    operation::{self, ListOperation, Phase},
    plain_id,
    remote::capability,
};
use crate::{
    config::Config,
    conflicts::ConflictStore,
    store::{
        bind_vdir_state, existing_vdir_state,
        fsio::{VdirLock, sync_dir, write_atomic},
        journal::Journal,
    },
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletePreview {
    pub backup: String,
    pub collection: String,
    pub name: String,
    pub resources: usize,
    pub local_files: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    pub href: String,
    pub etag: String,
    pub sha256: String,
    pub bytes: String,
}
impl Resource {
    pub fn data(&self) -> io::Result<Vec<u8>> {
        let bytes = decode(&self.bytes)?;
        if hash(&bytes) != self.sha256 {
            return Err(invalid("backup resource hash mismatch"));
        }
        Ok(bytes)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Remote {
    pub format: u32,
    pub properties: String,
    pub resources: Vec<Resource>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backup {
    pub format: u32,
    pub id: String,
    pub collection: String,
    pub name: String,
    pub created: DateTime<Utc>,
    pub local: BTreeMap<String, Vec<u8>>,
    pub local_hashes: BTreeMap<String, String>,
    pub remote: Remote,
    pub evidence: serde_json::Value,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn decode(value: &str) -> io::Result<Vec<u8>> {
    STANDARD
        .decode(value)
        .map_err(|_| invalid("invalid backup encoding"))
}
fn relative(value: &str) -> bool {
    !value.is_empty()
        && Path::new(value)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
fn directory(state: &Path, id: &str) -> io::Result<PathBuf> {
    if !plain_id(id) {
        return Err(invalid("invalid collection backup id"));
    }
    Ok(state.join("lists/backups").join(id))
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<T> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.is_symlink() || meta.len() > 192 * 1024 * 1024 {
        return Err(invalid("invalid collection backup file"));
    }
    serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)
}
fn validate(b: &Backup) -> io::Result<()> {
    if b.format != 1
        || b.remote.format != 1
        || !plain_id(&b.id)
        || !plain_id(&b.collection)
        || b.local.keys().any(|s| !relative(s))
        || b.local_hashes.len() != b.local.len()
        || b.local
            .iter()
            .any(|(name, bytes)| b.local_hashes.get(name) != Some(&hash(bytes)))
    {
        return Err(invalid("invalid collection backup manifest"));
    }
    decode(&b.remote.properties)?;
    for resource in &b.remote.resources {
        resource.data()?;
    }
    Ok(())
}
fn read(state: &Path, id: &str) -> io::Result<Backup> {
    let dir = directory(state, id)?;
    for path in [
        state.join("lists"),
        state.join("lists/backups"),
        dir.clone(),
    ] {
        let meta = fs::symlink_metadata(path)?;
        if !meta.is_dir() || meta.is_symlink() {
            return Err(invalid("invalid collection backup directory"));
        }
    }
    let backup: Backup = read_json(&dir.join("backup.json"))?;
    if backup.id != id {
        return Err(invalid("collection backup id mismatch"));
    }
    validate(&backup)?;
    Ok(backup)
}
pub fn get(cfg: &Config, id: &str) -> io::Result<Backup> {
    let state = existing_vdir_state(&cfg.state_dir(), &cfg.vdir)?
        .ok_or_else(|| invalid("no collection backups"))?;
    read(&state, id)
}
pub fn list(cfg: &Config) -> io::Result<Vec<Backup>> {
    let Some(state) = existing_vdir_state(&cfg.state_dir(), &cfg.vdir)? else {
        return Ok(vec![]);
    };
    let root = state.join("lists/backups");
    if !root.try_exists()? {
        return Ok(vec![]);
    }
    if fs::symlink_metadata(&root)?.is_symlink() {
        return Err(invalid("invalid collection backup directory"));
    }
    let mut backups = vec![];
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        // A failed read-only capture is not a completed backup.
        if entry.path().join("backup.json").try_exists()? {
            let id = entry
                .file_name()
                .into_string()
                .map_err(|_| invalid("invalid backup id"))?;
            backups.push(read(&state, &id)?);
        }
    }
    backups.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(backups)
}

/// Export exact raw bytes without importing them into the synchronized task store.
pub fn export(cfg: &Config, id: &str, destination: &Path) -> io::Result<()> {
    let backup = get(cfg, id)?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = fs::canonicalize(parent)?;
    if parent.starts_with(fs::canonicalize(&cfg.vdir)?) || destination.file_name().is_none() {
        return Err(invalid("export must be outside the synchronized vdir"));
    }
    let destination = parent.join(destination.file_name().unwrap());
    // create_dir, never create_dir_all, ensures an existing export cannot be overwritten.
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&destination)?;
    let write = |name: &Path, bytes: &[u8]| -> io::Result<()> {
        operation::private_dir(name.parent().unwrap())?;
        write_atomic(name, bytes)
    };
    for (name, bytes) in &backup.local {
        write(&destination.join("local").join(name), bytes)?;
    }
    for (index, resource) in backup.remote.resources.iter().enumerate() {
        write(
            &destination.join(format!("remote/{index:04}.bin")),
            &resource.data()?,
        )?;
    }
    write(
        &destination.join("properties.xml"),
        &decode(&backup.remote.properties)?,
    )?;
    write(
        &destination.join("manifest.json"),
        &serde_json::to_vec_pretty(&backup).map_err(io::Error::other)?,
    )?;
    sync_dir(&destination)?;
    sync_dir(&parent)
}

impl ListService {
    pub fn preview_delete(&mut self, id: &str) -> Result<DeletePreview, ListError> {
        self.validate_binding()?;
        let state = self.state(id)?;
        let mut request = HelperRequest::probe(&self.config)?;
        request.action = "preview-delete".into();
        request.collection = Some(id.into());
        let record = operation::prepare(&self.config, request, Utc::now())?;
        let result = (|| {
            helper::invoke(&self.config, &record.request)?;
            let bound = bind_vdir_state(&self.config.state_dir(), &self.config.vdir)?;
            let _run = crate::sync::run::run_lock(&bound.join("sync"), false)?;
            let _vdir = VdirLock::exclusive(&bound)?;
            let record = operation::read(&bound, &record.id)?;
            if record.phase != Phase::RemoteConfirmed
                || record.request.action != "preview-delete"
                || record.vdir != fs::canonicalize(&self.config.vdir)?
                || fs::read_to_string(bound.join("lists/active"))?.trim() != record.id
            {
                return Err(invalid("unconfirmed collection backup").into());
            }
            operation::validate_local(&record)?;
            let remote: Remote = read_json(&directory(&bound, &record.id)?.join("remote.json"))?;
            let journal = Journal::open(&bound)?
                .list()?
                .into_iter()
                .filter(|e| {
                    e.writes
                        .iter()
                        .any(|w| crate::retired::in_collection(&w.path, id))
                })
                .collect::<Vec<_>>();
            let conflicts = ConflictStore::open(&bound)?
                .list()?
                .into_iter()
                .filter(|c| c.collection == id)
                .collect::<Vec<_>>();
            let mut alternatives = BTreeMap::new();
            for conflict in &conflicts {
                alternatives.insert(
                    conflict.id.clone(),
                    operation::snapshot(&bound.join("conflicts").join(&conflict.id))?,
                );
            }
            let local = record
                .holds
                .clone()
                .ok_or_else(|| invalid("missing local collection snapshot"))?;
            let backup = Backup {
                format: 1,
                id: record.id.clone(),
                collection: id.into(),
                name: state.name.clone(),
                created: record.created,
                local_hashes: local.iter().map(|(k, v)| (k.clone(), hash(v))).collect(),
                local,
                remote,
                evidence: serde_json::json!({"journal":journal,"conflicts":conflicts,"alternatives":alternatives,"preferences":format!("{:?}",self.config.list_prefs(id)),"operation":record}),
            };
            validate(&backup)?;
            write_atomic(
                &directory(&bound, &backup.id)?.join("backup.json"),
                &serde_json::to_vec_pretty(&backup).map_err(io::Error::other)?,
            )?;
            Ok(DeletePreview {
                backup: backup.id,
                collection: id.into(),
                name: state.name,
                resources: backup.remote.resources.len(),
                local_files: backup.local.len(),
            })
        })();
        // This action is statically read-only, including after a timeout or partial backup.
        let release = operation::cancel_unstarted(&self.config, &record.id);
        match (result, release) {
            (Ok(p), Ok(())) => Ok(p),
            (Err(e), _) => Err(e),
            (_, Err(e)) => Err(e),
        }
    }

    pub fn delete_list(&mut self, preview: &DeletePreview) -> Result<ListResult, ListError> {
        self.validate_binding()?;
        capability(
            &self.matrix,
            self.config.writes.mode,
            "list-delete",
            "delete",
        )?;
        let state = bind_vdir_state(&self.config.state_dir(), &self.config.vdir)?;
        let saved = read(&state, &preview.backup).map_err(|_| {
            ListError::Refused("the deletion backup is missing or invalid; preview again".into())
        })?;
        if saved.collection != preview.collection {
            return Err(ListError::Refused(
                "the backup belongs to another list; preview again".into(),
            ));
        }
        let collection = saved.collection.as_str();
        if operation::snapshot(&self.config.vdir.join(collection))?.as_ref() != Some(&saved.local) {
            return Err(ListError::Refused(CHANGED.into()));
        }
        let mut request = HelperRequest::probe(&self.config)?;
        request.action = "delete".into();
        request.collection = Some(collection.into());
        request.backup = Some(saved.id.clone());
        let record = operation::prepare(&self.config, request, Utc::now())?;
        let id = record.id.clone();
        // Prepared under the locks: the bytes the operation holds are what will be retired.
        if record.holds.as_ref() != Some(&saved.local) {
            operation::cancel_unstarted(&self.config, &id)?;
            return Err(ListError::Refused(CHANGED.into()));
        }
        let unresolved = |e: ListError| {
            ListError::Refused(format!("list operation {id} needs reconciliation: {e}"))
        };
        let outcome = helper::exchange(&self.config, &record.request).and_then(|reply| {
            if reply.ok {
                Ok(())
            } else if reply.code == "changed" {
                Err(ListError::Refused(CHANGED.into()))
            } else {
                Err(ListError::Refused(helper::refusal(&reply.code).into()))
            }
        });
        if let Err(error) = outcome {
            // The helper proves a refusal before any DELETE; anything else may have reached
            // the server and stays fenced.
            return match operation::cancel_unstarted(&self.config, &id) {
                Ok(()) => Err(error),
                Err(_) => Err(unresolved(error)),
            };
        }
        self.complete_delete(&id).map_err(unresolved)
    }

    /// Reconcile an unfinished `delete`: read-only against the server, never another DELETE.
    pub(super) fn reconcile_delete(
        &mut self,
        record: &ListOperation,
    ) -> Result<ListResult, ListError> {
        let id = record.id.as_str();
        if record.phase == Phase::RemoteConfirmed {
            return self.complete_delete(id);
        }
        if operation::cancel_unstarted(&self.config, id).is_ok() {
            return Err(ListError::Refused(
                "the list deletion was not sent; preview it again if you still want to delete the list".into(),
            ));
        }
        let mut request = record.request.clone();
        request.mode = "reconcile".into();
        let reply = helper::exchange(&self.config, &request).map_err(|e| {
            ListError::Refused(format!("list operation {id} remains unresolved: {e}"))
        })?;
        if reply.ok {
            return self.complete_delete(id);
        }
        let message = match reply.code.as_str() {
            "present" => {
                "the list still exists on the server; delete it again if you still want to"
            }
            "changed" => CHANGED,
            code => {
                return Err(ListError::Refused(format!(
                    "list operation {id} remains unresolved: {}",
                    helper::refusal(code)
                )));
            }
        };
        operation::release_present(&self.config, id)?;
        Err(ListError::Refused(message.into()))
    }

    /// The server deletion is confirmed: retire the local copy, then release the fence.
    /// Safe to repeat after an interruption at any point.
    fn complete_delete(&mut self, id: &str) -> Result<ListResult, ListError> {
        let state = bind_vdir_state(&self.config.state_dir(), &self.config.vdir)?;
        let mut record = operation::read(&state, id)?;
        let collection = record
            .request
            .collection
            .clone()
            .filter(|c| plain_id(c))
            .ok_or_else(|| invalid("missing collection identity"))?;
        if record.phase != Phase::RemoteConfirmed
            || record.receipt.as_ref().and_then(|r| r.get("deleted")) != Some(&true.into())
        {
            return Err(ListError::Refused(
                "list deletion is not confirmed on the server".into(),
            ));
        }
        let saved = read(
            &state,
            record
                .request
                .backup
                .as_deref()
                .ok_or_else(|| invalid("missing deletion backup"))?,
        )?;
        // Once retired the directory is gone, which is how a repeat knows to skip this.
        if record.retired.is_none() && self.config.vdir.join(&collection).try_exists()? {
            let retired = crate::retired::forget_fenced(
                &self.config,
                &crate::retired::Preview {
                    collection,
                    name: saved.name.clone(),
                    tasks: saved.local.keys().filter(|f| f.ends_with(".ics")).count(),
                    pending: 0,
                    files: saved.local.clone(),
                },
                Utc::now(),
            )?;
            // Recorded before the fence is released, so the result names this exact copy.
            record.retired = Some(retired.id);
            operation::save(&state, &record)?;
        }
        operation::finish_deleted(&self.config, &mut record)?;
        self.deleted(&record)
    }

    pub(super) fn deleted(&self, record: &ListOperation) -> Result<ListResult, ListError> {
        let collection = record
            .request
            .collection
            .clone()
            .ok_or_else(|| invalid("missing collection identity"))?;
        let backup = record
            .request
            .backup
            .clone()
            .ok_or_else(|| invalid("missing deletion backup"))?;
        let saved = get(&self.config, &backup)?;
        let retired = match &record.retired {
            Some(id) => crate::retired::get(&self.config, id)?,
            // Interrupted between retiring the copy and recording it: the newest
            // retirement since this operation began is the one.
            None => crate::retired::list(&self.config)?
                .into_iter()
                .filter(|b| b.collection == collection && b.retired && b.created >= record.created)
                .max_by_key(|b| b.created)
                .ok_or_else(|| invalid("the retired local copy is missing"))?,
        };
        Ok(ListResult::Deleted {
            operation: record.id.clone(),
            collection,
            name: saved.name,
            backup,
            retired: retired.id,
        })
    }
}

const CHANGED: &str = "the list changed since the backup; review deletion again";
