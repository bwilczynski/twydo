//! Collection discovery: the lists on the server are listed here (never created), the ones
//! missing locally get an empty folder, then plain `vdirsyncer discover` refreshes its cache.
//! A list missing on the server is reported, never created.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::vdirsyncer::{Control, Outcome, TEST_REMOTE_ENV, Uncontrolled, Vdirsyncer, ensure};
use crate::config::Config;
use crate::lists::call::wording;
use crate::lists::dav::connect;

/// What a successful discovery changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    /// Collection ids newly created in the local vdir.
    pub adopted: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoverError {
    /// A local list whose collection is gone from the server (`:forget-list` it).
    Missing {
        id: String,
    },
    /// vdirsyncer cannot be started: a setup problem, not a verdict on the lists.
    Unavailable(String),
    /// Killed at its deadline; the marker-driven discovery is retried.
    TimedOut(String),
    /// Stopped on request (`sync/stop`, a daemon shutdown); not a failure.
    Stopped,
    Failed(String),
}

impl DiscoverError {
    /// The one wording of the failure. `name` is the missing list's display name, when known.
    pub fn describe(&self, name: Option<&str>) -> String {
        match self {
            Self::Missing { id } => format!(
                "list discovery stopped: list \"{}\" ({id}) no longer exists on the server; if you deleted it, run :forget-list {id}",
                name.unwrap_or(id)
            ),
            Self::Unavailable(m) => format!("list discovery unavailable: {m}"),
            Self::TimedOut(m) => m.clone(),
            Self::Stopped => "list discovery stopped".into(),
            Self::Failed(m) => format!("list discovery failed: {m}"),
        }
    }
}

impl std::fmt::Display for DiscoverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe(None))
    }
}

impl std::error::Error for DiscoverError {}

/// Refreshes vdirsyncer's collection cache. Callers hold the sync run lock and the shared
/// vdir lock (`state` is the vdir's state directory).
pub fn discover(cfg: &Config, state: &Path) -> Result<Discovered, DiscoverError> {
    let Some(sync) = cfg.sync.as_ref() else {
        return Err(DiscoverError::Failed(wording("config").into()));
    };
    let config = ensure(cfg, sync, state).map_err(|e| DiscoverError::Failed(e.to_string()))?;
    let vds = Vdirsyncer {
        sync,
        config: &config,
        vdir: &cfg.vdir,
        state,
        hook_log: None,
    };
    discover_watched(cfg, &vds, &Uncontrolled)
}

/// A stop request, as discovery's own checks see it: an unreadable request counts as none.
fn stopped(control: &dyn Control) -> bool {
    control.stop_requested().unwrap_or(false)
}

pub(crate) fn discover_watched(
    cfg: &Config,
    vds: &Vdirsyncer<'_>,
    control: &dyn Control,
) -> Result<Discovered, DiscoverError> {
    if stopped(control) {
        return Err(DiscoverError::Stopped);
    }
    if std::env::var_os(TEST_REMOTE_ENV).is_some_and(|v| !v.is_empty()) {
        // The test remote is a directory, not a CalDAV server: nothing to list.
        return Err(DiscoverError::Failed(wording("mapping").into()));
    }
    let refused = |r: crate::lists::call::Refused| match r.code {
        "timeout" => DiscoverError::TimedOut("list discovery timed out".into()),
        code => DiscoverError::Failed(wording(code).into()),
    };
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(vds.sync.timeout_secs))
        .unwrap_or_else(|| Instant::now() + Duration::from_secs(365 * 24 * 3600));
    let dav = connect(vds.sync, deadline).map_err(refused)?;
    let listed: BTreeSet<String> = dav
        .list_home()
        .map_err(refused)?
        .into_iter()
        .map(|l| l.id)
        .collect();
    let local = local_ids(&cfg.vdir);
    // A local list the server lacks usually means it was deleted elsewhere: report it, and
    // create nothing here or there.
    if let Some(id) = local.iter().find(|id| !listed.contains(*id)) {
        return Err(DiscoverError::Missing { id: id.clone() });
    }
    let mut adopted: Vec<PathBuf> = Vec::new();
    for id in listed.difference(&local) {
        let dir = cfg.vdir.join(id);
        if let Err(e) = std::fs::create_dir(&dir) {
            remove_adopted(&adopted);
            return Err(DiscoverError::Failed(format!(
                "cannot create the folder for a new list: {}",
                e.kind()
            )));
        }
        adopted.push(dir);
    }
    if stopped(control) {
        remove_adopted(&adopted);
        return Err(DiscoverError::Stopped);
    }
    let fail = |e: DiscoverError| {
        remove_adopted(&adopted);
        e
    };
    let ran = vds
        .discover(control)
        .map_err(|e| fail(DiscoverError::Failed(e.to_string())))?;
    match ran.outcome {
        Outcome::SpawnFailed(m) => return Err(fail(DiscoverError::Unavailable(m))),
        Outcome::Stopped => return Err(fail(DiscoverError::Stopped)),
        Outcome::TimedOut => {
            return Err(fail(DiscoverError::TimedOut(
                "list discovery timed out".into(),
            )));
        }
        Outcome::Exited(status) => {
            let logged = ran.stderr.lines().any(|l| {
                let l = l.trim_start();
                l.starts_with("error: ") || l.starts_with("critical: ")
            });
            if !status.success() || logged {
                return Err(fail(DiscoverError::Failed(
                    "vdirsyncer discover failed".into(),
                )));
            }
        }
    }
    Ok(Discovered {
        adopted: listed.difference(&local).cloned().collect(),
    })
}

/// The top-level folders of the vdir that name lists (not hidden).
fn local_ids(vdir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(vdir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .collect()
}

/// `rmdir` (never recursive) of the folders this run created.
fn remove_adopted(dirs: &[PathBuf]) {
    for dir in dirs {
        let _ = std::fs::remove_dir(dir);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::config::SyncConfig;
    use crate::sync::SyncError;

    /// Reports a stop from its `stop_at`th query on.
    struct StopAt {
        stop_at: usize,
        asked: Cell<usize>,
    }

    impl Control for StopAt {
        fn stop_requested(&self) -> Result<bool, SyncError> {
            self.asked.set(self.asked.get() + 1);
            Ok(self.asked.get() >= self.stop_at)
        }
    }

    fn run_with(
        stop_at: usize,
    ) -> (
        Result<Discovered, DiscoverError>,
        PathBuf,
        Vec<String>,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let server = twydo_testdav::FakeDav::start(&root);
        twydo_testdav::add_collection(&root, "phone-list", "Phone", "#112233");
        let vdir = root.join("vdir");
        std::fs::create_dir(&vdir).unwrap();
        let mut cfg = Config::with_vdir(vdir.clone());
        cfg.state_dir = Some(root.join("state"));
        let sync = SyncConfig {
            url: server.url(),
            username: "alice".into(),
            password_command: vec!["printf".into(), "secret".into()],
            command: twydo_testdav::fake_vdirsyncer(&root)
                .to_string_lossy()
                .into_owned(),
            timeout_secs: 30,
            ..SyncConfig::default()
        };
        let state = crate::store::bind_vdir_state(&cfg.state_dir(), &vdir).unwrap();
        let config = ensure(&cfg, &sync, &state).unwrap();
        let vds = Vdirsyncer {
            sync: &sync,
            config: &config,
            vdir: &vdir,
            state: &state,
            hook_log: None,
        };
        let control = StopAt {
            stop_at,
            asked: Cell::new(0),
        };
        let result = discover_watched(&cfg, &vds, &control);
        (result, vdir, twydo_testdav::calls(&root), dir)
    }

    #[test]
    fn a_stop_before_discover_adopts_nothing() {
        let (result, vdir, calls, _dir) = run_with(1);
        assert_eq!(result, Err(DiscoverError::Stopped));
        assert_eq!(std::fs::read_dir(&vdir).unwrap().count(), 0);
        assert!(calls.is_empty(), "{calls:?}");
    }

    #[test]
    fn a_stop_after_adopting_removes_the_adopted_folders() {
        let (result, vdir, calls, _dir) = run_with(2);
        assert_eq!(result, Err(DiscoverError::Stopped));
        assert_eq!(std::fs::read_dir(&vdir).unwrap().count(), 0);
        assert!(!calls.iter().any(|c| c == "DISCOVER"), "{calls:?}");
    }
}
