//! The "sync ended" event: every sync run that reaches the backoff check, clean or failed,
//! records `<vdir-state>/sync/ended`, so that a long-lived reader (the TUI) knows to reload.

use std::fs;
use std::io;
use std::path::Path;
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::store::existing_vdir_state;
use crate::store::fsio::write_atomic;

/// One recorded run end. `run_id` differs for every run, so equal timestamps still compare
/// unequal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ended {
    pub run_id: String,
    pub at: DateTime<Utc>,
}

fn path(state: &Path) -> std::path::PathBuf {
    state.join("sync").join("ended")
}

/// Atomically records that a run ended at `at`, with a fresh `run_id`.
pub fn write(state: &Path, at: DateTime<Utc>) -> io::Result<()> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let ended = Ended {
        run_id: format!("{}-{nanos}", process::id()),
        at,
    };
    fs::create_dir_all(state.join("sync"))?;
    let mut json = serde_json::to_string(&ended).map_err(io::Error::other)?;
    json.push('\n');
    write_atomic(&path(state), json.as_bytes())
}

/// The last recorded run end. `None` when there is no state dir or no file; creates nothing.
pub fn read(cfg: &Config) -> io::Result<Option<Ended>> {
    let Some(state) = existing_vdir_state(&cfg.state_dir(), &cfg.vdir)? else {
        return Ok(None);
    };
    match fs::read(path(&state)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("sync/ended: {e}"))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
