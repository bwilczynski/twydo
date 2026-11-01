//! `tdo check`: report whether every active repeat rule class is verified and no open task is
//! blocked from editing (spec §9).

use std::collections::BTreeMap;
use std::path::Path;

use twydo_core::compat::{Matrix, Verdict, combined_verdicts};
use twydo_core::model::Status;

use super::{CliError, load_snapshot, task_rule_class};

pub fn run(vdir: Option<&Path>, matrix_path: &Path, verbose: bool) -> Result<(), CliError> {
    let snap = load_snapshot(vdir)?;
    let errors = &snap.errors;

    // A missing matrix behaves as an empty one: every row counts as unverified.
    let matrix = if matrix_path.exists() {
        Matrix::load(matrix_path)?
    } else {
        println!("matrix not found: {}", matrix_path.display());
        Matrix::default()
    };

    // Active rule: an open task with an RRULE, archived lists included.
    let mut active: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for t in snap.tasks.iter().filter(|t| t.status == Status::Open) {
        if let Some(class) = task_rule_class(t, &snap) {
            active.entry(class).or_default().push(&t.summary);
        }
    }

    let mut unverified = 0;
    for (class, titles) in &active {
        let (p, x, e) = combined_verdicts(&matrix, class);
        if p != Verdict::Pass || x != Verdict::Pass {
            unverified += 1;
        }
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

    println!("parse errors: {}", errors.len());
    if verbose {
        for (path, msg) in errors {
            println!("    {}: {msg}", path.display());
        }
    }

    // Open tasks Twydo would refuse to edit (recurrence overrides, unsupported rules).
    let mut blocked: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for t in snap.tasks.iter().filter(|t| t.status == Status::Open) {
        if let Some(reason) = &t.blocked {
            blocked.entry(reason).or_default().push(&t.summary);
        }
    }
    let n_blocked: usize = blocked.values().map(Vec::len).sum();
    println!("blocked: {n_blocked}");
    for (reason, titles) in &blocked {
        println!("    {reason}: {}", titles.len());
        if verbose {
            for title in titles {
                println!("        {title}");
            }
        }
    }

    if unverified == 0 && errors.is_empty() && n_blocked == 0 {
        println!("GATE: PASSED");
    } else {
        let n = errors.len();
        let plural = if n == 1 { "" } else { "s" };
        println!(
            "GATE: NOT PASSED ({unverified} rule classes unverified, {n} parse error{plural}, \
             {n_blocked} blocked)"
        );
    }
    Ok(())
}
