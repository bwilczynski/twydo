//! Concrete gate probes shared by compatibility reports.

use chrono::{DateTime, Utc};
use twydo_core::compat::{Matrix, OperationId};
use twydo_core::config::WriteMode;
use twydo_core::views::Snapshot;

#[derive(Debug, PartialEq, Eq)]
pub enum Availability {
    Allowed,
    Refused(String),
    Inapplicable(&'static str),
    RequiresInput(&'static str),
}

pub struct Probe {
    pub action: &'static str,
    pub availability: Availability,
}

/// Probe field admission without planning or writing a replacement resource.
pub fn probes(
    snap: &Snapshot,
    matrix: &Matrix,
    mode: WriteMode,
    idx: usize,
    now: DateTime<Utc>,
) -> Vec<Probe> {
    let today = now.with_timezone(&snap.tz()).date_naive();
    let checker = twydo_core::gate::Checker::new(snap, matrix, mode, today);
    probes_with(&checker, idx)
}

/// Aggregate reports reuse the gate's snapshot facts without maintaining a second policy.
pub fn probes_with(checker: &twydo_core::gate::Checker<'_>, idx: usize) -> Vec<Probe> {
    use twydo_core::edit::{Change, Op};
    use twydo_core::model::Status;
    let snap = checker.snapshot();
    let task = &snap.tasks[idx];
    let uid = task.uid.clone();
    let check = |op| match checker.check(&op) {
        Ok(()) => Availability::Allowed,
        Err(b) => Availability::Refused(b.0),
    };
    let mut out = vec![
        Probe {
            action: OperationId::EditTitle.as_str(),
            availability: check(Op::Edit {
                uid: uid.clone(),
                changes: vec![Change::Summary(task.summary.clone())],
            }),
        },
        Probe {
            action: OperationId::EditNotes.as_str(),
            availability: check(Op::Edit {
                uid: uid.clone(),
                changes: vec![Change::Description(task.description.clone())],
            }),
        },
        Probe {
            action: OperationId::Complete.as_str(),
            availability: if task.status == Status::Open {
                check(Op::Complete { uid: uid.clone() })
            } else {
                Availability::Inapplicable("task is not open")
            },
        },
        Probe {
            action: OperationId::Reopen.as_str(),
            availability: if task.status == Status::Completed {
                check(Op::Reopen { uid: uid.clone() })
            } else {
                Availability::Inapplicable("task is not completed")
            },
        },
        Probe {
            action: OperationId::Delete.as_str(),
            availability: check(Op::Delete { uid }),
        },
    ];
    for (action, input) in [
        (OperationId::Create, "new task fields, type and list"),
        (OperationId::SetDue, "proposed due value"),
        (OperationId::SetStart, "proposed start value"),
        (OperationId::SetRepeat, "proposed repeat rule"),
        (OperationId::Convert, "destination task type"),
        (OperationId::Move, "destination list"),
        (OperationId::Reparent, "destination list and parent/context"),
        (OperationId::Reorder, "sibling anchor and placement"),
        (
            OperationId::Insert,
            "new task fields, sibling anchor and placement",
        ),
    ] {
        out.push(Probe {
            action: action.as_str(),
            availability: Availability::RequiresInput(input),
        });
    }
    out
}

pub fn task_lines(probes: &[Probe]) -> Vec<String> {
    let mut out: Vec<_> = probes
        .iter()
        .map(|p| {
            let result = match &p.availability {
                Availability::Allowed => "allowed".to_string(),
                Availability::Refused(reason) => format!("refused: {reason}"),
                Availability::Inapplicable(reason) => format!("inapplicable: {reason}"),
                Availability::RequiresInput(input) => format!("needs input: {input}"),
            };
            format!("{}: {result}", p.action)
        })
        .collect();
    out.push("Permission snapshot only; proposed values and later store preconditions may affect a write.".into());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use twydo_core::config::Config;
    use twydo_core::edit::{Change, Op};
    use twydo_core::gate;
    use twydo_core::model::Status;

    fn snap() -> Snapshot {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../twydo-core/tests/data/gate");
        let report = twydo_core::vdir::load(&root).unwrap();
        let mut cfg = Config::with_vdir(root);
        cfg.timezone = Some("America/New_York".into());
        Snapshot::new(report, cfg)
    }

    fn now() -> DateTime<Utc> {
        "2026-11-04T02:00:00Z".parse().unwrap()
    }

    #[test]
    fn concrete_probes_match_gate_across_shapes_and_modes() {
        let s = snap();
        let mut baseline = Matrix::embedded().clone();
        for row in &mut baseline.contract {
            row.twydo_to_iphone = twydo_core::compat::Verdict::Unverified;
        }
        for m in [Matrix::embedded(), &baseline] {
            for mode in [WriteMode::Verified, WriteMode::Test] {
                let checker = twydo_core::gate::Checker::new(
                    &s,
                    m,
                    mode,
                    now().with_timezone(&s.tz()).date_naive(),
                );
                for uid in [
                    "gate-date",
                    "gate-monthly-checklist",
                    "gate-from-completion",
                    "gate-done",
                    "gate-dup",
                ] {
                    let i = s.tasks.iter().position(|t| t.uid == uid).unwrap();
                    let t = &s.tasks[i];
                    let got = probes_with(&checker, i);
                    for (action, op) in [
                        (
                            "edit-title",
                            Op::Edit {
                                uid: uid.into(),
                                changes: vec![Change::Summary(t.summary.clone())],
                            },
                        ),
                        (
                            "edit-notes",
                            Op::Edit {
                                uid: uid.into(),
                                changes: vec![Change::Description(t.description.clone())],
                            },
                        ),
                        ("delete", Op::Delete { uid: uid.into() }),
                    ] {
                        let expected = match gate::check(
                            &s,
                            m,
                            mode,
                            &op,
                            now().with_timezone(&s.tz()).date_naive(),
                        ) {
                            Ok(()) => Availability::Allowed,
                            Err(b) => Availability::Refused(b.0),
                        };
                        assert_eq!(
                            got.iter()
                                .find(|p| p.action == action)
                                .unwrap()
                                .availability,
                            expected,
                            "{uid}: {action}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn lifecycle_and_parameterized_actions_have_honest_labels() {
        let mut s = snap();
        let i = s.tasks.iter().position(|t| t.uid == "gate-date").unwrap();
        for status in [Status::Open, Status::Completed, Status::Cancelled] {
            s.tasks[i].status = status;
            let got = probes(&s, Matrix::embedded(), WriteMode::Verified, i, now());
            for (action, applicable) in [
                ("complete", status == Status::Open),
                ("reopen", status == Status::Completed),
            ] {
                assert_eq!(
                    !matches!(
                        got.iter()
                            .find(|p| p.action == action)
                            .unwrap()
                            .availability,
                        Availability::Inapplicable(_)
                    ),
                    applicable
                );
            }
            for action in [
                "create",
                "set-due",
                "set-start",
                "set-repeat",
                "convert",
                "move",
                "reparent",
                "reorder",
                "insert",
            ] {
                assert!(matches!(
                    got.iter()
                        .find(|p| p.action == action)
                        .unwrap()
                        .availability,
                    Availability::RequiresInput(_)
                ));
            }
            let lines = task_lines(&got).join("\n");
            assert!(lines.contains("needs input"));
            assert!(lines.contains("snapshot"));
            assert!(!lines.contains("edit: ok"));
        }
    }
}
