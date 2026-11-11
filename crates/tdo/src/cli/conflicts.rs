//! `tdo conflicts`: sync conflicts kept outside the vdir.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use clap::{Subcommand, ValueEnum};
use twydo_core::conflicts::ConflictKind;
use twydo_core::sync::hook::{VDIR_ENV, conflict_hook};
use twydo_daemon::protocol::admin::{
    ConflictDetail, ConflictList, ConflictsResolveParams, ConflictsShowParams, ResolveResult,
    VersionView, WireConflictKind, WireConflictState, WireKeep, WireSide,
};

use super::{EXIT_ERROR, resolve_config};
use crate::client::{self, CLI_TIMEOUT, Target};

/// Which version to keep.
#[derive(Clone, Copy, ValueEnum)]
pub enum Keep {
    /// The version on this device
    Local,
    /// The version on the other device (the server)
    Remote,
    /// The task as this device holds it now, whichever version that is (not for a conflict
    /// vdirsyncer still holds); see `tdo conflicts show`
    Current,
}

#[derive(Subcommand)]
pub enum ConflictsCommand {
    /// Show both versions of a conflict
    Show { id: String },
    /// Keep one version of a conflict
    Resolve {
        id: String,
        #[arg(long, value_enum)]
        keep: Keep,
    },
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
pub fn run(target: &Target, command: Option<ConflictsCommand>) -> i32 {
    let vdir = target.vdir.as_deref();
    match command {
        None => list(target),
        Some(ConflictsCommand::Show { id }) => show(target, &id),
        Some(ConflictsCommand::Resolve { id, keep }) => resolve_cmd(target, &id, keep),
        Some(ConflictsCommand::Hook { a, b }) => hook(vdir, &a, &b),
    }
}

fn state_word(s: WireConflictState) -> &'static str {
    match s {
        WireConflictState::Open => "open",
        WireConflictState::Chosen => "chosen",
        WireConflictState::Resolved => "resolved",
        WireConflictState::Superseded => "superseded",
        WireConflictState::Unknown => "unknown",
    }
}

/// The kind in words (the core's `ConflictKind::words`).
pub(super) fn kind_words(k: WireConflictKind) -> &'static str {
    match k {
        WireConflictKind::BothChanged => ConflictKind::BothChanged,
        WireConflictKind::LocalDeleteRemoteEdit => ConflictKind::LocalDeleteRemoteEdit,
        WireConflictKind::RemoteDeleteLocalEdit => ConflictKind::RemoteDeleteLocalEdit,
        WireConflictKind::Journal => ConflictKind::Journal,
        WireConflictKind::Unknown => return "a conflict of a kind this tdo does not know",
    }
    .words()
}

/// How long a call waits for a daemon that is still starting.
const STARTUP_WAIT: Duration = Duration::from_secs(30);

fn ask<P: serde::Serialize, R: serde::de::DeserializeOwned>(
    target: &Target,
    method: &str,
    params: &P,
) -> Result<R, i32> {
    let client = super::sync::connect(target)?;
    client::call_ready(&client.caller(), method, params, CLI_TIMEOUT, STARTUP_WAIT)
        .map_err(|e| super::sync::client_error(&e))
}

fn list(target: &Target) -> i32 {
    let found: ConflictList = match ask(target, "conflicts.list", &serde_json::json!({})) {
        Ok(f) => f,
        Err(code) => return code,
    };
    if found.conflicts.is_empty() {
        println!("no conflicts");
    }
    for c in &found.conflicts {
        let state = match (c.state, c.choice) {
            (WireConflictState::Chosen, Some(WireSide::Local)) => "chosen: keep local".to_string(),
            (WireConflictState::Chosen, Some(WireSide::Remote)) => {
                "chosen: keep remote".to_string()
            }
            (s, _) => state_word(s).to_string(),
        };
        println!(
            "{}  {}  {}  {}  {}",
            c.id,
            c.collection,
            c.title.as_deref().unwrap_or(&c.uid),
            kind_words(c.kind),
            state
        );
    }
    0
}

fn print_version(v: &VersionView) {
    for (prop, value) in [
        ("SUMMARY", &v.summary),
        ("DUE", &v.due),
        ("STATUS", &v.status),
        ("LAST-MODIFIED", &v.last_modified),
    ] {
        println!("  {prop}: {}", value.as_deref().unwrap_or_default());
    }
}

fn show(target: &Target, id: &str) -> i32 {
    let d: ConflictDetail = match ask(
        target,
        "conflicts.show",
        &ConflictsShowParams { id: id.to_string() },
    ) {
        Ok(d) => d,
        Err(code) => return code,
    };
    println!("{}  {}", d.conflict.id, kind_words(d.conflict.kind));
    for (name, v) in [("local", &d.local), ("remote", &d.remote)] {
        println!("{name}:");
        match v {
            None => println!("  (deleted)"),
            Some(v) => {
                print_version(v);
                if let Some(f) = &v.file {
                    println!("  file: {}", f.display());
                }
            }
        }
    }
    println!("current:");
    match &d.current {
        Some(v) => print_version(v),
        None => println!("  (absent)"),
    }
    println!("vdir file: {}", d.vdir_file.display());
    0
}

/// `tdo conflicts resolve`: the daemon records or applies the choice (and schedules the
/// sync that follows); this prints what it did.
fn resolve_cmd(target: &Target, id: &str, keep: Keep) -> i32 {
    let mut client = match super::sync::connect(target) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let rid = format!("resolve-{}", client.next_request_id());
    let params = ConflictsResolveParams {
        request_id: rid.clone(),
        id: id.to_string(),
        keep: match keep {
            Keep::Local => WireKeep::Local,
            Keep::Remote => WireKeep::Remote,
            Keep::Current => WireKeep::Current,
        },
    };
    match super::sync::change::<_, ResolveResult>(
        target,
        &mut client,
        "conflicts.resolve",
        &rid,
        &params,
        crate::client::Unknown::Choice,
    ) {
        Ok(r) => {
            println!("{}", r.message);
            0
        }
        Err(code) => code,
    }
}

/// Exit 0 after recording or applying; 2 on any error, which makes vdirsyncer leave both
/// sides untouched. Without `--vdir`, the vdir is the one the runner names in
/// `TWYDO_SYNC_VDIR` (it may have been given its own `--vdir`), else the config's.
fn hook(vdir: Option<&Path>, a: &Path, b: &Path) -> i32 {
    let from_runner = std::env::var_os(VDIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let cfg = match resolve_config(vdir.or(from_runner.as_deref())) {
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
