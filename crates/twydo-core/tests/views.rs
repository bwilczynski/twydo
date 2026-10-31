//! View membership, date groups, sorting and counts. All content is synthetic.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Weekday};
use chrono_tz::America::New_York;
use chrono_tz::Tz;

use twydo_core::config::{Config, ListPrefs, Sort, SortField};
use twydo_core::ical::value::IcalDateTime;
use twydo_core::model::{Metadata, Status, Task, TaskKind};
use twydo_core::vdir::{Collection, LoadReport, load};
use twydo_core::views::{DateGroup, Snapshot, View, ViewResult, build_view, counts, date_group};

// ---------- builders ----------

fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn local(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
}

fn ny(s: &str) -> IcalDateTime {
    IcalDateTime::Zoned {
        local: local(s),
        tzid: "America/New_York".to_string(),
    }
}

/// Reference now: Wed 2026-11-04 10:00 America/New_York.
fn now() -> DateTime<Tz> {
    New_York
        .from_local_datetime(&local("2026-11-04 10:00"))
        .unwrap()
}

struct TaskBuilder(Task);

impl TaskBuilder {
    fn new(uid: &str, list: &str) -> Self {
        TaskBuilder(Task {
            uid: uid.to_string(),
            collection: list.to_string(),
            path: PathBuf::from(format!("{list}/{uid}.ics")),
            summary: uid.to_string(),
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
        })
    }
    fn title(mut self, s: &str) -> Self {
        self.0.summary = s.to_string();
        self
    }
    fn due(mut self, d: &str) -> Self {
        self.0.due = Some(IcalDateTime::Date(date(d)));
        self
    }
    fn due_at(mut self, t: &str) -> Self {
        self.0.due = Some(ny(t));
        self
    }
    fn start(mut self, d: &str) -> Self {
        self.0.start = Some(IcalDateTime::Date(date(d)));
        self
    }
    fn completed_at(mut self, t: &str) -> Self {
        self.0.status = Status::Completed;
        self.0.completed = Some(ny(t));
        self
    }
    fn parent(mut self, uid: &str) -> Self {
        self.0.parent = Some(uid.to_string());
        self
    }
    fn kind(mut self, k: TaskKind) -> Self {
        self.0.kind = k;
        self.0.meta.tokens.insert(
            "tasktype".to_string(),
            Some(
                match k {
                    TaskKind::Task => "task",
                    TaskKind::Project => "project",
                    TaskKind::Checklist => "checklist",
                }
                .to_string(),
            ),
        );
        self
    }
    fn rrule(mut self, r: &str) -> Self {
        self.0.rrule = Some(r.to_string());
        self
    }
    fn build(self) -> Task {
        self.0
    }
}

fn config(lists: &[(&str, ListPrefs)]) -> Config {
    Config {
        vdir: PathBuf::from("/nonexistent"),
        timezone: Some("America/New_York".to_string()),
        week_start: Weekday::Mon,
        morning_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        default_list: None,
        lists: lists
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect::<BTreeMap<_, _>>(),
        groups: vec![],
    }
}

fn archived() -> ListPrefs {
    ListPrefs {
        archived: true,
        ..ListPrefs::default()
    }
}

fn snapshot(tasks: Vec<Task>, lists: &[(&str, ListPrefs)]) -> Snapshot {
    let mut ids: Vec<String> = tasks.iter().map(|t| t.collection.clone()).collect();
    ids.sort();
    ids.dedup();
    let collections = ids
        .into_iter()
        .map(|id| Collection {
            path: PathBuf::from(&id),
            id,
            display_name: None,
            color: None,
        })
        .collect();
    Snapshot::new(
        LoadReport {
            collections,
            tasks,
            errors: vec![],
        },
        config(lists),
    )
}

fn idx(s: &Snapshot, uid: &str) -> usize {
    s.tasks.iter().position(|t| t.uid == uid).unwrap()
}

/// `(group, uid, depth)` for every row, in display order.
fn flat(s: &Snapshot, r: &ViewResult) -> Vec<(DateGroup, String, u8)> {
    r.groups
        .iter()
        .flat_map(|(g, rows)| {
            rows.iter()
                .map(move |row| (*g, s.tasks[row.task].uid.clone(), row.depth))
        })
        .collect()
}

fn uids(s: &Snapshot, r: &ViewResult) -> Vec<String> {
    flat(s, r).into_iter().map(|(_, u, _)| u).collect()
}

fn no_expand() -> HashSet<String> {
    HashSet::new()
}

// ---------- tests ----------

#[test]
fn today_membership() {
    let s = snapshot(
        vec![
            TaskBuilder::new("due-today", "inbox")
                .due("2026-11-04")
                .build(),
            TaskBuilder::new("due-0900", "inbox")
                .due_at("2026-11-04 09:00")
                .build(),
            TaskBuilder::new("due-1500", "inbox")
                .due_at("2026-11-04 15:00")
                .build(),
            TaskBuilder::new("due-yesterday", "inbox")
                .due("2026-11-03")
                .build(),
            TaskBuilder::new("started", "inbox")
                .start("2026-11-02")
                .due("2026-11-10")
                .build(),
            TaskBuilder::new("future-start", "inbox")
                .start("2026-11-05")
                .due("2026-11-04")
                .build(),
            TaskBuilder::new("completed", "inbox")
                .due("2026-11-04")
                .completed_at("2026-11-04 08:00")
                .build(),
            TaskBuilder::new("tomorrow", "inbox")
                .due("2026-11-05")
                .build(),
            TaskBuilder::new("archived-today", "old")
                .due("2026-11-04")
                .build(),
            TaskBuilder::new("archived-overdue", "old")
                .due("2026-11-01")
                .build(),
        ],
        &[("old", archived())],
    );
    let r = build_view(&s, &View::Today, now(), &no_expand());
    let mut got: Vec<(DateGroup, String)> = flat(&s, &r)
        .into_iter()
        .map(|(g, u, d)| {
            assert_eq!(d, 0);
            (g, u)
        })
        .collect();
    got.sort_by(|a, b| a.1.cmp(&b.1));
    assert_eq!(
        got,
        vec![
            (DateGroup::Overdue, "due-0900".to_string()),
            (DateGroup::LaterToday, "due-1500".to_string()),
            (DateGroup::Today, "due-today".to_string()),
            (DateGroup::Overdue, "due-yesterday".to_string()),
            (DateGroup::InProgress, "started".to_string()),
        ]
    );
    // Groups appear in enum order.
    let order: Vec<DateGroup> = r.groups.iter().map(|(g, _)| *g).collect();
    assert_eq!(
        order,
        vec![
            DateGroup::Overdue,
            DateGroup::InProgress,
            DateGroup::LaterToday,
            DateGroup::Today
        ]
    );
    // Within a group rows sort by due: 11-03 before 11-04 09:00.
    assert_eq!(
        r.groups[0]
            .1
            .iter()
            .map(|row| s.tasks[row.task].uid.as_str())
            .collect::<Vec<_>>(),
        vec!["due-yesterday", "due-0900"]
    );
    // Started-but-not-due counts in total only.
    assert_eq!(r.counts.total, 5);
    assert_eq!(r.counts.due_or_overdue, 4);
    assert_eq!(counts(&s, &View::Today, now()), r.counts);
}

#[test]
fn today_checklist_items_under_parent() {
    let s = snapshot(
        vec![
            TaskBuilder::new("cl", "inbox")
                .kind(TaskKind::Checklist)
                .due("2026-11-04")
                .build(),
            TaskBuilder::new("item-c", "inbox")
                .title("C")
                .parent("cl")
                .build(),
            TaskBuilder::new("item-a", "inbox")
                .title("A")
                .parent("cl")
                .build(),
            TaskBuilder::new("item-b", "inbox")
                .title("B")
                .parent("cl")
                .build(),
            // An item due today under a checklist that does not qualify stays out.
            TaskBuilder::new("cl2", "inbox")
                .kind(TaskKind::Checklist)
                .due("2026-11-20")
                .build(),
            TaskBuilder::new("item-x", "inbox")
                .parent("cl2")
                .due("2026-11-04")
                .build(),
        ],
        &[],
    );
    let r = build_view(&s, &View::Today, now(), &no_expand());
    let rows: Vec<(String, u8)> = flat(&s, &r).into_iter().map(|(_, u, d)| (u, d)).collect();
    assert_eq!(
        rows,
        vec![
            ("cl".to_string(), 0),
            ("item-a".to_string(), 1),
            ("item-b".to_string(), 1),
            ("item-c".to_string(), 1),
        ]
    );
    let item_rows: Vec<_> = r.groups.iter().flat_map(|(_, rows)| rows).skip(1).collect();
    assert!(
        item_rows
            .iter()
            .all(|row| row.parent_hint == Some(idx(&s, "cl")))
    );
    assert_eq!(r.counts.total, 1);
    assert_eq!(r.counts.due_or_overdue, 1);
}

#[test]
fn today_project_child_independent() {
    let s = snapshot(
        vec![
            TaskBuilder::new("proj", "inbox")
                .kind(TaskKind::Project)
                .due("2026-11-11")
                .build(),
            TaskBuilder::new("child", "inbox")
                .parent("proj")
                .due("2026-11-04")
                .build(),
        ],
        &[],
    );
    let r = build_view(&s, &View::Today, now(), &no_expand());
    let rows: Vec<_> = r.groups.iter().flat_map(|(_, rows)| rows).collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].task, idx(&s, "child"));
    assert_eq!(rows[0].depth, 0);
    assert_eq!(rows[0].parent_hint, Some(idx(&s, "proj")));
    assert_eq!(r.counts.total, 1);
}

#[test]
fn next3days_boundaries() {
    let s = snapshot(
        vec![
            TaskBuilder::new("today", "inbox").due("2026-11-04").build(),
            TaskBuilder::new("late-2359", "inbox")
                .due_at("2026-11-06 23:59")
                .build(),
            TaskBuilder::new("next-0000", "inbox")
                .due_at("2026-11-07 00:00")
                .build(),
            TaskBuilder::new("overdue-0900", "inbox")
                .due_at("2026-11-04 09:00")
                .build(),
            TaskBuilder::new("undated", "inbox").build(),
        ],
        &[],
    );
    let r = build_view(&s, &View::Next3Days, now(), &no_expand());
    let got: Vec<(DateGroup, String)> = flat(&s, &r).into_iter().map(|(g, u, _)| (g, u)).collect();
    assert_eq!(
        got,
        vec![
            (DateGroup::Today, "today".to_string()),
            (DateGroup::DayAfterTomorrow, "late-2359".to_string()),
        ]
    );
    assert_eq!(r.counts.total, 2);
    assert_eq!(r.counts.due_or_overdue, 1);
}

#[test]
fn buckets_by_day_offset() {
    let base = date("2026-11-04");
    let cases = [
        (0, DateGroup::Today),
        (1, DateGroup::Tomorrow),
        (2, DateGroup::DayAfterTomorrow),
        (3, DateGroup::Next7Days),
        (6, DateGroup::Next7Days),
        (7, DateGroup::Next2Weeks),
        (13, DateGroup::Next2Weeks),
        (14, DateGroup::Next30Days),
        (29, DateGroup::Next30Days),
        (30, DateGroup::Future),
    ];
    let tasks = cases
        .iter()
        .map(|(d, _)| {
            let due = (base + chrono::Duration::days(*d))
                .format("%Y-%m-%d")
                .to_string();
            TaskBuilder::new(&format!("d{d}"), "inbox")
                .due(&due)
                .build()
        })
        .chain([TaskBuilder::new("undated", "inbox").build()])
        .collect();
    let s = snapshot(tasks, &[]);
    for (d, want) in cases {
        assert_eq!(
            date_group(&s, idx(&s, &format!("d{d}")), now(), false),
            want,
            "d={d}"
        );
    }
    assert_eq!(
        date_group(&s, idx(&s, "undated"), now(), false),
        DateGroup::NoDueDate
    );
}

#[test]
fn calendar_days_across_dst() {
    // DST ends 2026-11-01 02:00 in New York: that day has 25 hours.
    let now = New_York
        .from_local_datetime(&local("2026-11-01 00:30"))
        .unwrap();
    let s = snapshot(
        vec![
            TaskBuilder::new("mon", "inbox").due("2026-11-02").build(),
            TaskBuilder::new("tue", "inbox").due("2026-11-03").build(),
            TaskBuilder::new("mon-2330", "inbox")
                .due_at("2026-11-02 23:30")
                .build(),
        ],
        &[],
    );
    assert_eq!(
        date_group(&s, idx(&s, "mon"), now, false),
        DateGroup::Tomorrow
    );
    assert_eq!(
        date_group(&s, idx(&s, "tue"), now, false),
        DateGroup::DayAfterTomorrow
    );
    assert_eq!(
        date_group(&s, idx(&s, "mon-2330"), now, false),
        DateGroup::Tomorrow
    );
}

#[test]
fn scheduled_hidden_elsewhere() {
    let s = snapshot(
        vec![
            TaskBuilder::new("later", "inbox")
                .start("2026-11-06")
                .due("2026-11-04")
                .build(),
            TaskBuilder::new("soon", "inbox")
                .start("2026-11-05")
                .due("2026-11-20")
                .build(),
            TaskBuilder::new("plain", "inbox").due("2026-11-04").build(),
            TaskBuilder::new("archived-future", "old")
                .start("2026-11-06")
                .build(),
        ],
        &[("old", archived())],
    );
    for v in [
        View::Today,
        View::All,
        View::Next3Days,
        View::Done,
        View::List("inbox".to_string()),
    ] {
        let r = build_view(&s, &v, now(), &no_expand());
        let u = uids(&s, &r);
        assert!(!u.contains(&"later".to_string()), "{v:?}: {u:?}");
        assert!(!u.contains(&"soon".to_string()), "{v:?}: {u:?}");
    }
    let r = build_view(&s, &View::Scheduled, now(), &no_expand());
    assert_eq!(r.groups.len(), 1);
    assert_eq!(r.groups[0].0, DateGroup::Scheduled);
    assert_eq!(uids(&s, &r), vec!["soon", "later"]);
    assert_eq!(r.counts.total, 2);
}

#[test]
fn done_view_and_counts() {
    let s = snapshot(
        vec![
            TaskBuilder::new("old-done", "old")
                .completed_at("2026-11-02 12:00")
                .build(),
            TaskBuilder::new("gone-done", "gone")
                .completed_at("2026-11-03 12:00")
                .build(),
            TaskBuilder::new("inbox-done", "inbox")
                .completed_at("2026-11-04 08:00")
                .build(),
            TaskBuilder::new("inbox-open", "inbox")
                .due("2026-11-04")
                .build(),
        ],
        &[
            ("old", archived()),
            (
                "gone",
                ListPrefs {
                    archived: true,
                    show_in_done: false,
                    ..ListPrefs::default()
                },
            ),
        ],
    );
    let r = build_view(&s, &View::Done, now(), &no_expand());
    let got: Vec<(DateGroup, String)> = flat(&s, &r).into_iter().map(|(g, u, _)| (g, u)).collect();
    assert_eq!(
        got,
        vec![
            (
                DateGroup::CompletedOn(date("2026-11-04")),
                "inbox-done".to_string()
            ),
            (
                DateGroup::CompletedOn(date("2026-11-02")),
                "old-done".to_string()
            ),
        ]
    );
    assert_eq!(r.counts.total, 2);
    assert_eq!(r.counts.due_or_overdue, 0);
    assert_eq!(counts(&s, &View::Done, now()), r.counts);
}

fn sample() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/sample-vdir"
    ))
}

#[test]
fn counts_today_all() {
    let report = load(sample()).unwrap();
    assert!(report.errors.is_empty());
    let s = Snapshot::new(report, config(&[]));
    let today = counts(&s, &View::Today, now());
    assert_eq!((today.due_or_overdue, today.total), (3, 3));
    let all = counts(&s, &View::All, now());
    // 15 open − 1 future start − 3 checklist items = 11; due-or-overdue: renew-library,
    // call-dentist, water-plants.
    assert_eq!((all.due_or_overdue, all.total), (3, 11));
    // Collapsed: the 2 project children and 3 checklist items are not rows.
    let r = build_view(&s, &View::All, now(), &no_expand());
    assert_eq!(r.counts, all);
    assert_eq!(
        r.groups.iter().map(|(_, rows)| rows.len()).sum::<usize>(),
        9
    );
    // Expanded: children nest at depth 1 right after their parent.
    let expanded: HashSet<String> = ["sv-bills-4", "sv-personal-2"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let r = build_view(&s, &View::All, now(), &expanded);
    let rows = flat(&s, &r);
    assert_eq!(rows.len(), 14);
    let p = rows.iter().position(|(_, u, _)| u == "sv-bills-4").unwrap();
    assert_eq!(rows[p].2, 0);
    assert_eq!(rows[p + 1].2, 1);
    assert_eq!(rows[p + 2].2, 1);
    assert_eq!(r.counts, all);
}

#[test]
fn orphan_and_cyclic_parents_render_top_level() {
    let s = snapshot(
        vec![
            TaskBuilder::new("orphan", "inbox")
                .parent("missing-uid")
                .build(),
            TaskBuilder::new("a", "inbox").parent("b").build(),
            TaskBuilder::new("b", "inbox").parent("a").build(),
            TaskBuilder::new("self", "inbox").parent("self").build(),
        ],
        &[],
    );
    for uid in ["orphan", "a", "b", "self"] {
        let i = idx(&s, uid);
        assert_eq!(s.parent(i), None, "{uid}");
        assert!(
            s.tasks[i]
                .warnings
                .iter()
                .any(|w| w == "broken parent link"),
            "{uid}: {:?}",
            s.tasks[i].warnings
        );
    }
    let all: HashSet<String> = ["a", "b", "self"].iter().map(|s| s.to_string()).collect();
    let r = build_view(&s, &View::All, now(), &all);
    let mut rows = flat(&s, &r);
    rows.sort_by(|x, y| x.1.cmp(&y.1));
    assert_eq!(
        rows.iter()
            .map(|(_, u, d)| (u.as_str(), *d))
            .collect::<Vec<_>>(),
        vec![("a", 0), ("b", 0), ("orphan", 0), ("self", 0)]
    );
}

#[test]
fn sort_title_desc_ungrouped() {
    let s = snapshot(
        vec![
            TaskBuilder::new("1", "inbox")
                .title("apple")
                .due("2026-11-03")
                .build(),
            TaskBuilder::new("2", "inbox").title("Cherry").build(),
            TaskBuilder::new("3", "inbox")
                .title("banana")
                .due("2026-12-30")
                .build(),
            TaskBuilder::new("4", "inbox")
                .title("cherry")
                .due("2026-11-04")
                .build(),
        ],
        &[(
            "inbox",
            ListPrefs {
                sort: Sort {
                    field: SortField::Title,
                    descending: true,
                },
                ..ListPrefs::default()
            },
        )],
    );
    let r = build_view(&s, &View::List("inbox".to_string()), now(), &no_expand());
    assert_eq!(r.groups.len(), 1);
    assert_eq!(r.groups[0].0, DateGroup::Ungrouped);
    let titles: Vec<&str> = r.groups[0]
        .1
        .iter()
        .map(|row| s.tasks[row.task].summary.as_str())
        .collect();
    // Case-insensitive title order, reversed; ties (cherry/Cherry) fall back to exact title, then UID.
    assert_eq!(titles, vec!["Cherry", "cherry", "banana", "apple"]);
}

#[test]
fn due_sort_timed_before_date_only_and_descending() {
    let tasks = vec![
        TaskBuilder::new("d-date", "inbox")
            .title("z")
            .due("2026-11-10")
            .build(),
        TaskBuilder::new("d-timed", "inbox")
            .title("y")
            .due_at("2026-11-10 18:00")
            .build(),
        TaskBuilder::new("d-early", "inbox")
            .title("x")
            .due_at("2026-11-10 08:00")
            .build(),
        TaskBuilder::new("d-tie", "inbox")
            .title("a")
            .due("2026-11-10")
            .build(),
    ];
    let s = snapshot(tasks.clone(), &[]);
    let r = build_view(&s, &View::List("inbox".to_string()), now(), &no_expand());
    assert_eq!(r.groups.len(), 1);
    assert_eq!(r.groups[0].0, DateGroup::Next7Days);
    assert_eq!(uids(&s, &r), vec!["d-early", "d-timed", "d-tie", "d-date"]);

    let desc = ListPrefs {
        sort: Sort {
            field: SortField::Due,
            descending: true,
        },
        ..ListPrefs::default()
    };
    let s = snapshot(
        tasks
            .into_iter()
            .chain([TaskBuilder::new("over", "inbox").due("2026-11-01").build()])
            .collect(),
        &[("inbox", desc)],
    );
    let r = build_view(&s, &View::List("inbox".to_string()), now(), &no_expand());
    // Groups keep their order; rows reverse within a group.
    assert_eq!(r.groups[0].0, DateGroup::Overdue);
    assert_eq!(
        r.groups[1]
            .1
            .iter()
            .map(|row| s.tasks[row.task].uid.as_str())
            .collect::<Vec<_>>(),
        vec!["d-tie", "d-date", "d-timed", "d-early"]
    );
}

#[test]
fn list_view_opens_archived_and_ignores_show_in_all() {
    let s = snapshot(
        vec![
            TaskBuilder::new("arch", "old").due("2026-11-04").build(),
            TaskBuilder::new("quiet", "q").due("2026-11-04").build(),
        ],
        &[
            ("old", archived()),
            (
                "q",
                ListPrefs {
                    show_in_all: false,
                    ..ListPrefs::default()
                },
            ),
        ],
    );
    assert!(uids(&s, &build_view(&s, &View::All, now(), &no_expand())).is_empty());
    assert_eq!(
        uids(
            &s,
            &build_view(&s, &View::List("old".into()), now(), &no_expand())
        ),
        vec!["arch"]
    );
    assert_eq!(
        uids(
            &s,
            &build_view(&s, &View::List("q".into()), now(), &no_expand())
        ),
        vec!["quiet"]
    );
    // show_in_all = false does not remove the list from Today.
    assert_eq!(
        uids(&s, &build_view(&s, &View::Today, now(), &no_expand())),
        vec!["quiet"]
    );
}

#[test]
fn child_of_ineligible_parent_shows_top_level_in_all() {
    let s = snapshot(
        vec![
            TaskBuilder::new("proj", "inbox")
                .kind(TaskKind::Project)
                .start("2026-12-01")
                .build(),
            TaskBuilder::new("child", "inbox")
                .parent("proj")
                .due("2026-11-20")
                .build(),
        ],
        &[],
    );
    let r = build_view(&s, &View::All, now(), &no_expand());
    let rows: Vec<_> = r.groups.iter().flat_map(|(_, rows)| rows).collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].task, idx(&s, "child"));
    assert_eq!(rows[0].depth, 0);
    assert_eq!(rows[0].parent_hint, Some(idx(&s, "proj")));
}

#[test]
fn untyped_parent_becomes_project() {
    let mut parent = TaskBuilder::new("p", "inbox").build();
    parent.meta = Metadata::default();
    let s = snapshot(
        vec![parent, TaskBuilder::new("c", "inbox").parent("p").build()],
        &[],
    );
    let p = idx(&s, "p");
    assert_eq!(s.tasks[p].kind, TaskKind::Project);
    assert!(
        s.tasks[p]
            .warnings
            .iter()
            .any(|w| w == "parent type not encoded; shown as project")
    );
    assert_eq!(s.children(p), &[idx(&s, "c")]);
    assert_eq!(s.parent(idx(&s, "c")), Some(p));
}

#[test]
fn unsupported_repeat_rule_blocks_edits() {
    let mut pre = TaskBuilder::new("pre", "inbox")
        .rrule("FREQ=HOURLY")
        .build();
    pre.blocked = Some("recurrence overrides are not supported".to_string());
    let s = snapshot(
        vec![
            TaskBuilder::new("bad", "inbox")
                .rrule("FREQ=HOURLY")
                .build(),
            TaskBuilder::new("good", "inbox")
                .rrule("FREQ=WEEKLY")
                .build(),
            pre,
        ],
        &[],
    );
    assert_eq!(
        s.tasks[idx(&s, "bad")].blocked.as_deref(),
        Some("unsupported repeat rule")
    );
    assert_eq!(s.tasks[idx(&s, "good")].blocked, None);
    assert_eq!(
        s.tasks[idx(&s, "pre")].blocked.as_deref(),
        Some("recurrence overrides are not supported")
    );
}

#[test]
fn build_view_is_deterministic() {
    let report = load(sample()).unwrap();
    let s = Snapshot::new(report, config(&[]));
    for v in [
        View::Today,
        View::All,
        View::Scheduled,
        View::Done,
        View::Next3Days,
    ] {
        let a = build_view(&s, &v, now(), &no_expand());
        let b = build_view(&s, &v, now(), &no_expand());
        assert_eq!(a, b);
    }
}
