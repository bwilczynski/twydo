//! Raw collection previews. No DELETE is issued until a server guard is demonstrated.
use super::{
    ListError, ListService,
    helper::{self, HelperRequest},
    invalid,
    operation::{self, Phase},
    plain_id,
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

    pub fn delete_list(
        &mut self,
        _preview: &DeletePreview,
    ) -> Result<super::ListResult, ListError> {
        Err(ListError::Refused("List deletion is unavailable: a server-enforced collection precondition has not been demonstrated. The preserved backup remains available; no DELETE was sent.".into()))
    }
}
