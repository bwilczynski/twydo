//! Concrete gate probes shared by compatibility reports.

use chrono::{DateTime, Utc};
use twydo_core::compat::Matrix;
use twydo_core::config::WriteMode;
use twydo_core::views::Snapshot;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Availability {
    Allowed,
    Refused(String),
    Inapplicable(&'static str),
    RequiresInput(&'static str),
}

pub(super) struct Probe {
    pub action: &'static str,
    pub availability: Availability,
}

/// Probe field admission without planning or writing a replacement resource.
pub(super) fn probes(
    snap: &Snapshot,
    matrix: &Matrix,
    mode: WriteMode,
    idx: usize,
    now: DateTime<Utc>,
) -> Vec<Probe> {
    use twydo_core::edit::{Change, Op};
    use twydo_core::model::Status;

    let task = &snap.tasks[idx];
    let uid = task.uid.clone();
    let today = now.with_timezone(&snap.tz()).date_naive();
    let check = |op| match twydo_core::gate::check(snap, matrix, mode, &op, today) {
        Ok(()) => Availability::Allowed,
        Err(b) => Availability::Refused(b.0),
    };
    let mut out = vec![
        Probe {
            action: "edit title",
            availability: check(Op::Edit {
                uid: uid.clone(),
                changes: vec![Change::Summary(task.summary.clone())],
            }),
        },
        Probe {
            action: "edit notes",
            availability: check(Op::Edit {
                uid: uid.clone(),
                changes: vec![Change::Description(task.description.clone())],
            }),
        },
        Probe {
            action: "complete",
            availability: if task.status == Status::Open {
                check(Op::Complete { uid: uid.clone() })
            } else {
                Availability::Inapplicable("task is not open")
            },
        },
        Probe {
            action: "reopen",
            availability: if task.status == Status::Completed {
                check(Op::Reopen { uid: uid.clone() })
            } else {
                Availability::Inapplicable("task is not completed")
            },
        },
        Probe {
            action: "delete",
            availability: check(Op::Delete { uid }),
        },
    ];
    for (action, input) in [
        ("create", "new task fields, type and list"),
        ("edit dates", "proposed due/start values"),
        ("edit repeat", "proposed repeat rule"),
        ("convert", "destination task type"),
        ("move", "destination list"),
        ("reparent", "destination list and parent/context"),
        ("reorder", "sibling anchor and placement"),
        ("insert", "new task fields, sibling anchor and placement"),
    ] {
        out.push(Probe {
            action,
            availability: Availability::RequiresInput(input),
        });
    }
    out
}

pub(super) fn task_lines(probes: &[Probe]) -> Vec<String> {
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
        for row in &mut baseline.operation {
            row.twydo_to_iphone = twydo_core::compat::Verdict::Unverified;
        }
        for m in [Matrix::embedded(), &baseline] {
            for mode in [WriteMode::Verified, WriteMode::Test] {
                for uid in [
                    "gate-date",
                    "gate-monthly-checklist",
                    "gate-from-completion",
                    "gate-done",
                    "gate-dup",
                ] {
                    let i = s.tasks.iter().position(|t| t.uid == uid).unwrap();
                    let t = &s.tasks[i];
                    let got = probes(&s, m, mode, i, now());
                    for (action, op) in [
                        (
                            "edit title",
                            Op::Edit {
                                uid: uid.into(),
                                changes: vec![Change::Summary(t.summary.clone())],
                            },
                        ),
                        (
                            "edit notes",
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
                "edit dates",
                "edit repeat",
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
