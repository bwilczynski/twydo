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
        work_days: vec![
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
        ],
        week_start: Weekday::Mon,
        morning_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        default_list: None,
        lists: lists
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect::<BTreeMap<_, _>>(),
        groups: vec![],
        writes: Default::default(),
        state_dir: None,
        sync: None,
        daemon: Default::default(),
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
            ..Default::default()
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
fn scheduled_stays_out_of_special_active_views() {
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
    for v in [View::Today, View::Next3Days, View::Done] {
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
    // 15 open − 3 checklist items = 12; due-or-overdue: renew-library,
    // call-dentist, water-plants.
    assert_eq!((all.due_or_overdue, all.total), (3, 12));
    // Collapsed: the 2 project children and 3 checklist items are not rows.
    let r = build_view(&s, &View::All, now(), &no_expand());
    assert_eq!(r.counts, all);
    assert_eq!(
        r.groups.iter().map(|(_, rows)| rows.len()).sum::<usize>(),
        11
    );
    // Expanded: children nest at depth 1 right after their parent.
    let expanded: HashSet<String> = ["sv-bills-4", "sv-personal-2"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let r = build_view(&s, &View::All, now(), &expanded);
    let rows = flat(&s, &r);
    assert_eq!(rows.len(), 16);
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

/// The project append is a delta: unchanged baseline resources are intentionally not copied.
fn item_order_capture(scenario: &str, through: &str) -> Snapshot {
    let temp = tempfile::tempdir().unwrap();
    let list = temp.path().join("list1");
    std::fs::create_dir(&list).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(scenario);
    for entry in std::fs::read_dir(root.join(through)).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_some_and(|ext| ext == "ics") {
            std::fs::copy(&p, list.join(p.file_name().unwrap())).unwrap();
        }
    }
    Snapshot::new(load(temp.path()).unwrap(), config(&[]))
}

#[test]
fn captured_checklist_and_project_keep_phone_item_order_under_list_sort_overrides() {
    for scenario in ["insert/checklist-items", "insert/project-items"] {
        for appended in [false, true] {
            for field in [SortField::Due, SortField::Title, SortField::Creation] {
                let mut s = item_order_capture(
                    scenario,
                    if appended {
                        "appended/list1"
                    } else {
                        "created/list1"
                    },
                );
                s.config.lists.insert(
                    "list1".into(),
                    ListPrefs {
                        sort: Sort {
                            field,
                            descending: true,
                        },
                        ..ListPrefs::default()
                    },
                );
                let parent = s
                    .tasks
                    .iter()
                    .find(|t| t.parent.is_none())
                    .unwrap()
                    .uid
                    .clone();
                for view in [View::All, View::List("list1".into())] {
                    let result = build_view(&s, &view, now(), &HashSet::from([parent.clone()]));
                    let titles: Vec<_> = result
                        .groups
                        .iter()
                        .flat_map(|(_, rows)| rows)
                        .filter(|row| row.depth == 1)
                        .map(|row| s.tasks[row.task].summary.as_str())
                        .collect();
                    let mut expected = vec!["Zulu", "Alpha", "Mike"];
                    if appended {
                        expected.push("Bravo");
                    }
                    assert_eq!(
                        titles, expected,
                        "{scenario}, {appended}, {view:?}, {field:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn captured_checklist_item_order_is_shared_by_flat_views() {
    for view in [View::Today, View::Next3Days, View::Scheduled] {
        let mut s = item_order_capture("insert/checklist-items", "appended/list1");
        let p = s.tasks.iter().position(|t| t.parent.is_none()).unwrap();
        s.tasks[p].due = Some(IcalDateTime::Date(now().date_naive()));
        if view == View::Scheduled {
            s.tasks[p].start = Some(IcalDateTime::Date(date("2026-12-01")));
        }
        let result = build_view(&s, &view, now(), &no_expand());
        let titles: Vec<_> = result
            .groups
            .iter()
            .flat_map(|(_, rows)| rows)
            .filter(|row| row.depth == 1)
            .map(|row| s.tasks[row.task].summary.as_str())
            .collect();
        assert_eq!(titles, ["Zulu", "Alpha", "Mike", "Bravo"], "{view:?}");
    }
}

#[test]
fn phone_edits_of_appended_items_keep_identity_order_and_unchanged_neighbors() {
    for scenario in ["insert/checklist-items", "insert/project-items"] {
        let before = item_order_capture(scenario, "twydo-appended/list1");
        let after = item_order_capture(scenario, "phone-return/list1");
        assert_eq!(before.tasks.len(), 6);
        assert_eq!(after.tasks.len(), 6);
        let echo = before.tasks.iter().find(|t| t.summary == "Echo").unwrap();
        let renamed = &after.tasks[after.find(&echo.uid).unwrap()];
        assert_eq!(renamed.summary, "ECHO");
        assert_eq!(echo.parent, renamed.parent);
        assert_eq!(echo.created, renamed.created);
        assert_eq!(
            echo.meta, renamed.meta,
            "phone preserves all ordering/identity metadata"
        );
        assert_eq!(renamed.meta.display_order(), 4);
        let json =
            twydo_core::model::meta_edit::MetaJson::parse(renamed.meta.raw.as_deref().unwrap())
                .unwrap();
        assert_eq!(json.get_str("parentuid"), renamed.parent.as_deref());
        for t in before.tasks.iter().filter(|t| t.uid != echo.uid) {
            let returned = &after.tasks[after.find(&t.uid).unwrap()];
            assert_eq!(
                before.source(&t.path),
                after.source(&returned.path),
                "{} unchanged",
                t.summary
            );
        }
        for (snap, last) in [(&before, "Echo"), (&after, "ECHO")] {
            let parent = snap
                .tasks
                .iter()
                .find(|t| t.parent.is_none())
                .unwrap()
                .uid
                .clone();
            let result = build_view(snap, &View::All, now(), &HashSet::from([parent]));
            let titles: Vec<_> = result
                .groups
                .iter()
                .flat_map(|(_, rows)| rows)
                .filter(|row| row.depth == 1)
                .map(|row| snap.tasks[row.task].summary.as_str())
                .collect();
            assert_eq!(titles, ["Zulu", "Alpha", "Mike", "Bravo", last]);
        }
    }
}

#[test]
fn nested_order_uses_position_then_creation_then_uid_with_safe_missing_values() {
    for kind in [TaskKind::Checklist, TaskKind::Project] {
        let mut tasks = vec![TaskBuilder::new("parent", "inbox").kind(kind).build()];
        for (uid, position, created, title) in [
            ("a", Some("0"), Some("2026-11-01 10:00"), "Zulu"),
            ("b", Some("-1"), Some("2026-11-02 10:00"), "Yankee"),
            ("c", Some("\"bad\""), Some("2026-11-03 10:00"), "Xray"),
            ("d", None, Some("2026-11-03 10:00"), "Whiskey"),
            ("e", Some("9223372036854775808"), None, "Victor"),
            ("f", None, None, "Uniform"),
            ("g", Some("3"), Some("2026-10-01 10:00"), "Alpha"),
        ] {
            let mut child = TaskBuilder::new(uid, "inbox")
                .parent("parent")
                .title(title)
                .build();
            child.created = created.map(ny);
            if let Some(position) = position {
                child.meta = twydo_core::model::parse_metadata(&format!(
                    "<2Do Meta>{{\"DisplayOrder\":{position}}}</2Do Meta>"
                ));
            }
            tasks.push(child);
        }
        let s = snapshot(tasks, &[]);
        let result = build_view(&s, &View::All, now(), &HashSet::from(["parent".into()]));
        assert_eq!(
            uids(&s, &result),
            ["parent", "a", "b", "c", "d", "e", "f", "g"]
        );
    }
}

#[test]
fn independent_project_items_in_flat_views_keep_date_sorting() {
    let mut s = item_order_capture("insert/project-items", "appended/list1");
    for t in &mut s.tasks {
        t.due = Some(IcalDateTime::Date(date(match t.summary.as_str() {
            "Bravo" => "2026-11-01",
            "Mike" => "2026-11-02",
            "Alpha" => "2026-11-03",
            _ => "2026-11-04",
        })));
    }
    let result = build_view(&s, &View::Today, now(), &no_expand());
    let titles: Vec<_> = result
        .groups
        .iter()
        .flat_map(|(_, rows)| rows)
        .filter(|row| s.tasks[row.task].parent.is_some())
        .map(|row| s.tasks[row.task].summary.as_str())
        .collect();
    assert_eq!(titles, ["Bravo", "Mike", "Alpha", "Zulu"]);
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
fn active_child_of_future_parent_shows_top_level_in_all() {
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
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].task, idx(&s, "proj"));
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
fn unrecognised_tasktype_parent_falls_back_to_project() {
    let mut parent = TaskBuilder::new("p", "inbox").build();
    parent
        .meta
        .tokens
        .insert("tasktype".to_string(), Some("folder".to_string()));
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

#[test]
fn unknown_tzid_warning_recorded_on_task() {
    let mut t = TaskBuilder::new("win-tz", "inbox").build();
    t.due = Some(IcalDateTime::Zoned {
        local: local("2026-11-04 08:00"),
        tzid: "W. Europe Standard Time".to_string(),
    });
    let s = snapshot(vec![t], &[]);
    let i = idx(&s, "win-tz");
    let w: Vec<&String> = s.tasks[i]
        .warnings
        .iter()
        .filter(|w| w.contains("unknown TZID"))
        .collect();
    assert_eq!(w.len(), 1, "{:?}", s.tasks[i].warnings);
    // Read in local time: 08:00 New York today, so overdue in Today.
    let r = build_view(&s, &View::Today, now(), &no_expand());
    assert_eq!(
        flat(&s, &r),
        vec![(DateGroup::Overdue, "win-tz".to_string(), 0)]
    );
}

#[test]
fn future_start_checklist_item_hidden_under_parent() {
    let s = snapshot(
        vec![
            TaskBuilder::new("cl", "inbox")
                .kind(TaskKind::Checklist)
                .due("2026-11-04")
                .build(),
            TaskBuilder::new("now-item", "inbox").parent("cl").build(),
            TaskBuilder::new("later-item", "inbox")
                .parent("cl")
                .start("2026-11-10")
                .build(),
        ],
        &[],
    );
    let r = build_view(&s, &View::Today, now(), &no_expand());
    assert_eq!(uids(&s, &r), vec!["cl", "now-item"]);
}

#[test]
fn open_items_counts_open_children() {
    let mut cancelled = TaskBuilder::new("c3", "inbox").parent("p").build();
    cancelled.status = Status::Cancelled;
    let s = snapshot(
        vec![
            TaskBuilder::new("p", "inbox")
                .kind(TaskKind::Project)
                .build(),
            TaskBuilder::new("c1", "inbox").parent("p").build(),
            TaskBuilder::new("c2", "inbox")
                .parent("p")
                .completed_at("2026-11-03 09:00")
                .build(),
            cancelled,
            TaskBuilder::new("c4", "inbox").parent("p").build(),
            TaskBuilder::new("alone", "inbox").build(),
        ],
        &[],
    );
    assert_eq!(s.open_items(idx(&s, "p")), 2);
    assert_eq!(s.open_items(idx(&s, "c1")), 0);
    assert_eq!(s.open_items(idx(&s, "alone")), 0);
}

#[test]
fn all_and_lists_always_show_future_and_completed_without_inflating_counts() {
    let mut cancelled = TaskBuilder::new("cancelled", "inbox").build();
    cancelled.status = Status::Cancelled;
    let s = snapshot(
        vec![
            TaskBuilder::new("active", "inbox")
                .due("2026-11-04")
                .build(),
            TaskBuilder::new("future", "inbox")
                .start("2026-11-06")
                .due("2026-11-03")
                .build(),
            TaskBuilder::new("older", "inbox")
                .due("2026-11-01")
                .completed_at("2026-11-02 12:00")
                .build(),
            TaskBuilder::new("newer", "inbox")
                .start("2026-12-01")
                .completed_at("2026-11-04 09:00")
                .build(),
            cancelled,
        ],
        &[],
    );
    for v in [View::All, View::List("inbox".into())] {
        let r = build_view(&s, &v, now(), &no_expand());
        assert_eq!(uids(&s, &r), ["active", "future", "newer", "older"]);
        assert_eq!(r.groups[1].0, DateGroup::Future);
        assert_eq!(r.groups.last().unwrap().0, DateGroup::Done);
        assert_eq!(
            r.groups
                .last()
                .unwrap()
                .1
                .iter()
                .map(|r| s.tasks[r.task].uid.as_str())
                .collect::<Vec<_>>(),
            ["newer", "older"]
        );
        assert_eq!((r.counts.due_or_overdue, r.counts.total), (2, 2));
        assert_eq!(counts(&s, &v, now()), r.counts);
    }
}

#[test]
fn list_sections_survive_non_date_sorts_and_empty_sections_are_omitted() {
    for field in [SortField::Title, SortField::Creation] {
        let s = snapshot(
            vec![
                TaskBuilder::new("z-active", "inbox").build(),
                TaskBuilder::new("a-future", "inbox")
                    .start("2026-11-06")
                    .build(),
                TaskBuilder::new("b-done", "inbox")
                    .completed_at("2026-11-04 09:00")
                    .build(),
            ],
            &[(
                "inbox",
                ListPrefs {
                    sort: Sort {
                        field,
                        descending: true,
                    },
                    ..Default::default()
                },
            )],
        );
        let r = build_view(&s, &View::List("inbox".into()), now(), &no_expand());
        assert_eq!(uids(&s, &r), ["z-active", "a-future", "b-done"]);
        assert_eq!(r.groups.len(), 3);
        assert_eq!(r.groups[0].0, DateGroup::Ungrouped);
        assert_eq!(r.groups[1].0, DateGroup::Future);
    }
    let s = snapshot(vec![TaskBuilder::new("active", "inbox").build()], &[]);
    let r = build_view(&s, &View::All, now(), &no_expand());
    assert_eq!(r.groups.len(), 1);
    assert_eq!(r.groups[0].0, DateGroup::NoDueDate);
}

#[test]
fn family_sections_show_completed_and_future_children_once_even_when_collapsed() {
    let s = snapshot(
        vec![
            TaskBuilder::new("parent", "inbox")
                .kind(TaskKind::Project)
                .due("2026-11-04")
                .build(),
            TaskBuilder::new("active-child", "inbox")
                .parent("parent")
                .due("2026-11-05")
                .build(),
            TaskBuilder::new("future-child", "inbox")
                .parent("parent")
                .start("2026-11-06")
                .build(),
            TaskBuilder::new("done-child", "inbox")
                .parent("parent")
                .completed_at("2026-11-04 09:00")
                .build(),
        ],
        &[],
    );
    let r = build_view(&s, &View::All, now(), &no_expand());
    assert_eq!(uids(&s, &r), ["parent", "future-child", "done-child"]);
    let r = build_view(&s, &View::All, now(), &HashSet::from(["parent".into()]));
    assert_eq!(
        uids(&s, &r),
        ["parent", "active-child", "future-child", "done-child"]
    );
    assert_eq!(r.groups[0].1[1].depth, 1);
    assert_eq!(r.groups[1].1[0].depth, 0);
    assert_eq!(r.groups[1].1[0].parent_hint, Some(idx(&s, "parent")));
    assert_eq!(r.counts.total, 3);
}

#[test]
fn newly_visible_rows_respect_all_exclusions_but_remain_in_direct_lists() {
    for prefs in [
        archived(),
        ListPrefs {
            hidden: true,
            ..Default::default()
        },
        ListPrefs {
            show_in_all: false,
            ..Default::default()
        },
    ] {
        let s = snapshot(
            vec![
                TaskBuilder::new("future", "excluded")
                    .start("2026-11-06")
                    .build(),
                TaskBuilder::new("done", "excluded")
                    .completed_at("2026-11-04 09:00")
                    .build(),
            ],
            &[("excluded", prefs)],
        );
        assert!(
            build_view(&s, &View::All, now(), &no_expand())
                .groups
                .is_empty()
        );
        assert_eq!(
            uids(
                &s,
                &build_view(&s, &View::List("excluded".into()), now(), &no_expand())
            ),
            ["future", "done"]
        );
    }
    let s = snapshot(
        vec![
            TaskBuilder::new("done", "inbox")
                .completed_at("2026-11-04 09:00")
                .build(),
        ],
        &[(
            "inbox",
            ListPrefs {
                show_in_done: false,
                ..Default::default()
            },
        )],
    );
    assert_eq!(
        uids(&s, &build_view(&s, &View::All, now(), &no_expand())),
        ["done"]
    );
    assert!(
        build_view(&s, &View::Done, now(), &no_expand())
            .groups
            .is_empty()
    );
}

#[test]
fn future_section_uses_due_order_across_scheduled_and_long_range_tasks() {
    let s = snapshot(
        vec![
            TaskBuilder::new("long-range", "inbox")
                .due("2027-01-01")
                .build(),
            TaskBuilder::new("scheduled", "inbox")
                .start("2026-11-05")
                .due("2026-11-06")
                .build(),
        ],
        &[],
    );
    let r = build_view(&s, &View::All, now(), &no_expand());
    assert_eq!(r.groups.len(), 1);
    assert_eq!(r.groups[0].0, DateGroup::Future);
    assert_eq!(uids(&s, &r), ["scheduled", "long-range"]);
}

#[test]
fn list_done_sections_keep_only_thirty_local_calendar_days() {
    let mut unknown = TaskBuilder::new("unknown-date", "inbox").build();
    unknown.status = Status::Completed;
    let s = snapshot(
        vec![
            TaskBuilder::new("today", "inbox")
                .completed_at("2026-11-04 09:00")
                .build(),
            TaskBuilder::new("first-day", "inbox")
                .completed_at("2026-10-06 00:00")
                .build(),
            TaskBuilder::new("too-old", "inbox")
                .completed_at("2026-10-05 23:59")
                .build(),
            TaskBuilder::new("tomorrow", "inbox")
                .completed_at("2026-11-05 09:00")
                .build(),
            unknown,
        ],
        &[],
    );
    for view in [View::All, View::List("inbox".into())] {
        let r = build_view(&s, &view, now(), &no_expand());
        assert_eq!(uids(&s, &r), ["today", "first-day"]);
        assert_eq!(r.counts.total, 0);
        let tomorrow = now() + chrono::Duration::days(1);
        let r = build_view(&s, &view, tomorrow, &no_expand());
        assert_eq!(uids(&s, &r), ["tomorrow", "today"]);
    }
    let r = build_view(&s, &View::Done, now(), &no_expand());
    assert_eq!(
        uids(&s, &r),
        ["tomorrow", "today", "first-day", "too-old", "unknown-date"]
    );
    assert_eq!(r.counts.total, 5);
}

#[test]
fn expired_completed_parent_does_not_hide_recent_children() {
    let s = snapshot(
        vec![
            TaskBuilder::new("old-parent", "inbox")
                .kind(TaskKind::Project)
                .completed_at("2026-10-01 10:00")
                .build(),
            TaskBuilder::new("recent-child", "inbox")
                .parent("old-parent")
                .completed_at("2026-11-04 09:00")
                .build(),
        ],
        &[],
    );
    for view in [View::All, View::List("inbox".into())] {
        let r = build_view(&s, &view, now(), &no_expand());
        assert_eq!(uids(&s, &r), ["recent-child"]);
        assert_eq!(r.groups[0].1[0].parent_hint, Some(idx(&s, "old-parent")));
    }
}
