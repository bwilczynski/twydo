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
fn a_claim_that_was_also_refused_is_dropped() {
    let env = env();
    let mut store = env.store(matrix());
    queued_edit(&env, &mut store, title(A, "renamed"));
    let q = queue(&env);
    let item = q.list().unwrap().remove(0);
    // A crash inside `Queue::refuse`: the refusal was written, the claim not removed.
    assert!(q.claim(&item.id).unwrap());
    q.refuse(&item, "gone", now()).unwrap();
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
