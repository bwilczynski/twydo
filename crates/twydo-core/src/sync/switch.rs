//! The automatic-sync switch: `<vdir-state>/sync/off` present means background runs (timer,
//! startup, waiter, size trigger) are skipped. Explicit syncs always run.

use std::fs;
use std::io;
use std::path::Path;

use crate::config::Config;
use crate::store::fsio::{remove_durable, write_atomic};
use crate::store::{bind_vdir_state, existing_vdir_state};

fn path(state: &Path) -> std::path::PathBuf {
    state.join("sync").join("off")
}

/// Whether automatic sync is switched off. Creates nothing: a vdir with no state is on.
pub fn is_off(cfg: &Config) -> io::Result<bool> {
    let Some(state) = existing_vdir_state(&cfg.state_dir(), &cfg.vdir)? else {
        return Ok(false);
    };
    path(&state).try_exists()
}

/// Switches automatic sync off (creates `sync/off`) or on (removes it), durably.
pub fn set(cfg: &Config, off: bool) -> io::Result<()> {
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir)?;
    let p = path(&state);
    if off {
        fs::create_dir_all(state.join("sync"))?;
        write_atomic(&p, b"")
    } else {
        remove_durable(&p)
    }
}
