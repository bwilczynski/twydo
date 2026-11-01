//! The vdirsyncer conflict hook (`tdo conflicts hook <A> <B>`).
//!
//! vdirsyncer calls it with `conflict_resolution = ["command", "tdo", "conflicts", "hook"]`
//! as `tdo conflicts hook <tmp>/<storage a> <tmp>/<storage b>`, each file holding one side's
//! version of an item changed on both sides. If the two files are byte-equal when the hook
//! exits 0, vdirsyncer writes that content to both sides; otherwise it leaves the item alone
//! and reports an error. A non-zero exit leaves both sides untouched.
//!
//! The hook never picks a winner by itself: it records both versions in the conflict store,
//! and only once the user has chosen a side does it copy that version over the other file.
//! Marking a conflict resolved is the sync runner's job, from the hook log.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::conflicts::{ConflictKind, ConflictState, ConflictStore, NewConflict, Side};
use crate::ical;
use crate::model::tasks_from_calendar;
use crate::store::bind_vdir_state;
use crate::store::fsio::VdirLock;
use crate::vdir;

/// Environment variable naming the hook log; the runner sets it for vdirsyncer.
pub const HOOK_LOG_ENV: &str = "TWYDO_SYNC_HOOKLOG";

/// One hook call, as one JSON line of the hook log.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct HookRecord {
    pub uid: String,
    pub collection: String,
    /// Conflict id.
    pub conflict: String,
    /// True when the hook made the two files equal (applied the user's choice).
    pub resolved: bool,
}

#[derive(Debug, PartialEq)]
pub enum HookOutcome {
    /// A new conflict was recorded (or a different unresolved one superseded).
    Recorded(String),
    /// The chosen side was copied over the other file.
    Applied(Side),
    /// The conflict is already recorded and still open.
    Unchanged(String),
}

#[derive(thiserror::Error, Debug)]
pub enum HookError {
    #[error("sync is not configured (no [sync] table in the config)")]
    NotConfigured,
    #[error(
        "cannot tell which file is the local side: exactly one of {} and {} must be named after sync.local_storage",
        .0.display(), .1.display()
    )]
    UnknownSide(PathBuf, PathBuf),
    #[error("no single vdir file holds task {0}")]
    NoLocalFile(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

fn invalid_data(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

fn read_text(path: &Path) -> io::Result<String> {
    let bytes = fs::read(path)?;
    String::from_utf8(bytes).map_err(|_| invalid_data(format!("{}: not UTF-8", path.display())))
}

pub(crate) fn uid_of(text: &str, path: &Path) -> io::Result<String> {
    let cal = ical::parse(text.as_bytes())
        .map_err(|e| invalid_data(format!("{}: {e}", path.display())))?;
    tasks_from_calendar(&cal, "", path)
        .into_iter()
        .map(|t| t.uid)
        .find(|u| !u.is_empty())
        .ok_or_else(|| invalid_data(format!("{}: no task UID", path.display())))
}

/// The vdir file of the task: the one whose bytes equal `local`, else the single file holding
/// `uid`. Returns the vdir-relative path and the collection.
fn find_local(root: &Path, uid: &str, local: &str) -> Result<(String, String), HookError> {
    let report = vdir::load(root)?;
    let by_bytes: Vec<&PathBuf> = report
        .sources
        .iter()
        .filter(|(_, b)| b.as_slice() == local.as_bytes())
        .map(|(p, _)| p)
        .collect();
    let path = match by_bytes.as_slice() {
        [one] => (*one).clone(),
        _ => {
            let mut by_uid: Vec<&PathBuf> = report
                .tasks
                .iter()
                .filter(|t| t.uid == uid)
                .map(|t| &t.path)
                .collect();
            by_uid.dedup();
            match by_uid.as_slice() {
                [one] => (*one).clone(),
                _ => return Err(HookError::NoLocalFile(uid.to_owned())),
            }
        }
    };
    let rel = path
        .strip_prefix(root)
        .map_err(|_| HookError::NoLocalFile(uid.to_owned()))?;
    let collection = match rel.parent().and_then(|p| p.to_str()) {
        Some(c) if !c.is_empty() => c.to_owned(),
        _ => return Err(HookError::NoLocalFile(uid.to_owned())),
    };
    let rel = rel
        .to_str()
        .ok_or_else(|| invalid_data(format!("{}: path is not UTF-8", rel.display())))?;
    Ok((rel.to_owned(), collection))
}

/// Handles one conflicted item; see the module docs. `a` and `b` are vdirsyncer's temp files,
/// one named after `sync.local_storage`.
pub fn conflict_hook(
    cfg: &Config,
    a: &Path,
    b: &Path,
    now: DateTime<Utc>,
) -> Result<HookOutcome, HookError> {
    let sync = cfg.sync.as_ref().ok_or(HookError::NotConfigured)?;
    let is_local = |p: &Path| {
        p.file_name()
            .is_some_and(|n| n == sync.local_storage.as_str())
    };
    let (local_path, remote_path) = match (is_local(a), is_local(b)) {
        (true, false) => (a, b),
        (false, true) => (b, a),
        _ => return Err(HookError::UnknownSide(a.to_owned(), b.to_owned())),
    };
    let local = read_text(local_path)?;
    let remote = read_text(remote_path)?;
    let uid = uid_of(&local, local_path)?;

    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir)?;
    let (path, collection) = {
        // The sync runner holds the shared lock too; a shared lock never waits on it.
        let _lock = VdirLock::shared(&state)?;
        find_local(&cfg.vdir, &uid, &local)?
    };
    let store = ConflictStore::open(&state)?;
    let held = store.unresolved()?.into_iter().rev().find(|c| {
        c.path == path
            && c.kind == ConflictKind::BothChanged
            && c.local.as_deref() == Some(local.as_str())
            && c.remote.as_deref() == Some(remote.as_str())
    });
    let (outcome, conflict, resolved) = match held.map(|c| (c.state, c.choice, c.id)) {
        Some((ConflictState::Chosen, Some(side), id)) => {
            // vdirsyncer writes the two files' (now equal) content to whichever side differs.
            match side {
                Side::Local => fs::write(remote_path, &local)?,
                Side::Remote => fs::write(local_path, &remote)?,
            }
            (HookOutcome::Applied(side), id, true)
        }
        Some((ConflictState::Open, _, id)) => (HookOutcome::Unchanged(id.clone()), id, false),
        // No conflict with these versions, so the versions changed since: record anew, which
        // supersedes the older one (a choice made for other versions no longer applies).
        _ => {
            let c = store.record(
                NewConflict {
                    uid: uid.clone(),
                    collection: collection.clone(),
                    path,
                    kind: ConflictKind::BothChanged,
                    held: true,
                    vdir_holds: Side::Local,
                    local: Some(local),
                    remote: Some(remote),
                },
                now,
            )?;
            (HookOutcome::Recorded(c.id.clone()), c.id, false)
        }
    };
    if let Some(log) = std::env::var_os(HOOK_LOG_ENV).filter(|v| !v.is_empty()) {
        append_hook_log(
            Path::new(&log),
            &HookRecord {
                uid,
                collection,
                conflict,
                resolved,
            },
        )?;
    }
    Ok(outcome)
}

fn append_hook_log(path: &Path, rec: &HookRecord) -> io::Result<()> {
    let mut line = serde_json::to_string(rec).map_err(io::Error::other)?;
    line.push('\n');
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(line.as_bytes())?;
    f.sync_data()
}

/// Reads the hook log. A missing file is an empty log; blank lines are skipped.
pub fn read_hook_log(path: &Path) -> io::Result<Vec<HookRecord>> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str(l).map_err(|e| invalid_data(format!("{}: {e}", path.display())))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SyncConfig;

    const NOW: &str = "2026-11-01T10:00:00Z";

    fn now() -> DateTime<Utc> {
        NOW.parse().unwrap()
    }

    fn task(uid: &str, summary: &str) -> String {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\nBEGIN:VTODO\r\n\
             UID:{uid}\r\nSUMMARY:{summary}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
        )
    }

    struct Fx {
        dir: tempfile::TempDir,
        cfg: Config,
    }

    impl Fx {
        fn new() -> Fx {
            let dir = tempfile::tempdir().unwrap();
            let vdir = dir.path().join("vdir");
            fs::create_dir_all(vdir.join("c1")).unwrap();
            let mut cfg = Config::with_vdir(vdir);
            cfg.state_dir = Some(dir.path().join("state"));
            cfg.sync = Some(SyncConfig::default());
            Fx { dir, cfg }
        }

        fn tmp(&self, name: &str, content: &str) -> PathBuf {
            let p = self.dir.path().join(name);
            fs::write(&p, content).unwrap();
            p
        }

        fn store(&self) -> ConflictStore {
            let s = bind_vdir_state(&self.cfg.state_dir(), &self.cfg.vdir).unwrap();
            ConflictStore::open(&s).unwrap()
        }
    }

    #[test]
    fn not_configured_without_sync() {
        let mut fx = Fx::new();
        fx.cfg.sync = None;
        let a = fx.tmp("twydo_remote", &task("u1", "r"));
        let b = fx.tmp("twydo_local", &task("u1", "l"));
        assert!(matches!(
            conflict_hook(&fx.cfg, &a, &b, now()),
            Err(HookError::NotConfigured)
        ));
    }

    #[test]
    fn unknown_side_when_both_or_neither_match() {
        let fx = Fx::new();
        let a = fx.tmp("x", &task("u1", "r"));
        let b = fx.tmp("y", &task("u1", "l"));
        assert!(matches!(
            conflict_hook(&fx.cfg, &a, &b, now()),
            Err(HookError::UnknownSide(..))
        ));
        fs::create_dir_all(fx.dir.path().join("d")).unwrap();
        let c = fx.tmp("d/twydo_local", &task("u1", "r"));
        let d = fx.tmp("twydo_local", &task("u1", "l"));
        assert!(matches!(
            conflict_hook(&fx.cfg, &c, &d, now()),
            Err(HookError::UnknownSide(..))
        ));
    }

    #[test]
    fn no_local_file_when_vdir_lacks_the_task() {
        let fx = Fx::new();
        let a = fx.tmp("twydo_remote", &task("u1", "r"));
        let b = fx.tmp("twydo_local", &task("u1", "l"));
        match conflict_hook(&fx.cfg, &a, &b, now()) {
            Err(HookError::NoLocalFile(uid)) => assert_eq!(uid, "u1"),
            other => panic!("{other:?}"),
        }
        assert!(fx.store().list().unwrap().is_empty());
    }

    #[test]
    fn falls_back_to_uid_when_bytes_differ() {
        let fx = Fx::new();
        fs::write(fx.cfg.vdir.join("c1/x.ics"), task("u1", "on disk")).unwrap();
        let a = fx.tmp("twydo_local", &task("u1", "l"));
        let b = fx.tmp("twydo_remote", &task("u1", "r"));
        let HookOutcome::Recorded(id) = conflict_hook(&fx.cfg, &a, &b, now()).unwrap() else {
            panic!("not recorded");
        };
        let c = fx.store().get(&id).unwrap().unwrap();
        assert_eq!(c.path, "c1/x.ics");
        assert_eq!(c.collection, "c1");
        assert_eq!(c.local.as_deref(), Some(task("u1", "l").as_str()));
        assert_eq!(c.remote.as_deref(), Some(task("u1", "r").as_str()));
        // Same versions again: unchanged.
        assert_eq!(
            conflict_hook(&fx.cfg, &a, &b, now()).unwrap(),
            HookOutcome::Unchanged(id)
        );
    }

    #[test]
    fn read_hook_log_missing_is_empty_and_parses_lines() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("hook.log");
        assert!(read_hook_log(&p).unwrap().is_empty());
        let rec = HookRecord {
            uid: "u1".into(),
            collection: "c1".into(),
            conflict: "id1".into(),
            resolved: true,
        };
        append_hook_log(&p, &rec).unwrap();
        append_hook_log(&p, &rec).unwrap();
        let read = read_hook_log(&p).unwrap();
        assert_eq!(read.len(), 2);
        assert!(read.iter().all(|r| *r == rec));
    }
}
