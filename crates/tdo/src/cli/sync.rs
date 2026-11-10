//! `tdo sync`, `tdo status`, and starting a background sync.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use twydo_core::config::Config;
use twydo_core::sync::batch::{self, Due, Trigger};
use twydo_core::sync::report::{NOT_CONFIGURED, describe};
use twydo_core::sync::{summary, switch};
use twydo_daemon::client::{Client, ClientError, Notification};
use twydo_daemon::protocol::events::Event;
use twydo_daemon::protocol::jobs::{JobGetParams, JobResult, JobStarted, JobState, JobView};
use twydo_daemon::protocol::methods::SubscribeResult;
use twydo_daemon::protocol::rpc::ErrorKind as RpcKind;
use twydo_daemon::protocol::sync::{
    SyncOutcome, SyncResult, SyncRunParams, SyncSetAutoParams, SyncSetAutoResult,
};
use twydo_daemon::protocol::wire::WireSnapshot;

use super::{EXIT_ERROR, op, resolve_config};
use crate::client::{self, CLI_TIMEOUT, Target};

const CLIENT: &str = "tdo-sync";
/// How often the wait for a sync asks `job.get` when no event arrives.
const JOB_POLL: Duration = Duration::from_secs(5);
/// How long a call waits for a daemon that is still starting.
const STARTUP_WAIT: Duration = Duration::from_secs(30);
/// How long the wait for a sync keeps trying to reach a daemon that went away.
const RECONNECT_FOR: Duration = Duration::from_secs(60);

/// Prints why a call to the daemon failed and returns the exit code (2).
pub(super) fn client_error(e: &ClientError) -> i32 {
    match e {
        ClientError::Rpc(_) => error(e),
        ClientError::TimedOut(_) => error(&"daemon did not answer"),
        _ => error(&format_args!("daemon not running: {e}")),
    }
}

/// A changing call, resent with the same request id until answered; when the window ends
/// the outcome is reported as unknown (`unknown` says what may have happened) and the exit code is 2.
pub(super) fn change<P: serde::Serialize + Clone, R: serde::de::DeserializeOwned>(
    target: &Target,
    client: &mut Client,
    method: &str,
    request_id: &str,
    params: &P,
    unknown: client::Unknown,
) -> Result<R, i32> {
    match client::call_changing(
        target,
        CLIENT,
        client,
        method,
        |_| params.clone(),
        client::reconnect_for(),
    ) {
        Ok(r) => Ok(r),
        Err(client::ChangeError::Failed(e)) => Err(client_error(&e)),
        Err(client::ChangeError::NoAnswer) => {
            Err(error(&client::no_answer_message(unknown, request_id)))
        }
    }
}

/// Connects to the daemon (starting it when none runs), or prints why not.
pub(super) fn connect(target: &Target) -> Result<Client, i32> {
    match client::connect(target, CLIENT, None) {
        Ok((c, _)) => Ok(c),
        Err(e) => Err(error(&e)),
    }
}

/// Prints the daemon's configuration warnings as `warning: <w>` on stderr.
fn print_config_warnings(client: &Client) -> Result<(), i32> {
    let snap: WireSnapshot = client::call_ready(
        &client.caller(),
        "snapshot.get",
        &serde_json::json!({}),
        CLI_TIMEOUT,
        STARTUP_WAIT,
    )
    .map_err(|e| client_error(&e))?;
    for w in &snap.config_warnings {
        eprintln!("warning: {w}");
    }
    Ok(())
}

/// `tdo sync`: the daemon runs it (behind any sync already running) and this waits for the
/// outcome. Exit 0 when every collection is fine, 1 when the run had conflicts, emptied
/// lists, a failure or was stopped, 2 when the daemon refused the call or is out of reach.
/// `--background` and `--when-due` only connect, so the installed timer keeps the daemon
/// alive; they print nothing.
pub fn sync(target: &Target, background: bool, _when_due: bool, force_delete: &[String]) -> i32 {
    let client = match connect(target) {
        Ok(c) => c,
        Err(code) => return code,
    };
    if background {
        return 0;
    }
    if let Err(code) = print_config_warnings(&client) {
        return code;
    }
    match run_and_wait(target, client, force_delete) {
        Ok(code) => code,
        Err(code) => code,
    }
}

fn run_and_wait(target: &Target, mut client: Client, force_delete: &[String]) -> Result<i32, i32> {
    let _: SubscribeResult = client
        .caller()
        .call_timeout("events.subscribe", serde_json::json!({}), CLI_TIMEOUT)
        .map_err(|e| client_error(&e))?;
    let rid = format!("sync-{}", client.next_request_id());
    let started: JobStarted = change(
        target,
        &mut client,
        "sync.run",
        &rid,
        &SyncRunParams {
            request_id: rid.clone(),
            force_delete: force_delete.to_vec(),
        },
        client::Unknown::SyncStarted,
    )?;
    let (client, state) = await_job(target, client, &started.job_id)?;
    match state {
        JobState::Finished {
            result: JobResult::Sync(r),
        } => {
            let _ = op::print_refusals(&client);
            Ok(report(&r))
        }
        JobState::Failed { message, .. } => Err(error(&message)),
        _ => Err(error(&"the daemon stopped before the sync ended")),
    }
}

/// Prints what the sync reported; the exit code.
fn report(r: &SyncResult) -> i32 {
    for p in &r.problems {
        eprintln!("tdo: {p}");
    }
    for w in &r.warnings {
        eprintln!("tdo: warning: {w}");
    }
    let failed = matches!(
        r.outcome,
        SyncOutcome::Failed | SyncOutcome::Stopped | SyncOutcome::Unknown
    );
    if !r.problems.is_empty() || failed {
        if r.problems.is_empty() {
            eprintln!(
                "tdo: sync {}",
                if r.outcome == SyncOutcome::Failed {
                    "failed"
                } else {
                    "stopped"
                }
            );
        }
        return 1;
    }
    if r.outcome == SyncOutcome::Ok {
        for n in &r.notes {
            println!("{n}");
        }
        println!("synced");
    }
    0
}

/// Waits for job `id` by `job.finished` events, asking `job.get` every 5 s, after `missed`
/// and after a lost connection (which is re-made, starting the daemon when nobody has).
/// Returns the connection it ended on with the job's final state.
fn await_job(target: &Target, client: Client, id: &str) -> Result<(Client, JobState), i32> {
    let mut client = client;
    let mut gone_since: Option<Instant> = None;
    let mut check_now = false;
    loop {
        if check_now {
            check_now = false;
            let got: Result<JobView, ClientError> = client.caller().call_timeout(
                "job.get",
                JobGetParams {
                    job_id: Some(id.to_string()),
                    request_id: None,
                },
                CLI_TIMEOUT,
            );
            match got {
                Ok(v) if v.state.is_final() => return Ok((client, v.state)),
                Ok(_) => gone_since = None,
                Err(ClientError::Rpc(e))
                    if e.data.as_ref().map(|d| d.kind) == Some(RpcKind::Unavailable) =>
                {
                    // Connected, but the daemon is not ready: ask again shortly.
                    std::thread::sleep(Duration::from_millis(200));
                    check_now = true;
                    continue;
                }
                Err(ClientError::Rpc(e)) => return Err(client_error(&ClientError::Rpc(e))),
                Err(_) => {
                    let since = *gone_since.get_or_insert_with(Instant::now);
                    if since.elapsed() > RECONNECT_FOR {
                        return Err(error(&"daemon not running: it did not come back"));
                    }
                    match client::connect(target, CLIENT, None) {
                        Ok((c, _)) => {
                            client = c;
                            let _: Result<SubscribeResult, _> = client.caller().call_timeout(
                                "events.subscribe",
                                serde_json::json!({}),
                                CLI_TIMEOUT,
                            );
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(500)),
                    }
                    check_now = true;
                    continue;
                }
            }
        }
        match client.notifications().recv_timeout(JOB_POLL) {
            Ok(Notification::Event(env)) => {
                if let Event::JobFinished { job_id, state, .. } = env.event
                    && job_id == id
                {
                    return Ok((client, state));
                }
            }
            Ok(Notification::Missed(_)) => check_now = true,
            Ok(Notification::Heartbeat(_)) => {}
            Err(_) => check_now = true,
        }
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

/// `tdo sync on|off`: the daemon switches automatic sync (turning it on with unsynced edits
/// starts a sync). Exit 0, or 2 when sync is not configured or the daemon is out of reach.
pub fn switch_command(target: &Target, on: bool) -> i32 {
    let mut client = match connect(target) {
        Ok(c) => c,
        Err(code) => return code,
    };
    if let Err(code) = print_config_warnings(&client) {
        return code;
    }
    let rid = format!("sync-{}", client.next_request_id());
    let params = SyncSetAutoParams {
        request_id: rid.clone(),
        on,
    };
    match change::<_, SyncSetAutoResult>(
        target,
        &mut client,
        "sync.set_auto",
        &rid,
        &params,
        client::Unknown::AutoSwitch { on },
    ) {
        Ok(r) => {
            println!("{}", r.message);
            0
        }
        Err(code) => code,
    }
}

/// Switches automatic sync for the old TUI (`:sync on|off`; goes with it). Turning it on with unsynced
/// edits starts a background sync at once. Once the switch is set, the call succeeds: an
/// error reading the unsynced edits starts a sync too, which checks again itself.
pub fn set_switch(cfg: &Config, vdir_flag: Option<&Path>, on: bool) -> std::io::Result<()> {
    switch::set(cfg, !on)?;
    if on && !matches!(batch::due(cfg, Utc::now()), Ok(Due::Nothing)) {
        spawn(cfg, vdir_flag, Trigger::SyncNow);
    }
    Ok(())
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
