//! `tdo sync`, `tdo status`, and starting a background sync.

use std::path::Path;
use std::process::{Command, Stdio};

use chrono::{DateTime, Utc};
use twydo_core::config::Config;
use twydo_core::store::DrainOutcome;
use twydo_core::store::queue::{Queue, refusal_message};
use twydo_core::sync::batch::{self, Due, Trigger};
use twydo_core::sync::report::{NOT_CONFIGURED, collection_id, describe, messages};
use twydo_core::sync::run::{RunOptions, run};
use twydo_core::sync::waiter::{SystemClock, wait_and_sync};
use twydo_core::sync::{summary, switch};

use super::{EXIT_ERROR, resolve_config};

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
    if report
        .drained
        .iter()
        .any(|d| matches!(d.outcome, DrainOutcome::Applied(_)))
        && let Some(trigger) = batch::after_write(&cfg, Utc::now())
    {
        spawn(&cfg, vdir, trigger);
    }
    if !background {
        print_refusals(&cfg);
    }
    let m = messages(&cfg, &report);
    for p in &m.problems {
        eprintln!("tdo: {p}");
    }
    for w in &m.warnings {
        eprintln!("tdo: warning: {w}");
    }
    if m.problems.is_empty() {
        if !background {
            for n in &m.notes {
                println!("{n}");
            }
            println!("synced");
        }
        0
    } else {
        1
    }
}

/// Tells the user about queued edits that were refused and not yet shown, once.
pub(super) fn print_refusals(cfg: &Config) {
    let Ok(Some(state)) = twydo_core::store::existing_vdir_state(&cfg.state_dir(), &cfg.vdir)
    else {
        return;
    };
    let Some(queue) = Queue::existing(&state) else {
        return;
    };
    for r in queue.unshown().unwrap_or_default() {
        eprintln!("tdo: {}", refusal_message(&r));
        let _ = queue.mark_shown(&r.item.id);
    }
}

fn error(e: &dyn std::fmt::Display) -> i32 {
    eprintln!("tdo: {e}");
    EXIT_ERROR
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
