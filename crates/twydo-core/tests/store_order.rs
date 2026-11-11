//! Shared sibling order, including guarded/queued writes on synthetic families.
mod common;

use chrono::{TimeZone, Utc};
use common::Env;
use std::fs;
use twydo_core::compat::{Matrix, OperationEntry, Verdict};
use twydo_core::config::WriteMode;
use twydo_core::edit::{NewTask, Op, Placement};
use twydo_core::model::TaskKind;
use twydo_core::store::fsio::VdirLock;
use twydo_core::store::queue;
use twydo_core::store::{Store, Submitted};
use twydo_core::views::{Snapshot, compare_items};

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 6, 17, 0, 0).unwrap()
}

fn matrix() -> Matrix {
    let mut m = Matrix::embedded().clone();
    for case in ["checklist", "project"] {
        for op in ["reorder", "insert"] {
            m.operation.retain(|r| !(r.case == case && r.op == op));
            m.operation.push(OperationEntry {
                case: case.into(),
                op: op.into(),
                iphone_baseline: Verdict::Unsupported,
                mac_to_iphone: Verdict::Unverified,
                iphone_to_mac: Verdict::Unverified,
                twydo_to_iphone: Verdict::Pass,
                iphone_to_twydo: Verdict::Pass,
                evidence: vec![],
                notes: String::new(),
            });
        }
    }
    m
}

fn setup(kind: TaskKind) -> (Env, Store, String, Vec<String>) {
    let scene = if kind == TaskKind::Checklist {
        "insert/checklist-items"
    } else {
        "insert/project-items"
    };
    let env = Env::new(WriteMode::Verified, &format!("{scene}/created/list1"));
    let store = env.store(matrix());
    let s = env.snap();
    let p = s
        .tasks
        .iter()
        .find(|t| t.parent.is_none())
        .unwrap()
        .uid
        .clone();
    let kids = ordered(&s, &p);
    (env, store, p, kids)
}

fn ordered(s: &Snapshot, parent: &str) -> Vec<String> {
    let mut kids = s.children(s.find(parent).unwrap()).to_vec();
    kids.sort_by(|&a, &b| compare_items(s, s.tz(), a, b));
    kids.iter().map(|&i| s.tasks[i].uid.clone()).collect()
}

fn reorder(uid: &str, anchor: &str, placement: Placement) -> Op {
    Op::Reorder {
        uid: uid.into(),
        anchor: anchor.into(),
        placement,
    }
}

#[test]
fn reorder_persists_and_undo_redo_restore_exact_bytes() {
    for kind in [TaskKind::Checklist, TaskKind::Project] {
        let (env, mut store, p, k) = setup(kind);
        let before = env.files();
        let s = env.snap();
        store
            .apply(&s, &reorder(&k[2], &k[0], Placement::Before), now())
            .unwrap();
        assert_eq!(
            ordered(&env.snap(), &p),
            [k[2].clone(), k[0].clone(), k[1].clone()]
        );
        let after = env.files();
        assert_eq!(
            after[&format!("list1/{p}.ics")],
            before[&format!("list1/{p}.ics")]
        );
        for t in &s.tasks {
            let new = env.snap();
            let n = &new.tasks[new.find(&t.uid).unwrap()];
            assert_eq!(t.created, n.created);
            assert_eq!(t.due, n.due);
            assert_eq!(t.status, n.status);
            assert_eq!(t.parent, n.parent);
            assert_eq!(t.rrule, n.rrule);
            assert_eq!(t.description, n.description);
        }
        store.undo_last(now()).unwrap();
        assert_eq!(env.files(), before);
        store.redo_last(now()).unwrap();
        assert_eq!(env.files(), after);
    }
}

#[test]
fn anchored_insertion_and_later_append_keep_their_positions() {
    for kind in [TaskKind::Checklist, TaskKind::Project] {
        for placement in [Placement::Before, Placement::After] {
            let (env, mut store, p, k) = setup(kind);
            let op = Op::CreateAt {
                task: NewTask {
                    parent: Some(p.clone()),
                    ..NewTask::plain("list1", "Inserted")
                },
                anchor: k[1].clone(),
                placement,
            };
            let id = store.apply(&env.snap(), &op, now()).unwrap();
            let new = env.journal().iter().find(|e| e.id == id).unwrap().uids[0].clone();
            let want = if placement == Placement::Before {
                vec![k[0].clone(), new, k[1].clone(), k[2].clone()]
            } else {
                vec![k[0].clone(), k[1].clone(), new, k[2].clone()]
            };
            assert_eq!(ordered(&env.snap(), &p), want);
            store
                .apply(
                    &env.snap(),
                    &Op::Create(NewTask {
                        parent: Some(p.clone()),
                        ..NewTask::plain("list1", "Append")
                    }),
                    now(),
                )
                .unwrap();
            let s = env.snap();
            let got = ordered(&s, &p);
            assert_eq!(&got[..4], &want);
            assert_eq!(s.tasks[s.find(&got[4]).unwrap()].summary, "Append");
        }
    }
}

#[test]
fn stale_sibling_and_new_member_refuse_without_partial_writes() {
    for mutation in ["edit", "join", "delete", "parent"] {
        let (env, mut store, p, k) = setup(TaskKind::Checklist);
        let s = env.snap();
        match mutation {
            "join" => {
                let mut other = env.store(matrix());
                other
                    .apply(
                        &s,
                        &Op::Create(NewTask {
                            parent: Some(p),
                            ..NewTask::plain("list1", "Joined")
                        }),
                        now(),
                    )
                    .unwrap();
            }
            "delete" => {
                fs::remove_file(env.file(&k[1])).unwrap();
            }
            _ => {
                let uid = if mutation == "parent" { &p } else { &k[1] };
                let path = env.file(uid);
                let mut b = fs::read(&path).unwrap();
                b.extend_from_slice(b"\r\n");
                fs::write(path, b).unwrap();
            }
        }
        let before = env.files();
        assert!(
            store
                .apply(&s, &reorder(&k[2], &k[0], Placement::Before), now())
                .is_err()
        );
        assert_eq!(env.files(), before);
    }
}

#[test]
fn queued_moves_plan_on_the_overlay_and_drain_in_sequence() {
    let (env, mut store, p, k) = setup(TaskKind::Project);
    let before = env.files();
    let lock = VdirLock::exclusive(&env.state).unwrap();
    assert!(matches!(
        store
            .submit(
                &env.snap(),
                &reorder(&k[2], &k[0], Placement::Before),
                "move",
                "tui",
                now()
            )
            .unwrap(),
        Submitted::Queued(_)
    ));
    let overlay = queue::view(twydo_core::vdir::load(&env.vdir).unwrap(), env.cfg.clone());
    assert_eq!(
        ordered(&overlay, &p),
        [k[2].clone(), k[0].clone(), k[1].clone()]
    );
    store
        .submit(
            &overlay,
            &reorder(&k[1], &k[2], Placement::Before),
            "move",
            "tui",
            now(),
        )
        .unwrap();
    assert_eq!(env.files(), before);
    drop(lock);
    store.drain(now()).unwrap();
    assert_eq!(
        ordered(&env.snap(), &p),
        [k[1].clone(), k[2].clone(), k[0].clone()]
    );
}

#[test]
fn verified_ordering_requires_phone_display_and_preservation_not_phone_reordering() {
    let (env, _, _, k) = setup(TaskKind::Checklist);
    let op = reorder(&k[2], &k[0], Placement::Before);
    let s = env.snap();
    assert!(
        twydo_core::gate::check(&s, &matrix(), WriteMode::Verified, &op, now().date_naive())
            .is_ok()
    );
    let mut unverified = matrix();
    unverified
        .operation
        .iter_mut()
        .filter(|r| r.op == "reorder")
        .for_each(|r| r.twydo_to_iphone = Verdict::Unverified);
    assert!(
        twydo_core::gate::check(
            &s,
            &unverified,
            WriteMode::Verified,
            &op,
            now().date_naive()
        )
        .is_err()
    );
    assert!(
        twydo_core::gate::check(
            &s,
            Matrix::embedded(),
            WriteMode::Verified,
            &op,
            now().date_naive()
        )
        .is_ok()
    );
}

#[test]
fn changing_display_order_does_not_depend_on_sibling_completion_state() {
    for kind in [TaskKind::Checklist, TaskKind::Project] {
        for status in [
            twydo_core::model::Status::Completed,
            twydo_core::model::Status::Cancelled,
        ] {
            let (env, _, _, k) = setup(kind);
            let mut snap = env.snap();
            let i = snap.find(&k[1]).unwrap();
            snap.tasks[i].status = status;
            let plain = Matrix::embedded();
            let op = reorder(&k[2], &k[0], Placement::Before);
            assert!(
                twydo_core::gate::check(&snap, plain, WriteMode::Verified, &op, now().date_naive())
                    .is_ok()
            );
        }
    }
}

#[test]
fn changing_display_order_does_not_depend_on_sibling_dates() {
    for kind in [TaskKind::Checklist, TaskKind::Project] {
        let (env, _, p, k) = setup(kind);
        let mut store = env.store(Matrix::embedded().clone());
        store
            .apply(
                &env.snap(),
                &reorder(&k[2], &k[0], Placement::Before),
                now(),
            )
            .unwrap();
        let op = Op::CreateAt {
            task: NewTask {
                parent: Some(p),
                ..NewTask::plain("list1", "Verified insertion")
            },
            anchor: k[0].clone(),
            placement: Placement::Before,
        };
        store.apply(&env.snap(), &op, now()).unwrap();
        let mut snap = env.snap();
        let i = snap.find(&k[1]).unwrap();
        snap.tasks[i].due = Some(twydo_core::ical::value::IcalDateTime::Date(
            now().date_naive(),
        ));
        assert!(
            twydo_core::gate::check(
                &snap,
                Matrix::embedded(),
                WriteMode::Verified,
                &reorder(&k[2], &k[0], Placement::Before),
                now().date_naive()
            )
            .is_ok()
        );
    }
}

#[test]
fn explicit_reorder_handles_legacy_ranks_and_keeps_other_metadata() {
    use twydo_core::{ical, model::meta_edit::MetaJson};
    for rank in [None, Some("bad"), Some("0"), Some("9223372036854775807")] {
        let (env, mut store, p, k) = setup(TaskKind::Checklist);
        for uid in &k {
            let path = env.file(uid);
            let mut cal = ical::parse(&fs::read(&path).unwrap()).unwrap();
            let todo = cal.root.children_named_mut("VTODO").next().unwrap();
            let prop = todo.prop_mut("X-2DOAPP-METADATA").unwrap();
            let mut m = MetaJson::parse(&prop.value).unwrap();
            m.set_str("UnknownOrderTest", "keep me");
            match rank {
                None => m.remove("DisplayOrder"),
                Some(v) => m.set_str("DisplayOrder", v),
            }
            prop.set_value(&m.encode());
            // No creation timestamp: UID is the documented deterministic fallback.
            todo.props.retain(|p| p.name != "CREATED");
            fs::write(path, ical::serialize(&cal)).unwrap();
        }
        let old = ordered(&env.snap(), &p);
        store
            .apply(
                &env.snap(),
                &reorder(&old[2], &old[0], Placement::Before),
                now(),
            )
            .unwrap();
        assert_eq!(
            ordered(&env.snap(), &p),
            [old[2].clone(), old[0].clone(), old[1].clone()]
        );
        for uid in &k {
            let cal = ical::parse(&fs::read(env.file(uid)).unwrap()).unwrap();
            let t = cal.root.children_named("VTODO").next().unwrap();
            assert!(t.prop("CREATED").is_none());
            assert_eq!(
                MetaJson::parse(&t.prop("X-2DOAPP-METADATA").unwrap().value)
                    .unwrap()
                    .get_str("UnknownOrderTest"),
                Some("keep me")
            );
        }
    }
}

#[test]
fn invalid_targets_or_metadata_refuse_without_writes() {
    for case in ["self", "missing", "parent", "metadata", "cross-parent"] {
        let (env, mut store, p, k) = setup(TaskKind::Project);
        let (uid, anchor) = match case {
            "self" => (k[0].clone(), k[0].clone()),
            "missing" => (k[0].clone(), "missing".into()),
            "parent" => (p.clone(), k[0].clone()),
            "cross-parent" => {
                let path = env.file(&k[1]);
                let s = fs::read_to_string(&path).unwrap();
                fs::write(
                    path,
                    s.replace(&format!("RELATED-TO:{p}"), "RELATED-TO:missing-parent"),
                )
                .unwrap();
                (k[1].clone(), k[0].clone())
            }
            _ => {
                let path = env.file(&k[1]);
                let mut cal = twydo_core::ical::parse(&fs::read(&path).unwrap()).unwrap();
                cal.root
                    .children_named_mut("VTODO")
                    .next()
                    .unwrap()
                    .prop_mut("X-2DOAPP-METADATA")
                    .unwrap()
                    .set_value("@tasktype:task");
                fs::write(path, twydo_core::ical::serialize(&cal)).unwrap();
                (k[2].clone(), k[0].clone())
            }
        };
        let before = env.files();
        assert!(
            store
                .apply(
                    &env.snap(),
                    &reorder(&uid, &anchor, Placement::Before),
                    now()
                )
                .is_err(),
            "{case}"
        );
        assert_eq!(env.files(), before);
    }
}

#[test]
fn completed_siblings_keep_order_and_unchanged_rank_keeps_exact_bytes() {
    let (env, mut store, p, k) = setup(TaskKind::Checklist);
    // First give all siblings distinct ranks, then swap only the last two.
    store
        .apply(
            &env.snap(),
            &reorder(&k[2], &k[1], Placement::Before),
            now(),
        )
        .unwrap();
    let first = fs::read(env.file(&k[0])).unwrap();
    let path = env.file(&k[1]);
    let b = fs::read_to_string(&path).unwrap();
    fs::write(path, b.replace("STATUS:NEEDS-ACTION", "STATUS:COMPLETED")).unwrap();
    store
        .apply(
            &env.snap(),
            &reorder(&k[1], &k[2], Placement::Before),
            now(),
        )
        .unwrap();
    assert_eq!(ordered(&env.snap(), &p), k);
    assert_eq!(fs::read(env.file(&k[0])).unwrap(), first);
}

#[test]
fn queued_insert_and_reorder_recover_partial_application_idempotently() {
    use twydo_core::store::journal::{Journal, Stage, recover};
    for insert in [false, true] {
        let (env, mut store, p, k) = setup(TaskKind::Checklist);
        let op = if insert {
            Op::CreateAt {
                task: NewTask {
                    parent: Some(p.clone()),
                    ..NewTask::plain("list1", "Inserted")
                },
                anchor: k[1].clone(),
                placement: Placement::Before,
            }
        } else {
            reorder(&k[2], &k[0], Placement::Before)
        };
        let lock = VdirLock::exclusive(&env.state).unwrap();
        store
            .submit(&env.snap(), &op, "order", "tui", now())
            .unwrap();
        let q = queue::Queue::open(&env.state).unwrap();
        let item = q.list().unwrap().remove(0);
        let queue::Kind::Op { prepared, .. } = item.kind else {
            panic!()
        };
        let mut e = prepared.entry;
        assert!(e.family.is_some());
        assert!(!e.holds.is_empty());
        e.stage = Stage::Prepared;
        Journal::open(&env.state).unwrap().prepare(&e).unwrap();
        let w = &e.writes[0];
        fs::write(env.vdir.join(&w.path), w.after.as_ref().unwrap()).unwrap();
        drop(lock);
        let report = recover(&env.vdir, &env.state).unwrap();
        assert!(report.stuck.is_empty(), "{report:?}");
        let after = env.files();
        recover(&env.vdir, &env.state).unwrap();
        assert_eq!(env.files(), after);
        if insert {
            let s = env.snap();
            assert_eq!(
                s.tasks[s.find(&ordered(&s, &p)[1]).unwrap()].summary,
                "Inserted"
            );
        } else {
            assert_eq!(
                ordered(&env.snap(), &p),
                [k[2].clone(), k[0].clone(), k[1].clone()]
            );
        }
    }
}

#[test]
fn queued_stale_order_and_undo_after_remote_edit_refuse() {
    let (env, mut store, _, k) = setup(TaskKind::Project);
    let lock = VdirLock::exclusive(&env.state).unwrap();
    store
        .submit(
            &env.snap(),
            &reorder(&k[2], &k[0], Placement::Before),
            "order",
            "tui",
            now(),
        )
        .unwrap();
    let path = env.file(&k[1]);
    let mut bytes = fs::read(&path).unwrap();
    bytes.extend_from_slice(b"\r\n");
    fs::write(&path, bytes).unwrap();
    let before = env.files();
    drop(lock);
    let report = store.drain(now()).unwrap();
    assert!(!report.is_empty());
    assert_eq!(env.files(), before);
    let (env, mut store, _, k) = setup(TaskKind::Project);
    store
        .apply(
            &env.snap(),
            &reorder(&k[2], &k[0], Placement::Before),
            now(),
        )
        .unwrap();
    let path = env.file(&k[1]);
    let mut b = fs::read(&path).unwrap();
    b.extend_from_slice(b"\r\n");
    fs::write(path, b).unwrap();
    let before = env.files();
    assert!(store.undo_last(now()).is_err());
    assert_eq!(env.files(), before);
}

#[test]
fn anchored_insertion_still_requires_ordinary_creation_compatibility() {
    use twydo_core::edit::Due;
    let (env, _, p, k) = setup(TaskKind::Project);
    let mut m = matrix();
    m.operation
        .retain(|r| !(r.case == "project" && r.op == "create"));
    let mut store = env.store(m);
    store
        .apply(
            &env.snap(),
            &reorder(&k[2], &k[0], Placement::Before),
            now(),
        )
        .unwrap();
    let before = env.files();
    let op = Op::CreateAt {
        task: NewTask {
            parent: Some(p),
            due: Some(Due::Date(now().date_naive())),
            ..NewTask::plain("list1", "Dated")
        },
        anchor: k[0].clone(),
        placement: Placement::Before,
    };
    assert!(store.apply(&env.snap(), &op, now()).is_err());
    assert_eq!(env.files(), before);
}

/// Compare every logical property/parameter/component, normalizing only the fields
/// this operation is allowed to change. Recurrence, alarms and dates stay in this set.
fn without_order_and_modification_stamps(bytes: &[u8]) -> Vec<common::Line> {
    let cal = twydo_core::ical::parse(bytes).unwrap();
    common::lines(&cal)
        .into_iter()
        .filter(|line| !common::is_stamp(&line.1))
        .map(|mut line| {
            if line.1 == "X-2DOAPP-METADATA" {
                let mut meta = twydo_core::model::meta_edit::MetaJson::parse(&line.3).unwrap();
                meta.remove("DisplayOrder");
                line.3 = meta.encode();
            }
            line
        })
        .collect()
}

fn assert_only_order_changed(
    before: &std::collections::BTreeMap<String, Vec<u8>>,
    after: &std::collections::BTreeMap<String, Vec<u8>>,
) {
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>()
    );
    for (path, bytes) in before {
        if bytes != &after[path] {
            assert_eq!(
                without_order_and_modification_stamps(bytes),
                without_order_and_modification_stamps(&after[path]),
                "{path}"
            );
        }
    }
}

#[test]
fn recurring_checklist_and_project_reorder_preserves_every_other_field() {
    use common::recurrence::{FC, SCENARIO, SET};
    for (fixture, parent) in [
        (format!("{SCENARIO}/{SET}"), FC),
        (
            "complete/family-lifecycle/reopen-child/list1".into(),
            "8f603cfaa41f4c229b9adcd6223e7393",
        ),
        // A project with an independently repeating child uses the same field write.
        (
            "complete/family-lifecycle/reopen-child/list1".into(),
            "09c49c2cd76f483498fc2ff3a0f66160",
        ),
    ] {
        let env = Env::new(WriteMode::Verified, &fixture);
        let mut store = env.store(Matrix::embedded().clone());
        let before = env.files();
        let s = env.snap();
        let kids = ordered(&s, parent);
        assert!(kids.len() >= 2);
        assert!(
            s.tasks[s.find(parent).unwrap()].rrule.is_some()
                || kids
                    .iter()
                    .any(|uid| s.tasks[s.find(uid).unwrap()].rrule.is_some())
        );
        let moved = kids.last().unwrap();
        store
            .apply(&s, &reorder(moved, &kids[0], Placement::Before), now())
            .unwrap();
        let mut want = kids.clone();
        want.pop();
        want.insert(0, moved.clone());
        assert_eq!(ordered(&env.snap(), parent), want);
        let after = env.files();
        assert_only_order_changed(&before, &after);
        assert_eq!(
            fs::read(env.file(parent)).unwrap(),
            before[&format!("list1/{parent}.ics")]
        );
        store.undo_last(now()).unwrap();
        assert_eq!(env.files(), before);
        store.redo_last(now()).unwrap();
        assert_eq!(env.files(), after);
    }
}

#[test]
fn recurring_family_reorder_queues_without_changing_recurrence_or_dates() {
    use common::recurrence::{FC, SCENARIO, SET};
    let env = Env::new(WriteMode::Verified, &format!("{SCENARIO}/{SET}"));
    let mut store = env.store(Matrix::embedded().clone());
    let s = env.snap();
    let kids = ordered(&s, FC);
    let before = env.files();
    let lock = VdirLock::exclusive(&env.state).unwrap();
    assert!(matches!(
        store
            .submit(
                &s,
                &reorder(kids.last().unwrap(), &kids[0], Placement::Before),
                "move",
                "tui",
                now()
            )
            .unwrap(),
        Submitted::Queued(_)
    ));
    let overlay = queue::view(twydo_core::vdir::load(&env.vdir).unwrap(), env.cfg.clone());
    assert_eq!(ordered(&overlay, FC)[0], *kids.last().unwrap());
    assert_eq!(
        overlay.tasks[overlay.find(FC).unwrap()].rrule,
        s.tasks[s.find(FC).unwrap()].rrule
    );
    assert_eq!(env.files(), before);
    drop(lock);
    store.drain(now()).unwrap();
    assert_eq!(ordered(&env.snap(), FC)[0], *kids.last().unwrap());
    assert_only_order_changed(&before, &env.files());
    store.undo_last(now()).unwrap();
    assert_eq!(env.files(), before);
}

#[test]
fn reordered_recurring_checklist_keeps_positions_through_supported_completion() {
    use common::recurrence::{FC, SCENARIO, SET};
    let env = Env::new(WriteMode::Verified, &format!("{SCENARIO}/{SET}"));
    let mut store = env.store(Matrix::embedded().clone());
    let before = env.files();
    let s = env.snap();
    let kids = ordered(&s, FC);
    store
        .apply(
            &s,
            &reorder(kids.last().unwrap(), &kids[0], Placement::Before),
            now(),
        )
        .unwrap();
    let s = env.snap();
    let reordered = env.files();
    let positions: std::collections::BTreeMap<_, _> = s
        .children(s.find(FC).unwrap())
        .iter()
        .map(|&i| (s.tasks[i].summary.clone(), s.tasks[i].meta.display_order()))
        .collect();
    store
        .apply(&s, &Op::Complete { uid: FC.into() }, now())
        .unwrap();
    let after = env.snap();
    for (title, position) in positions {
        let copies: Vec<_> = after.tasks.iter().filter(|t| t.summary == title).collect();
        assert_eq!(copies.len(), 2, "{title}");
        assert!(copies.iter().all(|t| t.meta.display_order() == position));
    }
    store.undo_last(now()).unwrap();
    assert_eq!(env.files(), reordered);
    store.undo_last(now()).unwrap();
    assert_eq!(env.files(), before);
}

#[test]
fn ordering_preserves_opaque_recurrence_dates_alarms_and_status_without_enabling_execution() {
    use twydo_core::ical::{self, Property};
    for kind in [TaskKind::Checklist, TaskKind::Project] {
        let (env, _, parent, kids) = setup(kind);
        for (n, uid) in std::iter::once(&parent).chain(kids.iter()).enumerate() {
            let path = env.file(uid);
            let mut cal = ical::parse(&fs::read(&path).unwrap()).unwrap();
            let todo = cal.root.children_named_mut("VTODO").next().unwrap();
            if n == 0 {
                todo.insert_prop(
                    Property::new("RRULE", vec![], "FREQ=YEARLY;X-OPAQUE=keep"),
                    None,
                );
            }
            todo.insert_prop(Property::new("DUE", vec![], "20400101T081500Z"), None);
            todo.insert_prop(Property::new("DTSTART", vec![], "20400101T081500Z"), None);
            if n == 1 {
                todo.prop_mut("STATUS").unwrap().set_value("COMPLETED");
            }
            if n == 2 {
                todo.prop_mut("STATUS").unwrap().set_value("CANCELLED");
            }
            let text = ical::serialize(&cal);
            // Include a real VALARM component in the preserved resource.
            let text=String::from_utf8(text).unwrap().replace("END:VTODO","BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nDESCRIPTION:Synthetic alarm\r\nEND:VALARM\r\nEND:VTODO");
            fs::write(path, text).unwrap();
        }
        let mut m = Matrix::embedded().clone();
        for rule in &mut m.rule {
            rule.execute = Verdict::Unverified;
            rule.preserve = Verdict::Unverified;
        }
        let mut store = env.store(m);
        let before = env.files();
        let s = env.snap();
        store
            .apply(&s, &reorder(&kids[2], &kids[0], Placement::Before), now())
            .unwrap();
        assert_only_order_changed(&before, &env.files());
        // Ordering approval must not leak into a recurrence-executing operation.
        let ordered = env.files();
        assert!(
            store
                .apply(&env.snap(), &Op::Complete { uid: parent }, now())
                .is_err()
        );
        assert_eq!(env.files(), ordered);
        store.undo_last(now()).unwrap();
        assert_eq!(env.files(), before);
    }
}

#[test]
fn explicit_order_materializes_integer_ranks_including_invalid_first_position() {
    use twydo_core::{ical, model::meta_edit::MetaJson};
    for kind in [TaskKind::Checklist, TaskKind::Project] {
        for invalid in ["missing", "malformed", "negative", "overflow"] {
            for insert in [false, true] {
                let (env, mut store, p, k) = setup(kind);
                for uid in &k {
                    let path = env.file(uid);
                    let mut cal = ical::parse(&fs::read(&path).unwrap()).unwrap();
                    let prop = cal
                        .root
                        .children_named_mut("VTODO")
                        .next()
                        .unwrap()
                        .prop_mut("X-2DOAPP-METADATA")
                        .unwrap();
                    let mut meta = MetaJson::parse(&prop.value).unwrap();
                    match invalid {
                        "missing" => meta.remove("DisplayOrder"),
                        "negative" => meta.set_i64("DisplayOrder", -1),
                        "overflow" => meta.set_str("DisplayOrder", "9223372036854775808"),
                        _ => meta.set_str("DisplayOrder", "bad"),
                    }
                    prop.set_value(&meta.encode());
                    fs::write(path, ical::serialize(&cal)).unwrap();
                }
                let op = if insert {
                    Op::CreateAt {
                        task: NewTask {
                            parent: Some(p),
                            ..NewTask::plain("list1", "Inserted")
                        },
                        anchor: k[1].clone(),
                        placement: Placement::After,
                    }
                } else {
                    reorder(&k[2], &k[0], Placement::Before)
                };
                let id = store.apply(&env.snap(), &op, now()).unwrap();
                let want = if insert {
                    let uid = env.journal().iter().find(|e| e.id == id).unwrap().uids[0].clone();
                    vec![k[0].clone(), k[1].clone(), uid, k[2].clone()]
                } else {
                    vec![k[2].clone(), k[0].clone(), k[1].clone()]
                };
                for (rank, uid) in want.iter().enumerate() {
                    let cal = ical::parse(&fs::read(env.file(uid)).unwrap()).unwrap();
                    let prop = cal
                        .root
                        .children_named("VTODO")
                        .next()
                        .unwrap()
                        .prop("X-2DOAPP-METADATA")
                        .unwrap();
                    assert_eq!(
                        MetaJson::parse(&prop.value)
                            .unwrap()
                            .get_i64("DisplayOrder"),
                        Some(rank as i64),
                        "{kind:?}/{invalid}/insert={insert}/rank={rank}"
                    );
                }
            }
        }
    }
}

#[test]
fn captured_reorder_ranks_override_creation_and_title_without_changing_task_semantics() {
    let env = Env::new(WriteMode::Verified, "reorder/family-items/seeded/list1");
    let before = env.snap();
    for step in ["parent-stamp/list1", "twydo-reordered/list1"] {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/reorder/family-items")
            .join(step);
        for e in fs::read_dir(dir).unwrap() {
            let path = e.unwrap().path();
            if path.extension().is_some_and(|e| e == "ics") {
                fs::copy(
                    &path,
                    env.vdir.join("list1").join(path.file_name().unwrap()),
                )
                .unwrap();
            }
        }
    }
    let after = env.snap();
    let parents: Vec<_> = after
        .tasks
        .iter()
        .filter(|t| t.parent.is_none())
        .map(|t| t.uid.clone())
        .collect();
    assert_eq!(parents.len(), 2);
    for parent in &parents {
        let old = ordered(&before, parent);
        let new = ordered(&after, parent);
        let titles = |s: &Snapshot, kids: &[String]| {
            kids.iter()
                .map(|uid| s.tasks[s.find(uid).unwrap()].summary.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(titles(&before, &old), ["Zulu", "Alpha", "Mike"]);
        assert_eq!(titles(&after, &new), ["Mike", "Zulu", "Alpha"]);
    }
    for task in &before.tasks {
        let new = &after.tasks[after.find(&task.uid).unwrap()];
        assert_eq!(task.created, new.created);
        assert_eq!(task.status, new.status);
        assert_eq!(task.due, new.due);
        assert_eq!(task.start, new.start);
        assert_eq!(task.rrule, new.rrule);
        assert_eq!(task.parent, new.parent);
    }
    let view = twydo_core::views::build_view(
        &after,
        &twydo_core::views::View::All,
        now().with_timezone(&after.tz()),
        &parents.into_iter().collect(),
    );
    let displayed: Vec<_> = view
        .groups
        .iter()
        .flat_map(|(_, rows)| rows)
        .filter(|r| r.depth > 0)
        .map(|r| after.tasks[r.task].summary.as_str())
        .collect();
    assert_eq!(
        displayed,
        ["Mike", "Zulu", "Alpha", "Mike", "Zulu", "Alpha"]
    );
    for step in ["manual-display/list1", "phone-rename/list1"] {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/reorder/family-items")
            .join(step);
        for e in fs::read_dir(dir).unwrap() {
            let path = e.unwrap().path();
            if path.extension().is_some_and(|e| e == "ics") {
                fs::copy(
                    &path,
                    env.vdir.join("list1").join(path.file_name().unwrap()),
                )
                .unwrap();
            }
        }
    }
    let returned = env.snap();
    for parent in returned.tasks.iter().filter(|t| t.parent.is_none()) {
        let kids = ordered(&returned, &parent.uid);
        let titles: Vec<_> = kids
            .iter()
            .map(|uid| returned.tasks[returned.find(uid).unwrap()].summary.as_str())
            .collect();
        assert_eq!(titles, ["MIKE", "Zulu", "Alpha"]);
    }
    for task in &after.tasks {
        let phone = &returned.tasks[returned.find(&task.uid).unwrap()];
        assert_eq!(task.parent, phone.parent);
        assert_eq!(task.created, phone.created);
        assert_eq!(task.status, phone.status);
        assert_eq!(task.due, phone.due);
        assert_eq!(task.start, phone.start);
        assert_eq!(task.rrule, phone.rrule);
        assert_eq!(task.meta, phone.meta);
    }
}

#[test]
fn captured_anchored_insertions_keep_positions_after_phone_renames_and_later_reorders() {
    let env = Env::new(WriteMode::Verified, "reorder/family-items/seeded/list1");
    for step in [
        "parent-stamp/list1",
        "twydo-reordered/list1",
        "manual-display/list1",
        "phone-rename/list1",
        "twydo-inserted/list1",
    ] {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/reorder/family-items")
            .join(step);
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|e| e == "ics") {
                fs::copy(&p, env.vdir.join("list1").join(p.file_name().unwrap())).unwrap();
            }
        }
    }
    let inserted = env.snap();
    assert_eq!(inserted.tasks.len(), 12);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/reorder/family-items/phone-insertion-rename/list1");
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|e| e == "ics") {
            fs::copy(&p, env.vdir.join("list1").join(p.file_name().unwrap())).unwrap();
        }
    }
    let returned = env.snap();
    assert_eq!(returned.tasks.len(), 12);
    assert!(
        returned
            .tasks
            .iter()
            .all(|t| t.blocked.is_none() && t.warnings.is_empty())
    );
    let parents: Vec<_> = returned
        .tasks
        .iter()
        .filter(|t| t.parent.is_none())
        .map(|t| t.uid.clone())
        .collect();
    for parent in &parents {
        let kids = ordered(&returned, parent);
        let titles: Vec<_> = kids
            .iter()
            .map(|uid| returned.tasks[returned.find(uid).unwrap()].summary.as_str())
            .collect();
        assert_eq!(titles, ["MIKE", "BEFORE", "Zulu", "Alpha", "AFTER"]);
        for (position, uid) in kids.iter().enumerate() {
            assert_eq!(
                returned.tasks[returned.find(uid).unwrap()]
                    .meta
                    .display_order(),
                position as i64
            );
        }
    }
    for old in &inserted.tasks {
        let phone = &returned.tasks[returned.find(&old.uid).unwrap()];
        assert_eq!(old.parent, phone.parent);
        assert_eq!(old.created, phone.created);
        assert_eq!(old.status, phone.status);
        assert_eq!(old.due, phone.due);
        assert_eq!(old.start, phone.start);
        assert_eq!(old.rrule, phone.rrule);
        assert_eq!(old.meta, phone.meta);
        if ["Before", "After"].contains(&old.summary.as_str()) {
            let before = twydo_core::ical::parse(inserted.source(&old.path).unwrap()).unwrap();
            let after = twydo_core::ical::parse(returned.source(&phone.path).unwrap()).unwrap();
            assert!(common::todo(&before).prop("X-2DOAPP-SYNC-IMAGE").is_none());
            for (key, value) in [
                ("X-2DOAPP-SYNC-IMAGE", "1"),
                ("X-2DOAPP-SYNC-AUDIO", "1"),
                ("X-2DOAPP-AUDIO-DURATION", "0.000000"),
            ] {
                assert_eq!(common::todo(&after).prop(key).unwrap().value, value);
            }
            assert_eq!(
                after.root.prop("PRODID").unwrap().value,
                "-//2DOAPP.COM//2Do 3.8//EN"
            );
        }
    }
    // A later Twydo reorder must preserve the phone-added properties too.
    let before = env.files();
    let mut store = env.store(Matrix::embedded().clone());
    for parent in &parents {
        let s = env.snap();
        let kids = ordered(&s, parent);
        store
            .apply(&s, &reorder(&kids[1], &kids[0], Placement::Before), now())
            .unwrap();
        let cal = twydo_core::ical::parse(&fs::read(env.file(&kids[1])).unwrap()).unwrap();
        assert_eq!(
            common::todo(&cal)
                .prop("X-2DOAPP-SYNC-IMAGE")
                .unwrap()
                .value,
            "1"
        );
    }
    store.undo_last(now()).unwrap();
    store.undo_last(now()).unwrap();
    assert_eq!(env.files(), before);
}
