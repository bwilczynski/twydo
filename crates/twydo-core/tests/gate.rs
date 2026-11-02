//! The write gate: matrix-driven policy with exact reasons. All content is synthetic
//! (`tests/data/gate/`); matrices are built from TOML strings here.

use std::path::Path;

use chrono::NaiveDate;

use twydo_core::compat::Matrix;
use twydo_core::config::{Config, WriteMode};
use twydo_core::edit::{Change, Due, NewTask, Op};
use twydo_core::gate::{Blocked, Cases, check, task_cases};
use twydo_core::views::Snapshot;

const ALL_OPS: [&str; 6] = ["create", "edit", "complete", "reopen", "move", "delete"];

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

#[test]
fn create_checks_list_and_attribute_cases() {
    let s = snap();
    let m = matrix(&[("one-time", "create")], "pass", "pass", &[]);
    let new = |collection: &str, due: Option<Due>| {
        Op::Create(NewTask {
            collection: collection.to_string(),
            summary: "Synthetic new".into(),
            description: None,
            due,
            start: None,
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
fn from_completion_blocked() {
    let s = snap();
    assert_eq!(
        check(
            &s,
            &full(),
            WriteMode::Test,
            &complete("gate-from-completion"),
            today()
        ),
        blocked("repeat from completion is not verified yet")
    );
}

#[test]
fn weekly_blocked() {
    let s = snap();
    assert_eq!(
        check(
            &s,
            &full(),
            WriteMode::Test,
            &complete("gate-weekly"),
            today()
        ),
        blocked("weeks repeats are not verified yet")
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
