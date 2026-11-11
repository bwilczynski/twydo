//! `check.report`: data health, recurrence evidence and concrete operation admission, the
//! report `tdo check` and `tdo op check` print.

use std::collections::BTreeMap;
use std::sync::Arc;

use std::path::Path;

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Map, Value, json};
use twydo_core::compat::{Matrix, combined_verdicts};
use twydo_core::config::WriteMode;
use twydo_core::ical::value::{IcalDateTime, Resolved};
use twydo_core::model::Status;
use twydo_core::model::{Task, TaskKind, Trigger};
use twydo_core::views::Snapshot;

use twydo_core::views::task_rule_class;

use super::Shared;
use super::permissions::{Availability, probes, probes_with, task_lines};
use crate::protocol::reports::{CheckReport, CheckReportParams, ExportResult};
use crate::protocol::rpc::{ErrorKind, RpcError};

/// `check.report`: the published snapshot (queued edits included), against the request's
/// matrix or the daemon's own. Runs on the request thread; it reads no file but the matrix.
pub fn check_report(shared: &Arc<Shared>, p: CheckReportParams) -> Result<Value, RpcError> {
    let loaded;
    let matrix = match &p.matrix {
        None => &shared.matrix,
        Some(path) => {
            loaded = Matrix::load(path)
                .map_err(|e| RpcError::refused(ErrorKind::Invalid, e.to_string()))?;
            &loaded
        }
    };
    let (_, published) = shared.publisher.current();
    let snap = &published.snap;
    let now = Utc::now();
    let mut warnings = Vec::new();
    let lines = match &p.uid {
        None => check_lines(snap, matrix, p.verbose, now),
        Some(uid) => {
            let Some(idx) = snap.find(uid) else {
                return Err(RpcError::refused(
                    ErrorKind::NotFound,
                    format!("no task with UID {uid}"),
                ));
            };
            let mode = snap.config.writes.mode;
            if mode == WriteMode::Test {
                warnings.push("test write mode: unverified operations are enabled".to_string());
            }
            task_lines(&probes(snap, matrix, mode, idx, now))
        }
    };
    serde_json::to_value(CheckReport { lines, warnings })
        .map_err(|e| RpcError::refused(ErrorKind::Internal, e.to_string()))
}

/// The whole-collection report, line by line.
pub fn check_lines(
    snap: &Snapshot,
    matrix: &Matrix,
    verbose: bool,
    now: DateTime<Utc>,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut p = |s: String| out.push(s);
    p("Data health".into());
    p(format!("tasks: {}", snap.tasks.len()));
    p(format!("parse errors: {}", snap.errors.len()));
    if !snap.errors.is_empty() {
        p("Permission counts cover loaded tasks only; some resources could not be read.".into());
    }
    if verbose {
        for (path, msg) in &snap.errors {
            p(format!("    {}: {msg}", path.display()));
        }
    }

    let mut read_only: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for task in &snap.tasks {
        if let Some(reason) = &task.blocked {
            read_only.entry(reason).or_default().push(&task.summary);
        }
    }
    p(format!(
        "model read-only resources: {}",
        read_only.values().map(Vec::len).sum::<usize>()
    ));
    for (reason, titles) in read_only {
        p(format!("    {reason}: {}", titles.len()));
        if verbose {
            for title in titles {
                p(format!("        {title}"));
            }
        }
    }

    p("".into());
    p("Recurrence evidence (open tasks; class verdicts are not operation permissions)".into());
    let mut active: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for task in snap.tasks.iter().filter(|t| t.status == Status::Open) {
        if let Some(class) = task_rule_class(task, snap) {
            active.entry(class).or_default().push(&task.summary);
        }
    }
    if active.is_empty() {
        p("No active repeat classes.".into());
    }
    for (class, titles) in active {
        let (pv, x, e) = combined_verdicts(matrix, &class);
        p(format!(
            "{class}  {} tasks  preserve={} execute={} edit={}",
            titles.len(),
            pv.as_str(),
            x.as_str(),
            e.as_str()
        ));
        if verbose {
            for title in titles {
                p(format!("    {title}"));
            }
        }
    }

    operation_summary(snap, matrix, verbose, now, &mut out);
    out
}

fn operation_summary(
    snap: &Snapshot,
    matrix: &Matrix,
    verbose: bool,
    now: DateTime<Utc>,
    out: &mut Vec<String>,
) {
    let mut p = |s: String| out.push(s);
    let mode = snap.config.writes.mode;
    let label = match mode {
        WriteMode::Verified => "verified",
        WriteMode::Test => "test",
    };
    p("".into());
    p(format!("Operation availability (mode={label})"));
    p("Permission snapshot only; proposed values and later store preconditions may affect a write.".into());
    if snap.tasks.is_empty() {
        p("No tasks to assess.".into());
        return;
    }
    let checker = twydo_core::gate::Checker::new(
        snap,
        matrix,
        mode,
        now.with_timezone(&snap.tz()).date_naive(),
    );
    let rows: Vec<_> = (0..snap.tasks.len())
        .map(|i| probes_with(&checker, i))
        .collect();
    for (col, probe) in rows[0].iter().enumerate() {
        let mut counts = [0usize; 4];
        let mut groups: BTreeMap<(&str, &str), Vec<&str>> = BTreeMap::new();
        for (i, row) in rows.iter().enumerate() {
            let (n, state, reason) = match &row[col].availability {
                Availability::Allowed => (0, "allowed", ""),
                Availability::Refused(reason) => (1, "refused", reason.as_str()),
                Availability::Inapplicable(reason) => (2, "inapplicable", *reason),
                Availability::RequiresInput(reason) => (3, "needs input", *reason),
            };
            counts[n] += 1;
            groups
                .entry((state, reason))
                .or_default()
                .push(&snap.tasks[i].summary);
        }
        p(format!(
            "{}: allowed={} refused={} inapplicable={} needs-input={}",
            probe.action, counts[0], counts[1], counts[2], counts[3]
        ));
        for ((state, reason), titles) in groups {
            if !reason.is_empty() {
                p(format!("    {state}: {reason}: {}", titles.len()));
            }
            if verbose {
                for title in titles {
                    p(format!("        {title}"));
                }
            }
        }
    }
}

/// `export`: the schema v1 document of the files as the last reload read them (queued edits
/// not applied), built on the request thread.
pub fn export(shared: &Arc<Shared>) -> Result<Value, RpcError> {
    let snap = shared.publisher.disk_snapshot();
    let result = ExportResult {
        skipped: snap.errors.len(),
        document: export_document(&snap),
    };
    serde_json::to_value(result).map_err(|e| RpcError::refused(ErrorKind::Internal, e.to_string()))
}

fn date_string(v: &IcalDateTime, snap: &Snapshot) -> String {
    match v {
        IcalDateTime::Date(d) => d.format("%Y-%m-%d").to_string(),
        IcalDateTime::Floating(n) => n.format("%Y-%m-%dT%H:%M:%S").to_string(),
        IcalDateTime::Utc(t) => t.to_rfc3339_opts(SecondsFormat::Secs, true),
        IcalDateTime::Zoned { .. } => match v.resolve(snap.tz()).0 {
            Resolved::Instant(t) => t.to_rfc3339_opts(SecondsFormat::Secs, false),
            Resolved::Date(d) => d.format("%Y-%m-%d").to_string(),
        },
    }
}

fn opt_date(v: &Option<IcalDateTime>, snap: &Snapshot) -> Value {
    v.as_ref()
        .map_or(Value::Null, |d| Value::String(date_string(d, snap)))
}

fn is_date(v: &Option<IcalDateTime>) -> bool {
    v.as_ref().is_some_and(|d| d.is_date_only())
}

/// Names of the `X-2DO*` properties on the resource's VTODO, sorted. Re-reads the file
/// (read-only); an unreadable file yields none.
fn x2do_props(path: &Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let Ok(cal) = twydo_core::ical::parse(&bytes) else {
        return Vec::new();
    };
    let mut names: Vec<String> = cal
        .root
        .children_named("VTODO")
        .flat_map(|c| c.props.iter())
        .filter(|p| p.name.starts_with("X-2DO"))
        .map(|p| p.name.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

fn task_json(t: &Task, snap: &Snapshot) -> Value {
    let kind = match t.kind {
        TaskKind::Task => "task",
        TaskKind::Project => "project",
        TaskKind::Checklist => "checklist",
    };
    let status = match t.status {
        Status::Open => "open",
        Status::Completed => "completed",
        Status::Cancelled => "cancelled",
    };
    let alarms: Vec<Value> = t
        .alarms
        .iter()
        .map(|a| match &a.trigger {
            Trigger::Relative {
                offset,
                related_end,
            } => json!({"relative_seconds": offset.num_seconds(), "related_end": related_end}),
            Trigger::Absolute(at) => json!({
                "absolute": at.to_rfc3339_opts(SecondsFormat::Secs, true),
                "related_end": false,
            }),
        })
        .collect();
    let tokens: Map<String, Value> = t
        .meta
        .tokens
        .iter()
        .map(|(k, v)| (k.clone(), v.clone().map_or(Value::Null, Value::String)))
        .collect();
    json!({
        "uid": t.uid,
        "collection": t.collection,
        "summary": t.summary,
        "kind": kind,
        "status": status,
        "due": opt_date(&t.due, snap),
        "due_is_date": is_date(&t.due),
        "start": opt_date(&t.start, snap),
        "start_is_date": is_date(&t.start),
        "completed": opt_date(&t.completed, snap),
        "rrule": t.rrule,
        "rule_class": task_rule_class(t, snap),
        "from_completion": t.from_completion,
        "parent": t.parent,
        "alarms": alarms,
        "x2do_props": x2do_props(&t.path),
        "meta_tokens": tokens,
        "warnings": t.warnings,
        "blocked": t.blocked,
    })
}

pub fn export_document(snap: &Snapshot) -> Value {
    let mut cols: Vec<_> = snap.collections.iter().collect();
    cols.sort_by(|a, b| a.id.cmp(&b.id));
    let collections: Vec<Value> = cols
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "display_name": c.display_name,
                "color": c.color.map(|(r, g, b)| format!("#{r:02X}{g:02X}{b:02X}")),
            })
        })
        .collect();
    let mut tasks: Vec<&Task> = snap.tasks.iter().collect();
    tasks.sort_by(|a, b| (&a.collection, &a.uid).cmp(&(&b.collection, &b.uid)));
    json!({
        "schema": 1,
        "collections": collections,
        "tasks": tasks.iter().map(|t| task_json(t, snap)).collect::<Vec<_>>(),
    })
}
