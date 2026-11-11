//! `check.report`: data health, recurrence evidence and concrete operation admission, the
//! report `tdo check` and `tdo op check` print.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::Value;
use twydo_core::compat::{Matrix, combined_verdicts};
use twydo_core::config::WriteMode;
use twydo_core::model::Status;
use twydo_core::views::Snapshot;

use twydo_core::views::task_rule_class;

use super::Shared;
use super::permissions::{Availability, probes, probes_with, task_lines};
use crate::protocol::reports::{CheckReport, CheckReportParams};
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
