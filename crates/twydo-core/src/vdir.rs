//! Read-only loader for a vdirsyncer-style vdir: one directory per collection, one `.ics` per task.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::ical;
use crate::model::{Task, tasks_from_calendar};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collection {
    /// Directory name.
    pub id: String,
    pub path: PathBuf,
    pub display_name: Option<String>,
    pub color: Option<(u8, u8, u8)>,
}

#[derive(Debug, Default)]
pub struct LoadReport {
    pub collections: Vec<Collection>,
    pub tasks: Vec<Task>,
    /// Files that could not be read or parsed, with the reason.
    pub errors: Vec<(PathBuf, String)>,
    /// The bytes of every `.ics` file that was read, keyed by its path, so a writer can
    /// check that a file has not changed since it was loaded.
    pub sources: HashMap<PathBuf, Vec<u8>>,
}

/// Parse `#RRGGBB` or `#RRGGBBAA` (alpha ignored).
pub fn parse_color(s: &str) -> Option<(u8, u8, u8)> {
    let hex = s.trim().strip_prefix('#')?;
    if !(hex.len() == 6 || hex.len() == 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some((byte(0)?, byte(2)?, byte(4)?))
}

fn read_trimmed(path: &Path) -> Option<String> {
    let s = fs::read_to_string(path).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn is_hidden(name: &std::ffi::OsStr) -> bool {
    name.to_string_lossy().starts_with('.')
}

/// Sorted non-hidden entries of `dir` with their metadata, following symlinks.
/// A dangling symlink carries its error.
fn entries(dir: &Path) -> io::Result<Vec<(String, PathBuf, io::Result<fs::Metadata>)>> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir)? {
        let e = e?;
        let name = e.file_name();
        if is_hidden(&name) {
            continue;
        }
        let path = e.path();
        let meta = fs::metadata(&path);
        out.push((name.to_string_lossy().into_owned(), path, meta));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Load every collection under `root`. Fails only when `root` itself cannot be read.
/// Never writes: files are opened for reading only.
pub fn load(root: &Path) -> io::Result<LoadReport> {
    let mut report = LoadReport::default();
    for (id, dir, meta) in entries(root)? {
        match meta {
            Ok(m) if m.is_dir() => {}
            Ok(_) => continue,
            Err(e) => {
                report.errors.push((dir, e.to_string()));
                continue;
            }
        }
        report.collections.push(Collection {
            display_name: read_trimmed(&dir.join("displayname")),
            color: read_trimmed(&dir.join("color")).and_then(|c| parse_color(&c)),
            id: id.clone(),
            path: dir.clone(),
        });
        let files = match entries(&dir) {
            Ok(f) => f,
            Err(e) => {
                report.errors.push((dir.clone(), e.to_string()));
                continue;
            }
        };
        for (name, path, meta) in files {
            if !name.ends_with(".ics") {
                continue;
            }
            match meta {
                Ok(m) if m.is_file() => {}
                Ok(_) => continue,
                Err(e) => {
                    report.errors.push((path, e.to_string()));
                    continue;
                }
            }
            match fs::read(&path) {
                Err(e) => report.errors.push((path, e.to_string())),
                Ok(bytes) => {
                    match ical::parse(&bytes) {
                        Err(e) => report.errors.push((path.clone(), e.to_string())),
                        Ok(cal) => report.tasks.extend(tasks_from_calendar(&cal, &id, &path)),
                    }
                    report.sources.insert(path, bytes);
                }
            }
        }
    }
    flag_duplicate_uids(&mut report.tasks);
    Ok(report)
}

fn flag_duplicate_uids(tasks: &mut [Task]) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for t in tasks.iter().filter(|t| !t.uid.is_empty()) {
        *counts.entry(t.uid.clone()).or_default() += 1;
    }
    for t in tasks.iter_mut() {
        if counts.get(&t.uid).is_some_and(|&n| n > 1) {
            t.warnings.push("duplicate UID".to_string());
        }
    }
}
