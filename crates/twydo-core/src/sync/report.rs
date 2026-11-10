//! What a sync run tells the user: the lines `tdo sync` prints and the daemon's
//! `sync.finished` carries, so both say the same thing.

use std::collections::BTreeMap;
use std::io;
use std::path::{Component, Path};

use super::run::RunReport;
use super::status::CollState;
use crate::config::Config;
use crate::store::existing_vdir_state;
use crate::store::fsio::VdirLock;
use crate::vdir::{self, LoadReport};

/// Why a sync cannot run: no `[sync]` table.
pub const NOT_CONFIGURED: &str = "sync is not configured: add a [sync] table (docs/sync.md)";

/// The lines about one run, without any prefix (`tdo sync` adds `tdo: ` and
/// `tdo: warning: `).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Messages {
    /// Things the user should act on; any makes the run a failure.
    pub problems: Vec<String>,
    /// Things that did not fail the run.
    pub warnings: Vec<String>,
    /// What the runner did on its own (an emptied list applied on the server).
    pub notes: Vec<String>,
}

/// The messages for `r`, naming lists by their display names where they can be read.
pub fn messages(cfg: &Config, r: &RunReport) -> Messages {
    let names = list_names(cfg, r);
    let mut warnings = r.warnings.clone();
    for path in &r.unexpected {
        warnings.push(format!(
            "{path} changed before its edit synced; the edit stays pending"
        ));
    }
    for path in &r.superseded {
        warnings.push(format!(
            "{path} was changed on the server after its edit synced; keeping the server's version"
        ));
    }
    Messages {
        problems: problems(r, &names),
        warnings,
        notes: r
            .auto_forced
            .iter()
            .map(|c| {
                format!(
                    "applied the emptied list \"{}\" on the server",
                    name_of(&names, c)
                )
            })
            .collect(),
    }
}

/// Reads the vdir under the shared vdir lock (taken only when the vdir's state dir exists);
/// errors are worded as `tdo` words them.
fn load_locked(cfg: &Config) -> Result<LoadReport, String> {
    let lock_err =
        |path: &Path, e: io::Error| format!("cannot lock state dir {}: {e}", path.display());
    let state = existing_vdir_state(&cfg.state_dir(), &cfg.vdir)
        .map_err(|e| lock_err(&cfg.state_dir(), e))?;
    let _lock = match state {
        Some(state) => VdirLock::reader(&state).map_err(|e| lock_err(&state, e))?,
        None => None,
    };
    vdir::load(&cfg.vdir).map_err(|e| format!("cannot read vdir {}: {e}", cfg.vdir.display()))
}

/// Display names of the lists a run reported on (best effort; empty when unreadable).
fn list_names(cfg: &Config, r: &RunReport) -> BTreeMap<String, String> {
    if r.collections.is_empty() && r.auto_forced.is_empty() {
        return BTreeMap::new();
    }
    load_locked(cfg)
        .map(|v| {
            v.collections
                .into_iter()
                .filter_map(|c| c.display_name.map(|n| (c.id, n)))
                .collect()
        })
        .unwrap_or_default()
}

/// The list's display name when known, else its collection id.
fn name_of<'a>(names: &'a BTreeMap<String, String>, coll: &'a str) -> &'a str {
    names.get(coll).map_or(coll, String::as_str)
}

/// One line per thing the user should know about after a run.
fn problems(r: &RunReport, names: &BTreeMap<String, String>) -> Vec<String> {
    let mut out = Vec::new();
    for (coll, state) in &r.collections {
        match state {
            CollState::Ok => {}
            CollState::Error { message }
                if message.trim_end_matches(';').ends_with("Not Found") =>
            {
                out.push(format!(
                    "list {:?} ({coll}): server returned Not Found; if you deleted this list on another device, run :forget-list to choose a list (or :forget-list {coll})",
                    name_of(names, coll)
                ));
            }
            CollState::Error { .. } if r.transient.is_some() => {}
            CollState::EmptiedLocal | CollState::EmptiedRemote => {
                let n = name_of(names, coll);
                let (where_, to) = if *state == CollState::EmptiedLocal {
                    ("here", "on the server")
                } else {
                    ("on the server", "here")
                };
                out.push(format!(
                    "list \"{n}\" was emptied {where_}; to apply that {to} run: tdo sync --force-delete \"{n}\""
                ));
            }
            other => out.push(format!("list \"{coll}\": {}", describe(other))),
        }
    }
    if let Some(t) = &r.transient {
        out.push(format!("sync failed: {t}"));
    }
    if !r.new_conflicts.is_empty() {
        out.push(format!(
            "{} new conflict(s); see tdo status",
            r.new_conflicts.len()
        ));
    }
    if let Some(m) = &r.metasync_error {
        out.push(format!("metasync failed: {m}"));
    }
    out
}

/// A list's sync state in words (`tdo status`).
pub fn describe(s: &CollState) -> String {
    match s {
        CollState::Ok => "ok".into(),
        CollState::Conflict => "conflict".into(),
        CollState::EmptiedLocal => "emptied here".into(),
        CollState::EmptiedRemote => "emptied on the server".into(),
        CollState::Error { message } => format!("error: {message}"),
    }
}

/// A `--force-delete` argument as a collection id: a collection directory name, else a
/// unique display name.
pub fn collection_id(cfg: &Config, name: &str) -> Result<String, String> {
    let plain = Path::new(name)
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
        && !name.contains('/');
    if plain && !name.is_empty() && cfg.vdir.join(name).is_dir() {
        return Ok(name.to_string());
    }
    let report = load_locked(cfg)?;
    let mut ids = report
        .collections
        .iter()
        .filter(|c| c.display_name.as_deref() == Some(name))
        .map(|c| c.id.clone());
    match (ids.next(), ids.next()) {
        (Some(id), None) => Ok(id),
        (Some(_), Some(_)) => Err(format!(
            "list name {name:?} is ambiguous; pass its directory name"
        )),
        (None, _) => Err(format!("no list {name:?} in {}", cfg.vdir.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_lists_identify_every_target_and_the_confirmation_command() {
        let r = RunReport {
            transient: Some("c1: Unknown error occurred: Not Found".into()),
            collections: BTreeMap::from([
                (
                    "c1".into(),
                    CollState::Error {
                        message: "Unknown error occurred: Not Found".into(),
                    },
                ),
                (
                    "c2".into(),
                    CollState::Error {
                        message: "Unknown error occurred: Not Found".into(),
                    },
                ),
            ]),
            ..RunReport::default()
        };
        let names = BTreeMap::from([("c1".into(), "Old Projects".into())]);
        let text = problems(&r, &names).join("\n");
        assert!(text.contains("Old Projects"), "{text}");
        assert!(text.contains(":forget-list c1"), "{text}");
        assert!(text.contains(":forget-list c2"), "{text}");
        assert!(text.contains(":forget-list to choose a list"), "{text}");
        assert!(
            !text.contains("was deleted"),
            "404 is not proof of deletion"
        );
    }
}
