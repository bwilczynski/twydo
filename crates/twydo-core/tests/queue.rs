use chrono::{DateTime, Duration, TimeZone, Utc};
use twydo_core::edit::{Change, Due, NewTask, Op};
use twydo_core::model::repeat::parse_rrule;
use twydo_core::repeatinput::RepeatSpec;
use twydo_core::store::journal::{Entry, FromQueue, Journal, Stage, Write};
use twydo_core::store::queue::{FORMAT, Item, Kind, Prepared, Queue, new_id, origin};

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 6, 12, 0, 0).unwrap()
}

fn entry(id: &str) -> Entry {
    Entry {
        format: 1,
        id: id.to_string(),
        created: t0(),
        kind: "edit".to_string(),
        uids: vec!["uid-a".to_string()],
        stage: Stage::Prepared,
        inverse_of: None,
        from_queue: None,
        synced: None,
        holds: Vec::new(),
        family: None,
        writes: vec![Write {
            path: "work/a.ics".to_string(),
            before: Some("old".to_string()),
            after: Some("new".to_string()),
        }],
    }
}

fn item(op: Op, now: DateTime<Utc>) -> Item {
    Item {
        format: FORMAT,
        id: new_id(now),
        created: now,
        origin: origin("op"),
        label: "a task".to_string(),
        kind: Kind::Op {
            op,
            prepared: Prepared {
                entry: entry("x"),
                holds: vec![("work/a.ics".to_string(), "old".to_string())],
                family: None,
            },
        },
    }
}

fn simple(now: DateTime<Utc>) -> Item {
    item(
        Op::Complete {
            uid: "uid-a".to_string(),
        },
        now,
    )
}

#[test]
fn ids_sort_in_creation_order() {
    let ids: Vec<String> = (0..1000).map(|_| new_id(t0())).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]));
    let later = new_id(t0() + Duration::microseconds(1));
    assert!(ids.iter().all(|i| *i < later));
}

#[test]
fn item_roundtrips() {
    let dir = tempfile::tempdir().unwrap();
    let q = Queue::open(dir.path()).unwrap();
    let spec = RepeatSpec {
        rule: parse_rrule("FREQ=WEEKLY;BYDAY=MO,TH;COUNT=3").unwrap(),
        from_completion: true,
    };
    let save = item(
        Op::Save {
            uid: "uid-a".to_string(),
            changes: vec![
                Change::Due(Some(Due::Date(t0().date_naive()))),
                Change::Repeat(Some(spec.clone())),
            ],
            to: Some("home".to_string()),
        },
        t0(),
    );
    let mut nt = NewTask::plain("work", "child");
    nt.parent = Some("uid-p".to_string());
    nt.repeat = Some(spec);
    nt.due = Some(Due::Time(t0().naive_utc()));
    let create = item(Op::Create(nt), t0() + Duration::microseconds(1));
    let revert = Item {
        id: new_id(t0() + Duration::microseconds(2)),
        kind: Kind::Revert {
            entry: "e1".to_string(),
            writes: entry("x").writes,
        },
        ..simple(t0())
    };
    for i in [&save, &create, &revert] {
        q.push(i).unwrap();
    }
    assert_eq!(q.list().unwrap(), vec![save, create, revert.clone()]);
    assert_eq!(revert.writes().len(), 1);
    assert_eq!(q.len().unwrap(), 3);
}

#[test]
fn old_journal_entries_load_without_from_queue() {
    let mut v = serde_json::to_value(entry("e1")).unwrap();
    assert!(v.as_object().unwrap().get("from_queue").is_none());
    let e: Entry = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(e.from_queue, None);
    v["from_queue"] = serde_json::json!({"id": "q1", "queued": "2026-11-06T12:00:00Z"});
    let e: Entry = serde_json::from_value(v).unwrap();
    assert_eq!(e.from_queue.unwrap().id, "q1");
}

#[test]
fn journal_finds_entry_by_queue_id() {
    let dir = tempfile::tempdir().unwrap();
    let j = Journal::open(dir.path()).unwrap();
    j.prepare(&entry("e1")).unwrap();
    let mut e2 = entry("e2");
    e2.from_queue = Some(FromQueue {
        id: "q1".to_string(),
        queued: t0(),
    });
    j.prepare(&e2).unwrap();
    assert_eq!(j.by_queue_id("q1").unwrap().unwrap().id, "e2");
    assert!(j.by_queue_id("q2").unwrap().is_none());
}

#[test]
fn claim_then_cancel_loses() {
    let dir = tempfile::tempdir().unwrap();
    let q = Queue::open(dir.path()).unwrap();
    let i = simple(t0());
    q.push(&i).unwrap();
    assert!(q.claim(&i.id).unwrap());
    assert!(!q.cancel(&i.id).unwrap());
    assert!(q.claim(&i.id).unwrap(), "claiming again stays true");
}

#[test]
fn cancel_then_claim_loses() {
    let dir = tempfile::tempdir().unwrap();
    let q = Queue::open(dir.path()).unwrap();
    let i = simple(t0());
    q.push(&i).unwrap();
    assert!(q.cancel(&i.id).unwrap());
    assert!(!q.claim(&i.id).unwrap());
    assert!(q.list().unwrap().is_empty());
}

#[test]
fn list_includes_claimed_items() {
    let dir = tempfile::tempdir().unwrap();
    let q = Queue::open(dir.path()).unwrap();
    let a = simple(t0());
    let b = simple(t0());
    q.push(&a).unwrap();
    q.push(&b).unwrap();
    q.claim(&a.id).unwrap();
    std::fs::write(dir.path().join("queue/garbage.json"), "{").unwrap();
    let ids: Vec<_> = q.list().unwrap().into_iter().map(|i| i.id).collect();
    assert_eq!(ids, vec![a.id.clone(), b.id]);
    q.done(&a.id).unwrap();
    assert_eq!(q.len().unwrap(), 1);
}

#[test]
fn existing_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    assert!(Queue::existing(dir.path()).is_none());
    assert!(!dir.path().join("queue").exists());
    Queue::open(dir.path()).unwrap();
    assert!(Queue::existing(dir.path()).is_some());
}

#[test]
fn refuse_moves_to_refused_and_is_unshown_until_marked() {
    let dir = tempfile::tempdir().unwrap();
    let q = Queue::open(dir.path()).unwrap();
    let i = simple(t0());
    q.push(&i).unwrap();
    assert!(q.claim(&i.id).unwrap());
    q.refuse(&i, "stale", false, t0()).unwrap();
    assert!(q.list().unwrap().is_empty());
    let u = q.unshown().unwrap();
    assert_eq!(u.len(), 1);
    assert_eq!(u[0].item, i);
    assert_eq!(u[0].reason, "stale");
    q.mark_shown(&i.id).unwrap();
    assert!(q.unshown().unwrap().is_empty());
}

#[test]
fn prune_drops_old_refusals() {
    let dir = tempfile::tempdir().unwrap();
    let q = Queue::open(dir.path()).unwrap();
    let old = simple(t0());
    let new = simple(t0());
    for (i, at) in [(&old, t0()), (&new, t0() + Duration::days(2))] {
        q.push(i).unwrap();
        q.claim(&i.id).unwrap();
        q.refuse(i, "r", false, at).unwrap();
    }
    assert_eq!(q.prune(t0() + Duration::days(1)).unwrap(), 1);
    let left = q.unshown().unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].item.id, new.id);
}

#[test]
fn refusal_message_names_the_task_and_the_reason() {
    let dir = tempfile::tempdir().unwrap();
    let q = Queue::open(dir.path()).unwrap();
    for (stale, want) in [
        (true, "Not saved: \"a task\" changed on another device"),
        (false, "Not saved: \"a task\": nope"),
    ] {
        let i = simple(t0());
        q.push(&i).unwrap();
        q.claim(&i.id).unwrap();
        q.refuse(&i, "nope", stale, t0()).unwrap();
        let r = q.unshown().unwrap().pop().unwrap();
        assert_eq!(twydo_core::store::queue::refusal_message(&r), want);
        q.mark_shown(&i.id).unwrap();
    }
}
