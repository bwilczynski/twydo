//! `tdo op`: the core write operations as scriptable commands.
//!
//! Exit 0 prints the operation id, or `queued (a sync is running): <queue id>` when a sync holds
//! the vdir and the write waits in the queue (`--wait` waits for its outcome instead of
//! returning at once); 1 means blocked, stale or stuck (reason on stderr); 2 is a usage,
//! configuration, IO or edit error.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use clap::{Args, Subcommand, ValueEnum};
use twydo_core::compat::Matrix;
use twydo_core::config::{Config, WriteMode};
use twydo_core::edit::{Change, Due, NewTask, Op};
use twydo_core::gate;
use twydo_core::model::TaskKind;
use twydo_core::repeatinput::{self, RepeatSpec};
use twydo_core::store::fsio::VdirLock;
use twydo_core::store::journal::Journal;
use twydo_core::store::queue::{self, Queue, Refusal, refusal_message};
use twydo_core::store::{Store, Submitted, WriteError, existing_vdir_state};
use twydo_core::sync::batch::after_write;
use twydo_core::views::Snapshot;

use super::sync::{print_refusals, spawn};
use super::{CliError, EXIT_ERROR, resolve_config};
use crate::write::op_title;

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

/// How often `--wait` looks at the queue.
const WAIT_POLL: Duration = Duration::from_millis(100);

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
    /// Revert an applied operation
    Undo {
        op_id: String,
        #[arg(long, help = WAIT_HELP)]
        wait: bool,
    },
    /// Show whether each operation kind is allowed for a task
    Check { uid: String },
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

impl From<WriteError> for Failure {
    fn from(e: WriteError) -> Self {
        match e {
            WriteError::Blocked(_) | WriteError::Stale(_) | WriteError::Stuck(_) => {
                Failure::Blocked(e.to_string())
            }
            _ => Failure::Error(e.to_string()),
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

/// Load the vdir (with queued writes laid over it) under the shared lock of `state` (the store's per-vdir state dir), so the
/// gate and `op check` never see a partly applied operation. The lock is released before
/// this returns: the store takes the exclusive lock on its own descriptor, and two flocks in
/// one process block each other (R2).
fn snapshot(cfg: &Config, state: &Path) -> Result<Snapshot, Failure> {
    let lock = VdirLock::reader(state).map_err(|source| CliError::Lock {
        path: state.to_path_buf(),
        source,
    })?;
    let report = twydo_core::vdir::load(&cfg.vdir).map_err(|source| CliError::Vdir {
        path: cfg.vdir.clone(),
        source,
    });
    drop(lock);
    Ok(queue::view(report?, cfg.clone()))
}

const CHECK_KINDS: [&str; 6] = ["create", "edit", "complete", "reopen", "move", "delete"];

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
    let task = &snap.tasks[idx];
    let today = now.with_timezone(&snap.tz()).date_naive();
    let other = snap.collections.iter().find(|c| c.id != task.collection);
    let mut out = Vec::new();
    for kind in CHECK_KINDS {
        let op = match kind {
            "create" => Op::Create(NewTask::plain(&task.collection, "x")),
            "edit" => Op::Edit {
                uid: uid.to_string(),
                changes: vec![Change::Summary(task.summary.clone())],
            },
            "complete" => Op::Complete {
                uid: uid.to_string(),
            },
            "reopen" => Op::Reopen {
                uid: uid.to_string(),
            },
            "move" => match other {
                Some(c) => Op::Move {
                    uid: uid.to_string(),
                    to: c.id.clone(),
                },
                None => {
                    out.push("move: no other list".to_string());
                    continue;
                }
            },
            _ => Op::Delete {
                uid: uid.to_string(),
            },
        };
        out.push(match gate::check(snap, matrix, mode, &op, today) {
            Ok(()) => format!("{kind}: ok"),
            Err(b) => format!("{kind}: {b}"),
        });
    }
    Ok(out)
}

/// Run an `op` subcommand; returns the process exit code.
pub fn run(vdir: Option<&Path>, matrix: Option<&PathBuf>, cmd: OpCommand) -> i32 {
    match run_inner(vdir, matrix, cmd) {
        Ok(()) => 0,
        Err(Failure::Blocked(m)) => {
            eprintln!("tdo: {m}");
            EXIT_BLOCKED
        }
        Err(Failure::Error(m)) => {
            eprintln!("tdo: {m}");
            EXIT_ERROR
        }
    }
}

fn run_inner(vdir: Option<&Path>, matrix: Option<&PathBuf>, cmd: OpCommand) -> Result<(), Failure> {
    let cfg = resolve_config(vdir)?;
    let matrix = match matrix {
        Some(p) => Matrix::load(p).map_err(CliError::from)?,
        None => Matrix::embedded().clone(),
    };
    if cfg.writes.mode == WriteMode::Test {
        eprintln!("tdo: test write mode: unverified operations are enabled");
    }
    let now = Utc::now();
    let today = now.with_timezone(&cfg.tz()).date_naive();
    let (mut store, report) = Store::open_with(&cfg, matrix.clone())?;
    print_refusals(&cfg);
    let journal_dir = store.state_dir().join("journal");
    for (id, reason) in &report.stuck {
        if reason.starts_with("unreadable journal entry") {
            eprintln!(
                "tdo: {}: unreadable journal entry; move it out of {} to continue",
                journal_dir.join(format!("{id}.json")).display(),
                journal_dir.display()
            );
        }
    }

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
                .map(|t| parse_repeat(t, today, &cfg))
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
            let (has_due, declared) = if a.repeat.is_some() || a.kind.is_some() {
                let snap = snapshot(&cfg, store.state_dir())?;
                let t = snap.find(&a.uid).map(|i| &snap.tasks[i]);
                (
                    t.is_some_and(|t| t.due.is_some()) && !a.clear_due,
                    t.map(twydo_core::views::declared_kind),
                )
            } else {
                (true, None)
            };
            Op::Edit {
                changes: edit_changes(&a, &cfg, today, has_due, declared)?,
                uid: a.uid,
            }
        }
        OpCommand::Complete { uid, all, wait: w } => {
            wait = w;
            if !all {
                let snap = snapshot(&cfg, store.state_dir())?;
                if let Some(i) = snap.find(&uid) {
                    let open = snap.open_items(i);
                    if open > 0 {
                        return Err(Failure::Blocked(format!(
                            "\"{}\" has {open} open {}; pass --all to complete them too",
                            snap.tasks[i].summary,
                            if open == 1 { "item" } else { "items" }
                        )));
                    }
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
        OpCommand::Undo { op_id, wait } => {
            let submitted = store.undo(&op_id, now)?;
            return finish(&mut store, &cfg, vdir, submitted, wait);
        }
        OpCommand::Abandon { op_id } => {
            store.abandon(&op_id)?;
            println!("abandoned {op_id}");
            return Ok(());
        }
        OpCommand::Recover => {
            println!("resumed: {}", report.resumed.len());
            println!("stuck: {}", report.stuck.len());
            for (id, reason) in &report.stuck {
                println!("  {id}: {reason}");
            }
            return Ok(());
        }
        OpCommand::Journal => {
            if report
                .stuck
                .iter()
                .any(|(_, r)| r.starts_with("unreadable journal entry"))
            {
                return usage("journal has unreadable entries");
            }
            let Some(state) = existing_vdir_state(&cfg.state_dir(), &cfg.vdir)? else {
                return Ok(());
            };
            let journal = twydo_core::store::journal::Journal::open(&state)?;
            for e in journal.list()? {
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
        OpCommand::Check { uid } => {
            let snap = snapshot(&cfg, store.state_dir())?;
            for line in check_lines(&snap, &matrix, cfg.writes.mode, &uid, now)? {
                println!("{line}");
            }
            return Ok(());
        }
    };
    let snap = snapshot(&cfg, store.state_dir())?;
    let label = op_title(&snap, &op);
    let submitted = store.submit(&snap, &op, &label, "op", now)?;
    finish(&mut store, &cfg, vdir, submitted, wait)
}

/// Reports what a submit did. Applied: the journal entry id. Queued: `queued (a sync is
/// running): <queue id>` at once, or with `--wait` the outcome once the item is applied or
/// refused.
fn finish(
    store: &mut Store,
    cfg: &Config,
    vdir: Option<&Path>,
    submitted: Submitted,
    wait: bool,
) -> Result<(), Failure> {
    match submitted {
        Submitted::Applied(id) => println!("{id}"),
        Submitted::Cancelled => println!("cancelled"),
        Submitted::Queued(item) => {
            if !wait {
                println!("queued (a sync is running): {}", item.id);
                return Ok(());
            }
            let id = wait_for(store, &item.id)?;
            println!("{id}");
        }
    }
    sync_after_write(cfg, vdir);
    Ok(())
}

/// Waits until queued item `id` is neither pending nor claimed, then returns the journal entry
/// the drain made from it. While it waits it drains the queue itself whenever the vdir lock is
/// free (the sync that held it may be gone, and nothing else is bound to drain). A refusal is printed, marked shown and fails with the blocked exit
/// code; an item that vanished without either (someone cancelled it) fails the same way.
fn wait_for(store: &mut Store, id: &str) -> Result<String, Failure> {
    let state = store.state_dir().to_path_buf();
    let state = state.as_path();
    let dir = state.join("queue");
    // Pending first, then claimed: a drain renames the one to the other, so checking in this
    // order never sees an item in flight as gone.
    while dir.join(format!("{id}.json")).exists() || dir.join(format!("{id}.claim")).exists() {
        match store.drain(Utc::now()) {
            Ok(_) | Err(WriteError::Busy) => {}
            Err(e) => return Err(e.into()),
        }
        std::thread::sleep(WAIT_POLL);
    }
    if state.join("journal").is_dir()
        && let Some(e) = Journal::open(state)?.by_queue_id(id)?
    {
        return Ok(e.id);
    }
    let refusal = std::fs::read(dir.join("refused").join(format!("{id}.json")))
        .ok()
        .and_then(|b| serde_json::from_slice::<Refusal>(&b).ok());
    match refusal {
        Some(r) => {
            if let Some(q) = Queue::existing(state) {
                let _ = q.mark_shown(id);
            }
            Err(Failure::Blocked(refusal_message(&r)))
        }
        None => Err(Failure::Blocked(format!(
            "queued write {id} was cancelled before it was applied"
        ))),
    }
}

/// After a write that exited 0: a background sync when the batch is due, else the batch
/// waiter (nothing without `[sync]` or while switched off).
fn sync_after_write(cfg: &Config, vdir: Option<&Path>) {
    if let Some(t) = after_write(cfg, Utc::now()) {
        spawn(cfg, vdir, t);
    }
}
