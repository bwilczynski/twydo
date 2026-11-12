//! Writing the config file: through a symlink (stow), atomically, and `~`-relative paths.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Follow `path` through symlinks (relative targets resolve against the link's directory).
fn resolve(path: &Path) -> io::Result<PathBuf> {
    let mut cur = path.to_path_buf();
    for _ in 0..40 {
        match fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => {
                let t = fs::read_link(&cur)?;
                cur = match cur.parent() {
                    Some(d) if t.is_relative() => d.join(t),
                    _ => t,
                };
            }
            _ => return Ok(cur),
        }
    }
    Err(io::Error::other("too many levels of symbolic links"))
}

/// Replace the file at `path` with `text` atomically (temp file in the same directory, then
/// rename). A symlink at `path` stays; its final target is replaced. The parent directory is
/// created when missing.
pub fn write_through(path: &Path, text: &str) -> io::Result<()> {
    let target = resolve(path)?;
    let dir = match target.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    fs::create_dir_all(&dir)?;
    let mode = fs::metadata(&target)
        .map(|m| m.permissions().mode() & 0o7777)
        .unwrap_or(0o644);
    let mut tmp = tempfile::NamedTempFile::new_in(&dir)?;
    tmp.write_all(text.as_bytes())?;
    tmp.as_file()
        .set_permissions(fs::Permissions::from_mode(mode))?;
    tmp.as_file().sync_all()?;
    tmp.persist(&target).map_err(|e| e.error)?;
    Ok(())
}

/// `~/rest` when `path` is under `home`, else the path as given.
pub fn home_relative(path: &Path, home: &Path) -> String {
    if !home.as_os_str().is_empty()
        && let Ok(rest) = path.strip_prefix(home)
    {
        return if rest.as_os_str().is_empty() {
            "~".to_string()
        } else {
            format!("~/{}", rest.display())
        };
    }
    path.display().to_string()
}
