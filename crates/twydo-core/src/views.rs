//! Views over a loaded vdir: membership, date groups, sorting and sidebar counts (spec §7, §8).
//!
//! Everything here is a pure function of a [`Snapshot`] and the caller's `now`; nothing writes.
//! Calendar arithmetic uses `now`'s timezone: `today` is `now.date_naive()` and days are
//! calendar days, so DST changes never shift a group.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;

use crate::config::{Config, ListPrefs, Sort, SortField};
use crate::ical::value::{IcalDateTime, Resolved};
use crate::model::repeat::{parse_rrule, rule_class};
use crate::model::{Status, Task, TaskKind};
use crate::vdir::{Collection, LoadReport};

const BROKEN_PARENT: &str = "broken parent link";
const UNTYPED_PARENT: &str = "parent type not encoded; shown as project";
const UNSUPPORTED_REPEAT: &str = "unsupported repeat rule";

/// Loaded tasks plus the parent/child structure derived from `RELATED-TO`.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub tasks: Vec<Task>,
    pub collections: Vec<Collection>,
    pub config: Config,
    /// Files the load could not read or parse, with the reason.
    pub errors: Vec<(PathBuf, String)>,
    parents: Vec<Option<usize>>,
    children: Vec<Vec<usize>>,
    sources: HashMap<PathBuf, Vec<u8>>,
}

impl Snapshot {
    /// Resolve parent links once. A link to a missing UID, and every link on a cycle, is cut
    /// and the task gets the warning `"broken parent link"`. Untyped tasks with children become
    /// projects; tasks whose repeat rule does not parse are blocked from editing.
    pub fn new(report: LoadReport, config: Config) -> Snapshot {
        let mut tasks = report.tasks;
        let mut by_uid: HashMap<&str, usize> = HashMap::new();
        for (i, t) in tasks.iter().enumerate() {
            by_uid.entry(t.uid.as_str()).or_insert(i);
        }

        let mut parents: Vec<Option<usize>> = Vec::with_capacity(tasks.len());
        let mut broken = vec![false; tasks.len()];
        for (i, t) in tasks.iter().enumerate() {
            let p = t.parent.as_deref().map(|uid| by_uid.get(uid).copied());
            match p {
                Some(None) => {
                    broken[i] = true;
                    parents.push(None);
                }
                Some(Some(p)) => parents.push(Some(p)),
                None => parents.push(None),
            }
        }
        drop(by_uid);
        cut_cycles(&mut parents, &mut broken);

        let mut children = vec![Vec::new(); tasks.len()];
        for (i, p) in parents.iter().enumerate() {
            if let Some(p) = p {
                children[*p].push(i);
            }
        }

        let tz = config.tz();
        for (i, t) in tasks.iter_mut().enumerate() {
            // Record timezone fallbacks (e.g. an unknown TZID read in local time) once.
            for v in [&t.due, &t.start, &t.completed].into_iter().flatten() {
                if let (_, Some(w)) = v.resolve(tz)
                    && !t.warnings.contains(&w)
                {
                    t.warnings.push(w);
                }
            }
            if broken[i] {
                t.warnings.push(BROKEN_PARENT.to_string());
            }
            // A Task kind here means no recognised `@tasktype` (an unrecognised value counts as missing).
            if t.kind == TaskKind::Task && !children[i].is_empty() {
                t.kind = TaskKind::Project;
                t.warnings.push(UNTYPED_PARENT.to_string());
            }
            if let Some(r) = &t.rrule
                && parse_rrule(r).is_err()
                && t.blocked.is_none()
            {
                t.blocked = Some(UNSUPPORTED_REPEAT.to_string());
            }
        }

        Snapshot {
            tasks,
            collections: report.collections,
            config,
            errors: report.errors,
            parents,
            children,
            sources: report.sources,
        }
    }

    /// Indices of the tasks whose parent is `idx`, in load order.
    pub fn children(&self, idx: usize) -> &[usize] {
        &self.children[idx]
    }

    pub fn parent(&self, idx: usize) -> Option<usize> {
        self.parents[idx]
    }

    /// How many children of `idx` are neither completed nor cancelled.
    pub fn open_items(&self, idx: usize) -> usize {
        self.children[idx]
            .iter()
            .filter(|&&c| self.tasks[c].status == Status::Open)
            .count()
    }

    /// The bytes of the file at `path` as loaded (a task's `path`).
    pub fn source(&self, path: &Path) -> Option<&[u8]> {
        self.sources.get(path).map(Vec::as_slice)
    }

    /// Replace the loaded bytes of the file at `path`: a grouped write plans its later part
    /// on what its earlier part wrote. The parsed tasks are left as they are.
    pub(crate) fn set_source(&mut self, path: &Path, bytes: Vec<u8>) {
        self.sources.insert(path.to_path_buf(), bytes);
    }

    /// Index of the first task with `uid`.
    pub fn find(&self, uid: &str) -> Option<usize> {
        self.tasks.iter().position(|t| t.uid == uid)
    }

    /// The configured timezone (see [`Config::tz`]).
    pub fn tz(&self) -> Tz {
        self.config.tz()
    }

    fn prefs(&self, idx: usize) -> ListPrefs {
        self.config.list_prefs(&self.tasks[idx].collection)
    }

    /// A checklist item: a child of a checklist.
    fn is_item(&self, idx: usize) -> bool {
        self.parent(idx)
            .is_some_and(|p| self.tasks[p].kind == TaskKind::Checklist)
    }
}

/// The kind the task's file declares. The snapshot shows a parent without `@tasktype` as a
/// project, but its file still says it is a plain task: writing a type compares with this.
pub fn declared_kind(t: &Task) -> TaskKind {
    if t.warnings.iter().any(|w| w == UNTYPED_PARENT) {
        TaskKind::Task
    } else {
        t.kind
    }
}

/// The matrix class of a task's repeat rule; `None` without an RRULE. A rule that does not
/// parse is `unrecognized`. The anchor is the due date in the snapshot's timezone.
pub fn task_rule_class(t: &Task, snap: &Snapshot) -> Option<String> {
    let raw = t.rrule.as_deref()?;
    Some(match parse_rrule(raw) {
        Ok(r) => {
            let anchor = t
                .due
                .as_ref()
                .map(|d: &IcalDateTime| d.local_date(snap.tz()));
            rule_class(&r, anchor, t.from_completion)
        }
        Err(_) => "unrecognized".to_string(),
    })
}

/// Cut the parent link of every task that lies on a cycle, marking it broken.
fn cut_cycles(parents: &mut [Option<usize>], broken: &mut [bool]) {
    // 0 = unvisited, 1 = on the current walk, 2 = finished.
    let mut state = vec![0u8; parents.len()];
    for start in 0..parents.len() {
        let mut walk = Vec::new();
        let mut cur = Some(start);
        while let Some(i) = cur {
            match state[i] {
                2 => break,
                1 => {
                    let from = walk.iter().position(|&w| w == i).expect("on walk");
                    for &c in &walk[from..] {
                        parents[c] = None;
                        broken[c] = true;
                    }
                    break;
                }
                _ => {
                    state[i] = 1;
                    walk.push(i);
                    cur = parents[i];
                }
            }
        }
        for i in walk {
            state[i] = 2;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum View {
    Today,
    All,
    Scheduled,
    Done,
    Next3Days,
    /// One collection, by id.
    List(String),
}

/// Section headers, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DateGroup {
    Overdue,
    InProgress,
    LaterToday,
    Today,
    Tomorrow,
    DayAfterTomorrow,
    Next7Days,
    Next2Weeks,
    Next30Days,
    Future,
    NoDueDate,
    Scheduled,
    CompletedOn(NaiveDate),
    /// A view sorted by title or creation: one list without a date header.
    Ungrouped,
    /// Completed work at the bottom of All and ordinary lists.
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Index into [`Snapshot::tasks`].
    pub task: usize,
    pub depth: u8,
    /// The task's parent, for rows shown away from it (or nested under it).
    pub parent_hint: Option<usize>,
}

/// Sidebar count, shown as `due_or_overdue|total`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub due_or_overdue: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewResult {
    pub groups: Vec<(DateGroup, Vec<Row>)>,
    pub counts: Counts,
}

// ---------- time ----------

/// A date-only value, or an instant in `now`'s zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum When {
    Date(NaiveDate),
    At(DateTime<Tz>),
}

impl When {
    fn of(v: &IcalDateTime, tz: Tz) -> When {
        match v.resolve(tz).0 {
            Resolved::Date(d) => When::Date(d),
            Resolved::Instant(t) => When::At(t.with_timezone(&tz)),
        }
    }

    fn date(self) -> NaiveDate {
        match self {
            When::Date(d) => d,
            When::At(t) => t.date_naive(),
        }
    }

    /// Before `now`: a date before today, or an instant before now.
    fn is_past(self, now: DateTime<Tz>) -> bool {
        match self {
            When::Date(d) => d < now.date_naive(),
            When::At(t) => t < now,
        }
    }

    /// Reached: a date on or before today, or an instant at or before now.
    fn is_reached(self, now: DateTime<Tz>) -> bool {
        match self {
            When::Date(d) => d <= now.date_naive(),
            When::At(t) => t <= now,
        }
    }

    /// Sort key: day, then timed before date-only, then instant.
    fn key(self) -> (NaiveDate, bool, Option<DateTime<Tz>>) {
        match self {
            When::Date(d) => (d, true, None),
            When::At(t) => (t.date_naive(), false, Some(t)),
        }
    }
}

struct Clock {
    now: DateTime<Tz>,
    tz: Tz,
    today: NaiveDate,
}

impl Clock {
    fn new(now: DateTime<Tz>) -> Clock {
        Clock {
            now,
            tz: now.timezone(),
            today: now.date_naive(),
        }
    }

    fn when(&self, v: Option<&IcalDateTime>) -> Option<When> {
        v.map(|v| When::of(v, self.tz))
    }

    fn overdue(&self, t: &Task) -> bool {
        self.when(t.due.as_ref())
            .is_some_and(|w| w.is_past(self.now))
    }

    fn started(&self, t: &Task) -> bool {
        self.when(t.start.as_ref())
            .is_some_and(|w| w.is_reached(self.now))
    }

    fn future_start(&self, t: &Task) -> bool {
        self.when(t.start.as_ref())
            .is_some_and(|w| !w.is_reached(self.now))
    }

    fn due_today(&self, t: &Task) -> bool {
        self.when(t.due.as_ref())
            .is_some_and(|w| w.date() == self.today)
    }

    fn due_or_overdue(&self, t: &Task) -> bool {
        self.due_today(t) || self.overdue(t)
    }
}

// ---------- membership ----------

/// Whether task `idx` belongs to view `v` on its own merits (checklist items included).
fn eligible(s: &Snapshot, idx: usize, v: &View, c: &Clock) -> bool {
    let t = &s.tasks[idx];
    let p = s.prefs(idx);
    let visible = !p.archived && !p.hidden;
    let open = t.status == Status::Open;
    let current = open && !c.future_start(t);
    match v {
        View::Today => {
            current
                && visible
                && p.show_in_today
                && (c.due_today(t) || c.overdue(t) || c.started(t))
        }
        View::All => (open || t.status == Status::Completed) && visible && p.show_in_all,
        View::List(id) => (open || t.status == Status::Completed) && &t.collection == id,
        View::Scheduled => open && c.future_start(t) && visible,
        View::Done => t.status == Status::Completed && p.show_in_done && !p.hidden,
        View::Next3Days => {
            current
                && visible
                && !c.overdue(t)
                && c.when(t.due.as_ref()).is_some_and(|w| {
                    let d = (w.date() - c.today).num_days();
                    (0..3).contains(&d)
                })
        }
    }
}

/// Checklist items are never counted, even when shown at depth 0.
fn counts_with(s: &Snapshot, v: &View, c: &Clock) -> Counts {
    let mut out = Counts::default();
    for i in 0..s.tasks.len() {
        if s.is_item(i)
            || !eligible(s, i, v, c)
            || (*v != View::Done && s.tasks[i].status != Status::Open)
        {
            continue;
        }
        out.total += 1;
        if *v != View::Done && c.due_or_overdue(&s.tasks[i]) {
            out.due_or_overdue += 1;
        }
    }
    out
}

/// Sidebar counts: eligible tasks excluding checklist items, project children included whether
/// or not they are expanded. Done reports its total only.
pub fn counts(s: &Snapshot, v: &View, now: DateTime<Tz>) -> Counts {
    counts_with(s, v, &Clock::new(now))
}

// ---------- groups ----------

fn bucket(days: i64) -> DateGroup {
    match days {
        0 => DateGroup::Today,
        1 => DateGroup::Tomorrow,
        2 => DateGroup::DayAfterTomorrow,
        3..=6 => DateGroup::Next7Days,
        7..=13 => DateGroup::Next2Weeks,
        14..=29 => DateGroup::Next30Days,
        _ => DateGroup::Future,
    }
}

fn group_by(s: &Snapshot, idx: usize, c: &Clock, in_today: bool, field: SortField) -> DateGroup {
    let t = &s.tasks[idx];
    if c.overdue(t) {
        return DateGroup::Overdue;
    }
    if in_today && c.started(t) {
        return DateGroup::InProgress;
    }
    if in_today
        && let Some(When::At(due)) = c.when(t.due.as_ref())
        && due.date_naive() == c.today
        && due >= c.now
    {
        return DateGroup::LaterToday;
    }
    let anchor = match field {
        SortField::Start => t.start.as_ref(),
        _ => t.due.as_ref(),
    };
    match c.when(anchor) {
        None => DateGroup::NoDueDate,
        Some(w) => match (w.date() - c.today).num_days() {
            // Only a start date can be in the past without the task being overdue.
            d if d < 0 => DateGroup::InProgress,
            d => bucket(d),
        },
    }
}

/// The due-date group of task `idx`. `in_today` enables the Today-only groups
/// (In Progress for started tasks, Later Today for timed tasks still ahead today).
pub fn date_group(s: &Snapshot, idx: usize, now: DateTime<Tz>, in_today: bool) -> DateGroup {
    group_by(s, idx, &Clock::new(now), in_today, SortField::Due)
}

// ---------- sorting ----------

#[derive(Clone, Copy)]
enum Key {
    Field(SortField),
    /// Completion time, newest first (Done).
    Completed,
}

fn date_cmp(a: Option<When>, b: Option<When>, descending: bool) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => {
            let o = a.key().cmp(&b.key());
            if descending { o.reverse() } else { o }
        }
        // Missing values sort last either way.
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn compare(s: &Snapshot, c: &Clock, key: Key, descending: bool, a: usize, b: usize) -> Ordering {
    let (ta, tb) = (&s.tasks[a], &s.tasks[b]);
    let primary = match key {
        Key::Field(SortField::Due) => {
            date_cmp(c.when(ta.due.as_ref()), c.when(tb.due.as_ref()), descending)
        }
        Key::Field(SortField::Start) => date_cmp(
            c.when(ta.start.as_ref()),
            c.when(tb.start.as_ref()),
            descending,
        ),
        Key::Field(SortField::Creation) => date_cmp(
            c.when(ta.created.as_ref()),
            c.when(tb.created.as_ref()),
            descending,
        ),
        Key::Field(SortField::Title) => {
            let o = ta.summary.to_lowercase().cmp(&tb.summary.to_lowercase());
            if descending { o.reverse() } else { o }
        }
        Key::Completed => date_cmp(
            c.when(ta.completed.as_ref()),
            c.when(tb.completed.as_ref()),
            !descending,
        ),
    };
    primary
        .then_with(|| ta.summary.to_lowercase().cmp(&tb.summary.to_lowercase()))
        .then_with(|| ta.summary.cmp(&tb.summary))
        .then_with(|| ta.uid.cmp(&tb.uid))
}

/// Nested items retain their sequence independently of the containing list's sort.
/// Phone captures have tied (zero) positions; creation time orders those older items.
/// `tz` resolves floating creation timestamps consistently with the current view.
pub fn compare_items(s: &Snapshot, tz: Tz, a: usize, b: usize) -> Ordering {
    let (a, b) = (&s.tasks[a], &s.tasks[b]);
    a.meta
        .display_order()
        .cmp(&b.meta.display_order())
        .then_with(|| {
            date_cmp(
                a.created.as_ref().map(|v| When::of(v, tz)),
                b.created.as_ref().map(|v| When::of(v, tz)),
                false,
            )
        })
        .then_with(|| a.uid.cmp(&b.uid))
}

// ---------- layout ----------

struct Plan {
    key: Key,
    descending: bool,
    /// Tree views nest children under expanded parents; flat views show checklist items
    /// under their checklist (when `items` is set).
    tree: bool,
    items: bool,
}

fn plan(s: &Snapshot, v: &View) -> Plan {
    let flat = |key, items| Plan {
        key,
        descending: false,
        tree: false,
        items,
    };
    match v {
        View::Today | View::Next3Days => flat(Key::Field(SortField::Due), true),
        View::Scheduled => flat(Key::Field(SortField::Start), true),
        View::Done => flat(Key::Completed, false),
        View::All => Plan {
            key: Key::Field(Sort::default().field),
            descending: Sort::default().descending,
            tree: true,
            items: false,
        },
        View::List(id) => {
            let sort = s.config.list_prefs(id).sort;
            Plan {
                key: Key::Field(sort.field),
                descending: sort.descending,
                tree: true,
                items: false,
            }
        }
    }
}

/// Tree sections are independent of ordinary date buckets: active families stay together.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TreeSection {
    Active,
    Future,
    Completed,
}

fn tree_section(s: &Snapshot, c: &Clock, idx: usize) -> TreeSection {
    if s.tasks[idx].status == Status::Completed {
        TreeSection::Completed
    } else if c.future_start(&s.tasks[idx]) {
        TreeSection::Future
    } else {
        TreeSection::Active
    }
}

fn group_of(s: &Snapshot, v: &View, plan: &Plan, c: &Clock, idx: usize) -> DateGroup {
    if plan.tree {
        match tree_section(s, c, idx) {
            TreeSection::Completed => return DateGroup::Done,
            TreeSection::Future => return DateGroup::Future,
            TreeSection::Active => {}
        }
    }
    match (v, plan.key) {
        (View::Scheduled, _) => DateGroup::Scheduled,
        (View::Done, _) => c
            .when(s.tasks[idx].completed.as_ref())
            .map_or(DateGroup::Ungrouped, |w| DateGroup::CompletedOn(w.date())),
        (_, Key::Field(SortField::Title | SortField::Creation) | Key::Completed) => {
            DateGroup::Ungrouped
        }
        (_, Key::Field(f)) => group_by(s, idx, c, *v == View::Today, f),
    }
}

/// Group display order: enum order, except Done lists the newest completion day first.
fn group_order(a: &DateGroup, b: &DateGroup) -> Ordering {
    match (a, b) {
        (DateGroup::CompletedOn(x), DateGroup::CompletedOn(y)) => y.cmp(x),
        _ => a.cmp(b),
    }
}

/// Append `idx` and, when expanded, its eligible descendants.
#[allow(clippy::too_many_arguments)]
fn push_tree(
    s: &Snapshot,
    c: &Clock,
    in_view: &[bool],
    expanded: &HashSet<String>,
    idx: usize,
    depth: u8,
    out: &mut Vec<Row>,
) {
    out.push(Row {
        task: idx,
        depth,
        parent_hint: s.parent(idx),
    });
    if !expanded.contains(&s.tasks[idx].uid) {
        return;
    }
    let mut kids: Vec<usize> = s
        .children(idx)
        .iter()
        .copied()
        .filter(|&k| in_view[k] && tree_section(s, c, k) == tree_section(s, c, idx))
        .collect();
    kids.sort_by(|&a, &b| compare_items(s, c.tz, a, b));
    for k in kids {
        push_tree(s, c, in_view, expanded, k, depth.saturating_add(1), out);
    }
}

/// Rows of view `v` under date-group headers, plus its counts. `expanded` holds the UIDs of
/// parents to show open in All and list views. The result is deterministic.
pub fn build_view(
    s: &Snapshot,
    v: &View,
    now: DateTime<Tz>,
    expanded: &HashSet<String>,
) -> ViewResult {
    let c = Clock::new(now);
    let plan = plan(s, v);
    let in_view: Vec<bool> = (0..s.tasks.len()).map(|i| eligible(s, i, v, &c)).collect();

    // Tree roots include children whose parent is absent or in another section. Ordinary
    // active date buckets do not split families. Flat views omit checklist items here.
    let mut top: Vec<usize> = (0..s.tasks.len())
        .filter(|&i| in_view[i])
        .filter(|&i| {
            if plan.tree {
                s.parent(i)
                    .is_none_or(|p| !in_view[p] || tree_section(s, &c, p) != tree_section(s, &c, i))
            } else {
                !s.is_item(i)
            }
        })
        .collect();
    top.sort_by(|&a, &b| {
        if plan.tree {
            let a_done = s.tasks[a].status == Status::Completed;
            let b_done = s.tasks[b].status == Status::Completed;
            a_done.cmp(&b_done).then_with(|| {
                if a_done {
                    compare(s, &c, Key::Completed, false, a, b)
                } else {
                    compare(s, &c, plan.key, plan.descending, a, b)
                }
            })
        } else {
            compare(s, &c, plan.key, plan.descending, a, b)
        }
    });

    let mut groups: Vec<(DateGroup, Vec<Row>)> = Vec::new();
    for idx in top {
        let g = group_of(s, v, &plan, &c, idx);
        let rows = match groups.iter_mut().find(|(k, _)| *k == g) {
            Some((_, rows)) => rows,
            None => {
                groups.push((g, Vec::new()));
                &mut groups.last_mut().expect("just pushed").1
            }
        };
        if plan.tree {
            push_tree(s, &c, &in_view, expanded, idx, 0, rows);
            continue;
        }
        rows.push(Row {
            task: idx,
            depth: 0,
            parent_hint: s.parent(idx),
        });
        if plan.items && s.tasks[idx].kind == TaskKind::Checklist {
            let mut items: Vec<usize> = s
                .children(idx)
                .iter()
                .copied()
                .filter(|&k| s.tasks[k].status == Status::Open && !c.future_start(&s.tasks[k]))
                .collect();
            items.sort_by(|&a, &b| compare_items(s, c.tz, a, b));
            rows.extend(items.into_iter().map(|k| Row {
                task: k,
                depth: 1,
                parent_hint: Some(idx),
            }));
        }
    }
    groups.sort_by(|a, b| {
        if plan.tree {
            match (a.0, b.0) {
                (DateGroup::Ungrouped, DateGroup::Ungrouped) => return Ordering::Equal,
                (DateGroup::Ungrouped, _) => return Ordering::Less,
                (_, DateGroup::Ungrouped) => return Ordering::Greater,
                _ => {}
            }
        }
        group_order(&a.0, &b.0)
    });

    ViewResult {
        groups,
        counts: counts_with(s, v, &c),
    }
}
