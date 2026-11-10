//! `tdo conflicts`: sync conflicts kept outside the vdir.

use std::path::{Path, PathBuf};

use chrono::Utc;
use clap::{Subcommand, ValueEnum};
use twydo_core::config::Config;
use twydo_core::conflicts::{ConflictState, ConflictStore, Side};
use twydo_core::ical;
use twydo_core::store::existing_vdir_state;
use twydo_core::sync::hook::{VDIR_ENV, conflict_hook};
use twydo_daemon::protocol::admin::{ConflictsResolveParams, ResolveResult, WireKeep};

use super::{EXIT_ERROR, resolve_config};
use crate::client::Target;

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
        None => list(vdir),
        Some(ConflictsCommand::Show { id }) => show(vdir, &id),
        Some(ConflictsCommand::Resolve { id, keep }) => resolve_cmd(target, &id, keep),
        Some(ConflictsCommand::Hook { a, b }) => hook(vdir, &a, &b),
    }
}

fn fail(msg: impl std::fmt::Display) -> i32 {
    eprintln!("tdo: {msg}");
    EXIT_ERROR
}

/// The conflict store of `cfg`'s vdir with its directory; `None` when it has no conflicts yet.
fn open_store(cfg: &Config) -> std::io::Result<Option<(ConflictStore, PathBuf)>> {
    let Some(state) = existing_vdir_state(&cfg.state_dir(), &cfg.vdir)? else {
        return Ok(None);
    };
    Ok(ConflictStore::existing(&state)?.map(|s| (s, state)))
}

/// The first VTODO property `name` of an `.ics` text, as written.
fn field(text: &str, name: &str) -> Option<String> {
    let cal = ical::parse(text.as_bytes()).ok()?;
    let todo = cal.root.children_named("VTODO").next()?;
    todo.prop(name).map(|p| p.value.clone())
}

fn state_word(s: ConflictState) -> &'static str {
    match s {
        ConflictState::Open => "open",
        ConflictState::Chosen => "chosen",
        ConflictState::Resolved => "resolved",
        ConflictState::Superseded => "superseded",
    }
}

fn list(vdir: Option<&Path>) -> i32 {
    let cfg = match resolve_config(vdir) {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let found = match open_store(&cfg).and_then(|s| match s {
        Some((store, _)) => store.unresolved(),
        None => Ok(Vec::new()),
    }) {
        Ok(f) => f,
        Err(e) => return fail(e),
    };
    if found.is_empty() {
        println!("no conflicts");
    }
    for c in &found {
        let state = match (c.state, c.choice) {
            (ConflictState::Chosen, Some(Side::Local)) => "chosen: keep local".to_string(),
            (ConflictState::Chosen, Some(Side::Remote)) => "chosen: keep remote".to_string(),
            (s, _) => state_word(s).to_string(),
        };
        println!(
            "{}  {}  {}  {}  {}",
            c.id,
            c.collection,
            c.title(),
            c.kind.words(),
            state
        );
    }
    0
}

fn show(vdir: Option<&Path>, id: &str) -> i32 {
    let cfg = match resolve_config(vdir) {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let found = open_store(&cfg).and_then(|s| match s {
        Some((store, dir)) => Ok(store.get(id)?.map(|c| (c, dir))),
        None => Ok(None),
    });
    let (c, state) = match found {
        Ok(Some(f)) => f,
        Ok(None) => return fail(format!("no conflict {id}")),
        Err(e) => return fail(e),
    };
    println!("{}  {}", c.id, c.kind.words());
    for (name, side, text) in [
        ("local", "local.ics", &c.local),
        ("remote", "remote.ics", &c.remote),
    ] {
        println!("{name}:");
        match text {
            None => println!("  (deleted)"),
            Some(t) => {
                for prop in ["SUMMARY", "DUE", "STATUS", "LAST-MODIFIED"] {
                    println!("  {prop}: {}", field(t, prop).unwrap_or_default());
                }
                println!(
                    "  file: {}",
                    state.join("conflicts").join(&c.id).join(side).display()
                );
            }
        }
    }
    let file = cfg.vdir.join(&c.path);
    println!("current:");
    match std::fs::read(&file) {
        Ok(b) => {
            let t = String::from_utf8_lossy(&b);
            for prop in ["SUMMARY", "DUE", "STATUS", "LAST-MODIFIED"] {
                println!("  {prop}: {}", field(&t, prop).unwrap_or_default());
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => println!("  (absent)"),
        Err(e) => return fail(format!("{}: {e}", file.display())),
    }
    println!("vdir file: {}", file.display());
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
