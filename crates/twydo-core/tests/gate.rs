//! The write gate: matrix-driven policy with exact reasons. All content is synthetic
//! (`tests/data/gate/`); matrices are built from TOML strings here.

mod common;

use std::path::Path;

use chrono::NaiveDate;

use twydo_core::compat::Matrix;
use twydo_core::config::{Config, WriteMode};
use twydo_core::edit::{Change, Due, NewTask, Op};
use twydo_core::gate::{Blocked, Cases, check, task_cases};
use twydo_core::ical::{parse, serialize};
use twydo_core::model::meta_edit::MetaJson;
use twydo_core::views::Snapshot;

const ALL_OPS: [&str; 7] = [
    "create", "edit", "complete", "reopen", "move", "delete", "repeat",
];

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 4).unwrap()
}

fn snap() -> Snapshot {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/gate");
    let report = twydo_core::vdir::load(&root).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let mut cfg = Config::with_vdir(root);
    cfg.timezone = Some("America/New_York".to_string());
    Snapshot::new(report, cfg)
}

/// A matrix where every listed (case, op) has `baseline`, and the twydo columns are `twydo`;
/// the listed rule classes pass all three columns. Unlisted rows are missing (unverified).
fn matrix(ops: &[(&str, &str)], baseline: &str, twydo: &str, rules: &[&str]) -> Matrix {
    let mut s = String::from("matrix_version = 1\n");
    for (case, op) in ops {
        s.push_str(&format!(
            "[[operation]]\ncase = \"{case}\"\nop = \"{op}\"\niphone_baseline = \"{baseline}\"\n\
             mac_to_iphone = \"unverified\"\niphone_to_mac = \"unverified\"\n\
             twydo_to_iphone = \"{twydo}\"\niphone_to_twydo = \"{twydo}\"\n"
        ));
    }
    for class in rules {
        s.push_str(&format!(
            "[[rule]]\nclass = \"{class}\"\npreserve = \"pass\"\nexecute = \"pass\"\nedit = \"pass\"\n"
        ));
    }
    Matrix::from_toml_str(&s).unwrap()
}

/// Every op on the given cases has `baseline` and the twydo columns `twydo`.
fn cases(cs: &[&str], baseline: &str, twydo: &str, rules: &[&str]) -> Matrix {
    let ops: Vec<(&str, &str)> = cs
        .iter()
        .flat_map(|c| ALL_OPS.iter().map(move |o| (*c, *o)))
        .collect();
    matrix(&ops, baseline, twydo, rules)
}

/// Everything a monthly/yearly/one-time task can need, fully verified.
fn full() -> Matrix {
    cases(
        &[
            "one-time",
            "monthly",
            "yearly",
            "date-only",
            "timed",
            "start-date",
        ],
        "pass",
        "pass",
        &["months-day-1-28", "years"],
    )
}

fn complete(uid: &str) -> Op {
    Op::Complete { uid: uid.into() }
}

fn reopen(uid: &str) -> Op {
    Op::Reopen { uid: uid.into() }
}

fn edit(uid: &str, changes: Vec<Change>) -> Op {
    Op::Edit {
        uid: uid.to_string(),
        changes,
    }
}

fn blocked(r: &str) -> Result<(), Blocked> {
    Err(Blocked(r.to_string()))
}

fn idx(s: &Snapshot, uid: &str) -> usize {
    s.tasks.iter().position(|t| t.uid == uid).unwrap()
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

#[test]
fn verified_mode_blocks_without_twydo_columns() {
    let s = snap();
    let m = cases(&["one-time"], "pass", "unverified", &[]);
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &complete("gate-date"), today()),
        blocked("complete for one-time tasks is not verified yet (twydo_to_iphone)")
    );
    let m = cases(&["one-time"], "unverified", "pass", &[]);
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &complete("gate-date"), today()),
        blocked("complete for one-time tasks is not verified yet (iphone_baseline)")
    );
    let m = cases(&["one-time"], "pass", "pass", &[]);
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &complete("gate-date"), today()),
        Ok(())
    );
}

#[test]
fn test_mode_allows_with_baseline_only() {
    let s = snap();
    let m = cases(&["one-time"], "pass", "unverified", &[]);
    assert_eq!(
        check(&s, &m, WriteMode::Test, &complete("gate-date"), today()),
        Ok(())
    );
    let m = cases(&["one-time"], "unverified", "unverified", &[]);
    assert_eq!(
        check(&s, &m, WriteMode::Test, &complete("gate-date"), today()),
        blocked("complete for one-time tasks is not verified yet (iphone_baseline)")
    );
}

#[test]
fn edit_due_adds_timed_case() {
    let s = snap();
    let mut ops: Vec<(&str, &str)> = vec![("one-time", "edit"), ("date-only", "edit")];
    let m = matrix(&ops, "pass", "pass", &[]);
    let t = NaiveDate::from_ymd_opt(2026, 11, 12)
        .unwrap()
        .and_hms_opt(9, 30, 0)
        .unwrap();
    let to_timed = edit("gate-date", vec![Change::Due(Some(Due::Time(t)))]);
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &to_timed, today()),
        blocked("edit for timed tasks is not verified yet (iphone_baseline)")
    );
    // A date-only → date-only edit needs only date-only.
    let to_date = edit(
        "gate-date",
        vec![Change::Due(Some(Due::Date(date(2026, 11, 12))))],
    );
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &to_date, today()),
        Ok(())
    );
    // A timed task losing its time still needs timed (before) as well as date-only (after).
    let from_timed = edit(
        "gate-timed",
        vec![Change::Due(Some(Due::Date(date(2026, 11, 12))))],
    );
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &from_timed, today()),
        blocked("edit for timed tasks is not verified yet (iphone_baseline)")
    );
    ops.push(("timed", "edit"));
    let m = matrix(&ops, "pass", "pass", &[]);
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &to_timed, today()),
        Ok(())
    );
    // A start change adds start-date; summary changes add nothing.
    let start = edit("gate-nodue", vec![Change::Start(Some(date(2026, 11, 5)))]);
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &start, today()),
        blocked("edit for start-date tasks is not verified yet (iphone_baseline)")
    );
    let title = edit("gate-nodue", vec![Change::Summary("Renamed".into())]);
    assert_eq!(check(&s, &m, WriteMode::Verified, &title, today()), Ok(()));
}

/// `m` plus the rows of `extra`.
fn with(m: Matrix, extra: Matrix) -> Matrix {
    let mut m = m;
    m.operation.extend(extra.operation);
    m.rule.extend(extra.rule);
    m.edge_case.extend(extra.edge_case);
    m
}

/// `m` plus a `[[rule]]` row for `class`.
fn rule(m: Matrix, class: &str, preserve: &str, edit: &str) -> Matrix {
    let extra = Matrix::from_toml_str(&format!(
        "matrix_version = 1\n[[rule]]\nclass = \"{class}\"\npreserve = \"{preserve}\"\n\
         execute = \"unverified\"\nedit = \"{edit}\"\n"
    ))
    .unwrap();
    with(m, extra)
}

fn repeat(text: &str) -> twydo_core::repeatinput::RepeatSpec {
    twydo_core::repeatinput::parse(text, today(), &[])
        .unwrap()
        .unwrap()
}

fn item(parent: &str) -> NewTask {
    NewTask {
        parent: Some(parent.into()),
        ..NewTask::plain("inbox", "Synthetic item")
    }
}

#[test]
fn create_item_needs_family_create_row() {
    let s = snap();
    for (parent, family) in [("gate-checklist", "checklist"), ("gate-project", "project")] {
        let op = Op::Create(item(parent));
        let m = cases(&["one-time"], "pass", "pass", &[]);
        assert_eq!(
            check(&s, &m, WriteMode::Verified, &op, today()),
            blocked(&format!(
                "create for {family} tasks is not verified yet (iphone_baseline)"
            ))
        );
        let m = cases(&["one-time", family], "pass", "pass", &[]);
        assert_eq!(check(&s, &m, WriteMode::Verified, &op, today()), Ok(()));
    }
    // A new parent's own family row.
    let project = Op::Create(NewTask {
        kind: twydo_core::model::TaskKind::Project,
        ..NewTask::plain("inbox", "Synthetic project")
    });
    let m = cases(&["one-time"], "pass", "pass", &[]);
    assert_eq!(
        check(&s, &m, WriteMode::Test, &project, today()),
        blocked("create for project tasks is not verified yet (iphone_baseline)")
    );
    // An item of a repeating project joins the recurring-project family; a repeating item of
    // a plain project is a project-recurring-child.
    let op = Op::Create(item("gate-monthly-project"));
    let m = fam();
    assert_eq!(check(&s, &m, WriteMode::Verified, &op, today()), Ok(()));
    let m = cases(&["one-time", "monthly", "project"], "pass", "pass", &[]);
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &op, today()),
        blocked("create for recurring-project tasks is not verified yet (iphone_baseline)")
    );
    let op = Op::Create(NewTask {
        due: Some(Due::Date(date(2026, 11, 9))),
        repeat: Some(repeat("monthly")),
        ..item("gate-project")
    });
    let m = cases(
        &["monthly", "date-only", "project"],
        "pass",
        "pass",
        &["months-day-1-28"],
    );
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &op, today()),
        blocked("create for project-recurring-child tasks is not verified yet (iphone_baseline)")
    );
    // The shipped matrix has verified Twydo's new items: an item of a checklist is allowed.
    assert_eq!(
        check(
            &s,
            Matrix::embedded(),
            WriteMode::Verified,
            &Op::Create(item("gate-checklist")),
            today()
        ),
        Ok(())
    );
    // A repeating item of a plain project is not verified yet.
    let got = check(&s, Matrix::embedded(), WriteMode::Verified, &op, today());
    let Err(Blocked(why)) = got else {
        panic!("allowed: {got:?}")
    };
    assert!(
        why.starts_with("create for project-recurring-child tasks is not verified yet"),
        "{why}"
    );
}

#[test]
fn create_item_under_plain_task_is_refused() {
    let s = snap();
    let m = fam();
    let at = |parent: &str| check(&s, &m, WriteMode::Test, &Op::Create(item(parent)), today());
    assert_eq!(
        at("gate-date"),
        blocked("subtasks of a plain task are not verified yet")
    );
    assert_eq!(
        at("gate-plain-parent"),
        blocked("subtasks of a plain task are not verified yet")
    );
    assert_eq!(at("gate-checklist-item"), blocked(NESTED));
    assert_eq!(at("nope"), blocked("no task with UID nope"));
    assert_eq!(
        at("gate-monthly-checklist"),
        blocked("create for recurring-checklist tasks is not verified yet (iphone_baseline)")
    );
    // An item is a task, in its parent's list.
    let op = Op::Create(NewTask {
        kind: twydo_core::model::TaskKind::Checklist,
        ..item("gate-project")
    });
    assert_eq!(
        check(&s, &m, WriteMode::Test, &op, today()),
        blocked(NESTED)
    );
    let op = Op::Create(NewTask {
        collection: "errands".into(),
        ..item("gate-project")
    });
    assert_eq!(
        check(&s, &m, WriteMode::Test, &op, today()),
        blocked("an item must be in the list of its checklist or project")
    );
}

#[test]
fn checklist_item_with_date_is_refused() {
    let s = snap();
    let m = rule(fam(), "days", "pass", "pass");
    let shapes = [
        NewTask {
            due: Some(Due::Date(date(2026, 11, 9))),
            ..item("gate-checklist")
        },
        NewTask {
            start: Some(date(2026, 11, 9)),
            ..item("gate-checklist")
        },
        NewTask {
            due: Some(Due::Date(date(2026, 11, 9))),
            repeat: Some(repeat("daily")),
            ..item("gate-checklist")
        },
    ];
    for n in shapes {
        assert_eq!(
            check(&s, &m, WriteMode::Test, &Op::Create(n.clone()), today()),
            blocked(ITEM_DATES),
            "{n:?}"
        );
    }
    // A repeating checklist needs its own create verdict, not the plain checklist's.
    let op = Op::Create(NewTask {
        kind: twydo_core::model::TaskKind::Checklist,
        due: Some(Due::Date(date(2026, 11, 9))),
        repeat: Some(repeat("monthly")),
        ..NewTask::plain("inbox", "Synthetic checklist")
    });
    assert_eq!(
        check(&s, &m, WriteMode::Test, &op, today()),
        blocked("create for recurring-checklist tasks is not verified yet (iphone_baseline)")
    );
}

#[test]
fn new_repeat_needs_rule_edit_verdict() {
    let s = snap();
    let due = Due::Date(date(2026, 11, 10));
    let create = Op::Create(NewTask {
        due: Some(due),
        repeat: Some(repeat("2nd tue")),
        ..NewTask::plain("inbox", "Synthetic repeat")
    });
    let set = edit(
        "gate-nodue",
        vec![
            Change::Due(Some(due)),
            Change::Repeat(Some(repeat("2nd tue"))),
        ],
    );
    let base = cases(&["one-time", "monthly", "date-only"], "pass", "pass", &[]);
    let why = "months-nth-weekday repeats are not verified yet";
    for op in [&create, &set] {
        let m = base.clone();
        assert_eq!(
            check(&s, &m, WriteMode::Verified, op, today()),
            blocked(why)
        );
        // Preserved is not enough: Twydo writes the rule.
        let m = rule(base.clone(), "months-nth-weekday", "pass", "unverified");
        assert_eq!(
            check(&s, &m, WriteMode::Verified, op, today()),
            blocked(why)
        );
        let m = rule(base.clone(), "months-nth-weekday", "pass", "pass");
        assert_eq!(check(&s, &m, WriteMode::Verified, op, today()), Ok(()));
    }
    // Daily rules use the daily rows.
    let daily = Op::Create(NewTask {
        due: Some(due),
        repeat: Some(repeat("daily")),
        ..NewTask::plain("inbox", "Synthetic daily")
    });
    let m = rule(base.clone(), "days", "pass", "pass");
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &daily, today()),
        blocked("create for daily tasks is not verified yet (iphone_baseline)")
    );
    let m = rule(
        cases(&["daily", "date-only"], "pass", "pass", &[]),
        "days",
        "pass",
        "pass",
    );
    assert_eq!(check(&s, &m, WriteMode::Verified, &daily, today()), Ok(()));
    // From completion has its own case, and its modifier needs verdicts too.
    let from = Op::Create(NewTask {
        due: Some(due),
        repeat: Some(repeat("daily after done")),
        ..NewTask::plain("inbox", "Synthetic from completion")
    });
    let m = rule(
        cases(&["from-completion", "date-only"], "pass", "pass", &[]),
        "days",
        "pass",
        "pass",
    );
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &from, today()),
        blocked("days+from-completion repeats are not verified yet")
    );
    let m = rule(m, "mod:from-completion", "pass", "pass");
    assert_eq!(check(&s, &m, WriteMode::Verified, &from, today()), Ok(()));
}

#[test]
fn changing_or_removing_a_repeat_needs_the_old_class_edit_verdict() {
    let s = snap();
    let base = cases(&["monthly", "yearly", "one-time"], "pass", "pass", &[]);
    let remove = edit("gate-monthly", vec![Change::Repeat(None)]);
    let m = rule(base.clone(), "months-day-1-28", "pass", "unverified");
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &remove, today()),
        blocked("months-day-1-28 repeats are not verified yet")
    );
    let m = rule(base.clone(), "months-day-1-28", "pass", "pass");
    assert_eq!(check(&s, &m, WriteMode::Verified, &remove, today()), Ok(()));
    // Removing the rule and then the due date is fine.
    let both = edit(
        "gate-monthly",
        vec![Change::Repeat(None), Change::Due(None)],
    );
    let m = with(m, cases(&["date-only"], "pass", "pass", &[]));
    assert_eq!(check(&s, &m, WriteMode::Verified, &both, today()), Ok(()));
    // Monthly to yearly needs both classes' edit verdicts.
    let change = edit("gate-monthly", vec![Change::Repeat(Some(repeat("yearly")))]);
    let m = rule(base.clone(), "months-day-1-28", "pass", "unverified");
    let m = rule(m, "years", "pass", "pass");
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &change, today()),
        blocked("months-day-1-28 repeats are not verified yet")
    );
    let m = rule(base.clone(), "months-day-1-28", "pass", "pass");
    let m = rule(m, "years", "pass", "unverified");
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &change, today()),
        blocked("years repeats are not verified yet")
    );
    let m = rule(base.clone(), "months-day-1-28", "pass", "pass");
    let m = rule(m, "years", "pass", "pass");
    assert_eq!(check(&s, &m, WriteMode::Verified, &change, today()), Ok(()));
    // The new own case is checked against its `repeat` row, not its `edit` row.
    let m = rule(
        cases(&["monthly"], "pass", "pass", &[]),
        "months-day-1-28",
        "pass",
        "pass",
    );
    let m = rule(m, "years", "pass", "pass");
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &change, today()),
        blocked("changing the repeat rule of yearly tasks is not verified yet (iphone_baseline)")
    );
}

#[test]
fn weekly_task_has_case_weekly() {
    let s = snap();
    assert_eq!(task_cases(&s, idx(&s, "gate-weekly")), cs("weekly", None));
    let m = rule(full(), "weeks", "pass", "pass");
    let op = edit("gate-weekly", vec![Change::Summary("Renamed".into())]);
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &op, today()),
        blocked("edit for weekly tasks is not verified yet (iphone_baseline)")
    );
    let m = with(m, cases(&["weekly"], "pass", "pass", &[]));
    assert_eq!(check(&s, &m, WriteMode::Verified, &op, today()), Ok(()));
}

fn kind(uid: &str, k: twydo_core::model::TaskKind) -> Op {
    edit(uid, vec![Change::Kind(k)])
}

#[test]
fn convert_needs_convert_row() {
    use twydo_core::model::TaskKind::{Checklist, Project, Task};
    let s = snap();
    let rows =
        |extra: &[(&str, &str)], baseline: &str| with(fam(), matrix(extra, baseline, "pass", &[]));
    let shapes = [
        (
            "gate-nodue",
            Checklist,
            "a task to a checklist",
            "task-to-checklist",
        ),
        (
            "gate-nodue",
            Project,
            "a task to a project",
            "task-to-project",
        ),
        (
            "gate-checklist",
            Project,
            "a checklist to a project",
            "checklist-to-project",
        ),
        (
            "gate-empty-project",
            Checklist,
            "an empty project to a checklist",
            "empty-project-to-checklist",
        ),
        (
            "gate-empty-project",
            Task,
            "an empty checklist or project to a task",
            "empty-parent-to-task",
        ),
    ];
    for (uid, to, label, case) in shapes {
        let op = kind(uid, to);
        assert_eq!(
            check(&s, &fam(), WriteMode::Verified, &op, today()),
            blocked(&format!(
                "converting {label} is not verified yet (iphone_baseline)"
            )),
            "{uid} to {to:?}"
        );
        let m = rows(&[(case, "convert")], "pass");
        assert_eq!(
            check(&s, &m, WriteMode::Verified, &op, today()),
            Ok(()),
            "{uid} to {to:?}"
        );
    }
    // The iPhone does not offer these, so the matrix has them unsupported.
    let refused = [
        (
            "gate-project",
            Checklist,
            "a project with items to a checklist",
            "project-to-checklist",
        ),
        (
            "gate-project",
            Task,
            "a checklist or project with items to a task",
            "parent-with-items-to-task",
        ),
        (
            "gate-checklist",
            Task,
            "a checklist or project with items to a task",
            "parent-with-items-to-task",
        ),
    ];
    for (uid, to, label, case) in refused {
        let m = rows(&[(case, "convert")], "unsupported");
        assert_eq!(
            check(&s, &m, WriteMode::Test, &kind(uid, to), today()),
            blocked(&format!("converting {label} is not supported by 2Do")),
            "{uid} to {to:?}"
        );
    }
    // The same type is no conversion.
    assert_eq!(
        check(
            &s,
            &fam(),
            WriteMode::Verified,
            &kind("gate-nodue", Task),
            today()
        ),
        Ok(())
    );
    // A repeating task needs the recurring checklist's edit verdict; items cannot be parents.
    let m = rows(&[("task-to-checklist", "convert")], "pass");
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Test,
            &kind("gate-monthly", Checklist),
            today()
        ),
        blocked("edit for recurring-checklist tasks is not verified yet (iphone_baseline)")
    );
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Test,
            &kind("gate-project-child", Checklist),
            today()
        ),
        blocked(NESTED)
    );
    // The embedded matrix carries the rows: task to checklist is verified both ways now.
    assert_eq!(
        check(
            &s,
            Matrix::embedded(),
            WriteMode::Verified,
            &kind("gate-nodue", Checklist),
            today()
        ),
        Ok(())
    );
    // An empty project to a checklist still lacks its iPhone baseline.
    assert_eq!(
        check(
            &s,
            Matrix::embedded(),
            WriteMode::Verified,
            &kind("gate-empty-project", Checklist),
            today()
        ),
        blocked("converting an empty project to a checklist is not verified yet (iphone_baseline)")
    );
    assert_eq!(
        check(
            &s,
            Matrix::embedded(),
            WriteMode::Test,
            &kind("gate-project", Checklist),
            today()
        ),
        blocked("converting a project with items to a checklist is not supported by 2Do")
    );
}

#[test]
fn save_checks_the_edit_and_the_move() {
    let s = snap();
    let save = |changes: Vec<Change>, to: Option<&str>| Op::Save {
        uid: "gate-date".into(),
        changes,
        to: to.map(str::to_string),
    };
    let title = vec![Change::Summary("Renamed".into())];
    let only = |op| matrix(&[("one-time", op)], "pass", "pass", &[]);
    let m = only("edit");
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Verified,
            &save(title.clone(), None),
            today()
        ),
        Ok(())
    );
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Verified,
            &save(title.clone(), Some("errands")),
            today()
        ),
        blocked("move for one-time tasks is not verified yet (iphone_baseline)")
    );
    let m = only("move");
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Verified,
            &save(title.clone(), Some("errands")),
            today()
        ),
        blocked("edit for one-time tasks is not verified yet (iphone_baseline)")
    );
    // A move alone skips the edit part.
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Verified,
            &save(Vec::new(), Some("errands")),
            today()
        ),
        Ok(())
    );
    let m = matrix(
        &[("one-time", "edit"), ("one-time", "move")],
        "pass",
        "pass",
        &[],
    );
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Verified,
            &save(title.clone(), Some("errands")),
            today()
        ),
        Ok(())
    );
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Verified,
            &save(title, Some("nope")),
            today()
        ),
        blocked("no list nope")
    );
}

#[test]
fn create_checks_list_and_attribute_cases() {
    let s = snap();
    let m = matrix(&[("one-time", "create")], "pass", "pass", &[]);
    let new = |collection: &str, due: Option<Due>| {
        Op::Create(NewTask {
            due,
            ..NewTask::plain(collection, "Synthetic new")
        })
    };
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &new("inbox", None), today()),
        Ok(())
    );
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &new("nope", None), today()),
        blocked("no list nope")
    );
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Verified,
            &new("inbox", Some(Due::Date(date(2026, 11, 9)))),
            today()
        ),
        blocked("create for date-only tasks is not verified yet (iphone_baseline)")
    );
}

#[test]
fn target_lookup_reasons() {
    let s = snap();
    let m = full();
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &complete("nope"), today()),
        blocked("no task with UID nope")
    );
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &complete("gate-dup"), today()),
        blocked("duplicate UID")
    );
    let mv = Op::Move {
        uid: "gate-date".into(),
        to: "nope".into(),
    };
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &mv, today()),
        blocked("no list nope")
    );
    let mv = Op::Move {
        uid: "gate-date".into(),
        to: "errands".into(),
    };
    assert_eq!(check(&s, &m, WriteMode::Verified, &mv, today()), Ok(()));
}

#[test]
fn task_blocked_reason_wins() {
    let mut s = snap();
    let i = idx(&s, "gate-date");
    s.tasks[i].blocked = Some("recurrence overrides are not supported".into());
    assert_eq!(
        check(
            &s,
            &full(),
            WriteMode::Verified,
            &complete("gate-date"),
            today()
        ),
        blocked("recurrence overrides are not supported")
    );
}

/// `full()` plus every family case, fully verified.
fn fam() -> Matrix {
    cases(
        &[
            "one-time",
            "monthly",
            "yearly",
            "date-only",
            "timed",
            "start-date",
            "checklist",
            "project",
            "recurring-project",
            "project-recurring-child",
        ],
        "pass",
        "pass",
        &["months-day-1-28", "years"],
    )
}

fn cs(own: &'static str, family: Option<&'static str>) -> Result<Cases, Blocked> {
    Ok(Cases { own, family })
}

#[test]
fn family_cases() {
    let s = snap();
    let at = |uid: &str| task_cases(&s, idx(&s, uid));
    assert_eq!(at("gate-date"), cs("one-time", None));
    assert_eq!(at("gate-monthly"), cs("monthly", None));
    assert_eq!(at("gate-yearly"), cs("yearly", None));
    assert_eq!(at("gate-checklist"), cs("one-time", Some("checklist")));
    assert_eq!(at("gate-checklist-item"), cs("one-time", Some("checklist")));
    assert_eq!(at("gate-project"), cs("one-time", Some("project")));
    assert_eq!(at("gate-project-child"), cs("one-time", Some("project")));
    assert_eq!(
        at("gate-project-repeating-child"),
        cs("monthly", Some("project-recurring-child"))
    );
    assert_eq!(
        at("gate-monthly-project"),
        cs("monthly", Some("recurring-project"))
    );
    assert_eq!(
        at("gate-monthly-project-child"),
        cs("one-time", Some("recurring-project"))
    );
    // A completed project without RRULE (like a recurring project's copy) is a plain project.
    assert_eq!(at("gate-done-project"), cs("one-time", Some("project")));
    assert_eq!(
        at("gate-monthly-checklist"),
        cs("monthly", Some("recurring-checklist"))
    );
    assert_eq!(
        at("gate-monthly-checklist-item"),
        cs("one-time", Some("recurring-checklist"))
    );
}

const NESTED: &str = "nested projects and checklists are not verified yet";

#[test]
fn family_refusals() {
    let s = snap();
    let m = fam();
    let refused = [
        ("gate-nested", NESTED),
        ("gate-nested-child", NESTED),
        ("gate-nested-grandchild", NESTED),
        (
            "gate-checklist-repeating-item",
            "checklist items take their date from the checklist",
        ),
        (
            "gate-recurring-repeating-child",
            "a repeating task inside a repeating project is not verified yet \
             (project-child-own-recurrence)",
        ),
        (
            "gate-plain-parent",
            "subtasks of a plain task are not verified yet",
        ),
        (
            "gate-plain-child",
            "subtasks of a plain task are not verified yet",
        ),
    ];
    for (uid, reason) in refused {
        assert_eq!(
            task_cases(&s, idx(&s, uid)),
            Err(Blocked(reason.into())),
            "{uid}"
        );
        assert_eq!(
            check(&s, &m, WriteMode::Test, &complete(uid), today()),
            blocked(reason),
            "{uid}"
        );
    }
}

#[test]
fn family_ops_check_own_and_family_rows() {
    let s = snap();
    let m = fam();
    for uid in [
        "gate-checklist",
        "gate-checklist-item",
        "gate-project",
        "gate-project-child",
        "gate-project-repeating-child",
        "gate-monthly-project",
        "gate-monthly-project-child",
    ] {
        assert_eq!(
            check(&s, &m, WriteMode::Verified, &complete(uid), today()),
            Ok(()),
            "{uid}"
        );
    }
    // The family row is checked as well as the own row.
    let m = full();
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Test,
            &complete("gate-project-child"),
            today()
        ),
        blocked("complete for project tasks is not verified yet (iphone_baseline)")
    );
    // The own row of a repeating child is checked before its family row.
    let m = cases(
        &["project-recurring-child"],
        "pass",
        "pass",
        &["months-day-1-28"],
    );
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Test,
            &complete("gate-project-repeating-child"),
            today()
        ),
        blocked("complete for monthly tasks is not verified yet (iphone_baseline)")
    );
    // An unverified family row blocks the op: editing in a recurring project.
    let m = matrix(
        &[("one-time", "edit"), ("recurring-project", "complete")],
        "pass",
        "pass",
        &[],
    );
    let op = edit(
        "gate-monthly-project-child",
        vec![Change::Summary("Renamed".into())],
    );
    assert_eq!(
        check(&s, &m, WriteMode::Test, &op, today()),
        blocked("edit for recurring-project tasks is not verified yet (iphone_baseline)")
    );
}

#[test]
fn parent_op_refused_when_child_blocked() {
    let s = snap();
    let m = fam();
    // A second task with a child's UID.
    let mut dup = s.clone();
    let child = dup.tasks[idx(&s, "gate-project-child")].clone();
    dup.tasks.push(child);
    let mv = Op::Move {
        uid: "gate-project".into(),
        to: "errands".into(),
    };
    for op in [
        complete("gate-project"),
        reopen("gate-project"),
        mv.clone(),
        Op::Delete {
            uid: "gate-project".into(),
        },
    ] {
        assert_eq!(
            check(&dup, &m, WriteMode::Test, &op, today()),
            blocked("duplicate UID"),
            "{op:?}"
        );
    }
    // Editing the parent alone does not touch the children.
    let op = edit("gate-project", vec![Change::Summary("Renamed".into())]);
    assert_eq!(check(&dup, &m, WriteMode::Test, &op, today()), Ok(()));

    let mut bad = s.clone();
    let i = idx(&s, "gate-checklist-item");
    bad.tasks[i].blocked = Some("unreadable 2Do metadata".into());
    assert_eq!(
        check(
            &bad,
            &m,
            WriteMode::Test,
            &complete("gate-checklist"),
            today()
        ),
        blocked("unreadable 2Do metadata")
    );
    assert_eq!(check(&s, &m, WriteMode::Test, &mv, today()), Ok(()));
}

#[test]
fn parent_op_refused_when_child_has_no_case() {
    let s = snap();
    let m = fam();
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Test,
            &complete("gate-checklist-mixed"),
            today()
        ),
        blocked("checklist items take their date from the checklist")
    );
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Test,
            &complete("gate-monthly-project-mixed"),
            today()
        ),
        blocked(
            "a repeating task inside a repeating project is not verified yet \
             (project-child-own-recurrence)"
        )
    );
    let mv = Op::Move {
        uid: "gate-project-weekly".into(),
        to: "errands".into(),
    };
    assert_eq!(
        check(&s, &m, WriteMode::Test, &mv, today()),
        blocked("weeks repeats are not verified yet")
    );
    // Editing the parent alone does not look at its children.
    let op = edit(
        "gate-project-weekly",
        vec![Change::Summary("Renamed".into())],
    );
    assert_eq!(check(&s, &m, WriteMode::Test, &op, today()), Ok(()));
}

#[test]
fn parent_op_refused_when_child_rule_not_preserved() {
    let s = snap();
    // The child's own case is "monthly", but its rule class has no `[[rule]]` verdict.
    assert_eq!(
        task_cases(&s, idx(&s, "gate-project-last-day-child")),
        cs("monthly", Some("project-recurring-child"))
    );
    let m = fam();
    let mv = Op::Move {
        uid: "gate-project-last-day".into(),
        to: "errands".into(),
    };
    for op in [
        complete("gate-project-last-day"),
        reopen("gate-project-last-day"),
        mv,
        Op::Delete {
            uid: "gate-project-last-day".into(),
        },
    ] {
        assert_eq!(
            check(&s, &m, WriteMode::Test, &op, today()),
            blocked("months-last-day repeats are not verified yet"),
            "{op:?}"
        );
    }
    // The same parent op passes once the class preserves.
    let m = cases(
        &["one-time", "monthly", "project", "project-recurring-child"],
        "pass",
        "pass",
        &["months-last-day"],
    );
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Test,
            &complete("gate-project-last-day"),
            today()
        ),
        Ok(())
    );
}

/// The real matrix: family baselines pass, the twydo columns and the recurring-project edit
/// row do not.
#[test]
fn embedded_matrix_allows_family_baselines() {
    let s = snap();
    let m = Matrix::embedded();
    for uid in [
        "gate-checklist-item",
        "gate-checklist",
        "gate-monthly-project",
    ] {
        assert_eq!(
            check(&s, m, WriteMode::Test, &complete(uid), today()),
            Ok(()),
            "{uid}"
        );
    }
    let op = edit(
        "gate-monthly-project-child",
        vec![Change::Summary("Renamed".into())],
    );
    // Verified both ways in the Plan 6 Task 7 session (rounds 01 and 07).
    assert_eq!(check(&s, m, WriteMode::Test, &op, today()), Ok(()));
    assert_eq!(check(&s, m, WriteMode::Verified, &op, today()), Ok(()));
    // Twydo completed a checklist and the phone showed it (Plan 6 Task 7); the phone
    // completed one and Twydo read it back (gap session round 04).
    assert_eq!(
        check(
            &s,
            m,
            WriteMode::Verified,
            &complete("gate-checklist"),
            today()
        ),
        Ok(())
    );
}

#[test]
fn unknown_task_type_and_broken_link_refused() {
    let s = snap();
    let m = fam();
    let typed = "this 2Do task type is not verified yet";
    for (uid, reason) in [
        ("gate-typed", typed),
        ("gate-typed-child", typed),
        ("gate-broken-child", "broken parent link"),
    ] {
        assert_eq!(
            task_cases(&s, idx(&s, uid)),
            Err(Blocked(reason.into())),
            "{uid}"
        );
        assert_eq!(
            check(&s, &m, WriteMode::Test, &complete(uid), today()),
            blocked(reason),
            "{uid}"
        );
    }
    // The typed child refuses its parent's family ops too.
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Test,
            &complete("gate-typed-project"),
            today()
        ),
        blocked(typed)
    );
}

#[test]
fn single_child_move_and_delete_blocked() {
    let s = snap();
    let m = fam();
    for uid in ["gate-project-child", "gate-checklist-item"] {
        let mv = Op::Move {
            uid: uid.into(),
            to: "errands".into(),
        };
        assert_eq!(
            check(&s, &m, WriteMode::Test, &mv, today()),
            blocked("moving one item out of its project or checklist is not verified yet"),
            "{uid}"
        );
        let del = Op::Delete { uid: uid.into() };
        assert_eq!(
            check(&s, &m, WriteMode::Test, &del, today()),
            blocked("deleting one item of a project or checklist is not verified yet"),
            "{uid}"
        );
    }
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Test,
            &reopen("gate-done-project-child"),
            today()
        ),
        blocked("reopening an item of a completed project or checklist is not verified yet")
    );
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Test,
            &reopen("gate-cancelled-checklist-item"),
            today()
        ),
        blocked("reopening an item of a completed project or checklist is not verified yet")
    );
    // Reopening the completed project itself is a family op.
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Test,
            &reopen("gate-done-project"),
            today()
        ),
        Ok(())
    );
}

#[test]
fn from_completion_needs_its_case() {
    let s = snap();
    assert_eq!(
        check(
            &s,
            &full(),
            WriteMode::Test,
            &complete("gate-from-completion"),
            today()
        ),
        blocked("complete for from-completion tasks is not verified yet (iphone_baseline)")
    );
}

#[test]
fn weekly_needs_its_case() {
    let s = snap();
    assert_eq!(
        check(
            &s,
            &full(),
            WriteMode::Test,
            &complete("gate-weekly"),
            today()
        ),
        blocked("complete for weekly tasks is not verified yet (iphone_baseline)")
    );
}

#[test]
fn monthly_day_31_blocked() {
    let s = snap();
    let op = edit("gate-monthly-31", vec![Change::Summary("Renamed".into())]);
    assert_eq!(
        check(&s, &full(), WriteMode::Test, &op, today()),
        blocked("months-day-29-31 repeats are not verified yet")
    );
}

#[test]
fn recurring_needs_rule_verdicts() {
    let s = snap();
    let m = cases(&["monthly"], "pass", "pass", &[]);
    let op = edit("gate-monthly", vec![Change::Summary("Renamed".into())]);
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &op, today()),
        blocked("months-day-1-28 repeats are not verified yet")
    );
    let m = full();
    assert_eq!(check(&s, &m, WriteMode::Verified, &op, today()), Ok(()));
    let op = edit("gate-monthly", vec![Change::RepeatInterval(2)]);
    assert_eq!(check(&s, &m, WriteMode::Verified, &op, today()), Ok(()));
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Verified,
            &complete("gate-monthly"),
            today()
        ),
        Ok(())
    );
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Verified,
            &complete("gate-yearly"),
            today()
        ),
        Ok(())
    );
}

const START_OFFSETS: &str =
    "completing a repeating task with a start date is not verified yet (start-due-alarm-offsets)";
const MISSED: &str = "completing a repeating task after a missed occurrence is not verified yet \
                      (monthly-15th-three-months-late)";

/// `m` plus an `[[edge_case]]` row.
fn edge(m: Matrix, id: &str, verdict: &str) -> Matrix {
    let mut m = m;
    let extra = Matrix::from_toml_str(&format!(
        "matrix_version = 1\n[[edge_case]]\nid = \"{id}\"\nverdict = \"{verdict}\"\n"
    ))
    .unwrap();
    m.edge_case.extend(extra.edge_case);
    m
}

#[test]
fn repeat_shapes_need_their_edge_cases() {
    let s = snap();
    let shapes = [
        "gate-monthly-start",
        "gate-monthly-alarm",
        "gate-monthly-timed-start",
    ];
    let unverified = edge(full(), "start-due-alarm-offsets", "unverified");
    let pass = edge(full(), "start-due-alarm-offsets", "pass");
    for uid in shapes {
        assert_eq!(
            check(&s, &unverified, WriteMode::Test, &complete(uid), today()),
            blocked(START_OFFSETS),
            "{uid}"
        );
        assert_eq!(
            check(&s, &pass, WriteMode::Test, &complete(uid), today()),
            Ok(()),
            "{uid}"
        );
    }
}

#[test]
fn unusual_start_date_blocked() {
    let s = snap();
    let m = edge(full(), "start-due-alarm-offsets", "pass");
    let i = idx(&s, "gate-monthly-start");
    // 0, negative, and not on a UTC midnight.
    for secs in ["0", "-86400", "1762563600"] {
        let mut bad = s.clone();
        let raw = bad.tasks[i].meta.raw.clone().unwrap();
        let changed = raw.replace(
            "StartDate%22%3A1762560000",
            &format!("StartDate%22%3A{secs}"),
        );
        assert_ne!(raw, changed);
        bad.tasks[i].meta.raw = Some(changed);
        assert_eq!(
            check(
                &bad,
                &m,
                WriteMode::Test,
                &complete("gate-monthly-start"),
                today()
            ),
            blocked("completing a repeating task with an unusual start date is not verified yet"),
            "{secs}"
        );
    }
}

#[test]
fn zoned_due_blocked() {
    let s = snap();
    let m = edge(full(), "start-due-alarm-offsets", "pass");
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Test,
            &complete("gate-monthly-zoned"),
            today()
        ),
        blocked("completing a repeating task with a zoned due time is not verified yet")
    );
}

#[test]
fn missed_occurrence_needs_its_edge_case() {
    let s = snap();
    let op = complete("gate-monthly-late");
    let unverified = edge(full(), "monthly-15th-three-months-late", "unverified");
    let pass = edge(full(), "monthly-15th-three-months-late", "pass");
    assert_eq!(
        check(&s, &unverified, WriteMode::Test, &op, today()),
        blocked(MISSED)
    );
    assert_eq!(check(&s, &pass, WriteMode::Test, &op, today()), Ok(()));
}

#[test]
fn absolute_alert_stays_blocked() {
    let s = snap();
    let m = edge(full(), "start-due-alarm-offsets", "pass");
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Test,
            &complete("gate-monthly-absolute"),
            today()
        ),
        blocked("completing a repeating task with an absolute alert is not verified yet")
    );
}

#[test]
fn start_day_delay_stays_blocked() {
    let s = snap();
    let m = edge(full(), "start-due-alarm-offsets", "pass");
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Test,
            &complete("gate-monthly-delay"),
            today()
        ),
        blocked("completing a repeating task with a start-day delay is not verified yet")
    );
}

#[test]
fn recurring_with_start_or_alarm_blocked() {
    let s = snap();
    // Rule verdicts pass, but the start-due-alarm-offsets edge case is unverified.
    let m = edge(full(), "start-due-alarm-offsets", "unverified");
    for uid in ["gate-monthly-start", "gate-monthly-alarm"] {
        assert_eq!(
            check(&s, &m, WriteMode::Test, &complete(uid), today()),
            blocked(START_OFFSETS),
            "{uid}"
        );
    }
    // Only completion is restricted: editing the title of the same task is allowed.
    let op = edit(
        "gate-monthly-start",
        vec![Change::Summary("Renamed".into())],
    );
    assert_eq!(check(&s, &m, WriteMode::Test, &op, today()), Ok(()));
}

#[test]
fn recurring_due_removal_blocked() {
    let s = snap();
    let m = full();
    let op = edit("gate-monthly", vec![Change::Due(None)]);
    assert_eq!(
        check(&s, &m, WriteMode::Test, &op, today()),
        blocked("removing the due date of a repeating task is not verified yet")
    );
    let op = edit("gate-date", vec![Change::Due(None)]);
    assert_eq!(check(&s, &m, WriteMode::Test, &op, today()), Ok(()));
}

#[test]
fn history_copy_reopen_blocked() {
    let s = snap();
    let m = cases(&["one-time"], "pass", "pass", &[]);
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Verified,
            &reopen("gate-history"),
            today()
        ),
        blocked("reopening a repeat's history copy is not verified yet")
    );
    // A plain completed one-time task is not a history copy.
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &reopen("gate-done"), today()),
        Ok(())
    );
    for case in ["monthly", "yearly"] {
        let m = matrix(
            &[("one-time", "reopen"), (case, "reopen")],
            "pass",
            "pass",
            &[],
        );
        assert_eq!(
            check(
                &s,
                &m,
                WriteMode::Verified,
                &reopen("gate-history"),
                today()
            ),
            Ok(()),
            "{case}"
        );
    }
}

/// Rule 6 uses the mode-aware row check: verified mode needs the monthly or yearly reopen
/// row's twydo columns too; test mode needs only the baseline.
#[test]
fn history_copy_reopen_mode_aware() {
    let s = snap();
    let mut s_toml = String::from("matrix_version = 1\n");
    for (case, twydo) in [
        ("one-time", "pass"),
        ("monthly", "unverified"),
        ("yearly", "unverified"),
    ] {
        s_toml.push_str(&format!(
            "[[operation]]\ncase = \"{case}\"\nop = \"reopen\"\niphone_baseline = \"pass\"\n\
             mac_to_iphone = \"unverified\"\niphone_to_mac = \"unverified\"\n\
             twydo_to_iphone = \"{twydo}\"\niphone_to_twydo = \"{twydo}\"\n"
        ));
    }
    let m = Matrix::from_toml_str(&s_toml).unwrap();
    let op = reopen("gate-history");
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &op, today()),
        blocked("reopening a repeat's history copy is not verified yet")
    );
    assert_eq!(check(&s, &m, WriteMode::Test, &op, today()), Ok(()));
    // One twydo column alone is not enough.
    let half = s_toml.replacen(
        "case = \"monthly\"\nop = \"reopen\"\niphone_baseline = \"pass\"\n\
         mac_to_iphone = \"unverified\"\niphone_to_mac = \"unverified\"\n\
         twydo_to_iphone = \"unverified\"",
        "case = \"monthly\"\nop = \"reopen\"\niphone_baseline = \"pass\"\n\
         mac_to_iphone = \"unverified\"\niphone_to_mac = \"unverified\"\n\
         twydo_to_iphone = \"pass\"",
        1,
    );
    assert_ne!(half, s_toml);
    let m = Matrix::from_toml_str(&half).unwrap();
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &op, today()),
        blocked("reopening a repeat's history copy is not verified yet")
    );
}

#[test]
fn blocked_displays_reason() {
    assert_eq!(Blocked("duplicate UID".into()).to_string(), "duplicate UID");
}

const ITEM_DATES: &str = "checklist items take their date from the checklist";
const DATED_ITEMS: &str = "removing a checklist's date, or giving it a time or a start date, \
                           is not verified yet while its items have dates of their own";

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 12, d).unwrap()
}

#[test]
fn checklist_item_dates_refused() {
    let s = snap();
    let m = fam();
    for uid in ["gate-checklist-item", "gate-checklist-dated-item"] {
        for change in [
            Change::Due(Some(Due::Date(day(11)))),
            Change::Due(None),
            Change::Start(Some(day(1))),
        ] {
            let op = edit(uid, vec![change.clone()]);
            assert_eq!(
                check(&s, &m, WriteMode::Test, &op, today()),
                blocked(ITEM_DATES),
                "{uid} {change:?}"
            );
        }
        // Other fields of an item stay editable.
        let op = edit(uid, vec![Change::Summary("Renamed".into())]);
        assert_eq!(
            check(&s, &m, WriteMode::Test, &op, today()),
            Ok(()),
            "{uid}"
        );
    }
}

#[test]
fn dated_checklist_takes_only_a_new_date() {
    let s = snap();
    let m = fam();
    let at = |changes| {
        check(
            &s,
            &m,
            WriteMode::Test,
            &edit("gate-checklist-dated", changes),
            today(),
        )
    };
    assert_eq!(at(vec![Change::Due(Some(Due::Date(day(11))))]), Ok(()));
    assert_eq!(at(vec![Change::Summary("Renamed".into())]), Ok(()));
    let time = day(11).and_hms_opt(9, 0, 0).unwrap();
    for changes in [
        vec![Change::Due(None)],
        vec![Change::Due(Some(Due::Time(time)))],
        vec![Change::Start(Some(day(1)))],
        vec![Change::Due(Some(Due::Date(day(11)))), Change::Start(None)],
    ] {
        assert_eq!(at(changes.clone()), blocked(DATED_ITEMS), "{changes:?}");
    }
    // A checklist whose items carry no dates keeps every date edit (session 1, H2).
    let op = edit("gate-checklist", vec![Change::Due(None)]);
    assert_eq!(check(&s, &m, WriteMode::Test, &op, today()), Ok(()));
}

#[test]
fn dated_checklist_date_change_needs_its_edge_case_when_verified() {
    let s = snap();
    let op = edit(
        "gate-checklist-dated",
        vec![Change::Due(Some(Due::Date(day(11))))],
    );
    let reason = "changing a checklist's date while its items have dates of their own is not \
                  verified yet (checklist-item-dates)";
    // fam() passes every row both ways, but has no edge case.
    let m = fam();
    assert_eq!(check(&s, &m, WriteMode::Test, &op, today()), Ok(()));
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &op, today()),
        blocked(reason)
    );
    let m = edge(fam(), "checklist-item-dates", "pass");
    assert_eq!(check(&s, &m, WriteMode::Verified, &op, today()), Ok(()));
    // Title edits of the same checklist are not affected.
    let title = edit(
        "gate-checklist-dated",
        vec![Change::Summary("Renamed".into())],
    );
    assert_eq!(
        check(&s, &fam(), WriteMode::Verified, &title, today()),
        Ok(())
    );
    // The shipped matrix verifies it (checklist-date round, plan6-baseline.md).
    assert_eq!(
        check(&s, Matrix::embedded(), WriteMode::Verified, &op, today()),
        Ok(())
    );
}

#[test]
fn a_new_repeat_needs_a_due_date() {
    let s = snap();
    let m = rule(
        cases(&["one-time", "monthly", "date-only"], "pass", "pass", &[]),
        "months-day-1-28",
        "pass",
        "pass",
    );
    let why = "a repeating task needs a due date";
    // A daily rule has a class without a due date, so only this rule stops it.
    let daily = rule(
        with(m.clone(), cases(&["daily"], "pass", "pass", &[])),
        "days",
        "pass",
        "pass",
    );
    let op = edit(
        "gate-date",
        vec![Change::Repeat(Some(repeat("daily"))), Change::Due(None)],
    );
    assert_eq!(
        check(&s, &daily, WriteMode::Test, &op, today()),
        blocked(why)
    );
    let op = edit(
        "gate-date",
        vec![Change::Repeat(Some(repeat("monthly"))), Change::Due(None)],
    );
    assert_eq!(check(&s, &m, WriteMode::Test, &op, today()), blocked(why));
    let op = edit("gate-nodue", vec![Change::Repeat(Some(repeat("monthly")))]);
    assert_eq!(check(&s, &m, WriteMode::Test, &op, today()), blocked(why));
    let op = Op::Create(NewTask {
        repeat: Some(repeat("monthly")),
        ..NewTask::plain("inbox", "Synthetic repeat")
    });
    assert_eq!(check(&s, &m, WriteMode::Test, &op, today()), blocked(why));
    // With a due date it passes.
    let op = edit(
        "gate-nodue",
        vec![
            Change::Due(Some(Due::Date(date(2026, 11, 10)))),
            Change::Repeat(Some(repeat("monthly"))),
        ],
    );
    assert_eq!(check(&s, &m, WriteMode::Test, &op, today()), Ok(()));
}

#[test]
fn save_move_checks_the_cases_the_edit_leaves() {
    use twydo_core::model::TaskKind::Checklist;
    let s = snap();
    let save = |uid: &str, changes: Vec<Change>| Op::Save {
        uid: uid.into(),
        changes,
        to: Some("errands".into()),
    };
    // A repeat set and moved: the new own case's move row.
    let due = Due::Date(date(2026, 11, 10));
    let op = save(
        "gate-nodue",
        vec![
            Change::Due(Some(due)),
            Change::Repeat(Some(repeat("monthly"))),
        ],
    );
    let base = rule(
        cases(&["one-time", "date-only"], "pass", "pass", &[]),
        "months-day-1-28",
        "pass",
        "pass",
    );
    let monthly_edit = matrix(
        &[("monthly", "edit"), ("monthly", "repeat")],
        "pass",
        "pass",
        &[],
    );
    let m = with(base.clone(), monthly_edit);
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &op, today()),
        blocked("move for monthly tasks is not verified yet (iphone_baseline)")
    );
    let m = with(base, cases(&["monthly"], "pass", "pass", &[]));
    assert_eq!(check(&s, &m, WriteMode::Verified, &op, today()), Ok(()));
    // A type change and moved: the new family's move row.
    let op = save("gate-nodue", vec![Change::Kind(Checklist)]);
    let m = with(
        cases(&["one-time"], "pass", "pass", &[]),
        matrix(
            &[("checklist", "edit"), ("task-to-checklist", "convert")],
            "pass",
            "pass",
            &[],
        ),
    );
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &op, today()),
        blocked("move for checklist tasks is not verified yet (iphone_baseline)")
    );
    let m = with(m, matrix(&[("checklist", "move")], "pass", "pass", &[]));
    assert_eq!(check(&s, &m, WriteMode::Verified, &op, today()), Ok(()));
    // A plain edit and a move needs only the task's own rows.
    let op = save("gate-date", vec![Change::Summary("Renamed".into())]);
    let m = cases(&["one-time"], "pass", "pass", &[]);
    assert_eq!(check(&s, &m, WriteMode::Verified, &op, today()), Ok(()));
}

#[test]
fn no_new_item_under_a_closed_parent() {
    let s = snap();
    let why = "adding an item to a completed or cancelled project or checklist is not verified yet";
    for parent in ["gate-done-project", "gate-cancelled-checklist"] {
        assert_eq!(
            check(
                &s,
                &fam(),
                WriteMode::Test,
                &Op::Create(item(parent)),
                today()
            ),
            blocked(why),
            "{parent}"
        );
    }
}

/// Setting, removing or reshaping a repeat (not only its interval) needs the `repeat` rows
/// of the cases before and after, and the before case's `edit` row. With the shipped matrix
/// these repeat writes are verified.
#[test]
fn embedded_verified_allows_the_repeat_writes_the_phone_checked() {
    let s = snap();
    let m = Matrix::embedded();
    let at = |op: &Op| check(&s, m, WriteMode::Verified, op, today());
    // One-time to monthly or weekly, removing a monthly repeat, weekly <-> monthly.
    for text in ["monthly", "3m", "weekly", "2w"] {
        let op = edit("gate-date", vec![Change::Repeat(Some(repeat(text)))]);
        assert_eq!(at(&op), Ok(()), "{text}");
    }
    let op = edit("gate-monthly", vec![Change::Repeat(None)]);
    assert_eq!(at(&op), Ok(()));
    let op = edit("gate-monthly", vec![Change::Repeat(Some(repeat("weekly")))]);
    assert_eq!(at(&op), Ok(()));
    // An end by count passes since session B (`mod:count` edit).
    let op = edit(
        "gate-monthly",
        vec![Change::Repeat(Some(repeat("monthly 3 times")))],
    );
    assert_eq!(at(&op), Ok(()));
    // Only the interval, within the class: Plan 4's verified edit.
    let op = edit("gate-monthly", vec![Change::Repeat(Some(repeat("2m")))]);
    assert_eq!(at(&op), Ok(()));
    // Editing an existing weekly task (Plan 7 session C, 03-twydo-edit: a new interval, and
    // E3 weekly -> monthly, E4 weekly removed, which the phone showed right).
    let due = Change::Due(Some(Due::Date(date(2026, 11, 10))));
    for changes in [
        vec![due],
        vec![Change::Summary("Renamed".into())],
        vec![Change::RepeatInterval(2)],
        vec![Change::Repeat(Some(repeat("monthly")))],
        vec![Change::Repeat(None)],
    ] {
        let op = edit("gate-weekly", changes.clone());
        assert_eq!(at(&op), Ok(()), "{changes:?}");
    }
}

/// Daily/yearly repeat transitions now pass; uncaptured anchor/family changes stay refused.
#[test]
fn embedded_verified_keeps_uncaptured_repeat_contexts_refused() {
    let s = snap();
    let m = Matrix::embedded();
    let due = Change::Due(Some(Due::Date(date(2026, 11, 10))));
    let at = |op: &Op| check(&s, m, WriteMode::Verified, op, today());
    for (uid, text) in [
        ("gate-weekly", "yearly"),
        ("gate-date", "yearly"),
        ("gate-date", "daily"),
        ("gate-monthly", "yearly"),
        ("gate-monthly", "monthly after done"),
    ] {
        assert_eq!(
            at(&edit(uid, vec![Change::Repeat(Some(repeat(text)))])),
            Ok(())
        );
    }
    let op = edit(
        "gate-monthly",
        vec![Change::Repeat(Some(repeat("monthly after done 3 times")))],
    );
    assert!(at(&op).is_err());
    // A repeat on a project makes it a recurring project.
    let op = edit(
        "gate-empty-project",
        vec![due, Change::Repeat(Some(repeat("monthly")))],
    );
    let Err(Blocked(why)) = at(&op) else {
        panic!("allowed")
    };
    assert!(why.starts_with("changing the repeat rule of "), "{why}");
}

#[test]
fn repeat_rows_with_a_baseline_pass_in_test_mode() {
    let s = snap();
    let m = Matrix::embedded();
    let at = |op: &Op| check(&s, m, WriteMode::Test, op, today());
    // The phone set every repeat kind on a plain task (Plan 7 capture 01).
    for text in ["monthly", "yearly", "daily", "weekly"] {
        let op = edit("gate-date", vec![Change::Repeat(Some(repeat(text)))]);
        let got = at(&op);
        // Rule verdicts still apply (days/weeks edit is unverified); the repeat rows do not stop it.
        if let Err(Blocked(why)) = &got {
            assert!(!why.contains("repeat rule of"), "{text}: {why}");
        }
    }
    let op = edit("gate-date", vec![Change::Repeat(Some(repeat("monthly")))]);
    assert_eq!(at(&op), Ok(()));
    // Not captured: a repeat on a project, or removing one.
    let op = edit(
        "gate-empty-project",
        vec![
            Change::Due(Some(Due::Date(date(2026, 11, 10)))),
            Change::Repeat(Some(repeat("monthly"))),
        ],
    );
    assert_eq!(
        at(&op),
        blocked("changing the repeat rule of project tasks is not verified yet (iphone_baseline)")
    );
    let op = edit("gate-monthly", vec![Change::Repeat(None)]);
    assert_eq!(at(&op), Ok(()), "monthly and one-time rows have a baseline");
    // A synthetic matrix without `repeat` rows refuses a new repeat even in test mode.
    let m = rule(
        matrix(
            &[("one-time", "edit"), ("monthly", "edit")],
            "pass",
            "pass",
            &[],
        ),
        "months-day-1-28",
        "pass",
        "pass",
    );
    let op = edit("gate-date", vec![Change::Repeat(Some(repeat("monthly")))]);
    assert_eq!(
        check(&s, &m, WriteMode::Test, &op, today()),
        blocked("changing the repeat rule of one-time tasks is not verified yet (iphone_baseline)")
    );
    let m = with(
        m,
        matrix(
            &[("one-time", "repeat"), ("monthly", "repeat")],
            "pass",
            "unverified",
            &[],
        ),
    );
    assert_eq!(check(&s, &m, WriteMode::Test, &op, today()), Ok(()));
}

/// The three kinds 2Do 3.91 on the iPhone misreads (rule `edit` = `fail`): Twydo reads them
/// but never writes them, and the refusal says so rather than "not verified yet".
#[test]
fn embedded_verified_never_writes_the_kinds_the_phone_misreads() {
    let s = snap();
    let m = Matrix::embedded();
    let due = Some(Due::Date(date(2026, 11, 10)));
    for (text, class) in [
        ("last day", "months-last-day"),
        ("last weekday", "months-nth-workday"),
        ("1st weekend day", "months-nth-weekend-day"),
    ] {
        let want = blocked(&format!(
            "{class} repeats are not supported by 2Do on the iPhone"
        ));
        let create = Op::Create(NewTask {
            due,
            repeat: Some(repeat(text)),
            ..NewTask::plain("inbox", "Synthetic repeat")
        });
        assert_eq!(
            check(&s, m, WriteMode::Verified, &create, today()),
            want,
            "create {text}"
        );
        let set = edit("gate-date", vec![Change::Repeat(Some(repeat(text)))]);
        assert_eq!(
            check(&s, m, WriteMode::Verified, &set, today()),
            want,
            "edit {text}"
        );
        // The rule verdict applies in test mode too.
        assert_eq!(
            check(&s, m, WriteMode::Test, &create, today()),
            want,
            "test create {text}"
        );
        assert_eq!(
            check(&s, m, WriteMode::Test, &set, today()),
            want,
            "test edit {text}"
        );
    }
    // A context that is merely unverified keeps the old wording.
    let op = edit(
        "gate-date",
        vec![Change::Repeat(Some(repeat("monthly after done")))],
    );
    let Err(Blocked(why)) = check(&s, m, WriteMode::Verified, &op, today()) else {
        panic!("allowed")
    };
    assert!(!why.contains("not supported by 2Do on the iPhone"), "{why}");
}

/// Task 14: the gate lets every repeat kind Twydo can create through to completion once its
/// rule's `execute` verdict and the case's `complete` row pass; since Plan 7 session C the
/// real matrix passes them, so verified mode allows it.
#[test]
fn new_repeat_kinds_complete_when_their_verdicts_pass() {
    let s = snap();
    let m = cases(
        &["one-time", "monthly", "weekly", "date-only", "start-date"],
        "pass",
        "pass",
        &["months-day-29-31", "weeks"],
    );
    // Monthly on the 31st: Oct 31 -> Nov 30 (no longer refused as a clamp).
    for mode in [WriteMode::Test, WriteMode::Verified] {
        assert_eq!(
            check(&s, &m, mode, &complete("gate-monthly-31"), today()),
            Ok(()),
            "{mode:?}"
        );
        assert_eq!(
            check(&s, &m, mode, &complete("gate-weekly"), today()),
            Ok(()),
            "{mode:?}"
        );
    }
    let real = Matrix::embedded();
    assert_eq!(
        check(
            &s,
            real,
            WriteMode::Verified,
            &complete("gate-monthly-31"),
            today()
        ),
        Ok(())
    );
}

/// A vdir with the given fixture steps, each as its own list.
fn steps_snap(steps: &[(&str, &str)]) -> (tempfile::TempDir, Snapshot) {
    let tmp = tempfile::tempdir().unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase0/v1");
    for (list, step) in steps {
        let dir = tmp.path().join(list);
        std::fs::create_dir_all(&dir).unwrap();
        for e in std::fs::read_dir(fixtures.join(step)).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "ics") {
                std::fs::copy(&p, dir.join(p.file_name().unwrap())).unwrap();
            }
        }
    }
    let snap = load_snap(tmp.path());
    (tmp, snap)
}

/// The last occurrence of an ended series, completed in place by the phone (`C1a` in session
/// C's `01-phone-complete`), keeps its RRULE. Reopening it is refused with the reason, before
/// the matrix rows (whose `reopen` verdicts would otherwise be cited), in every mode.
#[test]
fn reopening_an_ended_series_says_why_before_the_rows() {
    let (_tmp, s) = steps_snap(&[("c", "plan7-twydo-list2/01-phone-complete")]);
    let uid = by_title(&s, "c", "C1a");
    let why = "reopening the last occurrence of an ended repeat is not supported; use undo";
    for mode in [WriteMode::Verified, WriteMode::Test] {
        assert_eq!(
            check(&s, Matrix::embedded(), mode, &reopen(&uid), today()),
            blocked(why),
            "{mode:?}"
        );
    }
}

/// A vdir with the session C seed (`plan7-twydo-list2/00-seed`, list `c`) and session B's
/// first round (`plan7-twydo-list1/01-twydo-create`, list `b`), as Twydo wrote them.
fn session_snap() -> (tempfile::TempDir, Snapshot) {
    steps_snap(&[
        ("c", "plan7-twydo-list2/00-seed"),
        ("b", "plan7-twydo-list1/01-twydo-create"),
    ])
}

fn load_snap(root: &Path) -> Snapshot {
    let report = twydo_core::vdir::load(root).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let mut cfg = Config::with_vdir(root.to_path_buf());
    cfg.timezone = Some("America/New_York".to_string());
    Snapshot::new(report, cfg)
}

fn by_title(s: &Snapshot, list: &str, title: &str) -> String {
    s.tasks
        .iter()
        .find(|t| t.collection == list && t.summary == title)
        .unwrap_or_else(|| panic!("{list}/{title}"))
        .uid
        .clone()
}

/// Plan 7 session C: in verified mode with the shipped matrix, completing each kind the
/// phone and Twydo completed alike is allowed (the `b` twins as seeded), and the session's
/// edits of existing daily and weekly tasks too. What no session verified stays refused.
#[test]
fn embedded_verified_completes_every_kind_session_c_verified() {
    let (_tmp, s) = session_snap();
    let m = Matrix::embedded();
    let at = |op: &Op| check(&s, m, WriteMode::Verified, op, today());
    // Classes: days (D1, D3), weeks (W1, W2), weekdays-set (WD, WK, OR), months-nth-weekday
    // (N2, LF), months-day-29-31 (M31), years-feb-29 (Y29); modifiers from-completion (FC,
    // FS with a start date) and count (C1).
    for kind in [
        "D1", "D3", "W1", "W2", "WD", "WK", "OR", "N2", "LF", "M31", "Y29", "FC", "FS", "C1",
    ] {
        let uid = by_title(&s, "c", &format!("{kind}b"));
        assert_eq!(at(&complete(&uid)), Ok(()), "{kind}");
    }
    // Plan 8 also verifies this top-level date-only monthly UNTIL shape.
    let uid = by_title(&s, "c", "UNb");
    assert_eq!(at(&complete(&uid)), Ok(()));
    // Session C's edits (03-twydo-edit), which the phone showed right.
    let e = |t: &str| by_title(&s, "c", t);
    let due = Change::Due(Some(Due::Date(date(2026, 12, 8))));
    for (uid, changes) in [
        (e("E1"), vec![due, Change::Summary("E1x".into())]),
        (e("E2"), vec![Change::RepeatInterval(2)]),
        (e("E3"), vec![Change::Repeat(Some(repeat("monthly")))]),
        (e("E4"), vec![Change::Repeat(None)]),
    ] {
        assert_eq!(at(&edit(&uid, changes.clone())), Ok(()), "{changes:?}");
    }

    // Still refused: the kinds the phone misreads (session B), even to complete.
    for (title, class) in [
        ("LD", "months-last-day"),
        ("LW", "months-nth-workday"),
        ("WE", "months-nth-weekend-day"),
    ] {
        let uid = by_title(&s, "b", title);
        assert_eq!(
            at(&complete(&uid)),
            blocked(&format!("{class} repeats are not verified yet")),
            "{title}"
        );
    }
    // Plain daily/yearly reshaping is now covered by Plan 8.
    for text in ["yearly", "daily"] {
        assert_eq!(
            at(&edit(&e("E3"), vec![Change::Repeat(Some(repeat(text)))])),
            Ok(())
        );
    }
    let op = edit(
        &e("E3"),
        vec![Change::Repeat(Some(repeat("monthly after done 3 times")))],
    );
    assert!(at(&op).is_err());
    // Reopening a history copy of a daily or weekly repeat rests on the monthly/yearly rows;
    // reopening a daily or weekly task itself has no Twydo evidence.
    let op = reopen(&e("E1"));
    assert!(at(&op).is_err());
}

/// Session B's recurring project `RP` (monthly, with item `r1`).
const RP: &str = "c3e14351e7f745b4a713b0c5e64bb8fd";

/// Completing a recurring project is verified for a plain rule only: no capture shows 2Do
/// completing one that ends (count or until) or repeats from completion.
#[test]
fn embedded_verified_refuses_completing_a_recurring_project_with_an_end_or_from_completion() {
    let (tmp, s) = session_snap();
    let m = Matrix::embedded();
    assert_eq!(
        check(&s, m, WriteMode::Verified, &complete(RP), today()),
        Ok(())
    );
    let path = tmp.path().join("b").join(format!("{RP}.ics"));
    let original = std::fs::read(&path).unwrap();
    let why = "completing a recurring project that ends or repeats from completion is not \
               verified yet";
    for (name, rrule, meta) in [
        (
            "count",
            "FREQ=MONTHLY;COUNT=3",
            &[
                ("RecurrenceEndType", 2),
                ("RecurrenceEndRepetitions", 3),
                ("RecurrenceEndRepetitionsOrig", 3),
            ][..],
        ),
        (
            "until",
            "FREQ=MONTHLY;UNTIL=20270630",
            &[
                ("RecurrenceEndType", 1),
                ("RecurrenceEndDate", 1_814_356_800),
            ][..],
        ),
        (
            "from completion",
            "FREQ=MONTHLY",
            &[("RecurrenceFrom", 2)][..],
        ),
    ] {
        let mut cal = parse(&original).unwrap();
        {
            let t = cal.root.children_named_mut("VTODO").next().unwrap();
            t.prop_mut("RRULE").unwrap().set_value(rrule);
            let p = t.prop_mut("X-2DOAPP-METADATA").unwrap();
            let mut j = MetaJson::parse(&p.value).unwrap();
            for (k, v) in meta {
                j.set_i64(k, *v);
            }
            p.set_value(&j.encode());
        }
        std::fs::write(&path, serialize(&cal)).unwrap();
        let s = load_snap(tmp.path());
        for mode in [WriteMode::Verified, WriteMode::Test] {
            assert_eq!(
                check(&s, m, mode, &complete(RP), today()),
                blocked(why),
                "{name} {mode:?}"
            );
        }
    }
}

/// An Nth-weekday repeat has no interval in 2Do (`RecurrenceValue` is the ordinal), so
/// changing its interval is refused in every mode, by `RepeatInterval` or by an interval-only
/// `Change::Repeat`. Setting the same rule again passes.
#[test]
fn interval_of_an_nth_weekday_repeat_is_refused() {
    let (_tmp, s) = session_snap();
    let m = Matrix::embedded();
    let uid = by_title(&s, "c", "N2b");
    let why = blocked("changing the interval of an Nth-weekday repeat is not supported");
    let mut every_2 = repeat("2nd tue");
    every_2.rule.interval = 2;
    for mode in [WriteMode::Verified, WriteMode::Test] {
        for changes in [
            vec![Change::RepeatInterval(2)],
            vec![Change::RepeatInterval(1)],
            vec![Change::Repeat(Some(every_2.clone()))],
        ] {
            let op = edit(&uid, changes.clone());
            assert_eq!(
                check(&s, m, mode, &op, today()),
                why,
                "{mode:?} {changes:?}"
            );
        }
        let same = edit(&uid, vec![Change::Repeat(Some(repeat("2nd tue")))]);
        assert_eq!(check(&s, m, mode, &same, today()), Ok(()), "{mode:?}");
    }
}

/// A weekday set (`FREQ=WEEKLY;BYDAY=MO,TH`) completes only every week, so no op may give it
/// another interval, in any mode: by `RepeatInterval`, an interval-only `Change::Repeat`, a
/// reshape or a create. Interval 1 (the same rule) passes.
#[test]
fn interval_on_a_weekday_set_repeat_is_refused() {
    let (_tmp, s) = session_snap();
    let m = Matrix::embedded();
    let wd = by_title(&s, "c", "WDb");
    let monthly = by_title(&s, "c", "E3");
    let why = blocked("an interval is not supported on a repeat on set weekdays");
    let mut every_2 = repeat("mon thu");
    every_2.rule.interval = 2;
    let due = Due::Date(date(2026, 12, 7));
    for mode in [WriteMode::Verified, WriteMode::Test] {
        for op in [
            edit(&wd, vec![Change::RepeatInterval(2)]),
            edit(&wd, vec![Change::Repeat(Some(every_2.clone()))]),
            edit(
                &wd,
                vec![
                    Change::Repeat(Some(repeat("mon thu"))),
                    Change::RepeatInterval(3),
                ],
            ),
            edit(&monthly, vec![Change::Repeat(Some(every_2.clone()))]),
            Op::Create(NewTask {
                due: Some(due),
                repeat: Some(every_2.clone()),
                ..NewTask::plain("c", "Synthetic")
            }),
        ] {
            assert_eq!(check(&s, m, mode, &op, today()), why, "{mode:?} {op:?}");
        }
    }
    // The same interval, or an interval the next `Repeat` replaces, passes.
    for op in [
        edit(&wd, vec![Change::RepeatInterval(1)]),
        edit(
            &wd,
            vec![
                Change::RepeatInterval(2),
                Change::Repeat(Some(repeat("mon thu"))),
            ],
        ),
    ] {
        assert_eq!(
            check(&s, m, WriteMode::Test, &op, today()),
            Ok(()),
            "{op:?}"
        );
    }
}

/// Sessions B/C already supply fixture-comparison tests for these edges in edit_complete.rs.
/// Keep the summary records aligned without promoting the only partially captured edges.
#[test]
fn embedded_edge_records_match_existing_completion_captures() {
    use twydo_core::compat::Verdict;

    let m = Matrix::embedded();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (id, evidence) in [
        (
            "monthly-31st",
            "fixtures/phase0/v1/plan7-twydo-list2/04-phone-m31-again",
        ),
        (
            "yearly-feb-29",
            "fixtures/phase0/v1/plan7-twydo-list2/01-phone-complete",
        ),
        (
            "from-completion-anchor",
            "fixtures/phase0/v1/plan7-twydo-list2/01-phone-complete",
        ),
        (
            "end-after-n",
            "fixtures/phase0/v1/plan7-twydo-list1/03-phone-act",
        ),
    ] {
        let edge = m.edge_case(id).unwrap();
        assert_eq!(edge.verdict, Verdict::Pass, "{id}");
        assert!(edge.evidence.iter().any(|e| e == evidence), "{id}");
        for path in &edge.evidence {
            assert!(root.join(path).is_dir(), "{id}: {path}");
        }
    }
    assert_eq!(
        m.edge_case("missing-fifth-weekday").unwrap().verdict,
        Verdict::Unverified
    );
    assert_eq!(m.edge_case("end-on-date").unwrap().verdict, Verdict::Pass);
    assert_eq!(m.rule("mod:until").unwrap().execute, Verdict::Pass);
}

// Native phone round trips; see the capture log's regression-test map.
mod recurrence_phone_captures {
    use crate::common::recurrence::*;
    use crate::common::{Env, dtstamp, todo};
    use twydo_core::compat::{Matrix, Verdict};
    use twydo_core::config::WriteMode;
    use twydo_core::edit::{Change, Op};

    #[test]
    fn captured_until_reopen_is_gated_by_its_specific_operation_row() {
        let env = Env::new(WriteMode::Verified, &format!("{SCENARIO}/{END}"));
        let snap = env.snap();
        for (uid, case) in [(U1, "ended-until-daily"), (U2, "ended-until-monthly")] {
            let op = Op::Reopen { uid: uid.into() };
            let mut matrix = Matrix::embedded().clone();
            assert!(
                twydo_core::gate::check(
                    &snap,
                    &matrix,
                    WriteMode::Verified,
                    &op,
                    dtstamp(&cal(END, uid)).date_naive()
                )
                .is_ok()
            );
            matrix
                .operation
                .iter_mut()
                .find(|r| r.case == case && r.op == "reopen")
                .unwrap()
                .twydo_to_iphone = Verdict::Unverified;
            assert!(
                twydo_core::gate::check(
                    &snap,
                    &matrix,
                    WriteMode::Verified,
                    &op,
                    dtstamp(&cal(END, uid)).date_naive()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn baseline_only_matrix_does_not_enable_unchecked_writes() {
        for (step, uid, op) in [
            (SET, FC, Op::Complete { uid: FC.into() }),
            (SEED, U1, Op::Complete { uid: U1.into() }),
            (END, U1, Op::Reopen { uid: U1.into() }),
        ] {
            let env = Env::new(WriteMode::Verified, &format!("{SCENARIO}/{step}"));
            let mut matrix = Matrix::embedded().clone();
            matrix
                .rule
                .iter_mut()
                .find(|r| r.class == "mod:until")
                .unwrap()
                .execute = Verdict::Unverified;
            for case in ["recurring-checklist", "ended-until-daily"] {
                for row in matrix.operation.iter_mut().filter(|r| r.case == case) {
                    row.twydo_to_iphone = Verdict::Unverified;
                }
            }
            let mut store = env.store(matrix);
            let before = env.files();
            assert!(
                store
                    .apply(&env.snap(), &op, dtstamp(&cal(step, uid)))
                    .is_err()
            );
            assert_eq!(env.files(), before);
            assert!(env.journal().is_empty());
        }
    }

    #[test]
    fn checklist_repeat_only_reshape_uses_repeat_evidence_and_mixed_edits_still_need_edit() {
        const STEP: &str = "120-phone-second-round-followup";
        let env = Env::new(WriteMode::Verified, &format!("{SCENARIO}/{STEP}"));
        let snap = env.snap();
        let uid = step_files(STEP)
            .iter()
            .find(|(_, c)| {
                todo(c).prop("SUMMARY").unwrap().value == "FCb" && todo(c).prop("RRULE").is_some()
            })
            .unwrap()
            .0
            .clone();
        let mut matrix = Matrix::embedded().clone();
        matrix
            .operation
            .iter_mut()
            .find(|r| r.case == "recurring-checklist" && r.op == "repeat")
            .unwrap()
            .twydo_to_iphone = Verdict::Pass;
        // Retain a baseline-only matrix to test pure repeat evidence independently.
        matrix
            .operation
            .retain(|r| !(r.case == "recurring-checklist" && r.op == "edit"));
        assert!(matrix.operation("recurring-checklist", "edit").is_none());
        let now = dtstamp(&cal(STEP, &uid));
        let change = Change::Repeat(
            twydo_core::repeatinput::parse("weekly", now.date_naive(), &[]).unwrap(),
        );
        let op = Op::Edit {
            uid: uid.clone(),
            changes: vec![change.clone()],
        };
        assert!(
            twydo_core::gate::check(&snap, &matrix, WriteMode::Verified, &op, now.date_naive())
                .is_ok(),
            "a pure reshape needs its dedicated repeat row, not an ordinary-edit row"
        );
        for changes in [
            vec![
                change.clone(),
                Change::Description(Some("Synthetic mixed edit".into())),
            ],
            vec![
                change,
                Change::Summary("Synthetic renamed checklist".into()),
            ],
            vec![Change::Repeat(
                twydo_core::repeatinput::parse("2m", now.date_naive(), &[]).unwrap(),
            )],
        ] {
            let op = Op::Edit {
                uid: uid.clone(),
                changes,
            };
            let before = env.files();
            let mut store = env.store(matrix.clone());
            let result = store.apply(&snap, &op, now);
            assert!(
                result.is_err(),
                "mixed changes and interval-only edits still need edit evidence"
            );
            assert_eq!(env.files(), before, "refused saves write nothing");
            assert!(env.journal().is_empty());
        }
        matrix
            .operation
            .iter_mut()
            .find(|r| r.case == "recurring-checklist" && r.op == "repeat")
            .unwrap()
            .twydo_to_iphone = Verdict::Unverified;
        assert!(
            twydo_core::gate::check(&snap, &matrix, WriteMode::Verified, &op, now.date_naive())
                .is_err(),
            "unverified repeat changes remain refused"
        );
    }
}

// Effective permissions for captured recurrence shapes, including uncaptured neighbors.
mod verified_recurrence_shapes {
    use crate::common::Env;
    use chrono::{Duration, TimeZone, Utc};
    use twydo_core::compat::{Matrix, Verdict};
    use twydo_core::config::WriteMode;
    use twydo_core::edit::{Change, Op};
    use twydo_core::gate;
    use twydo_core::ical::value::IcalDateTime;
    use twydo_core::model::meta_edit::MetaJson;
    use twydo_core::model::{Alarm, Trigger};

    const FC: &str = "61ed74be3ba34212ab7476b698de1263";
    const U1: &str = "91fb1d1636754afda633eb2bda234417";
    const U2: &str = "be99b147d81b4df6a63dc134e559dab6";
    const RD: &str = "db95528b16fb4aa2b6a47e56a0a1000b";
    const RY: &str = "033d6526b6ba46999f3f510edecf888b";
    const SEED: &str = "001-after-upload";
    const SET: &str = "010-first-phone-actions";
    const BOUNDARY: &str = "020-boundary-and-checklist-complete";
    const END: &str = "030-end-series-and-items";
    const REOPEN: &str = "040-reopen-ended-and-complete-parent";

    fn env(step: &str) -> Env {
        Env::new(WriteMode::Verified, &format!("plan8-tui-a/{step}"))
    }
    fn now() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 4, 18, 0, 0).unwrap()
    }
    fn repeat(uid: &str, text: &str) -> Op {
        Op::Edit {
            uid: uid.into(),
            changes: vec![Change::Repeat(
                twydo_core::repeatinput::parse(text, now().date_naive(), &[]).unwrap(),
            )],
        }
    }
    fn promoted() -> Matrix {
        let mut m = Matrix::embedded().clone();
        for (case, op) in [
            ("daily", "repeat"),
            ("yearly", "repeat"),
            ("checklist", "repeat"),
            ("recurring-checklist", "repeat"),
            ("recurring-checklist", "complete"),
            ("ended-until-daily", "reopen"),
            ("ended-until-monthly", "reopen"),
            ("history-copy", "reopen"),
        ] {
            m.operation
                .iter_mut()
                .find(|r| r.case == case && r.op == op)
                .unwrap()
                .twydo_to_iphone = Verdict::Pass;
        }
        m.rule
            .iter_mut()
            .find(|r| r.class == "mod:until")
            .unwrap()
            .execute = Verdict::Pass;
        m
    }

    #[test]
    fn embedded_verified_operations_match_supported_phone_shapes() {
        let mut cases = Vec::new();
        for (step, uid, rule) in [
            (SEED, RD, "daily"),
            (SET, RD, "weekly"),
            (REOPEN, RD, "none"),
            (SEED, RY, "yearly"),
            (BOUNDARY, RY, "monthly"),
            (REOPEN, RY, "none"),
            (SEED, FC, "monthly"),
            (SET, FC, "weekly"),
            (SET, FC, "none"),
        ] {
            cases.push((step, repeat(uid, rule), 1));
        }
        for step in [SET, END] {
            cases.push((step, Op::Complete { uid: FC.into() }, 8));
        }
        for (step, uid, writes) in [
            (SEED, U1, 2),
            (SET, U1, 2),
            (BOUNDARY, U1, 1),
            (SEED, U2, 2),
            (SET, U2, 2),
            (BOUNDARY, U2, 1),
        ] {
            cases.push((step, Op::Complete { uid: uid.into() }, writes));
        }
        for uid in [U1, U2] {
            cases.push((END, Op::Reopen { uid: uid.into() }, 1));
        }
        cases.push((
            "050-remove-repeat-and-refinish",
            Op::Reopen {
                uid: "7435cbd64d9443888f9eb8ebc84518f4".into(),
            },
            1,
        ));
        for (step, op, writes) in cases {
            let e = env(step);
            let mut store = e.store(Matrix::embedded().clone());
            store
                .apply(&e.snap(), &op, now())
                .unwrap_or_else(|err| panic!("{step} {op:?}: {err}"));
            assert_eq!(e.journal()[0].writes.len(), writes, "{step} {op:?}");
        }
    }

    #[test]
    fn promoted_until_and_checklist_verdicts_do_not_admit_uncaptured_contexts() {
        for (uid, step, op) in [
            (U1, SET, Op::Complete { uid: U1.into() }),
            (FC, SET, Op::Complete { uid: FC.into() }),
            (U1, END, Op::Reopen { uid: U1.into() }),
        ] {
            for shape in ["timed", "start", "alarm", "start-delay", "interval"] {
                let e = env(step);
                let mut snap = e.snap();
                let i = snap.find(uid).unwrap();
                let t = &mut snap.tasks[i];
                match shape {
                    "timed" => t.due = Some(IcalDateTime::Floating(now().naive_utc())),
                    "start" => t.start = Some(IcalDateTime::Date(now().date_naive())),
                    "alarm" => t.alarms.push(Alarm {
                        trigger: Trigger::Relative {
                            offset: Duration::minutes(-5),
                            related_end: true,
                        },
                        index: 0,
                    }),
                    "start-delay" => {
                        let mut m = MetaJson::parse(t.meta.raw.as_deref().unwrap()).unwrap();
                        m.set_i64("StartDayDelay", 1);
                        t.meta.raw = Some(m.encode());
                    }
                    "interval" => {
                        t.rrule = Some(
                            t.rrule
                                .as_ref()
                                .unwrap()
                                .replace("FREQ=", "INTERVAL=2;FREQ="),
                        )
                    }
                    _ => unreachable!(),
                }
                assert!(
                    gate::check(
                        &snap,
                        &promoted(),
                        WriteMode::Verified,
                        &op,
                        now().date_naive()
                    )
                    .is_err(),
                    "{uid} {shape}"
                );
            }
        }
        for (uid, step) in [(U1, SET), (FC, SET), (U1, END)] {
            for (text, raw) in [
                ("weekly until", "FREQ=WEEKLY;UNTIL=20261120"),
                ("yearly until", "FREQ=YEARLY;UNTIL=20281120"),
                ("interval until", "FREQ=DAILY;INTERVAL=2;UNTIL=20261120"),
            ] {
                let e = env(step);
                let mut snap = e.snap();
                let i = snap.find(uid).unwrap();
                snap.tasks[i].rrule = Some(raw.into());
                let op = if step == END {
                    Op::Reopen { uid: uid.into() }
                } else {
                    Op::Complete { uid: uid.into() }
                };
                assert!(
                    gate::check(
                        &snap,
                        &promoted(),
                        WriteMode::Verified,
                        &op,
                        now().date_naive()
                    )
                    .is_err(),
                    "{uid} {step} {text}"
                );
            }
        }
    }

    #[test]
    fn timed_until_is_refused_for_completion_and_reopening() {
        for (uid, step, op) in [
            (U1, SET, Op::Complete { uid: U1.into() }),
            (U2, SET, Op::Complete { uid: U2.into() }),
            (U1, END, Op::Reopen { uid: U1.into() }),
            (U2, END, Op::Reopen { uid: U2.into() }),
        ] {
            let mut snap = env(step).snap();
            assert!(
                gate::check(
                    &snap,
                    Matrix::embedded(),
                    WriteMode::Verified,
                    &op,
                    now().date_naive()
                )
                .is_ok(),
                "baseline {uid} {step}"
            );
            let i = snap.find(uid).unwrap();
            let rule = snap.tasks[i].rrule.as_mut().unwrap();
            let start = rule.find("UNTIL=").unwrap() + "UNTIL=".len();
            let end = rule[start..].find(';').map_or(rule.len(), |n| start + n);
            rule.replace_range(start..end, "20261120T120000Z");
            assert!(
                gate::check(
                    &snap,
                    Matrix::embedded(),
                    WriteMode::Verified,
                    &op,
                    now().date_naive()
                )
                .is_err(),
                "timed until {uid} {step}"
            );
        }
    }

    #[test]
    fn checklist_repeat_permissions_cover_only_the_captured_rules() {
        for step in [SEED, SET] {
            for text in [
                "daily",
                "yearly",
                "2w",
                "2m",
                "2nd tue",
                "monthly after done",
                "monthly 3 times",
                "monthly until 2027-03-10",
            ] {
                let e = env(step);
                let op = repeat(FC, text);
                assert!(
                    gate::check(
                        &e.snap(),
                        &promoted(),
                        WriteMode::Verified,
                        &op,
                        now().date_naive()
                    )
                    .is_err(),
                    "{step} {text}"
                );
            }
        }
    }
    #[test]
    fn newly_verified_daily_yearly_changes_do_not_enable_end_or_ordinal_edits() {
        for uid in [RD, RY] {
            for text in [
                "daily 3 times",
                "yearly until 2028-12-07",
                "mon thu",
                "2nd tue",
                "monthly after done",
            ] {
                let e = env(SET);
                assert!(
                    gate::check(
                        &e.snap(),
                        Matrix::embedded(),
                        WriteMode::Verified,
                        &repeat(uid, text),
                        now().date_naive()
                    )
                    .is_err(),
                    "{uid} {text}"
                );
            }
        }
    }
    #[test]
    fn review_regression_plain_checklist_mixed_repeat_save_needs_destination_edit() {
        let e = env(SEED);
        let mut op = repeat(FC, "monthly");
        if let Op::Edit { changes, .. } = &mut op {
            changes.push(Change::Summary("Synthetic renamed checklist".into()));
        }
        let before = e.files();
        let mut store = e.store(Matrix::embedded().clone());
        assert!(store.apply(&e.snap(), &op, now()).is_err());
        assert_eq!(e.files(), before);
        assert!(e.journal().is_empty());
    }

    #[test]
    fn review_regression_existing_until_interval_apis_remain_equivalent() {
        let e = env(SEED);
        for op in [
            repeat(U1, "2d until 2026-11-20"),
            Op::Edit {
                uid: U1.into(),
                changes: vec![Change::RepeatInterval(2)],
            },
        ] {
            assert!(
                gate::check(
                    &e.snap(),
                    Matrix::embedded(),
                    WriteMode::Verified,
                    &op,
                    now().date_naive()
                )
                .is_ok(),
                "{op:?}"
            );
        }
    }
    #[test]
    fn weekly_checklist_completion_refused_but_history_preservation_admitted() {
        let e = env("050-remove-repeat-and-refinish");
        let snap = e.snap();
        assert!(
            gate::check(
                &snap,
                Matrix::embedded(),
                WriteMode::Verified,
                &Op::Complete { uid: FC.into() },
                now().date_naive()
            )
            .is_err(),
            "phone-captured weekly checklist still lacks completion evidence"
        );
        let history = "7435cbd64d9443888f9eb8ebc84518f4";
        for shape in ["timed", "start", "alarm"] {
            let mut snap = e.snap();
            let i = snap.find(history).unwrap();
            match shape {
                "timed" => snap.tasks[i].due = Some(IcalDateTime::Floating(now().naive_utc())),
                "start" => snap.tasks[i].start = Some(IcalDateTime::Date(now().date_naive())),
                _ => snap.tasks[i].alarms.push(Alarm {
                    trigger: Trigger::Relative {
                        offset: Duration::minutes(-5),
                        related_end: true,
                    },
                    index: 0,
                }),
            }
            assert!(
                gate::check(
                    &snap,
                    Matrix::embedded(),
                    WriteMode::Verified,
                    &Op::Reopen {
                        uid: history.into()
                    },
                    now().date_naive()
                )
                .is_ok(),
                "{shape}"
            );
        }
        let family = Env::new(WriteMode::Verified, "plan6-list1/05-complete-parent");
        assert!(
            gate::check(
                &family.snap(),
                Matrix::embedded(),
                WriteMode::Verified,
                &Op::Reopen {
                    uid: "14b5a37a52224e8c959b5002e2275fc9".into()
                },
                now().date_naive()
            )
            .is_ok(),
            "the dedicated family history row admits H7"
        );
    }
}

#[test]
fn verified_from_completion_edit_rows_do_not_enable_uncaptured_shapes() {
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("list");
    std::fs::create_dir(&list).unwrap();
    let seed = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../fixtures/phase0/v1/plan8-rfra/202-seeded-after-upload/twydo-p8-rfra-fields-p.ics",
    );
    std::fs::copy(seed, list.join("fields.ics")).unwrap();
    let m = Matrix::embedded().clone();
    let uid = "twydo-p8-rfra-fields-p";
    for (text, allowed) in [
        ("monthly after done", true),
        ("weekly after done", true),
        ("weekly", false),
        ("daily after done", false),
        ("yearly after done", false),
        ("monthly after done 3 times", false),
    ] {
        let s = load_snap(dir.path());
        let op = Op::Edit {
            uid: uid.into(),
            changes: vec![Change::Repeat(Some(repeat(text)))],
        };
        assert_eq!(
            check(&s, &m, WriteMode::Verified, &op, today()).is_ok(),
            allowed,
            "{text}"
        );
    }
    for (rule, change, allowed) in [
        ("FREQ=MONTHLY", Change::Summary("changed".into()), true),
        ("FREQ=WEEKLY", Change::Summary("changed".into()), false),
        ("FREQ=YEARLY", Change::Summary("changed".into()), false),
        (
            "FREQ=MONTHLY;INTERVAL=2",
            Change::Summary("changed".into()),
            false,
        ),
        ("FREQ=MONTHLY", Change::RepeatInterval(2), true),
        ("FREQ=MONTHLY", Change::RepeatInterval(3), false),
    ] {
        let mut s = load_snap(dir.path());
        let i = s.find(uid).unwrap();
        s.tasks[i].rrule = Some(rule.into());
        let op = Op::Edit {
            uid: uid.into(),
            changes: vec![change],
        };
        assert_eq!(
            check(&s, &m, WriteMode::Verified, &op, today()).is_ok(),
            allowed,
            "{rule}: {op:?}"
        );
    }
}

#[test]
fn verified_from_completion_repeat_changes_require_captured_transitions() {
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("list");
    std::fs::create_dir(&list).unwrap();
    let seed = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../fixtures/phase0/v1/plan8-rfra/202-seeded-after-upload/twydo-p8-rfra-fields-p.ics",
    );
    std::fs::copy(seed, list.join("fields.ics")).unwrap();
    let uid = "twydo-p8-rfra-fields-p";
    for (old, from_completion, new, allowed) in [
        (None, false, Some("monthly after done"), true),
        (None, false, Some("weekly after done"), false),
        (
            Some("FREQ=MONTHLY"),
            false,
            Some("monthly after done"),
            true,
        ),
        (
            Some("FREQ=MONTHLY"),
            false,
            Some("weekly after done"),
            false,
        ),
        (Some("FREQ=WEEKLY"), false, Some("weekly after done"), false),
        (
            Some("FREQ=WEEKLY"),
            false,
            Some("monthly after done"),
            false,
        ),
        (Some("FREQ=MONTHLY"), true, Some("weekly after done"), true),
        (Some("FREQ=MONTHLY"), true, Some("weekly"), false),
        (Some("FREQ=MONTHLY"), true, Some("monthly"), false),
        (Some("FREQ=MONTHLY"), true, None, true),
        (Some("FREQ=WEEKLY"), true, Some("monthly after done"), false),
        (Some("FREQ=WEEKLY"), true, Some("weekly"), true),
        (Some("FREQ=WEEKLY"), true, None, false),
    ] {
        let mut s = load_snap(dir.path());
        let i = s.find(uid).unwrap();
        s.tasks[i].rrule = old.map(str::to_owned);
        s.tasks[i].from_completion = from_completion;
        let op = Op::Edit {
            uid: uid.into(),
            changes: vec![Change::Repeat(new.map(repeat))],
        };
        assert_eq!(
            check(&s, Matrix::embedded(), WriteMode::Verified, &op, today()).is_ok(),
            allowed,
            "{old:?} from_completion={from_completion} -> {new:?}"
        );
    }
}

#[test]
fn verified_from_completion_due_edits_refuse_custom_zero_offset_alarms_atomically() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../fixtures/phase0/v1/plan8-rfra/250-phone-time-added/twydo-p8-rfra-fields-p.ics",
    );
    let original = std::fs::read_to_string(fixture).unwrap();
    let m = Matrix::embedded().clone();
    for variant in ["automatic", "custom", "related-end"] {
        let dir = tempfile::tempdir().unwrap();
        let list = dir.path().join("list");
        std::fs::create_dir(&list).unwrap();
        let text = match variant {
            "custom" => original.replace(
                "X-2DOAPP-ISAUTO;SHARE-SCOPE=GLOBAL:1",
                "X-2DOAPP-ISAUTO;SHARE-SCOPE=GLOBAL:0",
            ),
            "related-end" => original.replace("TRIGGER:PT0S", "TRIGGER;RELATED=END:PT0S"),
            _ => original.clone(),
        };
        let path = list.join("fields.ics");
        std::fs::write(&path, &text).unwrap();
        let s = load_snap(dir.path());
        let op = Op::Save {
            uid: "twydo-p8-rfra-fields-p".into(),
            changes: vec![
                Change::Summary("retained".into()),
                Change::Due(Some(Due::Time(today().and_hms_opt(11, 0, 0).unwrap()))),
            ],
            to: None,
        };
        assert_eq!(
            check(&s, &m, WriteMode::Verified, &op, today()).is_ok(),
            variant == "automatic",
            "{variant}"
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), text);
    }
}

#[test]
fn verified_from_completion_setting_refuses_reopened_history_but_keeps_live_foreign_series_ids() {
    use twydo_core::edit::reopen_one_time;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase0/v1/plan8-rfra");
    let history = std::fs::read_dir(root.join("363-phone-fields-live-note"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "ics"))
        .map(|p| parse(&std::fs::read(p).unwrap()).unwrap())
        .find(|c| {
            c.root
                .children_named("VTODO")
                .next()
                .unwrap()
                .prop("STATUS")
                .is_some_and(|p| p.value == "COMPLETED")
        })
        .unwrap();
    let mut reopened = history;
    reopen_one_time(
        &mut reopened,
        chrono::DateTime::parse_from_rfc3339("2026-11-04T21:00:00Z")
            .unwrap()
            .to_utc(),
    )
    .unwrap();
    let history_uid = reopened
        .root
        .children_named("VTODO")
        .next()
        .unwrap()
        .prop("UID")
        .unwrap()
        .value
        .clone();
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("list");
    std::fs::create_dir(&list).unwrap();
    std::fs::write(list.join("history.ics"), serialize(&reopened)).unwrap();
    let s = load_snap(dir.path());
    let op = Op::Edit {
        uid: history_uid,
        changes: vec![Change::Repeat(Some(repeat("monthly after done")))],
    };
    assert!(
        check(&s, Matrix::embedded(), WriteMode::Verified, &op, today()).is_err(),
        "reopened history must not become a second series with the old RUID"
    );

    let mut live = parse(
        &std::fs::read(root.join("202-seeded-after-upload/twydo-p8-rfra-fields-p.ics")).unwrap(),
    )
    .unwrap();
    let p = live
        .root
        .children_named_mut("VTODO")
        .next()
        .unwrap()
        .prop_mut("X-2DOAPP-METADATA")
        .unwrap();
    let mut meta = MetaJson::parse(&p.value).unwrap();
    meta.set_str("RUID", "synthetic-phone-live-series-id");
    p.set_value(&meta.encode());
    std::fs::write(list.join("live.ics"), serialize(&live)).unwrap();
    let s = load_snap(dir.path());
    for change in [
        Change::Summary("safe live edit".into()),
        Change::Repeat(Some(repeat("weekly after done"))),
    ] {
        let op = Op::Edit {
            uid: "twydo-p8-rfra-fields-p".into(),
            changes: vec![change],
        };
        assert!(
            check(&s, Matrix::embedded(), WriteMode::Verified, &op, today()).is_ok(),
            "live opaque series IDs are supported"
        );
    }
}

mod recurring_checklist_writes {
    use super::*;
    use crate::common::{Env, recurrence::*};
    use twydo_core::ical::value::IcalDateTime;
    use twydo_core::model::{Status, TaskKind};

    fn env(step: &str) -> Env {
        Env::new(WriteMode::Verified, &format!("{CHECKLIST_B}/{step}"))
    }

    fn parent() -> NewTask {
        NewTask {
            kind: TaskKind::Checklist,
            due: Some(Due::Date(date(2026, 12, 10))),
            repeat: Some(repeat("monthly")),
            ..NewTask::plain("list1", "c")
        }
    }

    #[test]
    fn verified_checklist_rows_allow_only_captured_creation_and_text_conversion() {
        let e = env("010-phone-create-convert");
        let s = e.snap();
        let m = Matrix::embedded().clone();
        for op in [
            Op::Create(parent()),
            Op::Create(NewTask {
                parent: Some(CHECKLIST_A_UID.into()),
                ..NewTask::plain("list1", "x")
            }),
            edit(
                CHECKLIST_A_UID,
                vec![
                    Change::Summary("A".into()),
                    Change::Description(Some("x".into())),
                ],
            ),
            edit("twydo-p8-c29-d", vec![Change::Kind(TaskKind::Checklist)]),
        ] {
            assert!(
                check(&s, &m, WriteMode::Verified, &op, today()).is_ok(),
                "{op:?}"
            );
            let mut baseline_only = m.clone();
            for row in &mut baseline_only.operation {
                if (row.case == "recurring-checklist"
                    && matches!(row.op.as_str(), "create" | "edit"))
                    || row.case == "recurring-checklist-item"
                {
                    row.twydo_to_iphone = twydo_core::compat::Verdict::Unverified;
                }
            }
            assert!(
                check(&s, &baseline_only, WriteMode::Verified, &op, today()).is_err(),
                "baseline-only evidence cannot bypass phone acceptance"
            );
        }
        for change in [
            Change::Due(Some(Due::Date(date(2026, 12, 11)))),
            Change::Start(Some(date(2026, 12, 9))),
            Change::RepeatInterval(2),
            Change::Kind(TaskKind::Project),
        ] {
            assert!(
                check(
                    &s,
                    &m,
                    WriteMode::Verified,
                    &edit(CHECKLIST_A_UID, vec![change.clone()]),
                    today()
                )
                .is_err(),
                "{change:?}"
            );
        }
        for text in [
            "weekly",
            "yearly",
            "2m",
            "monthly after done",
            "monthly 2 times",
        ] {
            let mut n = parent();
            if let Ok(Some(spec)) = twydo_core::repeatinput::parse(text, today(), &[]) {
                n.repeat = Some(spec);
                assert!(
                    check(&s, &m, WriteMode::Verified, &Op::Create(n), today()).is_err(),
                    "{text}"
                );
            } else {
                panic!("invalid negative repeat: {text}");
            }
        }
        for n in [
            NewTask {
                start: Some(date(2026, 12, 9)),
                ..parent()
            },
            NewTask {
                due: Some(Due::Time(date(2026, 12, 10).and_hms_opt(9, 0, 0).unwrap())),
                ..parent()
            },
            NewTask {
                due: Some(Due::Date(date(2026, 12, 31))),
                ..parent()
            },
        ] {
            assert!(
                check(&s, &m, WriteMode::Verified, &Op::Create(n.clone()), today()).is_err(),
                "{n:?}"
            );
        }
    }

    #[test]
    fn legacy_checklist_delete_requires_dedicated_evidence_and_open_sibling() {
        let e = env("011-phone-items-and-text");
        let mut s = e.snap();
        let mut m = Matrix::embedded().clone();
        m.operation
            .retain(|r| !matches!(r.case.as_str(), "checklist-item" | "project-item"));
        let p = s.find(CHECKLIST_A_UID).unwrap();
        let q = *s
            .children(p)
            .iter()
            .find(|&&c| s.tasks[c].summary == "q")
            .unwrap();
        let other = *s.children(p).iter().find(|&&c| c != q).unwrap();
        let op = Op::Delete {
            uid: s.tasks[q].uid.clone(),
        };
        assert!(check(&s, &m, WriteMode::Verified, &op, today()).is_ok());
        let mut missing = m.clone();
        missing
            .operation
            .retain(|r| r.case != "recurring-checklist-item");
        assert!(check(&s, &missing, WriteMode::Verified, &op, today()).is_err());
        for which in [p, q, other] {
            s.tasks[which].status = Status::Completed;
            assert!(
                check(&s, &m, WriteMode::Verified, &op, today()).is_err(),
                "completed family member {which}"
            );
            s.tasks[which].status = Status::Open;
        }
        s.tasks[p].rrule = Some("FREQ=WEEKLY".into());
        assert!(check(&s, &m, WriteMode::Verified, &op, today()).is_err());
        s.tasks[p].rrule = Some("FREQ=MONTHLY".into());
        s.tasks[q].due = Some(IcalDateTime::Date(date(2026, 12, 11)));
        assert!(
            check(&s, &m, WriteMode::Verified, &op, today()).is_err(),
            "foreign child schedule"
        );
    }
    #[test]
    fn legacy_checklist_last_item_and_uncaptured_parent_operations_stay_blocked() {
        let e = env("011d-phone-delete-items");
        let s = e.snap();
        let mut legacy = Matrix::embedded().clone();
        legacy.operation.retain(|r| {
            !matches!(
                r.case.as_str(),
                "checklist-item" | "project-item" | "standalone-resource" | "flat-family"
            )
        });
        let m = &legacy;
        let p = s.find(CHECKLIST_A_UID).unwrap();
        let child = s.children(p)[0];
        for uid in [&s.tasks[child].uid, &s.tasks[p].uid] {
            assert!(
                check(
                    &s,
                    m,
                    WriteMode::Verified,
                    &Op::Delete { uid: uid.clone() },
                    today()
                )
                .is_err()
            );
        }
        let e = env("002-seeded");
        let base = e.snap();
        for rule in [
            "FREQ=WEEKLY",
            "FREQ=MONTHLY;INTERVAL=2",
            "FREQ=YEARLY",
            "FREQ=MONTHLY;COUNT=2",
        ] {
            let mut s = base.clone();
            let i = s.find("twydo-p8-c29-d").unwrap();
            s.tasks[i].rrule = Some(rule.into());
            assert!(
                check(
                    &s,
                    m,
                    WriteMode::Verified,
                    &edit("twydo-p8-c29-d", vec![Change::Kind(TaskKind::Checklist)]),
                    today()
                )
                .is_err(),
                "{rule}"
            );
        }
    }
}

mod individual_item_deletion {
    use super::*;
    use common::item_deletion as evidence;
    use twydo_core::compat::Verdict;
    use twydo_core::ical::value::IcalDateTime;
    use twydo_core::model::{Alarm, Status, Trigger};

    #[test]
    fn delete_uses_item_evidence_for_open_completed_last_and_repeating_items() {
        let matrix = evidence::matrix("pass");
        for (step, targets) in [
            (evidence::SEED, vec!["twydo-i27-c-a-q", "twydo-i27-p-a-q"]),
            (
                evidence::CLOSED,
                vec![
                    "twydo-i27-c-a-x",
                    "twydo-i27-p-a-x",
                    "twydo-i27-m-a-q",
                    "twydo-i27-w-a-q",
                ],
            ),
        ] {
            let e = evidence::env(step);
            let s = e.snap();
            for uid in targets {
                let op = Op::Delete { uid: uid.into() };
                assert_eq!(
                    check(&s, &matrix, WriteMode::Verified, &op, today()),
                    Ok(()),
                    "{uid}"
                );
                assert_eq!(
                    check(&s, Matrix::embedded(), WriteMode::Verified, &op, today()),
                    Ok(()),
                    "shipping matrix: {uid}"
                );
            }
        }
    }

    #[test]
    fn delete_has_its_own_bidirectional_verdict_and_does_not_authorize_other_operations() {
        let e = evidence::env(evidence::SEED);
        let s = e.snap();
        let op = Op::Delete {
            uid: "twydo-i27-c-a-q".into(),
        };
        let baseline = evidence::matrix("unverified");
        assert!(check(&s, &baseline, WriteMode::Verified, &op, today()).is_err());
        assert_eq!(check(&s, &baseline, WriteMode::Test, &op, today()), Ok(()));
        let m = evidence::matrix("pass");
        for other in [
            Op::Delete {
                uid: "twydo-i27-m-a".into(),
            },
            Op::Move {
                uid: "twydo-i27-c-a-q".into(),
                to: "list2".into(),
            },
            complete("twydo-i27-w-a"),
            reopen("twydo-i27-c-a-x"),
        ] {
            assert!(
                check(&s, &m, WriteMode::Verified, &other, today()).is_err(),
                "{other:?}"
            );
        }
        for column in ["baseline", "outbound", "inbound"] {
            let mut m = evidence::matrix("pass");
            let row = &mut m.operation[0];
            match column {
                "baseline" => row.iphone_baseline = Verdict::Unverified,
                "outbound" => row.twydo_to_iphone = Verdict::Unverified,
                _ => row.iphone_to_twydo = Verdict::Unverified,
            }
            assert!(
                check(&s, &m, WriteMode::Verified, &op, today()).is_err(),
                "{column}"
            );
        }
    }

    #[test]
    fn delete_preserves_recognized_schedules_without_execution_or_field_edit_verdicts() {
        let e = evidence::env(evidence::SEED);
        let base = e.snap();
        let matrix = evidence::matrix("pass");
        let q = base.find("twydo-i27-c-a-q").unwrap();
        let parent = base.parent(q).unwrap();
        let sibling = base.find("twydo-i27-c-a-x").unwrap();
        for rule in [
            "FREQ=DAILY",
            "FREQ=WEEKLY;INTERVAL=3",
            "FREQ=WEEKLY;BYDAY=MO,WE,FR",
            "FREQ=MONTHLY;INTERVAL=2",
            "FREQ=MONTHLY;BYMONTHDAY=-1",
            "FREQ=MONTHLY;BYDAY=2TU",
            "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1",
            "FREQ=MONTHLY;BYDAY=SA,SU;BYSETPOS=1",
            "FREQ=YEARLY",
            "FREQ=MONTHLY;COUNT=3",
            "FREQ=MONTHLY;UNTIL=20270210",
        ] {
            let mut s = base.clone();
            s.tasks[parent].rrule = Some(rule.into());
            s.tasks[q].rrule = Some(rule.into());
            s.tasks[sibling].rrule = Some(rule.into());
            s.tasks[parent].from_completion = true;
            s.tasks[parent].status = Status::Cancelled;
            s.tasks[q].status = Status::Completed;
            s.tasks[sibling].status = Status::Cancelled;
            s.tasks[parent].start = Some(IcalDateTime::Date(date(2026, 12, 1)));
            s.tasks[parent].alarms.push(Alarm {
                trigger: Trigger::Absolute("2026-12-10T08:00:00Z".parse().unwrap()),
                index: 0,
            });
            assert_eq!(
                check(
                    &s,
                    &matrix,
                    WriteMode::Verified,
                    &Op::Delete {
                        uid: s.tasks[q].uid.clone()
                    },
                    today()
                ),
                Ok(()),
                "{rule}"
            );
        }
    }
}

mod capability_deletion {
    use super::*;
    use std::fs;

    use chrono::{DateTime, TimeZone, Utc};

    use common::Env;
    use twydo_core::compat::Matrix;
    use twydo_core::config::WriteMode;
    use twydo_core::edit::{Change, Op};
    use twydo_core::gate::{Blocked, check};
    use twydo_core::ical::value::IcalDateTime;
    use twydo_core::model::meta_edit::MetaJson;
    use twydo_core::model::{Alarm, Trigger};
    use twydo_core::views::Snapshot;

    const E: &str = "44123b09407c4ea1adae8f2ecdb64d49";
    const R: &str = "8f603cfaa41f4c229b9adcd6223e7393";
    const R1: &str = "35baad226105494080416dc20825f49c";
    const R2: &str = "72e9476833d14a77a76c64cafeedfa97";
    const R3: &str = "a8e8de2d00a24a8596cdd171fc971d4c";
    const R_COPY: &str = "14b5a37a52224e8c959b5002e2275fc9";
    const R1_COPY: &str = "780885dd967342ec8953c6824f1d674b";
    const R2_COPY: &str = "dd8f672c8feb47ccae449f8488918975";
    const R3_COPY: &str = "7357bbeb391e4c2da619ca0b0b78f919";

    const CHECKLIST_A: &str = "12d762a5639e406c8ec865517882f130";
    const A_Q: &str = "2cb7991c71624ddda29877c098acad43";
    const A_Z: &str = "3199749e2ea64f10bdaebc00197768ca";
    const CHECKLIST_B: &str = "twydo-p8-c29-b";
    const B_Q: &str = "96ee97f3cc8c403391c57ec8b8e5d192";
    const B_Z: &str = "f9ae2ffd776e42919f9696f37dd6e849";
    const D: &str = "twydo-p8-c29-d";

    const CA: &str = "twydo-i27-c-a";
    const CA_Q: &str = "twydo-i27-c-a-q";
    const PA: &str = "twydo-i27-p-a";
    const PLAIN_D: &str = "855a275473c34da9840a6a76c5f18385";

    fn instant() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 4, 15, 0, 0).unwrap()
    }

    fn key(uid: &str) -> String {
        format!("list1/{uid}.ics")
    }

    fn check_uid(s: &Snapshot, m: &Matrix, uid: &str) -> Result<(), Blocked> {
        check(
            s,
            m,
            WriteMode::Verified,
            &Op::Delete { uid: uid.into() },
            today(),
        )
    }

    fn weekly(env: &Env, uid: &str) {
        let path = env.file(uid);
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("RRULE:FREQ=MONTHLY", "RRULE:FREQ=WEEKLY");
        fs::write(path, text).unwrap();
    }

    fn set_meta(s: &mut Snapshot, idx: usize, key: &str, value: &str) {
        let raw = s.tasks[idx]
            .meta
            .raw
            .clone()
            .expect("fixture has JSON metadata");
        let mut meta = MetaJson::parse(&raw).expect("fixture has JSON metadata");
        meta.set_str(key, value);
        s.tasks[idx].meta.raw = Some(meta.encode());
    }

    fn set_meta_i64(s: &mut Snapshot, idx: usize, key: &str, value: i64) {
        let raw = s.tasks[idx]
            .meta
            .raw
            .clone()
            .expect("fixture has JSON metadata");
        let mut meta = MetaJson::parse(&raw).expect("fixture has JSON metadata");
        meta.set_i64(key, value);
        s.tasks[idx].meta.raw = Some(meta.encode());
    }

    fn capability_matrix(verdict: &str) -> Matrix {
        let mut text = String::from("matrix_version = 1\n");
        for case in ["standalone-resource", "flat-family"] {
            text.push_str(&format!(
                "[[operation]]\ncase = \"{case}\"\nop = \"delete\"\n\
                 iphone_baseline = \"pass\"\nmac_to_iphone = \"unverified\"\n\
                 iphone_to_mac = \"unverified\"\ntwydo_to_iphone = \"{verdict}\"\n\
                 iphone_to_twydo = \"{verdict}\"\n"
            ));
        }
        // Rule verdicts are deliberately failing: removal must not execute or edit them.
        text.push_str(
            "[[rule]]\nclass = \"weeks\"\npreserve = \"fail\"\nexecute = \"fail\"\nedit = \"fail\"\n",
        );
        text.push_str(
            "[[rule]]\nclass = \"months-day-1-28\"\npreserve = \"fail\"\nexecute = \"fail\"\nedit = \"fail\"\n",
        );
        Matrix::from_toml_str(&text).unwrap()
    }

    fn baseline_matrix(baseline: &str) -> Matrix {
        let mut text = String::from("matrix_version = 1\n");
        for case in ["standalone-resource", "flat-family"] {
            text.push_str(&format!(
                "[[operation]]\ncase = \"{case}\"\nop = \"delete\"\n\
                 iphone_baseline = \"{baseline}\"\nmac_to_iphone = \"unverified\"\n\
                 iphone_to_mac = \"unverified\"\ntwydo_to_iphone = \"pass\"\n\
                 iphone_to_twydo = \"pass\"\n"
            ));
        }
        Matrix::from_toml_str(&text).unwrap()
    }

    #[test]
    fn capability_delete_removes_only_selected_resource_or_flat_family() {
        // A standalone weekly resource.
        let e = Env::new(WriteMode::Verified, "plan4-list1/02-edit");
        weekly(&e, E);
        let s = e.snap();
        assert_eq!(
            check_uid(&s, Matrix::embedded(), E),
            Ok(()),
            "standalone weekly"
        );
        let before = e.files();
        let mut store = e.store(Matrix::embedded().clone());
        store
            .apply(&s, &Op::Delete { uid: E.into() }, instant())
            .unwrap();
        let after = e.files();
        assert!(!after.contains_key(&key(E)));
        for (path, bytes) in before {
            if path != key(E) {
                assert_eq!(after[&path], bytes, "{path}");
            }
        }

        // An empty weekly project.
        let e = Env::new(WriteMode::Verified, "plan6-list1/05-complete-parent");
        for child in [R1, R2, R3] {
            fs::remove_file(e.file(child)).unwrap();
        }
        weekly(&e, R);
        let s = e.snap();
        assert_eq!(
            check_uid(&s, Matrix::embedded(), R),
            Ok(()),
            "empty weekly project"
        );
        let before = e.files();
        let mut store = e.store(Matrix::embedded().clone());
        store
            .apply(&s, &Op::Delete { uid: R.into() }, instant())
            .unwrap();
        let after = e.files();
        assert!(!after.contains_key(&key(R)));
        for (path, bytes) in before {
            if path != key(R) {
                assert_eq!(after[&path], bytes, "{path}");
            }
        }

        // A nonempty recurring checklist: only A, q and z disappear.
        let e = Env::new(
            WriteMode::Verified,
            "plan8-checklist-b/011-phone-items-and-text",
        );
        let s = e.snap();
        assert_eq!(
            check_uid(&s, Matrix::embedded(), CHECKLIST_A),
            Ok(()),
            "nonempty recurring checklist"
        );
        let before = e.files();
        let mut store = e.store(Matrix::embedded().clone());
        store
            .apply(
                &s,
                &Op::Delete {
                    uid: CHECKLIST_A.into(),
                },
                instant(),
            )
            .unwrap();
        let after = e.files();
        for uid in [CHECKLIST_A, A_Q, A_Z] {
            assert!(!after.contains_key(&key(uid)), "{uid}");
        }
        for uid in [CHECKLIST_B, B_Q, B_Z, D] {
            assert_eq!(after[&key(uid)], before[&key(uid)], "{uid}");
        }

        // The same checklist with no items.
        let e = Env::new(
            WriteMode::Verified,
            "plan8-checklist-b/011-phone-items-and-text",
        );
        for child in [A_Q, A_Z] {
            fs::remove_file(e.file(child)).unwrap();
        }
        let s = e.snap();
        assert_eq!(
            check_uid(&s, Matrix::embedded(), CHECKLIST_A),
            Ok(()),
            "empty recurring checklist"
        );

        // Completed flat families: a project and a checklist.
        let e = Env::new(
            WriteMode::Verified,
            "issue27-items/020-phone-complete-project",
        );
        let s = e.snap();
        for uid in [PA, CA] {
            assert_eq!(
                check_uid(&s, Matrix::embedded(), uid),
                Ok(()),
                "completed {uid}"
            );
        }
    }

    #[test]
    fn capability_delete_preserves_series_history_and_other_resources() {
        let e = Env::new(WriteMode::Verified, "plan6-list1/05-complete-parent");
        let s = e.snap();
        let mut payload = s.clone();
        let r = payload.find(R).unwrap();
        payload.tasks[r].rrule = Some("FREQ=MONTHLY;COUNT=3".into());
        payload.tasks[r].from_completion = true;
        payload.tasks[r].start = Some(IcalDateTime::Date(date(2026, 12, 1)));
        payload.tasks[r].alarms.push(Alarm {
            trigger: Trigger::Relative {
                offset: chrono::Duration::minutes(-30),
                related_end: false,
            },
            index: 0,
        });
        payload.tasks[r].alarms.push(Alarm {
            trigger: Trigger::Absolute("2026-12-10T08:00:00Z".parse().unwrap()),
            index: 1,
        });
        assert_eq!(
            check_uid(&payload, Matrix::embedded(), R),
            Ok(()),
            "recognized date, start, alarm, count and completion metadata"
        );

        let before = e.files();
        let mut store = e.store(Matrix::embedded().clone());
        store
            .apply(&s, &Op::Delete { uid: R.into() }, instant())
            .unwrap();
        let after = e.files();
        for uid in [R, R1, R2, R3] {
            assert!(!after.contains_key(&key(uid)), "{uid}");
        }
        for uid in [R_COPY, R1_COPY, R2_COPY, R3_COPY] {
            assert_eq!(after[&key(uid)], before[&key(uid)], "history {uid}");
        }
        for (path, bytes) in &before {
            if !path.starts_with("list1/") || ![R, R1, R2, R3].iter().any(|uid| path == &key(uid)) {
                assert_eq!(after[path], *bytes, "outside family {path}");
            }
        }
        store.undo_last(instant()).unwrap();
        assert_eq!(e.files(), before);
    }

    #[test]
    fn capability_delete_rejects_unsafe_membership() {
        let m = Matrix::embedded();
        let s = snap();
        for uid in [
            "gate-broken-child",
            "gate-nested",
            "gate-plain-parent",
            "gate-dup",
        ] {
            assert!(s.find(uid).is_some(), "missing negative fixture {uid}");
            assert!(check_uid(&s, m, uid).is_err(), "{uid}");
        }

        let e = Env::new(WriteMode::Verified, "issue27-items/001-seeded");
        let base = e.snap();
        let parent = base.find(CA).unwrap();
        let child = base.find(CA_Q).unwrap();

        let mut s = base.clone();
        s.tasks.push(s.tasks[parent].clone());
        assert!(check_uid(&s, m, CA).is_err(), "duplicate target UID");

        let mut s = base.clone();
        s.tasks.push(s.tasks[child].clone());
        assert!(check_uid(&s, m, CA).is_err(), "duplicate child UID");

        let mut s = base.clone();
        s.tasks[child].blocked = Some("multiple parent links are not supported".into());
        assert!(check_uid(&s, m, CA).is_err(), "blocked child link");

        let mut s = base.clone();
        s.tasks[parent].parent = Some("unresolved-parent".into());
        assert!(check_uid(&s, m, CA).is_err(), "broken parent");

        let mut s = base.clone();
        s.tasks[child].collection = "list2".into();
        assert!(check_uid(&s, m, CA).is_err(), "cross-list child");

        let mut s = base.clone();
        s.tasks[child].meta.unreadable = true;
        assert!(check_uid(&s, m, CA).is_err(), "unreadable child metadata");

        let mut s = base.clone();
        s.tasks[child].rrule = Some("FREQ=HOURLY".into());
        assert!(check_uid(&s, m, CA).is_err(), "unrecognized child rule");

        let mut s = base.clone();
        set_meta_i64(&mut s, child, "TaskType", 9);
        assert!(check_uid(&s, m, CA).is_err(), "unknown child type");

        let e = Env::new(
            WriteMode::Verified,
            "plan8-checklist-b/011-phone-items-and-text",
        );
        let mut s = e.snap();
        let b_child = s.find(B_Q).unwrap();
        set_meta(&mut s, b_child, "parentuid", "disagreeing-parent");
        assert!(
            check_uid(&s, m, CHECKLIST_B).is_err(),
            "disagreeing JSON link"
        );
        let mut s = e.snap();
        let b = s.find(CHECKLIST_B).unwrap();
        set_meta(&mut s, b, "parentuid", "synthetic-other-parent");
        assert!(
            check_uid(&s, m, CHECKLIST_B).is_err(),
            "JSON-only parent link"
        );
        assert!(s.find(D).is_some(), "fixture sanity");

        // Raw duplicate identities and multiple parent links are refused before use.
        for (target, extra) in [
            (CA, "UID:another-synthetic-identity"),
            (CA_Q, "RELATED-TO:twydo-i27-p-a"),
        ] {
            let e = Env::new(WriteMode::Verified, "issue27-items/001-seeded");
            let path = e.file(target);
            let raw = fs::read_to_string(&path)
                .unwrap()
                .replace("END:VTODO", &format!("{extra}\r\nEND:VTODO"));
            fs::write(path, raw).unwrap();
            let s = e.snap();
            assert!(s.find(CA).is_some(), "unsupported resource stays readable");
            assert!(check_uid(&s, m, CA).is_err(), "{extra}");
        }

        // Raw malformed TaskType values are refused, not treated as an absent type.
        let e = Env::new(WriteMode::Verified, "plan4-list1/02-edit");
        let path = e.file(PLAIN_D);
        let original = fs::read_to_string(&path).unwrap();
        assert!(original.contains("%22TaskType%22%3A0"));
        for value in [
            "%22TaskType%22%3A9",
            "%22TaskType%22%3A%229%22",
            "%22TaskType%22%3Atrue",
            "%22TaskType%22%3A1.5",
            "%22TaskType%22%3A%22checklist%22",
        ] {
            fs::write(&path, original.replace("%22TaskType%22%3A0", value)).unwrap();
            let s = e.snap();
            assert!(
                s.find(PLAIN_D).is_some(),
                "malformed resource stays readable"
            );
            assert!(check_uid(&s, m, PLAIN_D).is_err(), "{value}");
        }
        fs::write(&path, original).unwrap();
    }

    #[test]
    fn capability_delete_requires_its_own_evidence() {
        let e = Env::new(WriteMode::Verified, "plan4-list1/02-edit");
        weekly(&e, E);
        let s = e.snap();

        let empty = Matrix::from_toml_str("matrix_version = 1\n").unwrap();
        assert!(
            check_uid(&s, &empty, E).is_err(),
            "missing row keeps legacy refusal"
        );

        let m = capability_matrix("pass");
        assert_eq!(check_uid(&s, &m, E), Ok(()), "own row admits standalone");

        let unverified = capability_matrix("unverified");
        assert!(
            check_uid(&s, &unverified, E).is_err(),
            "unverified row refuses"
        );
        assert_eq!(
            check(
                &s,
                &unverified,
                WriteMode::Test,
                &Op::Delete { uid: E.into() },
                today()
            ),
            Ok(()),
            "test mode still needs the baseline"
        );

        let unsupported = baseline_matrix("unsupported");
        assert!(
            check_uid(&s, &unsupported, E).is_err(),
            "unsupported baseline refuses"
        );

        // Rule execution/edit verdicts neither authorize nor block resource removal.
        let e = Env::new(
            WriteMode::Verified,
            "plan8-checklist-b/011-phone-items-and-text",
        );
        assert_eq!(
            check_uid(&e.snap(), &m, CHECKLIST_A),
            Ok(()),
            "family removal ignores failing rule verdicts"
        );

        // The matching case's row is required; a different new row does not dispatch.
        let mut only_standalone = String::from("matrix_version = 1\n");
        only_standalone.push_str(
            "[[operation]]\ncase = \"standalone-resource\"\nop = \"delete\"\n\
             iphone_baseline = \"pass\"\nmac_to_iphone = \"unverified\"\n\
             iphone_to_mac = \"unverified\"\ntwydo_to_iphone = \"pass\"\n\
             iphone_to_twydo = \"pass\"\n",
        );
        let only_standalone = Matrix::from_toml_str(&only_standalone).unwrap();
        assert!(
            check_uid(&e.snap(), &only_standalone, CHECKLIST_A).is_err(),
            "flat family needs its own row"
        );

        let mut only_family = String::from("matrix_version = 1\n");
        only_family.push_str(
            "[[operation]]\ncase = \"flat-family\"\nop = \"delete\"\n\
             iphone_baseline = \"pass\"\nmac_to_iphone = \"unverified\"\n\
             iphone_to_mac = \"unverified\"\ntwydo_to_iphone = \"pass\"\n\
             iphone_to_twydo = \"pass\"\n",
        );
        let only_family = Matrix::from_toml_str(&only_family).unwrap();
        assert!(
            check_uid(&s, &only_family, E).is_err(),
            "standalone needs its own row"
        );

        // No other operation is admitted by these rows.
        let e = Env::new(WriteMode::Verified, "plan6-list1/05-complete-parent");
        let s = e.snap();
        for op in [
            Op::Move {
                uid: R.into(),
                to: "list2".into(),
            },
            Op::Complete { uid: R.into() },
            Op::Edit {
                uid: R.into(),
                changes: vec![Change::Summary("changed".into())],
            },
        ] {
            assert!(
                check(&s, &m, WriteMode::Verified, &op, today()).is_err(),
                "{op:?}"
            );
        }
    }
}

#[test]
fn individual_delete_refuses_ambiguous_malformed_cross_list_and_nested_families() {
    let e = common::item_deletion::env(common::item_deletion::SEED);
    let base = e.snap();
    let m = common::item_deletion::matrix("pass");
    let q = base.find("twydo-i27-c-a-q").unwrap();
    let parent = base.parent(q).unwrap();
    let sibling = base.find("twydo-i27-c-a-x").unwrap();
    let op = Op::Delete {
        uid: base.tasks[q].uid.clone(),
    };
    for member in [q, parent, sibling] {
        let mut s = base.clone();
        s.tasks[member].blocked = Some("synthetic unsupported resource".into());
        assert!(check(&s, &m, WriteMode::Verified, &op, today()).is_err());
        let mut s = base.clone();
        s.tasks[member].meta.unreadable = true;
        assert!(check(&s, &m, WriteMode::Verified, &op, today()).is_err());
        let mut s = base.clone();
        s.tasks[member].rrule = Some("FREQ=HOURLY".into());
        assert!(check(&s, &m, WriteMode::Verified, &op, today()).is_err());
        let mut s = base.clone();
        s.tasks.push(s.tasks[member].clone());
        assert!(check(&s, &m, WriteMode::Verified, &op, today()).is_err());
    }
    let mut s = base.clone();
    s.tasks[sibling].collection = "list2".into();
    assert!(check(&s, &m, WriteMode::Verified, &op, today()).is_err());
    let mut s = base.clone();
    s.tasks[parent].parent = Some("unresolved-parent".into());
    assert!(check(&s, &m, WriteMode::Verified, &op, today()).is_err());
    let s = snap();
    for uid in [
        "gate-broken-child",
        "gate-nested-child",
        "gate-nested-grandchild",
        "gate-plain-child",
        "gate-dup",
    ] {
        assert!(s.find(uid).is_some(), "missing negative fixture {uid}");
        assert!(
            check(
                &s,
                &m,
                WriteMode::Verified,
                &Op::Delete { uid: uid.into() },
                today()
            )
            .is_err(),
            "{uid}"
        );
    }
}

#[test]
fn history_task_capability_preserves_schedule_and_series() {
    let mut s = snap();
    let i = idx(&s, "gate-history");
    s.tasks[i].start = Some(twydo_core::ical::value::IcalDateTime::Date(today()));
    let m = matrix(&[("history-task", "reopen")], "pass", "pass", &[]);
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Verified,
            &reopen("gate-history"),
            today()
        ),
        Ok(())
    );
}

#[test]
fn history_task_capability_refuses_live_cancelled_and_ended_series() {
    let m = matrix(&[("history-task", "reopen")], "pass", "pass", &[]);
    for uid in ["gate-done", "gate-monthly", "gate-cancelled-checklist-item"] {
        assert!(check(&snap(), &m, WriteMode::Verified, &reopen(uid), today()).is_err());
    }
    let mut s = snap();
    let i = idx(&s, "gate-history");
    s.tasks[i].rrule = Some("FREQ=MONTHLY;COUNT=1".into());
    assert!(
        check(
            &s,
            &m,
            WriteMode::Verified,
            &reopen("gate-history"),
            today()
        )
        .is_err()
    );
}

#[test]
fn history_task_capability_refuses_ambiguous_recurrence_metadata() {
    for field in [
        "RecurrenceType",
        "RecurrenceFrom",
        "RecurrenceValue",
        "RecurrenceEndType",
        "RecurrenceEndRepetitions",
        "RecurrenceEndRepetitionsOrig",
    ] {
        let mut s = snap();
        let i = idx(&s, "gate-history");
        let mut meta = MetaJson::parse(s.tasks[i].meta.raw.as_ref().unwrap()).unwrap();
        meta.set_str(field, "synthetic-malformed");
        s.tasks[i].meta.raw = Some(meta.encode());
        assert!(
            check(
                &s,
                Matrix::embedded(),
                WriteMode::Verified,
                &reopen("gate-history"),
                today()
            )
            .is_err(),
            "{field}"
        );
    }
}

#[test]
fn live_parent_with_foreign_series_identity_does_not_use_history_item_policy() {
    let env = common::Env::new(WriteMode::Verified, "plan6-list1/06-reopen-parent");
    let mut s = env.snap();
    let parent = s.find("14b5a37a52224e8c959b5002e2275fc9").unwrap();
    let child = "780885dd967342ec8953c6824f1d674b";
    let i = s.find(child).unwrap();
    s.tasks[parent].rrule = Some("FREQ=MONTHLY".into());
    s.tasks[i].status = twydo_core::model::Status::Completed;
    let m = common::matrix(
        &[
            "history-flat-family",
            "one-time",
            "recurring-project",
            "date-only",
        ],
        "pass",
        &[],
    );
    assert_eq!(
        check(&s, &m, WriteMode::Verified, &reopen(child), today()),
        Ok(())
    );
}
