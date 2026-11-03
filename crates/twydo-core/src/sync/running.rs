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
    /// The current vdirsyncer child (it leads its own process group); None between calls.
    pub child_pgid: Option<u32>,
    /// When the runner started vdirsyncer (wall clock).
    pub started: DateTime<Utc>,
}

/// How long a stop request holds off new syncs; older ones are stale.
pub const STOP_FRESH: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopResult {
    /// No `running.json`: nothing to stop.
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

/// The running sync of `cfg`'s vdir, if any. Creates nothing; `None` without `[sync]` or state.
pub fn running(cfg: &Config) -> io::Result<Option<Running>> {
    if cfg.sync.is_none() {
        return Ok(None);
    }
    match existing_vdir_state(&cfg.state_dir(), &cfg.vdir)? {
        Some(state) => load(&state),
        None => Ok(None),
    }
}

/// Asks the running sync to stop (see [`StopResult`]). Never waits for it.
pub fn request_stop(cfg: &Config) -> io::Result<StopResult> {
    if cfg.sync.is_none() {
        return Ok(StopResult::NothingRunning);
    }
    let Some(state) = existing_vdir_state(&cfg.state_dir(), &cfg.vdir)? else {
        return Ok(StopResult::NothingRunning);
    };
    let Some(rec) = load(&state)? else {
        return Ok(StopResult::NothingRunning);
    };
    fs::write(stop_path(&state), b"")?;
    if alive(rec.runner_pid) {
        return Ok(StopResult::Requested);
    }
    if let Some(pgid) = rec.child_pgid {
        kill_group(pgid);
    }
    match fs::remove_file(running_path(&state)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    Ok(StopResult::Killed)
}

/// Whether a fresh stop request is pending in `state`. A stale one is removed.
pub(crate) fn stop_requested(state: &Path) -> io::Result<bool> {
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

/// `kill -0 <pid>` through kill(1).
pub(crate) fn alive(pid: u32) -> bool {
    kill(&["-0", &pid.to_string()])
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
