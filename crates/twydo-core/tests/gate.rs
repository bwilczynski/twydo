//! The write gate: matrix-driven policy with exact reasons. All content is synthetic
//! (`tests/data/gate/`); matrices are built from TOML strings here.

use std::path::Path;

use chrono::NaiveDate;

use twydo_core::compat::Matrix;
use twydo_core::config::{Config, WriteMode};
use twydo_core::edit::{Change, Due, NewTask, Op};
use twydo_core::gate::{Blocked, check, task_case};
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

#[test]
fn project_and_checklist_blocked() {
    let s = snap();
    let m = full();
    let reason = "projects and checklists cannot be edited yet";
    for uid in [
        "gate-project",
        "gate-project-child",
        "gate-checklist",
        "gate-checklist-item",
        "gate-monthly-project",
    ] {
        assert_eq!(
            check(&s, &m, WriteMode::Test, &complete(uid), today()),
            blocked(reason),
            "{uid}"
        );
        assert_eq!(task_case(&s, idx(&s, uid)), Err(Blocked(reason.into())));
    }
    assert_eq!(task_case(&s, idx(&s, "gate-date")), Ok("one-time"));
    assert_eq!(task_case(&s, idx(&s, "gate-monthly")), Ok("monthly"));
    assert_eq!(task_case(&s, idx(&s, "gate-yearly")), Ok("yearly"));
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

#[test]
fn late_recurring_completion_blocked() {
    let s = snap();
    assert_eq!(
        check(
            &s,
            &full(),
            WriteMode::Test,
            &complete("gate-monthly-late"),
            today()
        ),
        blocked(
            "completing a repeating task after a missed occurrence is not verified yet \
             (monthly-15th-three-months-late)"
        )
    );
}

#[test]
fn recurring_with_start_or_alarm_blocked() {
    let s = snap();
    let m = full();
    let reason = "completing a repeating task with a start date is not verified yet (start-due-alarm-offsets)";
    for uid in ["gate-monthly-start", "gate-monthly-alarm"] {
        assert_eq!(
            check(&s, &m, WriteMode::Test, &complete(uid), today()),
            blocked(reason),
            "{uid}"
        );
    }
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

/// Ruling R3: only a row that stays unverified (projects) is asserted on the real matrix.
#[test]
fn embedded_matrix_blocks_projects() {
    let s = snap();
    assert_eq!(
        check(
            &s,
            Matrix::embedded(),
            WriteMode::Verified,
            &complete("gate-project"),
            today()
        ),
        blocked("projects and checklists cannot be edited yet")
    );
}

#[test]
fn blocked_displays_reason() {
    assert_eq!(Blocked("duplicate UID".into()).to_string(), "duplicate UID");
}
