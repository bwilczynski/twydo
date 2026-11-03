//! The sync that is running now, and asking it to stop.
//!
//! While vdirsyncer runs, the runner keeps `<state>/sync/running.json` (see [`Running`]).
//! Another `tdo` asks it to stop by creating `<state>/sync/stop`: the runner polls for that
//! file, kills its child's process group and records the run as stopped. A runner that is
//! gone (killed, Ctrl-C) cannot do that, so [`request_stop`] then kills the recorded group
//! itself. No lock is taken: the requester is usually a writer waiting for the vdir lock.
//!
//! The request belongs to the requester, who withdraws it with [`clear_stop`] after its
//! write. Until then, any runner that reaches vdirsyncer stops before starting it, so a
//! runner that was waiting behind the stopped sync cannot take the vdir first. A request
//! older than [`STOP_FRESH`] is stale (its requester is gone) and is ignored and removed.
//!
//! Every request also records when it was made, in `<state>/sync/last_stop`, which nobody
//! withdraws: a pass begun before that time (one that was waiting for the vdir behind the
//! stopped sync) stops too, however late it gets there.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::store::existing_vdir_state;

/// `<state>/sync/running.json`, present while a runner has vdirsyncer running.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Running {
    pub runner_pid: u32,
    /// When the runner process started, as `ps -o lstart=` prints it: with the pid, its
    /// identity (a pid alone may belong to another process by now). None: unknown.
    #[serde(default)]
    pub runner_start: Option<String>,
    /// The current vdirsyncer child (it leads its own process group); None between calls.
    pub child_pgid: Option<u32>,
    /// When that child started (see `runner_start`).
    #[serde(default)]
    pub child_start: Option<String>,
    /// When the runner started vdirsyncer (wall clock).
    pub started: DateTime<Utc>,
}

/// How long a stop request holds off new syncs; older ones are stale.
pub const STOP_FRESH: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopResult {
    /// No `running.json`: nothing to stop (the request is still left, to hold off a pass
    /// that is about to start).
    NothingRunning,
    /// `sync/stop` created; the live runner kills its child and records `sync stopped`.
    Requested,
    /// The runner is gone; its recorded group was killed and `running.json` removed. The
    /// request is left too, for any runner waiting behind the killed one.
    Killed,
}

pub(crate) fn running_path(state: &Path) -> PathBuf {
    state.join("sync").join("running.json")
}

fn last_stop_path(state: &Path) -> PathBuf {
    state.join("sync").join("last_stop")
}

pub(crate) fn stop_path(state: &Path) -> PathBuf {
    state.join("sync").join("stop")
}

/// The record in `state`, if any.
pub(crate) fn load(state: &Path) -> io::Result<Option<Running>> {
    match fs::read(running_path(state)) {
        Ok(b) => serde_json::from_slice(&b)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The running sync of `cfg`'s vdir, if any: a record whose runner or child is still the
/// process it names. Creates nothing; `None` without `[sync]` or state.
pub fn running(cfg: &Config) -> io::Result<Option<Running>> {
    if cfg.sync.is_none() {
        return Ok(None);
    }
    let Some(state) = existing_vdir_state(&cfg.state_dir(), &cfg.vdir)? else {
        return Ok(None);
    };
    Ok(load(&state)?.filter(|r| runner_is_live(r) || child_is_live(r).is_some()))
}

/// Whether the recorded runner is still that process.
fn runner_is_live(r: &Running) -> bool {
    is_process(r.runner_pid, r.runner_start.as_deref())
}

/// The recorded child's group, if the child is still that process.
fn child_is_live(r: &Running) -> Option<u32> {
    r.child_pgid
        .filter(|&pgid| is_process(pgid, r.child_start.as_deref()))
}

/// Whether `pid` is the process that started at `start`. An unknown start never matches:
/// nothing is signalled on a guess.
fn is_process(pid: u32, start: Option<&str>) -> bool {
    start.is_some() && process_start(pid).as_deref() == start
}

/// When process `pid` started, as `ps -o lstart= -p <pid>` prints it (Linux and macOS);
/// None when there is no such process or ps cannot tell.
pub fn process_start(pid: u32) -> Option<String> {
    let out = Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let start = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !start.is_empty()).then_some(start)
}

/// Asks the running sync to stop (see [`StopResult`]). Never waits for it.
pub fn request_stop(cfg: &Config) -> io::Result<StopResult> {
    if cfg.sync.is_none() {
        return Ok(StopResult::NothingRunning);
    }
    let Some(state) = existing_vdir_state(&cfg.state_dir(), &cfg.vdir)? else {
        return Ok(StopResult::NothingRunning);
    };
    // Written even with nothing recorded: a pass may be between vdirsyncer calls, or about
    // to start, and must not take the vdir before the requester's write.
    fs::create_dir_all(state.join("sync"))?;
    fs::write(stop_path(&state), b"")?;
    let last = last_stop_path(&state);
    let tmp = last.with_extension("tmp");
    fs::write(&tmp, Utc::now().to_rfc3339())?;
    fs::rename(&tmp, &last)?;
    let Some(rec) = load(&state)? else {
        return Ok(StopResult::NothingRunning);
    };
    if runner_is_live(&rec) {
        return Ok(StopResult::Requested);
    }
    // The runner is gone. Its child is killed only while it is still the recorded process;
    // otherwise the record is stale (its ids may belong to other processes by now).
    let killed = child_is_live(&rec).map(kill_group).is_some();
    match fs::remove_file(running_path(&state)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    if !killed {
        return Ok(StopResult::NothingRunning);
    }
    super::run::record_orphan_stop(cfg, &state)?;
    Ok(StopResult::Killed)
}

/// Whether a pass begun at `since` must stop: a fresh request is pending, or a stop was
/// asked for after the pass began. A stale request file is removed.
pub(crate) fn stop_requested(state: &Path, since: DateTime<Utc>) -> io::Result<bool> {
    if stop_pending(state)? {
        return Ok(true);
    }
    let asked = match fs::read_to_string(last_stop_path(state)) {
        Ok(t) => DateTime::parse_from_rfc3339(t.trim()).ok(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    Ok(asked.is_some_and(|t| t >= since))
}

/// Whether a fresh stop request file is pending in `state`. A stale one is removed.
fn stop_pending(state: &Path) -> io::Result<bool> {
    let path = stop_path(state);
    let modified = match fs::metadata(&path) {
        Ok(m) => m.modified()?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    // A request from the future (clock change) counts as fresh.
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default();
    if age < STOP_FRESH {
        return Ok(true);
    }
    match fs::remove_file(&path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(false),
    }
}

/// Runs `kill <args>` through kill(1); true if one succeeded.
fn kill(args: &[&str]) -> bool {
    ["/bin/kill", "kill"].into_iter().any(|k| {
        Command::new(k)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// `kill -KILL -- -<pgid>` through kill(1); false if no kill(1) succeeded.
pub(crate) fn kill_group(pgid: u32) -> bool {
    kill(&["-KILL", "--", &format!("-{pgid}")])
}

/// Withdraws a stop request once the requester has written (or given up).
pub fn clear_stop(cfg: &Config) -> io::Result<()> {
    let Some(state) = existing_vdir_state(&cfg.state_dir(), &cfg.vdir)? else {
        return Ok(());
    };
    match fs::remove_file(stop_path(&state)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}
