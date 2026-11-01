//! `tdo op`: the core write operations as scriptable commands.
//!
//! Exit 0 prints the operation id; 1 means blocked, stale or stuck (reason on stderr);
//! 2 is a usage, configuration, IO or edit error.

use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use clap::{Args, Subcommand};
use twydo_core::compat::Matrix;
use twydo_core::config::{Config, WriteMode};
use twydo_core::edit::{Change, Due, NewTask, Op};
use twydo_core::gate;
use twydo_core::store::{Store, WriteError, existing_vdir_state};
use twydo_core::views::Snapshot;

use super::{CliError, EXIT_ERROR, resolve_config};

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
    },
    /// Change fields of a task
    Edit(EditArgs),
    /// Complete a task
    Complete { uid: String },
    /// Reopen a completed task
    Reopen { uid: String },
    /// Delete a task
    Delete { uid: String },
    /// Move a task to another list
    Move {
        uid: String,
        #[arg(long, value_name = "ID")]
        to: String,
    },
    /// Revert an applied operation
    Undo { op_id: String },
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

fn edit_changes(a: &EditArgs) -> Result<Vec<Change>, Failure> {
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
    if changes.is_empty() {
        return usage("nothing to change; pass at least one option");
    }
    Ok(changes)
}

fn snapshot(cfg: &Config) -> Result<Snapshot, Failure> {
    let report = twydo_core::vdir::load(&cfg.vdir).map_err(|source| CliError::Vdir {
        path: cfg.vdir.clone(),
        source,
    })?;
    Ok(Snapshot::new(report, cfg.clone()))
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
            "create" => Op::Create(NewTask {
                collection: task.collection.clone(),
                summary: "x".to_string(),
                description: None,
                due: None,
                start: None,
            }),
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
    let (mut store, report) = Store::open_with(&cfg, matrix.clone())?;
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

    let op = match cmd {
        OpCommand::Add {
            list,
            title,
            notes,
            due,
            start,
        } => Op::Create(NewTask {
            collection: list,
            summary: title,
            description: notes,
            due: due.as_deref().map(parse_due).transpose()?,
            start: start.as_deref().map(parse_date).transpose()?,
        }),
        OpCommand::Edit(a) => Op::Edit {
            changes: edit_changes(&a)?,
            uid: a.uid,
        },
        OpCommand::Complete { uid } => Op::Complete { uid },
        OpCommand::Reopen { uid } => Op::Reopen { uid },
        OpCommand::Delete { uid } => Op::Delete { uid },
        OpCommand::Move { uid, to } => Op::Move { uid, to },
        OpCommand::Undo { op_id } => {
            println!("{}", store.undo(&op_id, now)?);
            return Ok(());
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
                    "{}  {}  {}  {}  {}",
                    e.id,
                    e.created.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    e.kind,
                    serde_json::to_value(e.stage)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_string))
                        .unwrap_or_default(),
                    e.uids.join(",")
                );
            }
            return Ok(());
        }
        OpCommand::Check { uid } => {
            let snap = snapshot(&cfg)?;
            for line in check_lines(&snap, &matrix, cfg.writes.mode, &uid, now)? {
                println!("{line}");
            }
            return Ok(());
        }
    };
    let snap = snapshot(&cfg)?;
    println!("{}", store.apply(&snap, &op, now)?);
    Ok(())
}
