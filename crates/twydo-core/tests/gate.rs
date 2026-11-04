//! The write gate: matrix-driven policy with exact reasons. All content is synthetic
//! (`tests/data/gate/`); matrices are built from TOML strings here.

use std::path::Path;

use chrono::NaiveDate;

use twydo_core::compat::Matrix;
use twydo_core::config::{Config, WriteMode};
use twydo_core::edit::{Change, Due, NewTask, Op};
use twydo_core::gate::{Blocked, Cases, check, task_cases};
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
        blocked("repeating checklists are not verified yet")
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
    // A new checklist cannot repeat.
    let op = Op::Create(NewTask {
        kind: twydo_core::model::TaskKind::Checklist,
        due: Some(Due::Date(date(2026, 11, 9))),
        repeat: Some(repeat("monthly")),
        ..NewTask::plain("inbox", "Synthetic checklist")
    });
    assert_eq!(
        check(&s, &m, WriteMode::Test, &op, today()),
        blocked("repeating checklists are not verified yet")
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
    // A repeating task cannot become a checklist; an item cannot become a parent.
    let m = rows(&[("task-to-checklist", "convert")], "pass");
    assert_eq!(
        check(
            &s,
            &m,
            WriteMode::Test,
            &kind("gate-monthly", Checklist),
            today()
        ),
        blocked("repeating checklists are not verified yet")
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
            "gate-monthly-checklist",
            "repeating checklists are not verified yet",
        ),
        (
            "gate-monthly-checklist-item",
            "repeating checklists are not verified yet",
        ),
        (
            "gate-checklist-repeating-item",
            "repeating checklists are not verified yet",
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
        blocked("repeating checklists are not verified yet")
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

/// What the phone never checked stays refused: setting or reshaping yearly, daily and
/// from-completion repeats, and repeats on a project.
#[test]
fn embedded_verified_refuses_the_repeat_writes_the_phone_never_checked() {
    let s = snap();
    let m = Matrix::embedded();
    let due = Change::Due(Some(Due::Date(date(2026, 11, 10))));
    let at = |op: &Op| check(&s, m, WriteMode::Verified, op, today());
    // An existing weekly task made yearly: the yearly `repeat` row has no Twydo evidence.
    let op = edit("gate-weekly", vec![Change::Repeat(Some(repeat("yearly")))]);
    assert_eq!(
        at(&op),
        blocked("changing the repeat rule of yearly tasks is not verified yet (twydo_to_iphone)")
    );
    // Yearly, daily and from completion.
    let op = edit("gate-date", vec![Change::Repeat(Some(repeat("yearly")))]);
    assert_eq!(
        at(&op),
        blocked("changing the repeat rule of yearly tasks is not verified yet (twydo_to_iphone)")
    );
    let op = edit("gate-date", vec![Change::Repeat(Some(repeat("daily")))]);
    assert_eq!(
        at(&op),
        blocked("changing the repeat rule of daily tasks is not verified yet (twydo_to_iphone)")
    );
    let op = edit("gate-monthly", vec![Change::Repeat(Some(repeat("yearly")))]);
    assert_eq!(
        at(&op),
        blocked("changing the repeat rule of yearly tasks is not verified yet (twydo_to_iphone)")
    );
    let op = edit(
        "gate-monthly",
        vec![Change::Repeat(Some(repeat("monthly after done")))],
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
    // A rule that is merely unverified keeps the old wording.
    let op = edit("gate-date", vec![Change::Repeat(Some(repeat("daily")))]);
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

/// A vdir with the session C seed (`plan7-twydo-list2/00-seed`, list `c`) and session B's
/// first round (`plan7-twydo-list1/01-twydo-create`, list `b`), as Twydo wrote them.
fn session_snap() -> (tempfile::TempDir, Snapshot) {
    let tmp = tempfile::tempdir().unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase0/v1");
    for (list, step) in [
        ("c", "plan7-twydo-list2/00-seed"),
        ("b", "plan7-twydo-list1/01-twydo-create"),
    ] {
        let dir = tmp.path().join(list);
        std::fs::create_dir_all(&dir).unwrap();
        for e in std::fs::read_dir(fixtures.join(step)).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "ics") {
                std::fs::copy(&p, dir.join(p.file_name().unwrap())).unwrap();
            }
        }
    }
    let report = twydo_core::vdir::load(tmp.path()).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let mut cfg = Config::with_vdir(tmp.path().to_path_buf());
    cfg.timezone = Some("America/New_York".to_string());
    (tmp, Snapshot::new(report, cfg))
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
    // Until: only the ending case (UN) was captured, not a series that goes on, so the
    // modifier's `execute` stays unverified.
    let uid = by_title(&s, "c", "UNb");
    assert_eq!(
        at(&complete(&uid)),
        blocked("months-day-1-28+until repeats are not verified yet")
    );
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
    // Reshaping to yearly or daily, or to from completion: no Twydo evidence.
    for (text, case) in [("yearly", "yearly"), ("daily", "daily")] {
        let op = edit(&e("E3"), vec![Change::Repeat(Some(repeat(text)))]);
        assert_eq!(
            at(&op),
            blocked(&format!(
                "changing the repeat rule of {case} tasks is not verified yet (twydo_to_iphone)"
            )),
            "{text}"
        );
    }
    let op = edit(
        &e("E3"),
        vec![Change::Repeat(Some(repeat("monthly after done")))],
    );
    assert!(at(&op).is_err());
    // Reopening a history copy of a daily or weekly repeat rests on the monthly/yearly rows;
    // reopening a daily or weekly task itself has no Twydo evidence.
    let op = reopen(&e("E1"));
    assert!(at(&op).is_err());
}
