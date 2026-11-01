//! `tdo conflicts`: sync conflicts kept outside the vdir.

use std::path::{Path, PathBuf};

use chrono::Utc;
use clap::Subcommand;
use twydo_core::sync::hook::conflict_hook;

use super::{EXIT_ERROR, resolve_config};

#[derive(Subcommand)]
pub enum ConflictsCommand {
    /// vdirsyncer conflict hook: `conflict_resolution = ["command", "tdo", "conflicts", "hook"]`
    #[command(hide = true)]
    Hook {
        /// vdirsyncer's temp file for storage a
        a: PathBuf,
        /// vdirsyncer's temp file for storage b
        b: PathBuf,
    },
}

/// Runs a `tdo conflicts` subcommand; returns the exit code.
pub fn run(vdir: Option<&Path>, command: ConflictsCommand) -> i32 {
    match command {
        ConflictsCommand::Hook { a, b } => hook(vdir, &a, &b),
    }
}

/// Exit 0 after recording or applying; 2 on any error, which makes vdirsyncer leave both
/// sides untouched.
fn hook(vdir: Option<&Path>, a: &Path, b: &Path) -> i32 {
    let cfg = match resolve_config(vdir) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("tdo: conflict hook: {e}");
            return EXIT_ERROR;
        }
    };
    match conflict_hook(&cfg, a, b, Utc::now()) {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("tdo: conflict hook: {e}");
            EXIT_ERROR
        }
    }
}
