//! The pure planner: gate and plan on a loaded snapshot, with no store and no state directory.

mod common;

use chrono::{DateTime, TimeZone, Utc};

use twydo_core::compat::Matrix;
use twydo_core::config::WriteMode;
use twydo_core::edit::{Change, NewTask, Op};
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

#[test]
fn create_and_delete_declare_exact_queue_and_journal_dependencies() {
    use std::collections::BTreeSet;

    let env = Env::new(WriteMode::Verified, "insert/project-items/created/list1");
    let snap = env.snap();
    let parent = "5def84c9a3234ad3bd16e9f3ecd76225";
    let parent_idx = snap.find(parent).unwrap();
    let members: Vec<_> = std::iter::once(&parent_idx)
        .chain(snap.children(parent_idx))
        .map(|&i| &snap.tasks[i])
        .collect();
    let child = &members[1].uid;
    let policy = Policy {
        matrix: Matrix::embedded(),
        mode: WriteMode::Verified,
    };
    let new = NewTask::plain("list1", "Synthetic task");
    let standalone = plan(&snap, &Op::Create(new.clone()), policy, now()).unwrap();
    assert!(standalone.family.is_none() && standalone.entry.family.is_none());
    assert!(standalone.holds.is_empty() && standalone.entry.holds.is_empty());
    assert!(standalone.families.is_empty() && standalone.entry.families.is_empty());

    let append = Op::Create(NewTask {
        parent: Some(parent.into()),
        ..new
    });
    for (op, held) in [
        (append, members.clone()),
        (
            Op::Delete { uid: child.clone() },
            members
                .iter()
                .copied()
                .filter(|t| t.uid != *child)
                .collect(),
        ),
        (Op::Delete { uid: parent.into() }, Vec::new()),
    ] {
        let prepared = plan(&snap, &op, policy, now()).unwrap();
        let entry = &prepared.entry;
        assert_eq!(prepared.family, entry.family);
        assert_eq!(prepared.holds, entry.holds);
        assert!(prepared.families.is_empty() && entry.families.is_empty());
        let family = entry.family.as_ref().unwrap();
        assert_eq!(family.collection, "list1");
        assert_eq!(family.parent_uid, parent);
        assert!(family.parent_aliases.contains(parent));
        let mut paths: BTreeSet<_> = members
            .iter()
            .map(|t| format!("list1/{}.ics", t.uid))
            .collect();
        if matches!(op, Op::Create(_)) {
            paths.insert(format!("list1/{}.ics", entry.uids[0]));
        }
        assert_eq!(family.loaded, paths);
        let expected: BTreeSet<_> = held
            .iter()
            .map(|t| {
                (
                    format!("list1/{}.ics", t.uid),
                    std::fs::read_to_string(&t.path).unwrap(),
                )
            })
            .collect();
        assert_eq!(
            entry.holds.iter().cloned().collect::<BTreeSet<_>>(),
            expected
        );
    }
}

#[test]
fn deleting_a_standalone_task_guards_against_its_first_child() {
    let env = Env::new(WriteMode::Verified, FIXTURE);
    let prepared = plan(
        &env.snap(),
        &Op::Delete { uid: A.into() },
        Policy {
            matrix: Matrix::embedded(),
            mode: WriteMode::Verified,
        },
        now(),
    )
    .unwrap();
    assert_eq!(prepared.family, prepared.entry.family);
    let family = prepared.family.unwrap();
    assert_eq!(family.parent_uid, A);
    assert_eq!(
        family.loaded,
        std::collections::BTreeSet::from([format!("list1/{A}.ics")])
    );
    assert!(prepared.holds.is_empty() && prepared.entry.holds.is_empty());
}
