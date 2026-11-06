//! `tdo sync`, `tdo status`, and starting a background sync.

use std::collections::BTreeMap;
use std::path::{Component, Path};
use std::process::{Command, Stdio};

use chrono::{DateTime, Utc};
use twydo_core::config::Config;
use twydo_core::sync::batch::{self, Due, Trigger};
use twydo_core::sync::run::{RunOptions, RunReport, run};
use twydo_core::sync::status::CollState;
use twydo_core::sync::waiter::{SystemClock, wait_and_sync};
use twydo_core::sync::{summary, switch};

use super::{EXIT_ERROR, load_vdir, resolve_config};

const NOT_CONFIGURED: &str = "sync is not configured: add a [sync] table (docs/sync.md)";

/// `tdo sync`: exit 0 when every collection is fine, 1 when the run had conflicts, emptied
/// lists or a failure, 2 on a configuration or IO error. `background` prints nothing on
/// success. `when_due` (with `background`) runs the batch waiter instead: it exits 0 unless
/// an IO error stops it.
pub fn sync(vdir: Option<&Path>, background: bool, when_due: bool, force_delete: &[String]) -> i32 {
    let cfg = match resolve_config(vdir) {
        Ok(c) => c,
        Err(e) => return error(&e),
    };
    if !background {
        print_warnings(&cfg);
    }
    if cfg.sync.is_none() {
        return error(&NOT_CONFIGURED);
    }
    if when_due {
        let opts = RunOptions {
            background: true,
            ..RunOptions::default()
        };
        let mut sync = || run(&cfg, &opts, Utc::now());
        return match wait_and_sync(&cfg, &mut SystemClock, &mut sync) {
            Ok(_) => 0,
            Err(e) => error(&e),
        };
    }
    let mut ids = Vec::with_capacity(force_delete.len());
    for name in force_delete {
        match collection_id(&cfg, name) {
            Ok(id) => ids.push(id),
            Err(msg) => return error(&msg),
        }
    }
    let opts = RunOptions {
        background,
        force_delete: ids,
    };
    let report = match run(&cfg, &opts, Utc::now()) {
        Ok(r) => r,
        Err(e) => return error(&e),
    };
    if report.skipped.is_some() {
        return 0;
    }
    let names = list_names(&cfg, &report);
    let problems = problems(&report, &names);
    for p in &problems {
        eprintln!("tdo: {p}");
    }
    for w in &report.warnings {
        eprintln!("tdo: warning: {w}");
    }
    for path in &report.unexpected {
        eprintln!("tdo: warning: {path} changed before its edit synced; the edit stays pending");
    }
    for path in &report.superseded {
        eprintln!(
            "tdo: warning: {path} was changed on the server after its edit synced; keeping the server's version"
        );
    }
    if problems.is_empty() {
        if !background {
            for c in &report.auto_forced {
                println!(
                    "applied the emptied list \"{}\" on the server",
                    name_of(&names, c)
                );
            }
            println!("synced");
        }
        0
    } else {
        1
    }
}

fn error(e: &dyn std::fmt::Display) -> i32 {
    eprintln!("tdo: {e}");
    EXIT_ERROR
}

/// Display names of the lists a run reported on (best effort; empty when unreadable).
fn list_names(cfg: &Config, r: &RunReport) -> BTreeMap<String, String> {
    if r.collections.is_empty() && r.auto_forced.is_empty() {
        return BTreeMap::new();
    }
    load_vdir(cfg)
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

fn describe(s: &CollState) -> String {
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
fn collection_id(cfg: &Config, name: &str) -> Result<String, String> {
    let plain = Path::new(name)
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
        && !name.contains('/');
    if plain && !name.is_empty() && cfg.vdir.join(name).is_dir() {
        return Ok(name.to_string());
    }
    let report = load_vdir(cfg).map_err(|e| e.to_string())?;
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

fn print_warnings(cfg: &Config) {
    for w in cfg.warnings() {
        eprintln!("warning: {w}");
    }
}

/// `tdo sync on|off`: switches automatic sync. Turning it on with unsynced edits starts a
/// background sync. Exit 0, or 2 when sync is not configured or on an IO error.
pub fn switch_command(vdir: Option<&Path>, on: bool) -> i32 {
    let cfg = match resolve_config(vdir) {
        Ok(c) => c,
        Err(e) => return error(&e),
    };
    print_warnings(&cfg);
    if cfg.sync.is_none() {
        return error(&NOT_CONFIGURED);
    }
    if let Err(e) = set_switch(&cfg, vdir, on) {
        return error(&e);
    }
    println!("{}", switch_message(on));
    0
}

/// Switches automatic sync (`tdo sync on|off`, `:sync on|off`). Turning it on with unsynced
/// edits starts a background sync at once. Once the switch is set, the call succeeds: an
/// error reading the unsynced edits starts a sync too, which checks again itself.
pub fn set_switch(cfg: &Config, vdir_flag: Option<&Path>, on: bool) -> std::io::Result<()> {
    switch::set(cfg, !on)?;
    if on && !matches!(batch::due(cfg, Utc::now()), Ok(Due::Nothing)) {
        spawn(cfg, vdir_flag, Trigger::SyncNow);
    }
    Ok(())
}

/// The confirmation for `tdo sync on|off` and `:sync on|off`.
pub fn switch_message(on: bool) -> &'static str {
    if on {
        "automatic sync on"
    } else {
        "automatic sync off"
    }
}

/// `tdo status`: last success, pending entries, conflicts, failures and per-list state.
pub fn status(vdir: Option<&Path>) -> i32 {
    let cfg = match resolve_config(vdir) {
        Ok(c) => c,
        Err(e) => return error(&e),
    };
    print_warnings(&cfg);
    let s = match summary(&cfg) {
        Ok(Some(s)) => s,
        Ok(None) => return error(&NOT_CONFIGURED),
        Err(e) => return error(&e),
    };
    let tz = cfg.tz();
    let when = |t: Option<DateTime<Utc>>| match t {
        Some(t) => t.with_timezone(&tz).format("%Y-%m-%d %H:%M").to_string(),
        None => "never".to_string(),
    };
    let st = &s.status;
    println!("last sync: {}", when(st.last_success));
    println!("last attempt: {}", when(st.last_attempt));
    println!("automatic sync: {}", if s.off { "off" } else { "on" });
    println!("pending: {}", s.pending);
    println!("conflicts: {}", s.conflicts.len());
    for c in &s.conflicts {
        println!("  {}  {}  {}", c.id, c.path, c.kind.words());
    }
    print!("failures: {}", st.failures);
    if st.failures > 0 {
        print!(" (next attempt {})", when(st.next_attempt));
    }
    println!();
    if let Some(e) = &st.last_error {
        println!("last error: {e}");
    }
    if let Some(e) = &st.metasync_error {
        println!("metasync error: {e}");
    }
    if !st.collections.is_empty() {
        println!("lists:");
        for (coll, state) in &st.collections {
            println!("  {coll}: {}", describe(state));
        }
    }
    0
}

/// The command that runs a background sync: `exe [--vdir <flag>] sync --background`, plus
/// `--when-due` for the batch waiter, with stdin, stdout and stderr null, in a process group
/// of its own so that closing the terminal does not kill it. `None` when sync is not
/// configured.
pub fn background_command(
    exe: &Path,
    config: &Config,
    vdir_flag: Option<&Path>,
    when_due: bool,
) -> Option<Command> {
    config.sync.as_ref()?;
    let mut cmd = Command::new(exe);
    if let Some(v) = vdir_flag {
        cmd.arg("--vdir").arg(v);
    }
    cmd.args(["sync", "--background"]);
    if when_due {
        cmd.arg("--when-due");
    }
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Some(cmd)
}

/// Starts `cmd` without waiting for it. A thread reaps it when it exits, so a long-lived
/// caller (the TUI) leaves no zombies. Errors are ignored.
pub fn detach(mut cmd: Command) {
    if let Ok(mut child) = cmd.spawn() {
        let _ = std::thread::Builder::new()
            .name("sync-reaper".into())
            .spawn(move || child.wait());
    }
}

/// Starts `tdo [--vdir <flag>] sync --background` detached when `config.sync` is set. Errors
/// are ignored: the timer catches up.
pub fn spawn_background(config: &Config, vdir_flag: Option<&Path>) {
    start(config, vdir_flag, false);
}

/// Starts what a write asked for, detached: a background sync now, or the batch waiter.
/// Nothing without `[sync]`; errors are ignored (the timer or the next edit catches up).
pub fn spawn(config: &Config, vdir_flag: Option<&Path>, t: Trigger) {
    start(config, vdir_flag, t == Trigger::Waiter);
}

fn start(config: &Config, vdir_flag: Option<&Path>, when_due: bool) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    if let Some(cmd) = background_command(&exe, config, vdir_flag, when_due) {
        detach(cmd);
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
