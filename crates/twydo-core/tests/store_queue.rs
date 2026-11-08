//! `Store::submit` and the queue drain: apply when the vdir lock is free, queue while a sync
//! holds it, and commit the queue under the next lock holder. Synthetic fixture content.

mod common;

use std::fs;
use std::time::Instant;

use chrono::{DateTime, TimeZone, Utc};

use twydo_core::compat::Matrix;
use twydo_core::config::WriteMode;
use twydo_core::edit::{Change, NewTask, Op};
use twydo_core::model::Status;
use twydo_core::store::fsio::VdirLock;
use twydo_core::store::journal::{Entry, FORMAT, Journal, Stage, Write};
use twydo_core::store::queue::{self, Queue};
use twydo_core::store::{DrainOutcome, Store, Submitted, WriteError};
use twydo_core::views::Snapshot;

use common::Env;

const A: &str = "0a04f22aeb294b719633c978fe1975e5";
const E: &str = "44123b09407c4ea1adae8f2ecdb64d49";
const B: &str = "41b9cdfea48e46fd9da52f9337c3f6c4";
const CASES: [&str; 5] = ["one-time", "monthly", "date-only", "timed", "start-date"];

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 4, 15, 0, 0).unwrap()
}

fn later() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 4, 15, 5, 0).unwrap()
}

fn matrix() -> Matrix {
    common::matrix(&CASES, "unverified", &[])
}

fn env() -> Env {
    Env::new(WriteMode::Test, "plan4-list1/02-edit")
}

fn title(uid: &str, text: &str) -> Op {
    Op::Edit {
        uid: uid.into(),
        changes: vec![Change::Summary(text.into())],
    }
}

#[test]
fn text_capability_queue_preserves_fields_and_refuses_changed_family() {
    use common::text_edit as text;
    for changed in ["none", "target", "parent", "sibling", "new-member"] {
        let env = text::closed();
        let mut store = env.store(Matrix::embedded().clone());
        let snap = env.snap();
        let p = snap.find(text::PARENT).unwrap();
        let child = snap.tasks[snap.children(p)[0]].uid.clone();
        let sibling = snap.tasks[snap.children(p)[1]].uid.clone();
        let before = env.files();
        let lock = VdirLock::exclusive(&env.state).unwrap();
        let submitted = store
            .submit(&snap, &title(&child, "Queued text"), "text", "op", now())
            .unwrap();
        assert!(matches!(submitted, Submitted::Queued(_)));
        assert_eq!(env.files(), before);
        match changed {
            "target" | "parent" | "sibling" => {
                let uid = match changed {
                    "target" => &child,
                    "parent" => text::PARENT,
                    _ => &sibling,
                };
                text::mutate(&env, uid, |t| {
                    t.prop_mut("SUMMARY").unwrap().set_value("Remote text")
                });
            }
            "new-member" => {
                let source = fs::read_to_string(env.file(&child)).unwrap();
                fs::write(
                    env.file("synthetic-new-member"),
                    source.replace(&format!("UID:{child}"), "UID:synthetic-new-member"),
                )
                .unwrap();
            }
            _ => {}
        }
        let remote = env.files();
        drop(lock);
        let drained = store.drain(later()).unwrap();
        assert_eq!(drained.len(), 1);
        if changed == "none" {
            assert!(matches!(drained[0].outcome, DrainOutcome::Applied(_)));
            let after = env.files();
            let path = format!("list1/{child}.ics");
            text::assert_preserved(&before[&path], &after[&path]);
            assert!(String::from_utf8_lossy(&after[&path]).contains("SUMMARY:Queued text"));
            for (p, bytes) in &before {
                if *p != path {
                    assert_eq!(&after[p], bytes);
                }
            }
            assert!(store.drain(later()).unwrap().is_empty());
            store.undo_last(later()).unwrap();
            assert_eq!(env.files(), before);
        } else {
            assert!(
                matches!(drained[0].outcome, DrainOutcome::Refused(_)),
                "{changed}: {:?}",
                drained[0].outcome
            );
            assert_eq!(env.files(), remote);
        }
    }
}

fn queue(env: &Env) -> Queue {
    Queue::open(&env.state).unwrap()
}

fn read(env: &Env, uid: &str) -> String {
    fs::read_to_string(env.file(uid)).unwrap()
}

#[test]
fn submit_with_free_lock_applies() {
    let env = env();
    let mut store = env.store(matrix());
    let got = store
        .submit(&env.snap(), &title(A, "renamed"), "rename", "op", now())
        .unwrap();
    assert!(matches!(got, Submitted::Applied(_)), "{got:?}");
    assert!(read(&env, A).contains("SUMMARY:renamed"));
    assert!(queue(&env).is_empty().unwrap());
    assert_eq!(env.journal().len(), 1);
}

#[test]
fn submit_while_locked_queues_without_waiting() {
    let env = env();
    let mut store = env.store(matrix());
    let before = read(&env, A);
    let _sync = VdirLock::exclusive(&env.state).unwrap();
    let started = Instant::now();
    let got = store
        .submit(&env.snap(), &title(A, "renamed"), "rename", "op", now())
        .unwrap();
    assert!(started.elapsed().as_secs() < 1, "{:?}", started.elapsed());
    assert!(matches!(got, Submitted::Queued(_)), "{got:?}");
    assert_eq!(read(&env, A), before);
    assert_eq!(queue(&env).len().unwrap(), 1);
}

#[test]
fn next_lock_holder_drains_in_id_order() {
    let env = env();
    let mut first = env.store(matrix());
    let mut second = env.store(matrix());
    let mut third_store = env.store(matrix());
    let snap = env.snap();
    let sync = VdirLock::exclusive(&env.state).unwrap();
    let one = first
        .submit(&snap, &title(A, "from first"), "one", "op", now())
        .unwrap();
    let two = second
        .submit(&snap, &title(E, "from second"), "two", "op", later())
        .unwrap();
    let (Submitted::Queued(one), Submitted::Queued(two)) = (one, two) else {
        panic!("both should queue");
    };
    assert!(one.id < two.id);
    drop(sync);

    // The third write is planned on the files the others have not changed yet; its own task
    // is untouched, so it commits after them.
    let snap = env.snap();
    let third = third_store
        .submit(&snap, &title(B, "from third"), "three", "op", later())
        .unwrap();
    assert!(matches!(third, Submitted::Applied(_)), "{third:?}");
    assert!(read(&env, B).contains("SUMMARY:from third"));
    assert!(read(&env, A).contains("SUMMARY:from first"));
    assert!(read(&env, E).contains("SUMMARY:from second"));
    // Journal ids within one second sort randomly, so compare as sets.
    let journal = env.journal();
    assert_eq!(journal.len(), 3);
    let mut ids: Vec<&str> = journal
        .iter()
        .filter_map(|e| e.from_queue.as_ref().map(|q| q.id.as_str()))
        .collect();
    ids.sort();
    assert_eq!(ids, vec![one.id.as_str(), two.id.as_str()]);
    assert!(queue(&env).is_empty().unwrap());
}

#[test]
fn drain_commits_in_id_order() {
    let env = env();
    let mut first = env.store(matrix());
    let mut second = env.store(matrix());
    let mut drainer = env.store(matrix());
    let snap = env.snap();
    let sync = VdirLock::exclusive(&env.state).unwrap();
    // Queue the later id first: id order, not arrival order, decides.
    let late = second
        .submit(&snap, &title(E, "late"), "late", "op", later())
        .unwrap();
    let early = first
        .submit(&snap, &title(A, "early"), "early", "op", now())
        .unwrap();
    let (Submitted::Queued(late), Submitted::Queued(early)) = (late, early) else {
        panic!("both should queue");
    };
    drop(sync);
    let drained = drainer.drain(later()).unwrap();
    let order: Vec<&str> = drained.iter().map(|d| d.item.id.as_str()).collect();
    assert_eq!(order, vec![early.id.as_str(), late.id.as_str()]);
    assert!(
        drained
            .iter()
            .all(|d| matches!(d.outcome, DrainOutcome::Applied(_)))
    );
}

#[test]
fn lost_wakeup_is_drained_by_the_recheck() {
    let env = env();
    let mut store = env.store(matrix());
    let mut held = Some(VdirLock::exclusive(&env.state).unwrap());
    let got = store
        .submit_with(
            &env.snap(),
            &title(A, "renamed"),
            "rename",
            "op",
            now(),
            &mut || {
                held.take();
            },
        )
        .unwrap();
    assert!(matches!(got, Submitted::Applied(_)), "{got:?}");
    assert!(read(&env, A).contains("SUMMARY:renamed"));
    assert!(queue(&env).is_empty().unwrap());
}

/// Queues one edit of `A` while a sync holds the lock, then lets the sync end.
fn queued_edit(env: &Env, store: &mut Store, op: Op) -> String {
    let sync = VdirLock::exclusive(&env.state).unwrap();
    let got = store.submit(&env.snap(), &op, "edit", "op", now()).unwrap();
    drop(sync);
    let Submitted::Queued(item) = got else {
        panic!("should queue");
    };
    item.id
}

#[test]
fn crash_after_apply_is_not_applied_twice() {
    let env = env();
    let mut store = env.store(matrix());
    let id = queued_edit(&env, &mut store, title(A, "renamed"));
    let item = queue(&env).list().unwrap().remove(0);
    let drained = store.drain(later()).unwrap();
    assert!(matches!(drained[0].outcome, DrainOutcome::Applied(_)));
    let applied = read(&env, A);

    // A crash before the claim was removed.
    let q = queue(&env);
    q.push(&item).unwrap();
    assert!(q.claim(&id).unwrap());
    let drained = store.drain(later()).unwrap();
    assert!(matches!(
        drained[0].outcome,
        DrainOutcome::AlreadyApplied(_)
    ));
    assert_eq!(read(&env, A), applied);
    assert_eq!(env.journal().len(), 1);
    assert!(q.is_empty().unwrap());
}

#[test]
fn io_failure_keeps_the_claim_until_recovery_finishes() {
    let env = env();
    let mut store = env.store(matrix());
    let sync = VdirLock::shared(&env.state).unwrap();
    for text in ["first", "latest"] {
        store
            .submit(&view(&env), &title(A, text), text, "op", now())
            .unwrap();
    }
    drop(sync);
    let before = read(&env, A);
    // Fail the vdir write after its journal entry is durable.
    let blocker = env.file(A).with_file_name(format!(".{A}.ics.tdo-tmp"));
    fs::create_dir(&blocker).unwrap();
    for _ in 0..2 {
        assert!(matches!(store.drain(later()), Err(WriteError::Io(_))));
        assert_eq!(queue(&env).len().unwrap(), 2);
        assert!(queue(&env).unshown().unwrap().is_empty());
        assert_eq!(read(&env, A), before);
        assert_eq!(env.journal()[0].stage, Stage::Prepared);
    }

    fs::remove_dir(&blocker).unwrap();
    let drained = store.drain(later()).unwrap();
    assert_eq!(drained.len(), 2);
    assert!(
        drained
            .iter()
            .all(|d| matches!(d.outcome, DrainOutcome::Applied(_))),
        "{drained:?}"
    );
    assert!(read(&env, A).contains("SUMMARY:latest"));
    assert!(queue(&env).is_empty().unwrap());
    assert!(queue(&env).unshown().unwrap().is_empty());
    assert_eq!(env.journal().len(), 2);
    assert!(env.journal().iter().all(|e| e.stage == Stage::Applied));
}

#[test]
fn io_failure_after_a_successful_item_stops_the_drain_without_refusing() {
    let env = env();
    let mut store = env.store(matrix());
    let sync = VdirLock::shared(&env.state).unwrap();
    store
        .submit(&view(&env), &title(E, "first"), "first", "op", now())
        .unwrap();
    store
        .submit(&view(&env), &title(A, "second"), "second", "op", later())
        .unwrap();
    drop(sync);
    let blocker = env.file(A).with_file_name(format!(".{A}.ics.tdo-tmp"));
    fs::create_dir(&blocker).unwrap();
    let drained = store.drain(later()).unwrap();
    assert_eq!(drained.len(), 1, "the failed item stays pending");
    assert!(matches!(drained[0].outcome, DrainOutcome::Applied(_)));
    assert_eq!(queue(&env).len().unwrap(), 1);
    assert!(queue(&env).unshown().unwrap().is_empty());
}

#[test]
fn a_claim_that_was_also_refused_is_dropped() {
    let env = env();
    let mut store = env.store(matrix());
    queued_edit(&env, &mut store, title(A, "renamed"));
    let q = queue(&env);
    let item = q.list().unwrap().remove(0);
    // A crash inside `Queue::refuse`: the refusal was written, the claim not removed.
    assert!(q.claim(&item.id).unwrap());
    q.refuse(&item, "gone", false, now()).unwrap();
    q.push(&item).unwrap();
    assert!(q.claim(&item.id).unwrap());
    let before = read(&env, A);
    assert!(store.drain(later()).unwrap().is_empty());
    assert_eq!(read(&env, A), before);
    assert!(q.is_empty().unwrap());
    assert_eq!(q.unshown().unwrap().len(), 1);
}

#[test]
fn drain_regates_on_current_files() {
    let env = env();
    let mut store = env.store(matrix());
    queued_edit(&env, &mut store, Op::Complete { uid: A.into() });
    let text = read(&env, A).replace("END:VTODO", "RRULE:FREQ=BOGUS\r\nEND:VTODO");
    fs::write(env.file(A), &text).unwrap();
    let drained = store.drain(later()).unwrap();
    let DrainOutcome::Refused(reason) = &drained[0].outcome else {
        panic!("{drained:?}");
    };
    assert!(reason.contains("unsupported repeat rule"), "{reason}");
    assert_eq!(read(&env, A), text);
    assert!(queue(&env).is_empty().unwrap());
    assert_eq!(queue(&env).unshown().unwrap().len(), 1);
}

#[test]
fn stale_queued_edit_is_refused() {
    let env = env();
    let mut store = env.store(matrix());
    queued_edit(&env, &mut store, title(A, "renamed"));
    let changed = read(&env, A).replace("DESCRIPTION:", "DESCRIPTION:changed elsewhere");
    fs::write(env.file(A), &changed).unwrap();
    let drained = store.drain(later()).unwrap();
    let DrainOutcome::Refused(reason) = &drained[0].outcome else {
        panic!("{drained:?}");
    };
    assert!(reason.contains("changed on disk"), "{reason}");
    assert_eq!(read(&env, A), changed);
    assert_eq!(queue(&env).unshown().unwrap().len(), 1);
}

#[test]
fn own_refusal_is_returned_to_the_submitter() {
    let env = env();
    let mut store = env.store(matrix());
    let snap = env.snap();
    // The file changes after the view was loaded: the direct path is stale too.
    fs::write(
        env.file(A),
        read(&env, A).replace("DESCRIPTION:", "DESCRIPTION:x"),
    )
    .unwrap();
    let got = store.submit(&snap, &title(A, "renamed"), "rename", "op", now());
    assert!(matches!(got, Err(WriteError::Stale(_))), "{got:?}");
}

/// An unfinished entry whose file does not hold what it expects can never finish.
fn make_stuck(env: &Env) {
    let journal = Journal::open(&env.state).unwrap();
    journal
        .prepare(&Entry {
            format: FORMAT,
            id: "20261104T145900Z-stuck".to_string(),
            created: now(),
            kind: "edit".to_string(),
            uids: vec![E.to_string()],
            stage: Stage::Prepared,
            inverse_of: None,
            from_queue: None,
            synced: None,
            holds: Vec::new(),
            family: None,
            families: Vec::new(),
            writes: vec![Write {
                path: format!("list1/{E}.ics"),
                before: Some("not what is on disk".to_string()),
                after: Some("whatever".to_string()),
            }],
        })
        .unwrap();
}

#[test]
fn stuck_journal_stops_the_drain() {
    let env = env();
    let mut store = env.store(matrix());
    queued_edit(&env, &mut store, title(A, "renamed"));
    make_stuck(&env);
    let before = read(&env, A);
    let got = store.drain(later());
    assert!(matches!(got, Err(WriteError::Stuck(_))), "{got:?}");
    assert_eq!(read(&env, A), before);
    assert_eq!(queue(&env).len().unwrap(), 1);
}

#[test]
fn submit_with_stuck_older_item_stays_queued() {
    let env = env();
    let mut older = env.store(matrix());
    let mut store = env.store(matrix());
    let older_id = queued_edit(&env, &mut older, title(A, "older"));
    make_stuck(&env);
    let before = read(&env, B);
    let mut held = Some(VdirLock::exclusive(&env.state).unwrap());
    let got = store
        .submit_with(
            &env.snap(),
            &title(B, "ours"),
            "ours",
            "op",
            later(),
            &mut || {
                held.take();
            },
        )
        .unwrap();
    let Submitted::Queued(item) = got else {
        panic!("{got:?}");
    };
    assert_eq!(read(&env, B), before);
    let ids: Vec<String> = queue(&env)
        .list()
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(ids, vec![older_id, item.id]);
}

#[test]
fn submit_whose_own_item_hits_stuck_stays_queued() {
    let env = env();
    let mut store = env.store(matrix());
    make_stuck(&env);
    let before = read(&env, A);
    let mut held = Some(VdirLock::exclusive(&env.state).unwrap());
    let got = store
        .submit_with(
            &env.snap(),
            &title(A, "ours"),
            "ours",
            "op",
            now(),
            &mut || {
                held.take();
            },
        )
        .unwrap();
    let Submitted::Queued(item) = got else {
        panic!("{got:?}");
    };
    assert_eq!(read(&env, A), before);
    let listed = queue(&env).list().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, item.id);
    assert!(env.journal().iter().all(|e| e.from_queue.is_none()));
}

/// The overlaid view of the vdir, as the TUI builds it.
fn view(env: &Env) -> Snapshot {
    let report = twydo_core::vdir::load(&env.vdir).unwrap();
    queue::view(report, env.cfg.clone())
}

fn idx(snap: &Snapshot, uid: &str) -> usize {
    snap.find(uid)
        .unwrap_or_else(|| panic!("{uid} not in view"))
}

#[test]
fn queued_completion_stays_completed_until_applied() {
    let env = env();
    let mut store = env.store(matrix());
    let sync = VdirLock::exclusive(&env.state).unwrap();
    let got = store
        .submit(
            &view(&env),
            &Op::Complete { uid: A.into() },
            "done",
            "op",
            now(),
        )
        .unwrap();
    assert!(matches!(got, Submitted::Queued(_)), "{got:?}");
    for _ in 0..2 {
        let snap = view(&env);
        let i = idx(&snap, A);
        assert_eq!(snap.tasks[i].status, Status::Completed);
        assert!(snap.is_pending(i));
    }
    drop(sync);
    store.drain(later()).unwrap();
    let snap = view(&env);
    let i = idx(&snap, A);
    assert_eq!(snap.tasks[i].status, Status::Completed);
    assert!(!snap.is_pending(i));
}

#[test]
fn refused_item_returns_to_the_file() {
    let env = env();
    let mut store = env.store(matrix());
    let sync = VdirLock::exclusive(&env.state).unwrap();
    store
        .submit(
            &view(&env),
            &title(A, "queued title"),
            "rename",
            "op",
            now(),
        )
        .unwrap();
    let snap = view(&env);
    assert_eq!(snap.tasks[idx(&snap, A)].summary, "queued title");
    // Another writer changes the file: the item can no longer apply.
    let text = read(&env, A).replace("SUMMARY:", "SUMMARY:other ");
    fs::write(env.file(A), &text).unwrap();
    let snap = view(&env);
    let i = idx(&snap, A);
    assert!(
        snap.tasks[i].summary.starts_with("other "),
        "{}",
        snap.tasks[i].summary
    );
    assert!(!snap.is_pending(i));
    drop(sync);
}

#[test]
fn second_queued_edit_builds_on_the_first() {
    let env = env();
    let mut store = env.store(matrix());
    let sync = VdirLock::exclusive(&env.state).unwrap();
    let Submitted::Queued(one) = store
        .submit(&view(&env), &title(A, "first"), "rename", "op", now())
        .unwrap()
    else {
        panic!("queue");
    };
    let Submitted::Queued(two) = store
        .submit(
            &view(&env),
            &Op::Complete { uid: A.into() },
            "done",
            "op",
            later(),
        )
        .unwrap()
    else {
        panic!("queue");
    };
    drop(sync);
    let drained = store.drain(later()).unwrap();
    assert_eq!(drained.len(), 2);
    assert!(
        drained
            .iter()
            .all(|d| matches!(d.outcome, DrainOutcome::Applied(_))),
        "{drained:?}"
    );
    let text = read(&env, A);
    assert!(text.contains("SUMMARY:first"));
    assert!(text.contains("STATUS:COMPLETED"));
    assert_eq!(
        two.writes()[0].before,
        one.writes()[0].after,
        "the second edit is planned on the first's result"
    );
}

#[test]
fn drained_edits_preserve_the_latest_version_in_conflicts() {
    let env = env();
    let mut store = env.store(matrix());
    let sync = VdirLock::shared(&env.state).unwrap();
    for text in ["first", "latest"] {
        store
            .submit(&view(&env), &title(A, text), text, "op", now())
            .unwrap();
    }
    drop(sync);
    let drained = store.drain(later()).unwrap();
    let journal = Journal::open(&env.state).unwrap();
    let entries: Vec<_> = drained
        .iter()
        .map(|d| {
            let DrainOutcome::Applied(id) = &d.outcome else {
                panic!("{d:?}")
            };
            journal.get(id).unwrap().unwrap()
        })
        .collect();
    assert!(
        entries[0].created < entries[1].created,
        "commit times must preserve drain order"
    );

    // Another writer uses the same clock reading after the drain.
    let mut next = env.store(matrix());
    let Submitted::Applied(id) = next
        .submit(&env.snap(), &title(A, "newest"), "newest", "op", later())
        .unwrap()
    else {
        panic!("free lock")
    };
    assert!(entries[1].created < journal.get(&id).unwrap().unwrap().created);

    let conflicts = twydo_core::conflicts::ConflictStore::open(&env.state).unwrap();
    let output = twydo_core::sync::output::parse(
        "twydo",
        &format!("Syncing twydo/list1\nCopying (uploading) item {A} to twydo_remote/list1\n"),
    );
    let states = [("list1".to_string(), twydo_core::sync::status::CollState::Ok)]
        .into_iter()
        .collect();
    twydo_core::sync::reconcile::reconcile(
        &env.vdir,
        &journal,
        &conflicts,
        &Default::default(),
        &states,
        &journal.pending().unwrap(),
        &output,
        &[],
        later(),
    )
    .unwrap();
    let recorded = conflicts.list().unwrap();
    assert_eq!(recorded.len(), 1);
    assert!(
        recorded[0]
            .local
            .as_ref()
            .unwrap()
            .contains("SUMMARY:newest")
    );
}

#[test]
fn queued_create_keeps_its_uid() {
    let env = env();
    let mut store = env.store(matrix());
    let sync = VdirLock::exclusive(&env.state).unwrap();
    let before = env.files().len();
    let create = Op::Create(NewTask::plain("list1", "brand new"));
    store
        .submit(&view(&env), &create, "add", "op", now())
        .unwrap();
    let snap = view(&env);
    let i = snap
        .tasks
        .iter()
        .position(|t| t.summary == "brand new")
        .expect("the queued create shows");
    assert!(snap.is_pending(i));
    let uid = snap.tasks[i].uid.clone();
    let got = store
        .submit(&snap, &title(&uid, "renamed new"), "rename", "op", later())
        .unwrap();
    assert!(matches!(got, Submitted::Queued(_)), "{got:?}");
    drop(sync);
    let drained = store.drain(later()).unwrap();
    assert!(
        drained
            .iter()
            .all(|d| matches!(d.outcome, DrainOutcome::Applied(_))),
        "{drained:?}"
    );
    assert_eq!(env.files().len(), before + 1);
    let snap = view(&env);
    let i = idx(&snap, &uid);
    assert_eq!(snap.tasks[i].summary, "renamed new");
    assert_eq!(snap.tasks.iter().filter(|t| t.uid == uid).count(), 1);
}

#[test]
fn summary_counts_queued_items() {
    let env = env();
    let mut cfg = env.cfg.clone();
    cfg.sync = Some(Default::default());
    let mut store = env.store(matrix());
    assert_eq!(twydo_core::sync::summary(&cfg).unwrap().unwrap().queued, 0);
    let _sync = VdirLock::exclusive(&env.state).unwrap();
    store
        .submit(&env.snap(), &title(A, "x"), "rename", "op", now())
        .unwrap();
    assert_eq!(twydo_core::sync::summary(&cfg).unwrap().unwrap().queued, 1);
}

#[test]
fn view_hides_an_applied_item_whose_claim_is_still_there() {
    let env = env();
    let mut store = env.store(matrix());
    let id = queued_edit(&env, &mut store, title(A, "renamed"));
    let item = queue(&env).list().unwrap().remove(0);
    store.drain(later()).unwrap();
    // A crash left the claim of an applied item.
    let q = queue(&env);
    q.push(&item).unwrap();
    assert!(q.claim(&id).unwrap());
    let snap = view(&env);
    assert!(!snap.is_pending(idx(&snap, A)));
}

// Undo and redo of queued items.

#[test]
fn undo_of_a_queued_item_removes_it() {
    let env = env();
    let mut store = env.store(matrix());
    let before = read(&env, A);
    queued_edit(&env, &mut store, title(A, "renamed"));
    assert_eq!(queue(&env).len().unwrap(), 1);
    let got = store.undo_last(later()).unwrap();
    assert!(matches!(got, Submitted::Cancelled), "{got:?}");
    assert!(queue(&env).is_empty().unwrap());
    assert_eq!(read(&env, A), before);
    let snap = view(&env);
    assert!(!snap.is_pending(idx(&snap, A)));
    assert!(env.journal().is_empty());
}

#[test]
fn redo_requeues_it() {
    let env = env();
    let mut store = env.store(matrix());
    let sync = VdirLock::exclusive(&env.state).unwrap();
    let Submitted::Queued(item) = store
        .submit(&env.snap(), &title(A, "renamed"), "edit", "op", now())
        .unwrap()
    else {
        panic!("should queue");
    };
    store.undo_last(later()).unwrap();
    assert!(queue(&env).is_empty().unwrap());
    // The sync still runs: the redo puts the same item back.
    let Submitted::Queued(again) = store.redo_last(later()).unwrap() else {
        panic!("should queue");
    };
    assert_eq!(again.id, item.id);
    assert_eq!(queue(&env).len().unwrap(), 1);
    let snap = view(&env);
    assert!(snap.is_pending(idx(&snap, A)));
    drop(sync);
    store.drain(later()).unwrap();
    assert!(read(&env, A).contains("SUMMARY:renamed"));
    let journal = env.journal();
    assert_eq!(journal.len(), 1);
    assert_eq!(journal[0].from_queue.as_ref().unwrap().id, item.id);
}

#[test]
fn redo_with_a_free_lock_applies_it() {
    let env = env();
    let mut store = env.store(matrix());
    queued_edit(&env, &mut store, title(A, "renamed"));
    store.undo_last(later()).unwrap();
    let got = store.redo_last(later()).unwrap();
    assert!(matches!(got, Submitted::Applied(_)), "{got:?}");
    assert!(read(&env, A).contains("SUMMARY:renamed"));
}

#[test]
fn undo_after_drain_reverts_the_entry() {
    let env = env();
    let mut store = env.store(matrix());
    let before = read(&env, A);
    queued_edit(&env, &mut store, title(A, "renamed"));
    store.drain(later()).unwrap();
    assert!(read(&env, A).contains("SUMMARY:renamed"));
    let got = store.undo_last(later()).unwrap();
    assert!(matches!(got, Submitted::Applied(_)), "{got:?}");
    assert_eq!(read(&env, A), before);
}

#[test]
fn undo_after_claim_reverts_the_entry() {
    let env = env();
    let mut store = env.store(matrix());
    let before = read(&env, A);
    let id = queued_edit(&env, &mut store, title(A, "renamed"));
    let item = queue(&env).list().unwrap().remove(0);
    // A drainer mid-commit has claimed the item but not yet written its entry.
    assert!(queue(&env).claim(&id).unwrap());
    assert!(matches!(store.undo_last(later()), Err(WriteError::Busy)));
    assert_eq!(read(&env, A), before);
    // The step stayed on the stack. Let the drainer finish (as a recovery drain would).
    queue(&env).done(&id).unwrap();
    queue(&env).push(&item).unwrap();
    store.drain(later()).unwrap();
    assert!(read(&env, A).contains("SUMMARY:renamed"));
    let got = store.undo_last(later()).unwrap();
    assert!(matches!(got, Submitted::Applied(_)), "{got:?}");
    assert_eq!(read(&env, A), before);
}

#[test]
fn undo_while_locked_queues_a_revert() {
    let env = env();
    let mut store = env.store(matrix());
    let before = read(&env, A);
    let applied = store
        .submit(&env.snap(), &title(A, "renamed"), "edit", "op", now())
        .unwrap();
    let Submitted::Applied(entry_id) = applied else {
        panic!("the lock is free");
    };
    let sync = VdirLock::exclusive(&env.state).unwrap();
    let started = Instant::now();
    let got = store.undo_last(later()).unwrap();
    assert!(started.elapsed().as_secs() < 1, "{:?}", started.elapsed());
    let Submitted::Queued(item) = got else {
        panic!("{got:?}");
    };
    assert!(read(&env, A).contains("SUMMARY:renamed"));
    let snap = view(&env);
    assert_eq!(snap.tasks[idx(&snap, A)].summary, "A2");
    assert!(snap.is_pending(idx(&snap, A)));
    drop(sync);
    store.drain(later()).unwrap();
    assert_eq!(read(&env, A), before);
    let journal = env.journal();
    let revert = journal
        .iter()
        .find(|e| e.inverse_of.as_deref() == Some(entry_id.as_str()))
        .expect("a revert entry");
    assert_eq!(revert.from_queue.as_ref().unwrap().id, item.id);
}

#[test]
fn queued_revert_is_refused_if_the_file_changed() {
    let env = env();
    let mut store = env.store(matrix());
    store
        .submit(&env.snap(), &title(A, "renamed"), "edit", "op", now())
        .unwrap();
    let sync = VdirLock::exclusive(&env.state).unwrap();
    let got = store.undo_last(later()).unwrap();
    assert!(matches!(got, Submitted::Queued(_)), "{got:?}");
    // Another device changed the file while the sync ran.
    let changed = read(&env, A).replace("SUMMARY:renamed", "SUMMARY:elsewhere");
    fs::write(env.file(A), &changed).unwrap();
    drop(sync);
    let drained = store.drain(later()).unwrap();
    assert!(
        matches!(drained[0].outcome, DrainOutcome::Refused(_)),
        "{:?}",
        drained[0].outcome
    );
    assert_eq!(read(&env, A), changed);
    assert!(queue(&env).is_empty().unwrap());
}

#[test]
fn undo_skips_a_refused_queued_step() {
    let env = env();
    let mut store = env.store(matrix());
    let before = read(&env, A);
    store
        .submit(&env.snap(), &title(A, "first"), "edit", "op", now())
        .unwrap();
    let sync = VdirLock::exclusive(&env.state).unwrap();
    let got = store
        .submit(&env.snap(), &title(E, "second"), "edit", "op", now())
        .unwrap();
    assert!(matches!(got, Submitted::Queued(_)), "{got:?}");
    // Another device changes E while the sync runs: the drain refuses the queued edit.
    let changed = read(&env, E).replace("SUMMARY:", "SUMMARY:elsewhere ");
    fs::write(env.file(E), &changed).unwrap();
    drop(sync);
    let drained = store.drain(later()).unwrap();
    assert!(matches!(drained[0].outcome, DrainOutcome::Refused(_)));
    let got = store.undo_last(later()).unwrap();
    assert!(matches!(got, Submitted::Applied(_)), "{got:?}");
    assert_eq!(read(&env, A), before);
    assert_eq!(read(&env, E), changed);
    assert!(matches!(
        store.undo_last(later()),
        Err(WriteError::NothingToUndo)
    ));
}

#[test]
fn individual_item_deletes_queue_and_refuse_changed_family_bytes() {
    use common::item_deletion as evidence;
    for prefix in ["c", "p"] {
        for changed in [false, true] {
            let e = evidence::env(evidence::CLOSED);
            let mut store = e.store(evidence::matrix("pass"));
            let parent = format!("twydo-i27-{prefix}-a");
            let uid = format!("{parent}-x");
            let snap = e.snap();
            let lock = VdirLock::exclusive(&e.state).unwrap();
            let queued = store
                .submit(
                    &snap,
                    &Op::Delete { uid: uid.clone() },
                    "delete item",
                    "op",
                    now(),
                )
                .unwrap();
            assert!(matches!(queued, Submitted::Queued(_)));
            if changed {
                let path = e.file(&parent);
                let bytes = fs::read_to_string(&path)
                    .unwrap()
                    .replace("DESCRIPTION:x", "DESCRIPTION:remote synthetic");
                fs::write(path, bytes).unwrap();
            }
            let before = e.files();
            drop(lock);
            let result = store.drain(later()).unwrap();
            assert_eq!(result.len(), 1);
            if changed {
                assert!(matches!(result[0].outcome, DrainOutcome::Refused(_)));
                assert_eq!(e.files(), before);
                assert!(e.journal().is_empty());
            } else {
                assert!(matches!(result[0].outcome, DrainOutcome::Applied(_)));
                let mut expected = before;
                expected.remove(&format!("list1/{uid}.ics"));
                assert_eq!(e.files(), expected);
                store.undo_last(later()).unwrap();
                assert!(e.file(&uid).exists());
            }
            assert!(Queue::open(&e.state).unwrap().is_empty().unwrap());
        }
    }
}

#[test]
fn capability_move_queue_reuses_prepared_identities() {
    let env = Env::new(WriteMode::Verified, "plan6-list1/04-reopen-child");
    let mut store = env.store(Matrix::embedded().clone());
    let sync = VdirLock::exclusive(&env.state).unwrap();
    let result = store
        .submit(
            &env.snap(),
            &Op::Move {
                uid: "09c49c2cd76f483498fc2ff3a0f66160".into(),
                to: "list2".into(),
            },
            "move",
            "op",
            now(),
        )
        .unwrap();
    let Submitted::Queued(item) = result else {
        panic!("expected queue");
    };
    let writes = item.writes().to_vec();
    drop(sync);
    assert!(matches!(
        store.drain(later()).unwrap()[0].outcome,
        DrainOutcome::Applied(_)
    ));
    assert_eq!(env.journal()[0].writes, writes);
    assert!(store.drain(later()).unwrap().is_empty());
    store.undo_last(later()).unwrap();
    assert!(env.file("09c49c2cd76f483498fc2ff3a0f66160").exists());
}

#[test]
fn capability_move_stale_family_and_destination_refuse() {
    for change in [
        "destination-path",
        "destination-uid",
        "source-edit",
        "source-remove",
        "source-join",
    ] {
        let env = Env::new(WriteMode::Verified, "plan6-list1/04-reopen-child");
        let mut store = env.store(Matrix::embedded().clone());
        let sync = VdirLock::exclusive(&env.state).unwrap();
        let Submitted::Queued(item) = store
            .submit(
                &env.snap(),
                &Op::Move {
                    uid: "09c49c2cd76f483498fc2ff3a0f66160".into(),
                    to: "list2".into(),
                },
                "move",
                "op",
                now(),
            )
            .unwrap()
        else {
            panic!("expected queue");
        };
        let writes = item.writes();
        match change {
            "destination-path" => {
                fs::write(env.vdir.join(&writes[0].path), "synthetic collision").unwrap()
            }
            "destination-uid" => fs::write(
                env.vdir.join("list2/remote-collision.ics"),
                writes[0].after.as_ref().unwrap(),
            )
            .unwrap(),
            "source-edit" => fs::write(
                env.file("e668818024cc48f4a404fb9ced368789"),
                "synthetic remote edit",
            )
            .unwrap(),
            "source-remove" => {
                fs::remove_file(env.file("e668818024cc48f4a404fb9ced368789")).unwrap()
            }
            _ => {
                let bytes = fs::read_to_string(env.file("e668818024cc48f4a404fb9ced368789"))
                    .unwrap()
                    .replace("e668818024cc48f4a404fb9ced368789", "synthetic-new-member");
                fs::write(env.vdir.join("list1/remote.ics"), bytes).unwrap();
            }
        }
        let before = env.files();
        drop(sync);
        assert!(
            matches!(
                store.drain(later()).unwrap()[0].outcome,
                DrainOutcome::Refused(_)
            ),
            "{change}"
        );
        assert_eq!(env.files(), before, "{change}");
    }
}
