//! `tdo check`: data health, recurrence evidence and concrete operation admission.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::Utc;
use twydo_core::compat::{Matrix, combined_verdicts};
use twydo_core::config::WriteMode;
use twydo_core::model::Status;
use twydo_core::views::Snapshot;

use super::permissions::{Availability, probes_with};
use super::{CliError, permission_snapshot, recover_at_startup, resolve_config, task_rule_class};

pub fn run(vdir: Option<&Path>, matrix_path: Option<&Path>, verbose: bool) -> Result<(), CliError> {
    let matrix = match matrix_path {
        None => Matrix::embedded().clone(),
        Some(p) => Matrix::load(p)?,
    };
    let cfg = resolve_config(vdir)?;
    recover_at_startup(&cfg);
    let snap = permission_snapshot(&cfg)?;
    println!("Data health");
    println!("tasks: {}", snap.tasks.len());
    println!("parse errors: {}", snap.errors.len());
    if !snap.errors.is_empty() {
        println!("Permission counts cover loaded tasks only; some resources could not be read.");
    }
    if verbose {
        for (path, msg) in &snap.errors {
            println!("    {}: {msg}", path.display());
        }
    }

    let mut read_only: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for task in &snap.tasks {
        if let Some(reason) = &task.blocked {
            read_only.entry(reason).or_default().push(&task.summary);
        }
    }
    println!(
        "model read-only resources: {}",
        read_only.values().map(Vec::len).sum::<usize>()
    );
    for (reason, titles) in read_only {
        println!("    {reason}: {}", titles.len());
        if verbose {
            for title in titles {
                println!("        {title}");
            }
        }
    }

    println!("\nRecurrence evidence (open tasks; class verdicts are not operation permissions)");
    let mut active: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for task in snap.tasks.iter().filter(|t| t.status == Status::Open) {
        if let Some(class) = task_rule_class(task, &snap) {
            active.entry(class).or_default().push(&task.summary);
        }
    }
    if active.is_empty() {
        println!("No active repeat classes.");
    }
    for (class, titles) in active {
        let (p, x, e) = combined_verdicts(&matrix, &class);
        println!(
            "{class}  {} tasks  preserve={} execute={} edit={}",
            titles.len(),
            p.as_str(),
            x.as_str(),
            e.as_str()
        );
        if verbose {
            for title in titles {
                println!("    {title}");
            }
        }
    }

    operation_summary(&snap, &matrix, verbose);
    Ok(())
}

fn operation_summary(snap: &Snapshot, matrix: &Matrix, verbose: bool) {
    let mode = snap.config.writes.mode;
    let label = match mode {
        WriteMode::Verified => "verified",
        WriteMode::Test => "test",
    };
    println!("\nOperation availability (mode={label})");
    println!(
        "Permission snapshot only; proposed values and later store preconditions may affect a write."
    );
    if snap.tasks.is_empty() {
        println!("No tasks to assess.");
        return;
    }
    let now = Utc::now();
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
        println!(
            "{}: allowed={} refused={} inapplicable={} needs-input={}",
            probe.action, counts[0], counts[1], counts[2], counts[3]
        );
        for ((state, reason), titles) in groups {
            if !reason.is_empty() {
                println!("    {state}: {reason}: {}", titles.len());
            }
            if verbose {
                for title in titles {
                    println!("        {title}");
                }
            }
        }
    }
}
