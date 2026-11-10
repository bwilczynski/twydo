//! Live `config.toml`: the daemon rereads the file when it changes and applies what may change
//! at run time. The binding fields (`vdir`, `state_dir`, `writes`, `sync`, `timezone`,
//! `work_days`) never change in a running daemon; a file that names them differently is
//! reported in `status.config.restart_needed` and not applied. An unreadable or invalid file
//! keeps the last good configuration and sets `status.config.error`. The comparisons mirror
//! the TUI's (`tdo/src/term/preferences.rs`): canonical vdir, absolute state directory.

use std::fs;
use std::io;
use std::path::Path;

use twydo_core::config::Config;

use super::Shared;
use crate::protocol::events::Cause;
use crate::protocol::methods::ConfigStatus;

/// The binding settings of `new` that differ from `bound`, in a fixed order.
fn restart_needed(bound: &Config, new: &Config) -> Vec<String> {
    let canon = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let abs = |p: std::path::PathBuf| std::path::absolute(&p).unwrap_or(p);
    let mut out = Vec::new();
    if canon(&bound.vdir) != canon(&new.vdir) {
        out.push("vdir");
    }
    if abs(bound.state_dir()) != abs(new.state_dir()) {
        out.push("state_dir");
    }
    if bound.writes != new.writes {
        out.push("writes");
    }
    if bound.sync != new.sync {
        out.push("sync");
    }
    // The queue's drain gate reads "today" and the work days when the daemon starts; a live
    // change would be half applied.
    if bound.timezone != new.timezone {
        out.push("timezone");
    }
    if bound.work_days != new.work_days {
        out.push("work_days");
    }
    out.into_iter().map(String::from).collect()
}

/// Reads the file; `None` when it is absent and nothing was read before.
fn read(shared: &Shared, seen: &Option<Vec<u8>>) -> Option<Result<Vec<u8>, String>> {
    match fs::read(&shared.instance.config) {
        Ok(b) => Some(Ok(b)),
        Err(e) if e.kind() == io::ErrorKind::NotFound && seen.is_none() => None,
        Err(e) => Some(Err(format!("cannot read the configuration: {e}"))),
    }
}

/// The watcher's debounced callback (and the startup look): applies the file if it changed
/// since the last read.
pub fn changed(shared: &Shared) {
    // Held throughout, so two calls never apply out of order.
    let mut seen = shared.config_seen.lock().unwrap();
    let status = match read(shared, &seen) {
        None => return,
        Some(Ok(bytes)) => {
            if seen.as_ref() == Some(&bytes) {
                return;
            }
            *seen = Some(bytes);
            apply(shared)
        }
        Some(Err(message)) => {
            // Forget the bytes: the same good bytes coming back must be evaluated again to
            // clear the error.
            *seen = None;
            ConfigStatus {
                error: Some(message),
                restart_needed: Vec::new(),
            }
        }
    };
    shared.publisher.set_config_status(status);
    shared.publisher.refresh_status();
}

/// Loads the file and applies it; returns the status to report.
fn apply(shared: &Shared) -> ConfigStatus {
    let path = &shared.instance.config;
    let mut loaded = match Config::load(path) {
        Ok(c) => c,
        Err(e) => {
            return ConfigStatus {
                error: Some(e.to_string()),
                restart_needed: Vec::new(),
            };
        }
    };
    // A `--vdir` given at start wins over the file, as in `resolve_config`.
    if let Some(v) = &shared.instance.vdir {
        loaded.vdir = v.clone();
    }
    let needed = restart_needed(&shared.cfg, &loaded);
    // The live configuration: everything from the file except the binding fields.
    let mut live = loaded;
    live.vdir = shared.cfg.vdir.clone();
    live.state_dir = shared.cfg.state_dir.clone();
    live.writes = shared.cfg.writes;
    live.sync = shared.cfg.sync.clone();
    live.timezone = shared.cfg.timezone.clone();
    live.work_days = shared.cfg.work_days.clone();
    if live != shared.publisher.config() {
        shared.publisher.set_config(live);
        if let Err(e) = shared.publisher.reload(Cause::Config, Vec::new()) {
            eprintln!("tdo daemon: reload after a configuration change failed: {e}");
        }
    }
    ConfigStatus {
        error: None,
        restart_needed: needed,
    }
}
