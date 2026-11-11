//! Concrete admission characterization before the operation-contract migration.
//! This catches accidental widening/narrowing, especially mixed saves. Expectations
//! were captured from the approved 0.1.9 gate before replacing the matrix.
mod common;

use chrono::NaiveDate;
use twydo_core::{
    compat::Matrix,
    config::{Config, WriteMode},
    edit::{Change, Due, Op},
    gate,
    views::Snapshot,
};

#[test]
fn current_contract_decisions_are_preserved_in_both_modes() {
    let mut results = String::new();
    for state in [
        "plan4-list1/02-edit",
        "plan6-list1/04-reopen-child",
        "plan7-twydo-list2/00-seed",
        "plan8-tui-a/030-end-series-and-items",
        "plan8-tui-a/040-reopen-ended-and-complete-parent",
        "plan8-rfra/202-seeded-after-upload",
    ] {
        let env = common::Env::new(WriteMode::Verified, state);
        let report = twydo_core::vdir::load(&env.vdir).unwrap();
        let cfg = Config::with_vdir(env.vdir.clone());
        let snap = Snapshot::new(report, cfg);
        let day = NaiveDate::from_ymd_opt(2026, 11, 4).unwrap();
        let due = Due::Date(NaiveDate::from_ymd_opt(2026, 12, 15).unwrap());
        let mut tasks: Vec<_> = snap.tasks.iter().collect();
        tasks.sort_by_key(|t| &t.uid);
        for task in tasks {
            let uid = task.uid.clone();
            let operations = [
                Op::Edit {
                    uid: uid.clone(),
                    changes: vec![Change::Summary("Synthetic title".into())],
                },
                Op::Edit {
                    uid: uid.clone(),
                    changes: vec![Change::Description(Some("Synthetic note".into()))],
                },
                Op::Complete { uid: uid.clone() },
                Op::Reopen { uid: uid.clone() },
                Op::Delete { uid: uid.clone() },
                Op::Move {
                    uid: uid.clone(),
                    to: "list2".into(),
                },
                Op::Edit {
                    uid: uid.clone(),
                    changes: vec![Change::Due(Some(due)), Change::RepeatInterval(2)],
                },
                Op::Save {
                    uid,
                    changes: vec![Change::Due(Some(due))],
                    to: Some("list2".into()),
                },
            ];
            results.push_str(&format!("{state} {} ", task.uid));
            for mode in [WriteMode::Verified, WriteMode::Test] {
                for op in &operations {
                    results.push(
                        if gate::check(&snap, Matrix::embedded(), mode, op, day).is_ok() {
                            '+'
                        } else {
                            '-'
                        },
                    );
                }
                results.push(' ');
            }
            results.push('\n');
        }
    }
    assert_eq!(results, include_str!("data/admission.txt"));
}
