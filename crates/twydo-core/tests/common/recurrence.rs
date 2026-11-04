//! Shared fixture identities and exact comparisons for recurrence/checklist phone captures.
//! Assertions live in the behavior-based edit, store, gate and round-trip test suites.
use super::{Line, fixture, is_stamp, lines, sorted, stamp, todo};
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use twydo_core::compat::{Matrix, OperationEntry, Verdict};
use twydo_core::ical::{Calendar, parse};
use twydo_core::model::meta_edit::MetaJson;

pub const SCENARIO: &str = "plan8-tui-a";
pub const SEED: &str = "001-after-upload";
pub const SET: &str = "010-first-phone-actions";
pub const ADVANCE: &str = "020-boundary-and-checklist-complete";
pub const END: &str = "030-end-series-and-items";
pub const REOPEN: &str = "040-reopen-ended-and-complete-parent";
pub const REMOVE: &str = "050-remove-repeat-and-refinish";
pub const HISTORY: &str = "060-remove-checklist-repeat-and-history-reopen";
pub const FC: &str = "61ed74be3ba34212ab7476b698de1263";
pub const RD: &str = "db95528b16fb4aa2b6a47e56a0a1000b";
pub const RY: &str = "033d6526b6ba46999f3f510edecf888b";

pub fn cal(step: &str, uid: &str) -> Calendar {
    parse(&fixture(SCENARIO, step, uid)).unwrap()
}

pub fn step_files(step: &str) -> BTreeMap<String, Calendar> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/phase0/v1")
        .join(SCENARIO)
        .join(step);
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "ics"))
        .map(|p| {
            let cal = parse(&fs::read(p).unwrap()).unwrap();
            (todo(&cal).prop("UID").unwrap().value.clone(), cal)
        })
        .collect()
}

/// The only excluded properties are phone layout, modification stamps (asserted separately),
/// and CREATED (the phone's first-receipt time differs from the copied source's creation).
/// COMPLETED, all recurrence/end fields, alarms and identity links remain in the comparison.
pub fn comparable(cal: &Calendar, ids: &BTreeMap<String, String>) -> (Vec<Line>, MetaJson) {
    let mut meta = None;
    let mut result = Vec::new();
    for mut line in lines(cal) {
        if is_stamp(&line.1)
            || matches!(
                line.1.as_str(),
                "PRODID"
                    | "X-2DOAPP-SYNC-IMAGE"
                    | "X-2DOAPP-SYNC-AUDIO"
                    | "X-2DOAPP-AUDIO-DURATION"
            )
        {
            continue;
        }
        for (from, to) in ids {
            line.3 = line.3.replace(from, to);
        }
        match line.1.as_str() {
            "CREATED" => line.3 = "<creation time>".into(),
            "X-2DOAPP-METADATA" => {
                meta = MetaJson::parse(&line.3);
                line.3 = "<metadata compared as JSON>".into();
            }
            _ => {}
        }
        result.push(line);
    }
    (sorted(result), meta.expect("readable metadata"))
}

pub fn same(ctx: &str, got: &Calendar, want: &Calendar, ids: &BTreeMap<String, String>) {
    assert_eq!(
        comparable(got, ids),
        comparable(want, &BTreeMap::new()),
        "{ctx}"
    );
}

pub fn stamped(cal: &Calendar, now: DateTime<Utc>) {
    for key in ["DTSTAMP", "LAST-MODIFIED"] {
        assert_eq!(todo(cal).prop(key).unwrap().value, stamp(now), "{key}");
    }
}

pub fn preserved_created(before: &Calendar, after: &Calendar) {
    assert_eq!(todo(before).prop("CREATED"), todo(after).prop("CREATED"));
}

/// This is deliberately a candidate, not the embedded matrix. Device verification of the
/// fixed Twydo output must precede shipping any new operation/rule verdict.
pub fn candidate() -> Matrix {
    let mut matrix = Matrix::embedded().clone();
    matrix
        .rule
        .iter_mut()
        .find(|r| r.class == "mod:until")
        .unwrap()
        .execute = Verdict::Pass;
    for (case, op) in [
        ("recurring-checklist", "complete"),
        ("ended-until-daily", "reopen"),
        ("ended-until-monthly", "reopen"),
    ] {
        let entry = OperationEntry {
            case: case.into(),
            op: op.into(),
            iphone_baseline: Verdict::Pass,
            mac_to_iphone: Verdict::Unverified,
            iphone_to_mac: Verdict::Unverified,
            twydo_to_iphone: Verdict::Pass,
            iphone_to_twydo: Verdict::Pass,
            evidence: Vec::new(),
            notes: "Local candidate only".into(),
        };
        if let Some(existing) = matrix
            .operation
            .iter_mut()
            .find(|r| r.case == case && r.op == op)
        {
            *existing = entry;
        } else {
            matrix.operation.push(entry);
        }
    }
    matrix
}
