//! Admission of custom alert edits (`set-alert.task`) on the alerts-round tasks.

mod common;

use std::fs;

use chrono::{DateTime, NaiveDate, Utc};
use twydo_core::compat::{ContractId, Matrix, Verdict};
use twydo_core::config::WriteMode;
use twydo_core::edit::{Change, Op};
use twydo_core::gate::check;
use twydo_core::model::AlertOffset;

const BATCH1: &str = "set-alert/phone-encodings/phone-a-batch1/list1";
const A1: &str = "ae49d9b7655647d5aa1788c1bff3ac8f";
const A3: &str = "92c11511b21b49288ff0ce7ac11c0313";
const A4: &str = "b1e7bcf4b25c46f3b91c4aff8e812e63";
const A8: &str = "42200c41e91e4e289db9aed8e481d356";

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 15).unwrap()
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-15T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn passing() -> Matrix {
    let mut m = Matrix::embedded().clone();
    common::set_contract(&mut m, ContractId::SetAlertTask, Verdict::Pass);
    m
}

fn alert(uid: &str, minutes: Option<u32>) -> Op {
    Op::Edit {
        uid: uid.to_string(),
        changes: vec![Change::Alert(minutes.map(|n| AlertOffset::new(n).unwrap()))],
    }
}

fn reason(r: Result<(), twydo_core::gate::Blocked>) -> String {
    r.expect_err("refused").to_string()
}

fn edit_file(env: &common::Env, uid: &str, f: impl Fn(String) -> String) {
    let p = env.file(uid);
    fs::write(&p, f(fs::read_to_string(&p).unwrap())).unwrap();
}

#[test]
fn set_alert_admits_timed_date_only_and_started_tasks() {
    let env = common::Env::new(WriteMode::Verified, BATCH1);
    let s = env.snap();
    for uid in [A1, A3, A4, A8] {
        for minutes in [Some(45), None] {
            check(
                &s,
                &passing(),
                WriteMode::Verified,
                &alert(uid, minutes),
                today(),
            )
            .unwrap_or_else(|e| panic!("{uid} {minutes:?}: {e}"));
        }
    }
}

#[test]
fn set_alert_needs_the_set_alert_contract() {
    let env = common::Env::new(WriteMode::Verified, BATCH1);
    let s = env.snap();
    let mut m = Matrix::embedded().clone();
    common::set_contract(&mut m, ContractId::SetAlertTask, Verdict::Unverified);
    let why = reason(check(
        &s,
        &m,
        WriteMode::Verified,
        &alert(A1, Some(30)),
        today(),
    ));
    assert!(why.contains("setting an alert"), "{why}");
}

#[test]
fn set_alert_admits_repeating_tasks() {
    let env = common::Env::new(
        WriteMode::Verified,
        "set-alert/phone-encodings/phone-a-batch3-open/list1",
    );
    let s = env.snap();
    let a7 = "7c316d0aae534f26a202befff6a94643";
    check(
        &s,
        &passing(),
        WriteMode::Verified,
        &alert(a7, Some(60)),
        today(),
    )
    .unwrap();
}

#[test]
fn set_alert_refuses_unsupported_alerts() {
    let env = common::Env::new(WriteMode::Verified, BATCH1);
    let original = fs::read(env.file(A1)).unwrap();
    for (name, f) in [
        (
            "absolute",
            Box::new(|t: String| {
                t.replace("TRIGGER:-PT15M", "TRIGGER;VALUE=DATE-TIME:20261120T080000Z")
            }) as Box<dyn Fn(String) -> String>,
        ),
        (
            "after due",
            Box::new(|t: String| t.replace("TRIGGER:-PT15M", "TRIGGER:PT15M")),
        ),
        (
            "end-related",
            Box::new(|t: String| t.replace("TRIGGER:-PT15M", "TRIGGER;RELATED=END:-PT15M")),
        ),
        (
            "two alerts",
            Box::new(|t: String| {
                let i = t.find("BEGIN:VALARM").unwrap();
                let e = t.find("END:VALARM").unwrap();
                let j = e + t[e..].find('\n').unwrap() + 1;
                let block = t[i..j].to_string();
                format!("{}{}{}", &t[..j], block, &t[j..])
            }),
        ),
    ] {
        fs::write(env.file(A1), &original).unwrap();
        edit_file(&env, A1, &f);
        let s = env.snap();
        let why = reason(check(
            &s,
            &passing(),
            WriteMode::Verified,
            &alert(A1, Some(30)),
            today(),
        ));
        assert_eq!(why, "this task has alerts Twydo can't edit", "{name}");
    }
}

#[test]
fn set_alert_refuses_without_due() {
    let env = common::Env::new(
        WriteMode::Verified,
        "set-alert/phone-encodings/phone-a-batch2/list1",
    );
    let s = env.snap();
    let why = reason(check(
        &s,
        &passing(),
        WriteMode::Verified,
        &alert(A3, Some(15)),
        today(),
    ));
    assert_eq!(why, "an alert needs a due date");
    // Removing the due date in the same save leaves nothing to count from.
    let env = common::Env::new(WriteMode::Verified, BATCH1);
    let s = env.snap();
    let op = Op::Edit {
        uid: A1.to_string(),
        changes: vec![Change::Due(None), Change::Alert(AlertOffset::new(15))],
    };
    assert_eq!(
        reason(check(&s, &passing(), WriteMode::Verified, &op, today())),
        "an alert needs a due date"
    );
}

#[test]
fn set_alert_refuses_checklist_items() {
    let env = common::Env::new(
        WriteMode::Verified,
        "complete/checklist-reset/phone-reset-item-edits/list1",
    );
    let s = env.snap();
    let item = s
        .tasks
        .iter()
        .enumerate()
        .find(|(i, _)| {
            s.parent(*i)
                .is_some_and(|p| s.tasks[p].kind == twydo_core::model::TaskKind::Checklist)
        })
        .map(|(_, t)| t.uid.clone())
        .expect("a checklist item");
    let why = reason(check(
        &s,
        &passing(),
        WriteMode::Verified,
        &alert(&item, Some(15)),
        today(),
    ));
    assert_eq!(why, "checklist items have no alerts");
}

#[test]
fn undo_set_alert_restores_bytes() {
    let env = common::Env::new(WriteMode::Verified, BATCH1);
    let mut store = env.store(passing());
    let before = env.files();
    let s = env.snap();
    store.apply(&s, &alert(A1, Some(90)), now()).unwrap();
    let after = env.files();
    assert_ne!(after, before);
    assert!(
        String::from_utf8_lossy(&after[&format!("list1/{A1}.ics")]).contains("TRIGGER:-PT1H30M")
    );
    let later = now() + chrono::Duration::minutes(1);
    store.undo_last(later).unwrap();
    assert_eq!(env.files(), before);
    store.redo_last(later).unwrap();
    assert_eq!(env.files(), after);
}

/// Review finding 1: from-completion date edits were captured with the automatic alert only, so
/// one save may not change the dates and add a custom alert together.
#[test]
fn set_alert_with_a_from_completion_date_change_is_refused() {
    let env = common::Env::new(
        WriteMode::Verified,
        "set-due/from-completion/seeded-after-upload/list1",
    );
    let uid = env
        .snap()
        .tasks
        .iter()
        .find(|t| t.from_completion && t.due.is_some() && t.parent.is_none())
        .map(|t| t.uid.clone())
        .expect("a from-completion task");
    // Give it 2Do's automatic alert, the captured shape from-completion edits accept.
    edit_file(&env, &uid, |t| {
        t.replace(
            "END:VTODO",
            "BEGIN:VALARM\nTRIGGER:PT0S\nACTION:AUDIO\nDESCRIPTION:x\nX-2DOAPP-SOUND;SHARE-SCOPE=GLOBAL:Basso\nX-2DOAPP-ISAUTO;SHARE-SCOPE=GLOBAL:1\nX-2DOAPP-RELDUETIME;SHARE-SCOPE=GLOBAL:1\nEND:VALARM\nEND:VTODO",
        )
    });
    let s = env.snap();
    let t = &s.tasks[s.find(&uid).unwrap()];
    assert_eq!(t.custom_alert, twydo_core::model::CustomAlert::AtDue);
    let due = t.due.as_ref().unwrap().local_date(s.tz());
    let op = Op::Edit {
        uid: t.uid.clone(),
        changes: vec![
            Change::Due(Some(twydo_core::edit::Due::Date(
                due + chrono::Duration::days(1),
            ))),
            Change::Alert(AlertOffset::new(15)),
        ],
    };
    let why = reason(check(&s, &passing(), WriteMode::Verified, &op, today()));
    assert_eq!(
        why,
        "set the alert of a task that repeats from completion in a save of its own"
    );
}

/// Review finding 3: an alert-only edit still needs a resource Twydo can write.
#[test]
fn set_alert_refuses_unwritable_resources() {
    let env = common::Env::new(WriteMode::Verified, BATCH1);
    let original = fs::read_to_string(env.file(A1)).unwrap();
    for (name, text) in [
        (
            "recurrence override",
            original.replace(
                "END:VCALENDAR",
                &format!("BEGIN:VTODO\nUID:{A1}\nRECURRENCE-ID:20261120T100000\nSUMMARY:x\nEND:VTODO\nEND:VCALENDAR"),
            ),
        ),
        (
            "unreadable metadata",
            original.replace("<2Do \n Meta>%7B", "<2Do \n Meta>%7Bbroken"),
        ),
    ] {
        fs::write(env.file(A1), &text).unwrap();
        let report = twydo_core::vdir::load(&env.vdir).unwrap();
        let s = twydo_core::views::Snapshot::new(report, env.cfg.clone());
        let why = reason(check(&s, &passing(), WriteMode::Verified, &alert(A1, Some(30)), today()));
        let want = if name == "recurrence override" {
            "recurrence overrides are not supported"
        } else {
            "alert editing needs a recognized task with one readable 2Do resource"
        };
        assert_eq!(why, want, "{name}");
    }
}

/// Review finding 4: a checklist's own alert was never captured.
#[test]
fn set_alert_refuses_checklists() {
    let env = common::Env::new(
        WriteMode::Verified,
        "complete/checklist-reset/phone-reset-item-edits/list1",
    );
    let s = env.snap();
    let head = s
        .tasks
        .iter()
        .find(|t| {
            t.kind == twydo_core::model::TaskKind::Checklist
                && t.status == twydo_core::model::Status::Open
        })
        .map(|t| t.uid.clone())
        .expect("an open checklist");
    let why = reason(check(
        &s,
        &passing(),
        WriteMode::Verified,
        &alert(&head, Some(15)),
        today(),
    ));
    assert_eq!(why, "an alert on a checklist is not verified yet");
}
