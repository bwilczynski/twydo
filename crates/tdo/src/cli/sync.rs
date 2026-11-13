//! `tdo sync`, `tdo status` and `tdo sync on|off`.

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use twydo_core::sync::report::NOT_CONFIGURED;
use twydo_daemon::client::{Client, ClientError, Notification};
use twydo_daemon::protocol::admin::{ConflictList, WireConflictState};
use twydo_daemon::protocol::events::Event;
use twydo_daemon::protocol::jobs::{JobGetParams, JobResult, JobStarted, JobState, JobView};
use twydo_daemon::protocol::methods::{StatusView, SubscribeResult, WireCollState};
use twydo_daemon::protocol::rpc::ErrorKind as RpcKind;
use twydo_daemon::protocol::sync::{
    SyncOutcome, SyncResult, SyncRunParams, SyncSetAutoParams, SyncSetAutoResult,
};
use twydo_daemon::protocol::wire::WireSnapshot;

use super::conflicts::kind_words;
use super::{EXIT_ERROR, op};
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
pub fn sync(
    target: &Target,
    background: bool,
    _when_due: bool,
    force_delete: &[String],
    discover: bool,
) -> i32 {
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
    match run_and_wait(target, client, force_delete, discover) {
        Ok(code) => code,
        Err(code) => code,
    }
}

fn run_and_wait(
    target: &Target,
    mut client: Client,
    force_delete: &[String],
    discover: bool,
) -> Result<i32, i32> {
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
            discover,
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

/// A list's sync state in words, as `describe` words the core's `CollState`.
pub(super) fn describe_wire(s: &WireCollState) -> String {
    match s {
        WireCollState::Ok => "ok".into(),
        WireCollState::Conflict => "conflict".into(),
        WireCollState::EmptiedLocal => "emptied here".into(),
        WireCollState::EmptiedRemote => "emptied on the server".into(),
        WireCollState::Error { message } => format!("error: {message}"),
        WireCollState::Unknown => "unknown".into(),
    }
}

/// Warns on stderr when journal recovery left stuck entries (`status.get`); a failed call
/// is not reported here, the command's own calls report it.
pub(super) fn warn_stuck_journal(client: &Client) {
    if let Ok(s) = ask::<StatusView>(client, "status.get")
        && !s.journal.stuck.is_empty()
    {
        eprintln!("tdo: journal has unfinished operations; run 'tdo op journal'");
    }
}

/// `tdo status`: last success, pending entries, conflicts, failures and per-list state, all
/// from the daemon. Exit 0, or 2 when sync is not configured or the daemon is out of reach.
pub fn status(target: &Target) -> i32 {
    let client = match connect(target) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let snap: WireSnapshot = match ask(&client, "snapshot.get") {
        Ok(v) => v,
        Err(e) => return client_error(&e),
    };
    let st: StatusView = match ask(&client, "status.get") {
        Ok(v) => v,
        Err(e) => return client_error(&e),
    };
    let found: ConflictList = match ask(&client, "conflicts.list") {
        Ok(v) => v,
        Err(e) => return client_error(&e),
    };
    for w in &snap.config_warnings {
        eprintln!("warning: {w}");
    }
    warn_stuck_journal(&client);
    let Some(sync) = &st.sync else {
        return error(&NOT_CONFIGURED);
    };
    let tz = snap.config.tz();
    let when = |t: Option<DateTime<Utc>>| match t {
        Some(t) => t.with_timezone(&tz).format("%Y-%m-%d %H:%M").to_string(),
        None => "never".to_string(),
    };
    let open: Vec<_> = found
        .conflicts
        .iter()
        .filter(|c| matches!(c.state, WireConflictState::Open | WireConflictState::Chosen))
        .collect();
    println!("last sync: {}", when(sync.last_success));
    println!("last attempt: {}", when(sync.last_attempt));
    println!("automatic sync: {}", if sync.auto { "on" } else { "off" });
    println!("pending: {}", st.journal.unfinished);
    println!("conflicts: {}", open.len());
    for c in &open {
        println!("  {}  {}  {}", c.id, c.path, kind_words(c.kind));
    }
    print!("failures: {}", sync.failures);
    if sync.failures > 0 {
        print!(" (next attempt {})", when(sync.next_attempt));
    }
    println!();
    if let Some(e) = &sync.last_error {
        println!("last error: {e}");
    }
    if let Some(e) = &sync.metasync_error {
        println!("metasync error: {e}");
    }
    if let Some(e) = &sync.discovery_error {
        println!("discovery error: {e}");
    }
    if !sync.collections.is_empty() {
        println!("lists:");
        for (coll, state) in &sync.collections {
            println!("  {}: {}", list_label(&snap, coll), describe_wire(state));
        }
    }
    0
}

/// A list as `tdo status` names it: its id, then its display name when it has another one.
fn list_label(snap: &WireSnapshot, id: &str) -> String {
    match snap
        .collections
        .iter()
        .find(|c| c.id == id)
        .and_then(|c| c.display_name.as_deref())
    {
        Some(name) if name != id => format!("{id} ({name})"),
        _ => id.to_string(),
    }
}

fn ask<R: serde::de::DeserializeOwned>(client: &Client, method: &str) -> Result<R, ClientError> {
    client::call_ready(
        &client.caller(),
        method,
        &serde_json::json!({}),
        CLI_TIMEOUT,
        STARTUP_WAIT,
    )
}
