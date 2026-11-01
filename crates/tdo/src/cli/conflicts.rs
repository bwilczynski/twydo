//! `tdo conflicts`: sync conflicts kept outside the vdir.

use std::path::{Path, PathBuf};

use chrono::Utc;
use clap::{Subcommand, ValueEnum};
use twydo_core::config::Config;
use twydo_core::conflicts::{Conflict, ConflictState, ConflictStore, Side};
use twydo_core::ical;
use twydo_core::store::existing_vdir_state;
use twydo_core::sync::hook::conflict_hook;
use twydo_core::sync::resolve::{Resolution, resolve};

use super::sync::spawn_background;
use super::{EXIT_ERROR, resolve_config};

/// Which version to keep.
#[derive(Clone, Copy, ValueEnum)]
pub enum Keep {
    /// The version on this device
    Local,
    /// The version on the other device (the server)
    Remote,
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
pub fn run(vdir: Option<&Path>, command: Option<ConflictsCommand>) -> i32 {
    match command {
        None => list(vdir),
        Some(ConflictsCommand::Show { id }) => show(vdir, &id),
        Some(ConflictsCommand::Resolve { id, keep }) => resolve_cmd(vdir, &id, keep),
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

/// The title from either version (local first), else the UID.
fn title(c: &Conflict) -> String {
    [&c.local, &c.remote]
        .into_iter()
        .flatten()
        .find_map(|t| field(t, "SUMMARY"))
        .unwrap_or_else(|| c.uid.clone())
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
            title(c),
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
    println!("vdir file: {}", cfg.vdir.join(&c.path).display());
    0
}

fn resolve_cmd(vdir: Option<&Path>, id: &str, keep: Keep) -> i32 {
    let cfg = match resolve_config(vdir) {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let side = match keep {
        Keep::Local => Side::Local,
        Keep::Remote => Side::Remote,
    };
    match resolve(&cfg, id, side, Utc::now()) {
        Ok(r) => {
            match r {
                Resolution::Recorded => {
                    println!("recorded; the next sync applies it");
                }
                Resolution::AlreadyInPlace => println!("already in place; conflict resolved"),
                Resolution::Written(_) => println!("written; conflict resolved"),
            }
            if cfg.sync.as_ref().is_some_and(|s| s.after_edit) {
                spawn_background(&cfg, vdir);
            }
            0
        }
        Err(e) => fail(e),
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
