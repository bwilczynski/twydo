//! Task details wording shared by the TUI details pane and the daemon's `tasks.get`.

use chrono::TimeZone;
use chrono_tz::Tz;

use crate::format::describe_alarm;
use crate::ical::value::Resolved;
use crate::model::repeat::{describe, parse_rrule};
use crate::model::{Status, Task, TaskKind};
use crate::views::{Snapshot, compare_items};

/// The type a task shows: its own, except that a plain task with items shows as a project.
pub fn shown_kind(snap: &Snapshot, idx: usize) -> TaskKind {
    match snap.tasks[idx].kind {
        TaskKind::Task if !snap.children(idx).is_empty() => TaskKind::Project,
        k => k,
    }
}

/// The repeat rule in words; `None` without a rule, `"unsupported rule"` when it does not parse.
pub fn repeat_text(t: &Task, tz: Tz) -> Option<String> {
    let raw = t.rrule.as_ref()?;
    Some(match parse_rrule(raw) {
        Ok(r) => describe(
            &r,
            t.due.as_ref().map(|d| d.local_date(tz)),
            t.from_completion,
        ),
        Err(_) => "unsupported rule".to_string(),
    })
}

/// One line per alarm, in order.
pub fn alert_lines(t: &Task, tz: Tz) -> Vec<String> {
    t.alarms.iter().map(|a| describe_alarm(a, t, tz)).collect()
}

/// A parent's children: the open ones in sibling order, then the finished ones, newest
/// completion first; those without a completion time keep their order, last.
pub fn child_order(snap: &Snapshot, tz: Tz, idx: usize) -> Vec<usize> {
    let (mut open, mut done): (Vec<usize>, Vec<usize>) = snap
        .children(idx)
        .iter()
        .partition(|&&c| !matches!(snap.tasks[c].status, Status::Completed | Status::Cancelled));
    open.sort_by(|&a, &b| compare_items(snap, tz, a, b));
    let stamp = |c: usize| {
        snap.tasks[c]
            .completed
            .as_ref()
            .map(|v| match v.resolve(tz).0 {
                Resolved::Instant(t) => t.timestamp(),
                Resolved::Date(d) => tz
                    .from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap())
                    .earliest()
                    .map_or_else(
                        || d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp(),
                        |t| t.timestamp(),
                    ),
            })
    };
    // Sibling order first, so the stable sort below keeps it among equal times and for every
    // child without one; `None` sorts after every `Some`.
    done.sort_by(|&a, &b| compare_items(snap, tz, a, b));
    done.sort_by_key(|&c| match stamp(c) {
        Some(s) => (0, -s),
        None => (1, 0),
    });
    open.extend(done);
    open
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::NaiveDate;
    use chrono_tz::America::New_York;

    use super::*;
    use crate::config::Config;
    use crate::ical::value::IcalDateTime;
    use crate::model::Metadata;
    use crate::vdir::{Collection, LoadReport};

    fn task(uid: &str) -> Task {
        Task {
            uid: uid.into(),
            collection: "list1".into(),
            path: PathBuf::from(format!("list1/{uid}.ics")),
            summary: format!("Synthetic {uid}"),
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
            custom_alert: Default::default(),
            meta: Metadata::default(),
            from_completion: false,
            warnings: vec![],
            blocked: None,
        }
    }

    fn child(uid: &str) -> Task {
        let mut t = task(uid);
        t.parent = Some("p".into());
        t
    }

    fn snapshot(tasks: Vec<Task>) -> Snapshot {
        Snapshot::new(
            LoadReport {
                tasks,
                collections: vec![Collection {
                    id: "list1".into(),
                    path: PathBuf::from("list1"),
                    display_name: Some("One".into()),
                    color: None,
                }],
                ..LoadReport::default()
            },
            Config::with_vdir(PathBuf::from("/synthetic/vdir")),
        )
    }

    fn day(d: u32) -> IcalDateTime {
        IcalDateTime::Date(NaiveDate::from_ymd_opt(2026, 11, d).unwrap())
    }

    #[test]
    fn shown_kind_plain_parent_is_project() {
        let mut list = task("c");
        list.kind = TaskKind::Checklist;
        let mut item = task("i");
        item.parent = Some("c".into());
        let snap = snapshot(vec![task("p"), child("k"), list, item, task("solo")]);
        let at = |u: &str| snap.tasks.iter().position(|t| t.uid == u).unwrap();
        assert_eq!(shown_kind(&snap, at("p")), TaskKind::Project);
        assert_eq!(shown_kind(&snap, at("c")), TaskKind::Checklist);
        assert_eq!(shown_kind(&snap, at("solo")), TaskKind::Task);
    }

    #[test]
    fn repeat_text_none_and_unsupported() {
        let mut t = task("r");
        assert_eq!(repeat_text(&t, New_York), None);
        t.rrule = Some("RRULE:FREQ=DAILY".into());
        let want = describe(&parse_rrule("RRULE:FREQ=DAILY").unwrap(), None, false);
        assert_eq!(repeat_text(&t, New_York), Some(want));
        t.rrule = Some("RRULE:FREQ=SECONDLY".into());
        assert_eq!(
            repeat_text(&t, New_York).as_deref(),
            Some("unsupported rule")
        );
    }

    #[test]
    fn child_order_open_then_newest_completed() {
        let mut a = child("a");
        let mut b = child("b");
        // Sibling order falls back to creation time: b is older than a.
        a.created = Some(day(5));
        b.created = Some(day(4));
        let mut c = child("c");
        c.status = Status::Completed;
        c.completed = Some(day(10));
        let mut d = child("d");
        d.status = Status::Completed;
        d.completed = Some(day(12));
        let mut e = child("e");
        e.status = Status::Completed;
        let snap = snapshot(vec![task("p"), a, b, c, d, e]);
        let p = snap.tasks.iter().position(|t| t.uid == "p").unwrap();
        let uids: Vec<&str> = child_order(&snap, New_York, p)
            .into_iter()
            .map(|i| snap.tasks[i].uid.as_str())
            .collect();
        assert_eq!(uids, ["b", "a", "d", "c", "e"]);
    }

    #[test]
    fn child_order_completed_without_time_in_sibling_order() {
        // Loaded e before f, but f is older: sibling order is f, e.
        let mut e = child("e");
        e.status = Status::Completed;
        e.created = Some(day(5));
        let mut f = child("f");
        f.status = Status::Cancelled;
        f.created = Some(day(4));
        let snap = snapshot(vec![task("p"), e, f]);
        let p = snap.tasks.iter().position(|t| t.uid == "p").unwrap();
        let uids: Vec<&str> = child_order(&snap, New_York, p)
            .into_iter()
            .map(|i| snap.tasks[i].uid.as_str())
            .collect();
        assert_eq!(uids, ["f", "e"]);
    }
}
