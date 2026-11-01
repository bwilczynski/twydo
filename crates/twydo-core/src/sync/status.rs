//! Sync status (`<state>/sync/status.json`), backoff, and per-collection states.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::hook::HookRecord;
use super::output::SyncOutput;
use crate::store::fsio::write_atomic;

/// How one collection came out of the last run.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum CollState {
    Ok,
    Conflict,
    EmptiedLocal,
    EmptiedRemote,
    Error { message: String },
}

/// What the runner remembers between runs.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SyncStatus {
    pub last_attempt: Option<DateTime<Utc>>,
    pub last_success: Option<DateTime<Utc>>,
    pub failures: u32,
    pub next_attempt: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub collections: BTreeMap<String, CollState>,
    pub metasync_error: Option<String>,
}

fn status_path(state: &Path) -> PathBuf {
    state.join("sync").join("status.json")
}

impl SyncStatus {
    /// Reads `<state>/sync/status.json`; a missing file is the default (never synced).
    pub fn load(state: &Path) -> io::Result<SyncStatus> {
        let path = status_path(state);
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}: {e}", path.display()),
                )
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(SyncStatus::default()),
            Err(e) => Err(e),
        }
    }

    /// Writes `<state>/sync/status.json` atomically as pretty JSON.
    pub fn save(&self, state: &Path) -> io::Result<()> {
        let path = status_path(state);
        fs::create_dir_all(state.join("sync"))?;
        let mut json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        json.push('\n');
        write_atomic(&path, json.as_bytes())
    }
}

/// Delay before the next background attempt after `failures` transient failures in a row:
/// none for 0, else 60 s · 2^(failures − 1), capped at `max`.
pub fn backoff(failures: u32, max: Duration) -> Duration {
    if failures == 0 {
        return Duration::ZERO;
    }
    let factor = 1u64.checked_shl(failures - 1).unwrap_or(u64::MAX);
    Duration::from_secs(60u64.saturating_mul(factor)).min(max)
}

/// Error lines that belong to a conflict or an emptied collection rather than a failure.
fn explained(line: &str) -> bool {
    is_conflict_line(line)
        || line.starts_with("See also ")
        || line.starts_with("Item ID: ")
        || line.starts_with("Item href on side ")
        || (line.starts_with("Storage \"") && line.contains("\" was completely emptied"))
}

fn is_conflict_line(line: &str) -> bool {
    line.starts_with("One item changed on both sides")
}

/// Classifies every collection seen in `out` or `hook`:
/// `Error` when it has an error line that is not part of a conflict or emptied report;
/// `Conflict` when the hook log has an unresolved record for it or vdirsyncer reported an
/// unhooked conflict; `EmptiedLocal`/`EmptiedRemote` when a side was emptied (local when
/// that side is `local_storage`); otherwise `Ok`.
pub fn coll_states(
    out: &SyncOutput,
    hook: &[HookRecord],
    local_storage: &str,
) -> BTreeMap<String, CollState> {
    let names: BTreeSet<&String> = out
        .synced
        .iter()
        .chain(out.errors.keys())
        .chain(out.emptied.keys())
        .chain(out.unhooked.keys())
        .chain(hook.iter().map(|r| &r.collection))
        .collect();
    names
        .into_iter()
        .map(|c| {
            let errors = out.errors.get(c).map(Vec::as_slice).unwrap_or_default();
            let state = if let Some(e) = errors.iter().find(|l| !explained(l)) {
                CollState::Error { message: e.clone() }
            } else if out.unhooked.contains_key(c)
                || errors.iter().any(|l| is_conflict_line(l))
                || hook.iter().any(|r| &r.collection == c && !r.resolved)
            {
                CollState::Conflict
            } else if let Some(storage) = out.emptied.get(c) {
                if storage == local_storage {
                    CollState::EmptiedLocal
                } else {
                    CollState::EmptiedRemote
                }
            } else {
                CollState::Ok
            };
            (c.clone(), state)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::output::parse;

    fn fixture(name: &str) -> String {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/vdirsyncer")
            .join(name);
        fs::read_to_string(p).unwrap()
    }

    fn states(name: &str, hook: &[HookRecord]) -> BTreeMap<String, CollState> {
        coll_states(&parse("twydo", &fixture(name)), hook, "twydo_local")
    }

    fn rec(coll: &str, resolved: bool) -> HookRecord {
        HookRecord {
            uid: "u1".into(),
            collection: coll.into(),
            conflict: "id".into(),
            resolved,
        }
    }

    #[test]
    fn clean_is_ok() {
        let s = states("clean.txt", &[]);
        assert_eq!(s["c1"], CollState::Ok);
        assert_eq!(s["Twydo test"], CollState::Ok);
    }

    #[test]
    fn unhooked_conflict_is_conflict_not_error() {
        assert_eq!(
            states("no-hook-conflict.txt", &[])["c1"],
            CollState::Conflict
        );
    }

    #[test]
    fn hook_records_decide_conflict() {
        let s = states("hook-unresolved.txt", &[rec("c1", false)]);
        assert_eq!(s["c1"], CollState::Conflict);
        let s = states("hook-resolved.txt", &[rec("c1", true)]);
        assert_eq!(s["c1"], CollState::Ok);
    }

    #[test]
    fn emptied_local_and_remote() {
        assert_eq!(
            states("emptied-local.txt", &[])["c1"],
            CollState::EmptiedLocal
        );
        let s = coll_states(
            &parse("twydo", &fixture("emptied-local.txt")),
            &[],
            "other_local",
        );
        assert_eq!(s["c1"], CollState::EmptiedRemote);
    }

    #[test]
    fn failed_hook_is_error() {
        match &states("hook-failed.txt", &[])["c1"] {
            CollState::Error { message } => {
                assert!(message.starts_with("Unknown error occurred"), "{message}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn serializes_tagged_kebab_case() {
        let json = serde_json::to_string(&CollState::EmptiedRemote).unwrap();
        assert_eq!(json, r#"{"state":"emptied-remote"}"#);
        let json = serde_json::to_string(&CollState::Error {
            message: "x".into(),
        })
        .unwrap();
        assert_eq!(json, r#"{"state":"error","message":"x"}"#);
    }
}
