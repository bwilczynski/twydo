//! The pure planner: gate and plan on a loaded snapshot, with no store and no state directory.

mod common;

use chrono::{DateTime, TimeZone, Utc};

use twydo_core::compat::Matrix;
use twydo_core::config::WriteMode;
use twydo_core::edit::{Change, Op};
use twydo_core::gate;
use twydo_core::store::WriteError;
use twydo_core::store::plan::{Policy, plan};

use common::Env;

const FIXTURE: &str = "complete/standalone-lifecycle/edit/list1";
const A: &str = "0a04f22aeb294b719633c978fe1975e5";
const FAMILY_FIXTURE: &str = "complete/family-lifecycle/reopen-child/list1";
const K: &str = "a8dcf93bcaa14211a07481eb87af8f11";
const K_MEMBERS: [&str; 4] = [
    K,
    "28ae80e7369d40feaba54fe228eda2c6",
    "c57dfd30e9574d6ba195117e73dcc6d9",
    "58c8af9fe5694a498f0f0d07a20e5027",
];

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 4, 15, 0, 0).unwrap()
}

fn title_edit() -> Op {
    Op::Edit {
        uid: A.into(),
        changes: vec![Change::Summary("Planned title".into())],
    }
}

#[test]
fn plan_of_a_title_edit_writes_one_file_and_takes_no_state() {
    let env = Env::new(WriteMode::Test, FIXTURE);
    let snap = env.snap();
    let matrix = Matrix::embedded();
    let policy = Policy {
        matrix,
        mode: WriteMode::Test,
    };
    let prepared = plan(&snap, &title_edit(), policy, now()).unwrap();
    assert_eq!(prepared.entry.writes.len(), 1);
    let w = &prepared.entry.writes[0];
    let loaded = std::fs::read_to_string(env.file(A)).unwrap();
    assert_eq!(w.before.as_deref(), Some(loaded.as_str()));
    assert!(
        w.after
            .as_deref()
            .unwrap()
            .contains("SUMMARY:Planned title")
    );
    assert!(!env.cfg.state_dir().exists());
    assert!(!env.state.exists());
}

#[test]
fn plan_of_a_family_completion_holds_every_member() {
    let env = Env::new(WriteMode::Test, FAMILY_FIXTURE);
    let snap = env.snap();
    let matrix = common::matrix(
        "unverified",
        &[
            (twydo_core::compat::ContractId::CompleteStartOffsets, "pass"),
            (twydo_core::compat::ContractId::CompleteLateMonthly, "pass"),
        ],
    );
    let policy = Policy {
        matrix: &matrix,
        mode: WriteMode::Test,
    };
    let prepared = plan(&snap, &Op::Complete { uid: K.into() }, policy, now()).unwrap();
    for uid in K_MEMBERS {
        let path = format!("list1/{uid}.ics");
        assert!(
            prepared.entry.writes.iter().any(|w| w.path == path)
                || prepared.holds.iter().any(|h| h.0 == path),
            "{uid} is neither written nor held"
        );
    }
}

#[test]
fn plan_refuses_what_the_gate_refuses() {
    let env = Env::new(WriteMode::Test, FIXTURE);
    let snap = env.snap();
    let matrix = Matrix::embedded();
    let op = Op::Complete { uid: "nope".into() };
    let today = now().with_timezone(&snap.tz()).date_naive();
    let want = gate::check(&snap, matrix, WriteMode::Test, &op, today).unwrap_err();
    let policy = Policy {
        matrix,
        mode: WriteMode::Test,
    };
    match plan(&snap, &op, policy, now()) {
        Err(WriteError::Blocked(got)) => assert_eq!(got.0, want.0),
        other => panic!("expected Blocked, got {other:?}"),
    }
}

#[test]
fn plan_matches_store_prepare() {
    let env = Env::new(WriteMode::Test, FIXTURE);
    let snap = env.snap();
    let matrix = Matrix::embedded();
    let policy = Policy {
        matrix,
        mode: WriteMode::Test,
    };
    let prepared = plan(&snap, &title_edit(), policy, now()).unwrap();
    let mut store = env.store(matrix.clone());
    store.apply(&snap, &title_edit(), now()).unwrap();
    let journal = env.journal();
    assert_eq!(journal.len(), 1);
    assert_eq!(journal[0].writes, prepared.entry.writes);
}
