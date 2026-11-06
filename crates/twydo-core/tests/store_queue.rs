//! `Store::submit` and the queue drain: apply when the vdir lock is free, queue while a sync
//! holds it, and commit the queue under the next lock holder. Synthetic fixture content.

mod common;

use std::fs;
use std::time::Instant;

use chrono::{DateTime, TimeZone, Utc};

use twydo_core::compat::Matrix;
use twydo_core::config::WriteMode;
use twydo_core::edit::{Change, Op};
use twydo_core::store::fsio::VdirLock;
use twydo_core::store::journal::{Entry, FORMAT, Journal, Stage, Write};
use twydo_core::store::queue::Queue;
use twydo_core::store::{DrainOutcome, Store, Submitted, WriteError};

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

#[test]
fn stuck_journal_stops_the_drain() {
    let env = env();
    let mut store = env.store(matrix());
    queued_edit(&env, &mut store, title(A, "renamed"));
    // An unfinished entry whose file does not hold what it expects can never finish.
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
    let before = read(&env, A);
    let got = store.drain(later());
    assert!(matches!(got, Err(WriteError::Stuck(_))), "{got:?}");
    assert_eq!(read(&env, A), before);
    assert_eq!(queue(&env).len().unwrap(), 1);
}
