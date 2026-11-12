//! Explicitly retiring a remotely deleted list, with inspectable local backups.
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::conflicts::ConflictStore;
use crate::store::fsio::{VdirLock, sync_dir, write_atomic};
use crate::store::journal::{Entry, Journal, new_op_id, recover_locked};
use crate::store::{bind_vdir_state, existing_vdir_state};
use crate::sync::run::run_lock;
use crate::vdir;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub collection: String,
    pub name: String,
    pub tasks: usize,
    pub pending: usize,
    pub files: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backup {
    pub format: u32,
    pub id: String,
    pub collection: String,
    pub name: String,
    pub created: DateTime<Utc>,
    pub tasks: usize,
    pub pending: usize,
    pub files: Vec<String>,
    pub journal: Vec<Entry>,
    pub conflicts: Vec<String>,
    pub retired: bool,
    pub recovery: Option<Entry>,
    pub destination: Option<String>,
}

pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

pub(crate) fn plain(s: &str) -> bool {
    !s.is_empty()
        && Path::new(s).components().count() == 1
        && Path::new(s)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

fn relative(s: &str) -> bool {
    !s.is_empty()
        && Path::new(s)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

fn root(state: &Path) -> PathBuf {
    state.join("retired")
}
fn record(state: &Path, id: &str) -> io::Result<PathBuf> {
    if !plain(id) || id.starts_with('.') {
        return Err(invalid("invalid backup id"));
    }
    Ok(root(state).join(id))
}

fn private_dir(path: &Path) -> io::Result<()> {
    if let Ok(m) = fs::symlink_metadata(path)
        && (!m.is_dir() || m.is_symlink())
    {
        return Err(invalid("backup path is not a plain directory"));
    }
    let mut b = fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(path)
}

/// Reads a tree without following symlinks; used both at confirmation and commit.
fn tree(dir: &Path) -> io::Result<BTreeMap<String, Vec<u8>>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) -> io::Result<()> {
        let m = fs::symlink_metadata(dir)?;
        if !m.is_dir() || m.is_symlink() {
            return Err(invalid("list/backup contains a symlink or special file"));
        }
        for item in fs::read_dir(dir)? {
            let p = item?.path();
            let m = fs::symlink_metadata(&p)?;
            if m.is_symlink() {
                return Err(invalid(format!("refusing symlink {}", p.display())));
            }
            if m.is_dir() {
                walk(base, &p, out)?;
            } else if m.is_file() {
                let rel = p
                    .strip_prefix(base)
                    .unwrap()
                    .to_str()
                    .ok_or_else(|| invalid("non-UTF-8 filename"))?
                    .to_string();
                out.insert(rel, fs::read(&p)?);
            } else {
                return Err(invalid("list contains a special file"));
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out)?;
    Ok(out)
}

fn copy_tree(dir: &Path, files: &BTreeMap<String, Vec<u8>>) -> io::Result<()> {
    private_dir(dir)?;
    for (name, bytes) in files {
        if !relative(name) {
            return Err(invalid("invalid backup file path"));
        }
        let path = dir.join(name);
        private_dir(path.parent().unwrap())?;
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        sync_dir(path.parent().unwrap())?;
    }
    sync_dir(dir)
}

/// Persist every directory entry bottom-up before publishing the backup. Syncing a
/// child directory does not persist the entry naming it in its parent.
fn sync_tree(dir: &Path) -> io::Result<()> {
    for item in fs::read_dir(dir)? {
        let item = item?;
        let meta = item.file_type()?;
        if meta.is_symlink() {
            return Err(invalid("backup contains a symlink"));
        }
        if meta.is_dir() {
            sync_tree(&item.path())?;
        }
    }
    sync_dir(dir)
}

pub(crate) fn save(state: &Path, b: &Backup) -> io::Result<()> {
    write_atomic(
        &record(state, &b.id)?.join("backup.json"),
        &serde_json::to_vec_pretty(b).map_err(io::Error::other)?,
    )
}

pub(crate) fn read(state: &Path, id: &str) -> io::Result<Backup> {
    let dir = record(state, id)?;
    let m = fs::symlink_metadata(&dir)?;
    if !m.is_dir() || m.is_symlink() {
        return Err(invalid("invalid backup directory"));
    }
    let b: Backup =
        serde_json::from_slice(&fs::read(dir.join("backup.json"))?).map_err(io::Error::other)?;
    if b.format != 1 || b.id != id || !plain(&b.collection) || b.files.iter().any(|f| !relative(f))
    {
        return Err(invalid("invalid backup manifest"));
    }
    Ok(b)
}

pub(crate) fn list_state(state: &Path) -> io::Result<Vec<Backup>> {
    let dir = root(state);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for item in fs::read_dir(dir)? {
        let item = item?;
        let name = item
            .file_name()
            .into_string()
            .map_err(|_| invalid("invalid backup name"))?;
        if name.starts_with('.') {
            continue;
        }
        out.push(read(state, &name)?);
    }
    out.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(out)
}

pub fn list(cfg: &Config) -> io::Result<Vec<Backup>> {
    match existing_vdir_state(&cfg.state_dir(), &cfg.vdir)? {
        Some(state) => list_state(&state),
        None => Ok(Vec::new()),
    }
}
pub fn get(cfg: &Config, id: &str) -> io::Result<Backup> {
    let state = existing_vdir_state(&cfg.state_dir(), &cfg.vdir)?
        .ok_or_else(|| invalid("no preserved lists"))?;
    read(&state, id)
}
pub fn data_dir(cfg: &Config, id: &str) -> io::Result<PathBuf> {
    let state = existing_vdir_state(&cfg.state_dir(), &cfg.vdir)?
        .ok_or_else(|| invalid("no preserved lists"))?;
    read(&state, id)?;
    Ok(record(&state, id)?.join("data"))
}

pub fn collection_id(cfg: &Config, name: &str) -> io::Result<String> {
    let report = vdir::load(&cfg.vdir)?;
    if plain(name) && report.collections.iter().any(|c| c.id == name) {
        return Ok(name.to_string());
    }
    let ids: Vec<_> = report
        .collections
        .iter()
        .filter(|c| c.display_name.as_deref() == Some(name))
        .collect();
    match ids.as_slice() {
        [c] => Ok(c.id.clone()),
        [] => Err(invalid(format!("no list {name:?}"))),
        _ => Err(invalid(format!(
            "list name {name:?} is ambiguous; use its collection id"
        ))),
    }
}

pub fn preview(cfg: &Config, name: &str) -> io::Result<Preview> {
    let collection = collection_id(cfg, name)?;
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir)?;
    let _lock = VdirLock::shared(&state)?;
    let files = tree(&cfg.vdir.join(&collection))?;
    let report = vdir::load(&cfg.vdir)?;
    let name = report
        .collections
        .iter()
        .find(|c| c.id == collection)
        .and_then(|c| c.display_name.clone())
        .unwrap_or_else(|| collection.clone());
    let pending = Journal::open(&state)?
        .pending()?
        .iter()
        .filter(|e| e.writes.iter().any(|w| in_collection(&w.path, &collection)))
        .count();
    let tasks = files.keys().filter(|f| f.ends_with(".ics")).count();
    Ok(Preview {
        collection,
        name,
        tasks,
        pending,
        files,
    })
}

pub(crate) fn in_collection(path: &str, collection: &str) -> bool {
    Path::new(path).parent() == Some(Path::new(collection))
}

pub fn forget(cfg: &Config, p: &Preview, now: DateTime<Utc>) -> io::Result<Backup> {
    retire(cfg, p, now, true)
}

/// [`forget`] for a list the server has just deleted, under that deletion's operation fence.
/// Journal recovery refuses to run while the fence is up, and the operation was prepared
/// with no pending task writes for the list, so it is skipped here.
pub(crate) fn forget_fenced(cfg: &Config, p: &Preview, now: DateTime<Utc>) -> io::Result<Backup> {
    retire(cfg, p, now, false)
}

fn retire(cfg: &Config, p: &Preview, now: DateTime<Utc>, recover: bool) -> io::Result<Backup> {
    if !plain(&p.collection) {
        return Err(invalid("invalid collection id"));
    }
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir)?;
    private_dir(&state.join("sync"))?;
    let _run = run_lock(&state.join("sync"), false)?;
    let _lock = VdirLock::exclusive(&state)?;
    let journal = Journal::open(&state)?;
    let recovered = if recover {
        recover_locked(&cfg.vdir, &journal)?
    } else {
        Default::default()
    };
    if !recovered.stuck.is_empty() {
        return Err(invalid(
            "unfinished journal operations must be resolved first",
        ));
    }
    if tree(&cfg.vdir.join(&p.collection))? != p.files {
        return Err(invalid(
            "the list changed since confirmation; reload and try again",
        ));
    }
    private_dir(&root(&state))?;
    // Persist the new `retired` entry in the state directory as well as the
    // backup's entries within it.
    sync_dir(&state)?;
    if fs::canonicalize(root(&state))?.starts_with(fs::canonicalize(&cfg.vdir)?) {
        return Err(invalid("backups must be outside the synchronized vdir"));
    }
    let id = new_op_id(now);
    let building = root(&state).join(format!(".{id}"));
    private_dir(&building)?;
    copy_tree(&building.join("data").join(&p.collection), &p.files)?;
    let entries: Vec<_> = journal
        .list()?
        .into_iter()
        .filter(|e| {
            e.writes
                .iter()
                .any(|w| in_collection(&w.path, &p.collection))
                || e.holds
                    .iter()
                    .any(|(path, _)| in_collection(path, &p.collection))
                || e.family
                    .as_ref()
                    .is_some_and(|f| f.collection == p.collection)
        })
        .collect();
    let conflicts = match ConflictStore::existing(&state)? {
        Some(s) => s.list()?,
        None => Vec::new(),
    };
    let mut conflict_ids = Vec::new();
    for c in conflicts.iter().filter(|c| c.collection == p.collection) {
        let files = tree(&state.join("conflicts").join(&c.id))?;
        copy_tree(&building.join("conflicts").join(&c.id), &files)?;
        conflict_ids.push(c.id.clone());
    }
    let b = Backup {
        format: 1,
        id: id.clone(),
        collection: p.collection.clone(),
        name: p.name.clone(),
        created: now,
        tasks: p.tasks,
        pending: p.pending,
        files: p.files.keys().cloned().collect(),
        journal: entries,
        conflicts: conflict_ids,
        retired: false,
        recovery: None,
        destination: None,
    };
    write_atomic(
        &building.join("backup.json"),
        &serde_json::to_vec_pretty(&b).map_err(io::Error::other)?,
    )?;
    sync_tree(&building)?;
    fs::rename(&building, record(&state, &id)?)?;
    sync_dir(&root(&state))?;
    resume_locked(&cfg.vdir, &state)?;
    read(&state, &id)
}

/// Finish manifest-backed retirement before journal recovery can recreate the source.
/// The caller holds the exclusive vdir lock.
pub(crate) fn resume_locked(vdir: &Path, state: &Path) -> io::Result<()> {
    for mut b in list_state(state)? {
        if b.retired {
            continue;
        }
        let source = vdir.join(&b.collection);
        if source.exists() {
            let archived = tree(&record(state, &b.id)?.join("data").join(&b.collection))?;
            if tree(&source)?
                .iter()
                .any(|(name, bytes)| archived.get(name) != Some(bytes))
            {
                return Err(invalid(
                    "list changed during retirement; preserved backup retained; refusing removal",
                ));
            }
        }
        // The marker survives a crash before cleanup or before ordinary discovery.
        private_dir(&state.join("sync"))?;
        write_atomic(&state.join("sync/rediscover"), b.id.as_bytes())?;
        Journal::open(state)?.retire_collection(&b.collection)?;
        if source.exists() {
            fs::remove_dir_all(&source)?;
            sync_dir(vdir)?;
        }
        b.retired = true;
        save(state, &b)?;
    }
    Ok(())
}

pub(crate) fn archived_conflict(state: &Path, id: &str) -> io::Result<bool> {
    Ok(list_state(state)?
        .iter()
        .any(|b| b.conflicts.iter().any(|c| c == id)))
}

/// The local snapshot only; historical conflict alternatives remain in the backup.
pub fn snapshot(cfg: &Config, id: &str) -> io::Result<crate::views::Snapshot> {
    let b = get(cfg, id)?;
    let root = data_dir(cfg, id)?;
    let files = tree(&root.join(&b.collection))?;
    if files.keys().cloned().collect::<Vec<_>>() != b.files {
        return Err(invalid("backup files no longer match its manifest"));
    }
    let mut report = vdir::load(&root)?;
    let rebase = |p: &Path| cfg.vdir.join(p.strip_prefix(&root).unwrap());
    for c in &mut report.collections {
        c.path = rebase(&c.path);
    }
    for t in &mut report.tasks {
        t.path = rebase(&t.path);
    }
    report.sources = report
        .sources
        .into_iter()
        .map(|(p, bytes)| (rebase(&p), bytes))
        .collect();
    Ok(crate::views::Snapshot::new(report, cfg.clone()))
}
