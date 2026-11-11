//! Write messages are pure snapshot descriptions; no store or daemon is needed.

use std::path::PathBuf;

use twydo_core::config::Config;
use twydo_core::edit::describe::success_message;
use twydo_core::edit::{Change, Op};
use twydo_core::model::{Metadata, Status, Task, TaskKind};
use twydo_core::vdir::{Collection, LoadReport};
use twydo_core::views::Snapshot;

fn task(uid: &str, summary: &str) -> Task {
    Task {
        uid: uid.into(),
        collection: "list1".into(),
        path: PathBuf::from(format!("list1/{uid}.ics")),
        summary: summary.into(),
        description: None,
        due: None,
        start: None,
        status: Status::Open,
        completed: None,
        created: None,
        last_modified: None,
        rrule: None,
        parent: None,
        kind: TaskKind::Task,
        alarms: vec![],
        meta: Metadata::default(),
        from_completion: false,
        warnings: vec![],
        blocked: None,
    }
}

fn snapshot(tasks: Vec<Task>) -> Snapshot {
    Snapshot::new(
        LoadReport {
            tasks,
            collections: vec![Collection {
                id: "list2".into(),
                path: PathBuf::from("list2"),
                display_name: Some("Two".into()),
                color: None,
            }],
            ..LoadReport::default()
        },
        Config::with_vdir(PathBuf::from("/synthetic/vdir")),
    )
}

fn family(statuses: &[Status]) -> Snapshot {
    let mut parent = task("parent", "Synthetic parent");
    parent.kind = TaskKind::Checklist;
    let mut tasks = vec![parent];
    for (i, status) in statuses.iter().enumerate() {
        let mut child = task(&format!("child-{i}"), "Synthetic item");
        child.parent = Some("parent".into());
        child.status = *status;
        tasks.push(child);
    }
    snapshot(tasks)
}

#[test]
fn success_messages_for_single_tasks() {
    let snap = snapshot(vec![task("task", "Synthetic task")]);
    for (op, expected) in [
        (
            Op::Edit {
                uid: "task".into(),
                changes: vec![Change::Summary("Changed".into())],
            },
            "saved",
        ),
        (Op::Delete { uid: "task".into() }, "deleted · u to undo"),
        (
            Op::Move {
                uid: "task".into(),
                to: "list2".into(),
            },
            "moved to Two",
        ),
    ] {
        assert_eq!(success_message(&snap, &op), expected, "{op:?}");
    }
}

#[test]
fn family_messages_count_the_children_the_operation_affects() {
    use Status::{Completed, Open};

    let uid = || "parent".to_string();
    for (statuses, op, expected) in [
        (
            vec![Open, Open, Completed],
            Op::Complete { uid: uid() },
            "done: \"Synthetic parent\" and 2 items",
        ),
        (
            vec![Completed, Completed, Completed],
            Op::Reopen { uid: uid() },
            "reopened: \"Synthetic parent\" and 3 items",
        ),
        (
            vec![Open, Open, Completed],
            Op::Reopen { uid: uid() },
            "reopened: \"Synthetic parent\" and 1 item",
        ),
        (
            vec![Open, Open, Open],
            Op::Move {
                uid: uid(),
                to: "list2".into(),
            },
            "moved \"Synthetic parent\" and 3 items to Two",
        ),
        (
            vec![Open, Open, Open],
            Op::Delete { uid: uid() },
            "deleted \"Synthetic parent\" and 3 items · u to undo",
        ),
        (
            vec![Open, Completed],
            Op::Complete { uid: uid() },
            "done: \"Synthetic parent\" and 1 item",
        ),
    ] {
        let mut snap = family(&statuses);
        if matches!(op, Op::Reopen { .. }) {
            snap.tasks[0].status = Completed;
        }
        assert_eq!(
            success_message(&snap, &op),
            expected,
            "{op:?}: {statuses:?}"
        );
    }
}
