//! `tdo op`: the core write operations as scriptable commands.
//!
//! Writes go to the daemon over its socket (nothing here reads the vdir or the config file;
//! `op check` is the exception, a local read). Exit 0 prints the operation id, or
//! `queued (a sync is running): <queue id>` when the daemon's lane is busy and the write
//! waits in its queue (`--wait` waits for its outcome instead of returning at once); 1 means
//! blocked, stale or stuck (reason on stderr); 2 is a usage, daemon or edit error.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use clap::{Args, Subcommand, ValueEnum};
use twydo_core::compat::Matrix;
use twydo_core::config::{Config, WriteMode};
use twydo_core::edit::{Change, Due, NewTask, Op};
use twydo_core::model::TaskKind;
use twydo_core::repeatinput::{self, RepeatSpec};
use twydo_core::views::Snapshot;
use twydo_daemon::client::{Client, ClientError, Notification};
use twydo_daemon::protocol::admin::{
    AbandonResult, JournalAbandonParams, JournalEntryView, JournalRevertParams, RecoverResult,
};
use twydo_daemon::protocol::events::Event;
use twydo_daemon::protocol::methods::{
    AckParams, BasedOn, OpGetParams, OpState, StatusView, SubmitParams, SubscribeResult,
    WriteOutcome, WriteResult,
};
use twydo_daemon::protocol::rpc::ErrorKind as RpcKind;
use twydo_daemon::protocol::wire::{WireSnapshot, from_wire};

use super::permissions::{probes, task_lines};
use super::{CliError, EXIT_ERROR, permission_snapshot, resolve_config};
use crate::client::{self, CLI_TIMEOUT, Target};

/// A task type, as `--kind` spells it.
#[derive(Clone, Copy, ValueEnum)]
pub enum KindArg {
    Task,
    Project,
    Checklist,
}

impl From<KindArg> for TaskKind {
    fn from(k: KindArg) -> Self {
        match k {
            KindArg::Task => TaskKind::Task,
            KindArg::Project => TaskKind::Project,
            KindArg::Checklist => TaskKind::Checklist,
        }
    }
}

/// How often `--wait` asks `op.get` when no event arrives.
const WAIT_POLL: Duration = Duration::from_secs(5);
/// How long to wait for a starting daemon to become ready.
const STARTUP_WAIT: Duration = Duration::from_secs(30);
/// How long `--wait` keeps trying to reach a daemon that went away.
const RECONNECT_FOR: Duration = Duration::from_secs(60);

const WAIT_HELP: &str =
    "If a sync holds the vdir, wait until the queued write is applied or refused";

/// Exit code for blocked, stale or stuck operations.
pub const EXIT_BLOCKED: i32 = 1;

#[derive(Subcommand)]
pub enum OpCommand {
    /// Create a task
    Add {
        #[arg(long, value_name = "ID")]
        list: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        notes: Option<String>,
        /// YYYY-MM-DD or YYYY-MM-DDTHH:MM
        #[arg(long, value_name = "DATE")]
        due: Option<String>,
        /// YYYY-MM-DD
        #[arg(long, value_name = "DATE")]
        start: Option<String>,
        /// Type of the new task
        #[arg(long, value_enum, default_value = "task")]
        kind: KindArg,
        /// UID of the checklist or project the new task belongs to
        #[arg(long, value_name = "UID")]
        parent: Option<String>,
        /// Repeat rule in words, e.g. "weekly", "2w", "mon thu" (without --due: due today)
        #[arg(long, value_name = "TEXT")]
        repeat: Option<String>,
        #[arg(long, help = WAIT_HELP)]
        wait: bool,
    },
    /// Change fields of a task
    Edit(EditArgs),
    /// Complete a task
    Complete {
        uid: String,
        /// Also complete the open items of a checklist or project
        #[arg(long)]
        all: bool,
        #[arg(long, help = WAIT_HELP)]
        wait: bool,
    },
    /// Reopen a completed task
    Reopen {
        uid: String,
        #[arg(long, help = WAIT_HELP)]
        wait: bool,
    },
    /// Delete a task
    Delete {
        uid: String,
        #[arg(long, help = WAIT_HELP)]
        wait: bool,
    },
    /// Move a task to another list
    Move {
        uid: String,
        #[arg(long, value_name = "ID")]
        to: String,
        #[arg(long, help = WAIT_HELP)]
        wait: bool,
    },
    /// Change one leaf’s parent membership (candidate; dedicated evidence required)
    Reparent {
        uid: String,
        #[arg(long, value_name = "ID")]
        list: String,
        #[arg(long, value_name = "UID")]
        parent: Option<String>,
        #[arg(long, help = WAIT_HELP)]
        wait: bool,
    },
    /// Revert an applied operation
    Undo {
        op_id: String,
        #[arg(long, help = WAIT_HELP)]
        wait: bool,
    },
    /// Show concrete permissions and actions needing proposed input for a task
    Check {
        uid: String,
        /// Compatibility matrix file (test hook)
        #[arg(long, value_name = "PATH", hide = true)]
        matrix: Option<PathBuf>,
    },
    /// List journal entries
    Journal,
    /// Run journal recovery and report the result
    Recover,
    /// Give up on an unfinished operation
    Abandon { op_id: String },
}

#[derive(Args)]
pub struct EditArgs {
    uid: String,
    #[arg(long)]
    title: Option<String>,
    #[arg(long, conflicts_with = "clear_notes")]
    notes: Option<String>,
    #[arg(long)]
    clear_notes: bool,
    /// YYYY-MM-DD or YYYY-MM-DDTHH:MM
    #[arg(long, value_name = "DATE", conflicts_with = "clear_due")]
    due: Option<String>,
    #[arg(long)]
    clear_due: bool,
    /// YYYY-MM-DD
    #[arg(long, value_name = "DATE", conflicts_with = "clear_start")]
    start: Option<String>,
    #[arg(long)]
    clear_start: bool,
    /// Repeat interval
    #[arg(long, value_name = "N")]
    every: Option<u32>,
    /// Repeat rule in words, e.g. "weekly", "2w", "mon thu"
    #[arg(long, value_name = "TEXT", conflicts_with = "no_repeat")]
    repeat: Option<String>,
    /// Remove the repeat rule
    #[arg(long)]
    no_repeat: bool,
    /// Change the task's type
    #[arg(long, value_enum)]
    kind: Option<KindArg>,
    #[arg(long, help = WAIT_HELP)]
    wait: bool,
}

/// A failure of an `op` command.
enum Failure {
    /// Exit 1.
    Blocked(String),
    /// Exit 2.
    Error(String),
}

impl From<CliError> for Failure {
    fn from(e: CliError) -> Self {
        Failure::Error(e.to_string())
    }
}

impl From<client::ConnectError> for Failure {
    fn from(e: client::ConnectError) -> Self {
        Failure::Error(e.to_string())
    }
}

impl From<ClientError> for Failure {
    fn from(e: ClientError) -> Self {
        match &e {
            ClientError::Rpc(r) => match r.data.as_ref().map(|d| d.kind) {
                Some(RpcKind::Blocked | RpcKind::Stale | RpcKind::Stuck) => {
                    Failure::Blocked(e.to_string())
                }
                _ => Failure::Error(e.to_string()),
            },
            ClientError::TimedOut(_) => Failure::Error("daemon did not answer; try again".into()),
            _ => Failure::Error(format!("daemon not running: {e}")),
        }
    }
}

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        Failure::Error(e.to_string())
    }
}

fn usage<T>(msg: impl Into<String>) -> Result<T, Failure> {
    Err(Failure::Error(msg.into()))
}

fn parse_date(s: &str) -> Result<NaiveDate, Failure> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .or_else(|_| usage(format!("invalid date {s:?}; expected YYYY-MM-DD")))
}

fn parse_due(s: &str) -> Result<Due, Failure> {
    if s.contains('T') {
        match NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M") {
            Ok(t) => Ok(Due::Time(t)),
            Err(_) => usage(format!(
                "invalid due {s:?}; expected YYYY-MM-DD or YYYY-MM-DDTHH:MM"
            )),
        }
    } else {
        parse_date(s).map(Due::Date)
    }
}

/// Parses `--repeat` text with the config's work days; a parse error is a usage error.
fn parse_repeat(text: &str, today: NaiveDate, cfg: &Config) -> Result<RepeatSpec, Failure> {
    match repeatinput::parse(text, today, &cfg.work_days) {
        Ok(Some(spec)) => Ok(spec),
        Ok(None) => usage(format!("{text:?} is not a repeat rule")),
        Err(e) => usage(format!("invalid repeat {text:?}: {e}")),
    }
}

/// `has_due` is whether the task will have a due date once the other changes apply. Core never
/// reads the clock, so a repeat on an undated task gets today's date first (2Do does the same).
/// `declared` is the type the task's file declares: `--kind` naming it changes nothing, so it
/// is left out rather than written as new stamps alone.
fn edit_changes(
    a: &EditArgs,
    cfg: &Config,
    today: NaiveDate,
    has_due: bool,
    declared: Option<TaskKind>,
) -> Result<Vec<Change>, Failure> {
    let mut changes = Vec::new();
    if let Some(t) = &a.title {
        changes.push(Change::Summary(t.clone()));
    }
    if let Some(n) = &a.notes {
        changes.push(Change::Description(Some(n.clone())));
    } else if a.clear_notes {
        changes.push(Change::Description(None));
    }
    if let Some(d) = &a.due {
        changes.push(Change::Due(Some(parse_due(d)?)));
    } else if a.clear_due {
        changes.push(Change::Due(None));
    }
    if let Some(d) = &a.start {
        changes.push(Change::Start(Some(parse_date(d)?)));
    } else if a.clear_start {
        changes.push(Change::Start(None));
    }
    if let Some(n) = a.every {
        changes.push(Change::RepeatInterval(n));
    }
    let mut same_kind = None;
    if let Some(k) = a.kind {
        let k: TaskKind = k.into();
        if declared == Some(k) {
            same_kind = Some(k);
        } else {
            changes.push(Change::Kind(k));
        }
    }
    if let Some(text) = &a.repeat {
        let spec = parse_repeat(text, today, cfg)?;
        if !has_due && a.due.is_none() {
            changes.push(Change::Due(Some(Due::Date(today))));
        }
        changes.push(Change::Repeat(Some(spec)));
    } else if a.no_repeat {
        changes.push(Change::Repeat(None));
    }
    if changes.is_empty() {
        if let Some(k) = same_kind {
            let name = match k {
                TaskKind::Task => "task",
                TaskKind::Checklist => "checklist",
                TaskKind::Project => "project",
            };
            return usage(format!("the task is already a {name}; nothing to change"));
        }
        return usage("nothing to change; pass at least one option");
    }
    Ok(changes)
}

fn check_lines(
    snap: &Snapshot,
    matrix: &Matrix,
    mode: WriteMode,
    uid: &str,
    now: DateTime<Utc>,
) -> Result<Vec<String>, Failure> {
    let Some(idx) = snap.find(uid) else {
        return Err(Failure::Blocked(format!("no task with UID {uid}")));
    };
    Ok(task_lines(&probes(snap, matrix, mode, idx, now)))
}

/// `tdo op check`: a local read of the vdir (no daemon), with `--matrix` for tests.
pub fn check_command(vdir: Option<&Path>, matrix: Option<&PathBuf>, uid: &str) -> i32 {
    match check_inner(vdir, matrix, uid) {
        Ok(()) => 0,
        Err(f) => report(f),
    }
}

fn report(f: Failure) -> i32 {
    match f {
        Failure::Blocked(m) => {
            eprintln!("tdo: {m}");
            EXIT_BLOCKED
        }
        Failure::Error(m) => {
            eprintln!("tdo: {m}");
            EXIT_ERROR
        }
    }
}

fn check_inner(vdir: Option<&Path>, matrix: Option<&PathBuf>, uid: &str) -> Result<(), Failure> {
    let cfg = resolve_config(vdir)?;
    let matrix = match matrix {
        Some(p) => Matrix::load(p).map_err(CliError::from)?,
        None => Matrix::embedded().clone(),
    };
    if cfg.writes.mode == WriteMode::Test {
        eprintln!("tdo: test write mode: unverified operations are enabled");
    }
    let snap = permission_snapshot(&cfg)?;
    for line in check_lines(&snap, &matrix, cfg.writes.mode, uid, Utc::now())? {
        println!("{line}");
    }
    Ok(())
}

/// Run an `op` subcommand; returns the process exit code.
pub fn run(target: &Target, cmd: OpCommand) -> i32 {
    if let OpCommand::Check { uid, matrix } = &cmd {
        return check_command(target.vdir.as_deref(), matrix.as_ref(), uid);
    }
    match run_inner(target, cmd) {
        Ok(()) => 0,
        Err(f) => report(f),
    }
}

const CLIENT: &str = "tdo-op";

/// A request id: `op-` marks it as a CLI write, so other CLI runs print its refusal.
fn request_id(c: &Client) -> String {
    format!("op-{}", c.next_request_id())
}

fn rpc<P: serde::Serialize, R: serde::de::DeserializeOwned>(
    c: &Client,
    method: &str,
    params: P,
) -> Result<R, Failure> {
    Ok(c.caller().call_timeout(method, params, CLI_TIMEOUT)?)
}

/// `snapshot.get`, retried while the daemon is still starting (it answers hello first).
fn snapshot(c: &Client) -> Result<WireSnapshot, Failure> {
    Ok(client::call_ready(
        &c.caller(),
        "snapshot.get",
        &serde_json::json!({}),
        CLI_TIMEOUT,
        STARTUP_WAIT,
    )?)
}

fn run_inner(target: &Target, cmd: OpCommand) -> Result<(), Failure> {
    let (client, hello) = client::connect(target, CLIENT, None)?;
    let session = hello.session.clone();
    let wire = snapshot(&client)?;
    let based_on = BasedOn {
        epoch: hello.epoch.clone(),
        revision: wire.revision,
    };
    let snap = from_wire(wire);
    let cfg = &snap.config;
    if cfg.writes.mode == WriteMode::Test {
        eprintln!("tdo: test write mode: unverified operations are enabled");
    }
    print_refusals(&client)?;
    let now = Utc::now();
    let today = now.with_timezone(&cfg.tz()).date_naive();

    let wait;
    let op = match cmd {
        OpCommand::Add {
            list,
            title,
            notes,
            due,
            start,
            kind,
            parent,
            repeat,
            wait: w,
        } => {
            wait = w;
            let mut due = due.as_deref().map(parse_due).transpose()?;
            let repeat = repeat
                .as_deref()
                .map(|t| parse_repeat(t, today, cfg))
                .transpose()?;
            if repeat.is_some() && due.is_none() {
                due = Some(Due::Date(today));
            }
            Op::Create(NewTask {
                description: notes,
                due,
                start: start.as_deref().map(parse_date).transpose()?,
                kind: kind.into(),
                parent,
                repeat,
                ..NewTask::plain(&list, &title)
            })
        }
        OpCommand::Edit(a) => {
            wait = a.wait;
            let t = snap.find(&a.uid).map(|i| &snap.tasks[i]);
            let (has_due, declared) = if a.repeat.is_some() || a.kind.is_some() {
                (
                    t.is_some_and(|t| t.due.is_some()) && !a.clear_due,
                    t.map(twydo_core::views::declared_kind),
                )
            } else {
                (true, None)
            };
            Op::Edit {
                changes: edit_changes(&a, cfg, today, has_due, declared)?,
                uid: a.uid,
            }
        }
        OpCommand::Complete { uid, all, wait: w } => {
            wait = w;
            if !all && let Some(i) = snap.find(&uid) {
                let open = snap.open_items(i);
                if open > 0 {
                    return Err(Failure::Blocked(format!(
                        "\"{}\" has {open} open {}; pass --all to complete them too",
                        snap.tasks[i].summary,
                        if open == 1 { "item" } else { "items" }
                    )));
                }
            }
            Op::Complete { uid }
        }
        OpCommand::Reopen { uid, wait: w } => {
            wait = w;
            Op::Reopen { uid }
        }
        OpCommand::Delete { uid, wait: w } => {
            wait = w;
            Op::Delete { uid }
        }
        OpCommand::Move { uid, to, wait: w } => {
            wait = w;
            Op::Move { uid, to }
        }
        OpCommand::Reparent {
            uid,
            list,
            parent,
            wait: w,
        } => {
            wait = w;
            Op::Reparent {
                uid,
                collection: list,
                parent,
            }
        }
        OpCommand::Undo { op_id, wait } => {
            let r: WriteResult = rpc(
                &client,
                "journal.revert",
                JournalRevertParams {
                    session,
                    request_id: request_id(&client),
                    op_id,
                },
            )?;
            return finish(target, client, r, wait);
        }
        OpCommand::Abandon { op_id } => {
            let _: AbandonResult = rpc(
                &client,
                "journal.abandon",
                JournalAbandonParams {
                    request_id: request_id(&client),
                    op_id: op_id.clone(),
                },
            )?;
            println!("abandoned {op_id}");
            return Ok(());
        }
        OpCommand::Recover => {
            let r: RecoverResult = rpc(&client, "journal.recover", serde_json::json!({}))?;
            println!("resumed: {}", r.resumed.len());
            println!("stuck: {}", r.stuck.len());
            for s in &r.stuck {
                println!("  {}: {}", s.id, s.reason);
            }
            return Ok(());
        }
        OpCommand::Journal => {
            let entries: Vec<JournalEntryView> =
                rpc(&client, "journal.list", serde_json::json!({}))?;
            for e in entries {
                println!(
                    "{}  {}  {}  {}  {}  {}",
                    e.id,
                    e.created.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    e.kind,
                    serde_json::to_value(e.stage)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_string))
                        .unwrap_or_default(),
                    e.uids.join(","),
                    match e.synced {
                        Some(t) => format!(
                            "synced {}",
                            t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
                        ),
                        None => "pending".to_string(),
                    }
                );
            }
            return Ok(());
        }
        OpCommand::Check { .. } => unreachable!("handled by run"),
    };
    let r: WriteResult = rpc(
        &client,
        "op.submit",
        SubmitParams {
            session,
            request_id: request_id(&client),
            based_on,
            op,
        },
    )?;
    finish(target, client, r, wait)
}

/// Prints the refusals of queued writes no screen has shown yet (those of earlier CLI runs,
/// or without a session) once, and acknowledges them.
fn print_refusals(c: &Client) -> Result<(), Failure> {
    let status: StatusView = rpc(c, "status.get", serde_json::json!({}))?;
    let mut ids = Vec::new();
    for r in status.refusals {
        let ours = r.session.is_none()
            || r.request_id
                .as_deref()
                .is_some_and(|id| id.starts_with("op-"));
        if ours {
            eprintln!("tdo: {}", r.message);
            ids.push(r.id);
        }
    }
    if !ids.is_empty() {
        let _: serde_json::Value = rpc(
            c,
            "refusals.ack",
            AckParams {
                request_id: request_id(c),
                ids,
            },
        )?;
    }
    Ok(())
}

/// Reports what a submit did. Applied: the journal entry id. Queued: `queued (a sync is
/// running): <queue id>` at once, or with `--wait` the outcome once the item is applied or
/// refused.
fn finish(target: &Target, client: Client, r: WriteResult, wait: bool) -> Result<(), Failure> {
    match r.outcome {
        WriteOutcome::Queued => {
            if !wait {
                println!("queued (a sync is running): {}", r.op_id);
                return Ok(());
            }
            let id = wait_for(target, client, &r.op_id)?;
            println!("{id}");
        }
        _ => println!("{}", r.op_id),
    }
    Ok(())
}

/// What `op.get` or `op.finished` says about a queued write.
enum Verdict {
    Pending,
    Done(String),
    Failed(Failure),
}

fn verdict(c: &Client, queue_id: &str, state: OpState) -> Verdict {
    match state {
        OpState::Applied { op_id } | OpState::Satisfied { op_id } => Verdict::Done(op_id),
        OpState::Refused { kind, message } => {
            // Shown here, so nothing repeats it later.
            let _: Result<serde_json::Value, _> = rpc(
                c,
                "refusals.ack",
                AckParams {
                    request_id: request_id(c),
                    ids: vec![queue_id.to_string()],
                },
            );
            Verdict::Failed(match kind {
                RpcKind::Blocked | RpcKind::Stale | RpcKind::Stuck => Failure::Blocked(message),
                _ => Failure::Error(message),
            })
        }
        OpState::Cancelled => Verdict::Failed(Failure::Blocked(format!(
            "queued write {queue_id} was cancelled before it was applied"
        ))),
        OpState::Pending | OpState::Unknown => Verdict::Pending,
    }
}

/// Waits for the queued write `id` to be applied or refused, by `op.finished` events, asking
/// `op.get` every 5 s and after `missed` or a reconnect. Returns the journal entry id.
fn wait_for(target: &Target, client: Client, id: &str) -> Result<String, Failure> {
    eprintln!("waiting for the sync to finish (queued {id})");
    let mut client = client;
    let mut subscribed =
        rpc::<_, SubscribeResult>(&client, "events.subscribe", serde_json::json!({})).is_ok();
    let mut gone_since: Option<Instant> = None;
    let mut check_now = true;
    loop {
        if check_now {
            check_now = false;
            let got: Result<OpState, ClientError> = client.caller().call_timeout(
                "op.get",
                OpGetParams {
                    op_id: Some(id.to_string()),
                    request_id: None,
                },
                CLI_TIMEOUT,
            );
            match got {
                Ok(state) => {
                    gone_since = None;
                    match verdict(&client, id, state) {
                        Verdict::Done(op) => return Ok(op),
                        Verdict::Failed(f) => return Err(f),
                        Verdict::Pending => {}
                    }
                }
                Err(ClientError::Rpc(e))
                    if matches!(e.data.as_ref().map(|d| d.kind), Some(RpcKind::Unavailable)) =>
                {
                    // Connected, but the daemon is not ready (or is shutting down): ask
                    // again shortly, for as long as it keeps saying so.
                    let since = *gone_since.get_or_insert_with(Instant::now);
                    if since.elapsed() > RECONNECT_FOR {
                        return Err(Failure::Error(
                            "daemon not running: it did not come back".into(),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(200));
                    check_now = true;
                    continue;
                }
                Err(ClientError::Rpc(e)) => return Err(ClientError::Rpc(e).into()),
                Err(_) => {
                    // The daemon went away (or is starting): connect again, which starts it
                    // when nobody has. The session is gone; `op.get` needs none.
                    let since = *gone_since.get_or_insert_with(Instant::now);
                    if since.elapsed() > RECONNECT_FOR {
                        return Err(Failure::Error(
                            "daemon not running: it did not come back".into(),
                        ));
                    }
                    match client::connect(target, CLIENT, None) {
                        Ok((c, _)) => {
                            client = c;
                            subscribed = rpc::<_, SubscribeResult>(
                                &client,
                                "events.subscribe",
                                serde_json::json!({}),
                            )
                            .is_ok();
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(500)),
                    }
                    check_now = true;
                    continue;
                }
            }
            if !subscribed {
                std::thread::sleep(Duration::from_millis(500));
                check_now = true;
                continue;
            }
        }
        match client.notifications().recv_timeout(WAIT_POLL) {
            Ok(Notification::Event(env)) => {
                if let Event::OpFinished { op_id, state, .. } = env.event
                    && op_id == id
                {
                    match verdict(&client, id, state) {
                        Verdict::Done(op) => return Ok(op),
                        Verdict::Failed(f) => return Err(f),
                        Verdict::Pending => check_now = true,
                    }
                }
            }
            Ok(Notification::Missed(_)) => check_now = true,
            Ok(Notification::Heartbeat(_)) => {}
            Err(_) => check_now = true,
        }
    }
}
