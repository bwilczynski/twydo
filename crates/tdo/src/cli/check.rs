//! `tdo check`: report whether every active repeat rule class is verified (spec §9).

use std::collections::BTreeMap;
use std::path::Path;

use twydo_core::compat::{Matrix, Verdict};
use twydo_core::model::Status;

use super::{CliError, load_snapshot, task_rule_class};

pub fn run(vdir: Option<&Path>, matrix_path: &Path, verbose: bool) -> Result<(), CliError> {
    let (snap, errors) = load_snapshot(vdir)?;

    let matrix = if matrix_path.exists() {
        Some(Matrix::load(matrix_path)?)
    } else {
        println!("matrix not found: {}", matrix_path.display());
        None
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
        let base = class.split('+').next().unwrap_or(class);
        let entry = matrix.as_ref().and_then(|m| m.rule(base));
        let (p, x, e) = entry.map_or(
            (
                Verdict::Unverified,
                Verdict::Unverified,
                Verdict::Unverified,
            ),
            |r| (r.preserve, r.execute, r.edit),
        );
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
        for (path, msg) in &errors {
            println!("    {}: {msg}", path.display());
        }
    }
    if unverified == 0 {
        println!("GATE: PASSED");
    } else {
        println!("GATE: NOT PASSED ({unverified} rule classes unverified)");
    }
    Ok(())
}
