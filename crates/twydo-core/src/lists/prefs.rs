//! Small, stale-checked config edits that retain comments and unrelated preferences.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use crate::config::Config;
use crate::store::bind_vdir_state;
use crate::store::fsio::{VdirLock, sync_dir};

use super::{invalid, plain_id};

pub fn set_archived(
    path: &Path,
    vdir: &Path,
    id: &str,
    archived: bool,
    expected_bytes: &[u8],
) -> io::Result<Config> {
    if !plain_id(id) {
        return Err(invalid("invalid list identity"));
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_symlink() || !metadata.is_file() {
        return Err(invalid("configuration must be a plain file"));
    }
    let config = Config::load(path).map_err(|e| invalid(e.to_string()))?;
    if fs::canonicalize(&config.vdir)? != fs::canonicalize(vdir)? {
        return Err(invalid("configuration belongs to a different vdir"));
    }
    let state = bind_vdir_state(&config.state_dir(), vdir)?;
    let _vdir_lock = VdirLock::exclusive(&state)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let lock_path = parent.join(".twydo-config.lock");
    if fs::symlink_metadata(&lock_path).is_ok_and(|m| m.is_symlink() || !m.is_file()) {
        return Err(invalid("configuration lock is not a plain file"));
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    lock.lock()?;
    let list_meta = fs::symlink_metadata(vdir.join(id))?;
    if list_meta.is_symlink() || !list_meta.is_dir() {
        return Err(invalid("list must be a plain directory"));
    }
    if fs::read(path)? != expected_bytes {
        return Err(invalid("configuration changed; reload and try again"));
    }
    let text = std::str::from_utf8(expected_bytes).map_err(|e| invalid(e.to_string()))?;
    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| invalid(e.to_string()))?;
    let item = &mut doc["lists"][id]["archived"];
    let decor = item.as_value().map(|v| v.decor().clone());
    *item = toml_edit::value(archived);
    if let Some(decor) = decor {
        *item.as_value_mut().expect("boolean value").decor_mut() = decor;
    }
    let output = doc.to_string();
    // Validate the full resulting configuration before replacing anything.
    toml::from_str::<Config>(&output).map_err(|e| invalid(e.to_string()))?;
    let tmp = parent.join(format!(".twydo-config-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.set_permissions(metadata.permissions())?;
        file.write_all(output.as_bytes())?;
        file.sync_all()?;
        if fs::read(path)? != expected_bytes || fs::symlink_metadata(path)?.is_symlink() {
            return Err(invalid("configuration changed; reload and try again"));
        }
        fs::rename(&tmp, path)?;
        sync_dir(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result?;
    Config::load(path).map_err(|e| invalid(e.to_string()))
}
