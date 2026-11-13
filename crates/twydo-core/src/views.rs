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
use sha2::{Digest, Sha256};

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
    /// UIDs of tasks with a queued edit laid over them.
    pending: HashSet<String>,
    /// uid -> [`rev_of`] the file's bytes.
    revs: HashMap<String, String>,
}

/// A task file's revision: the first 16 lowercase hex digits of the SHA-256 of its bytes.
pub fn rev_of(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))[..16].to_string()
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
            if broken[i] && !t.warnings.iter().any(|w| w == BROKEN_PARENT) {
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

        let revs = tasks
            .iter()
            .filter_map(|t| {
                report
                    .sources
                    .get(&t.path)
                    .map(|b| (t.uid.clone(), rev_of(b)))
            })
            .collect();
        Snapshot {
            tasks,
            collections: report.collections,
            config,
            errors: report.errors,
            parents,
            children,
            sources: report.sources,
            pending: HashSet::new(),
            revs,
        }
    }

    /// A snapshot with no tasks and no lists: what a client shows before the daemon has sent
    /// one.
    pub fn empty(config: Config) -> Snapshot {
        Snapshot::new(LoadReport::default(), config)
    }

    /// Replaces the per-task revisions (uid -> rev), e.g. with those a daemon sent.
    pub fn with_revs(mut self, revs: HashMap<String, String>) -> Snapshot {
        self.revs = revs;
        self
    }

    /// The revision of task `idx`'s file, when known.
    pub fn rev(&self, idx: usize) -> Option<&str> {
        self.revs.get(&self.tasks[idx].uid).map(String::as_str)
    }

    /// Marks the tasks with these UIDs as having a queued, not yet applied, edit.
    pub fn with_pending(mut self, uids: HashSet<String>) -> Snapshot {
        self.pending = uids;
        self
    }

    /// Whether task `idx` shows a queued edit that is not on disk yet.
    pub fn is_pending(&self, idx: usize) -> bool {
        self.pending.contains(&self.tasks[idx].uid)
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

    /// The last 30 local calendar days, including today. Unknown dates stay in Done only.
    fn recently_completed(&self, t: &Task) -> bool {
        t.status == Status::Completed
            && self
                .when(t.completed.as_ref())
                .is_some_and(|w| (0..30).contains(&(self.today - w.date()).num_days()))
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
        View::All => (open || c.recently_completed(t)) && visible && p.show_in_all,
        View::List(id) => (open || c.recently_completed(t)) && &t.collection == id,
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
    /// Tree views nest children under expanded parents; flat views show an open parent's
    /// children that are not rows of their own. With `items`, checklists start open.
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

/// Which parents show their children, shared by every view: expanding or collapsing a parent in
/// one view does the same in the others. A parent in neither set is collapsed, except that
/// checklists in Today, Next 3 Days and Scheduled start open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Expansion {
    pub expanded: HashSet<String>,
    pub collapsed: HashSet<String>,
}

impl FromIterator<String> for Expansion {
    /// Expanded parents, by UID.
    fn from_iter<I: IntoIterator<Item = String>>(uids: I) -> Self {
        Expansion {
            expanded: uids.into_iter().collect(),
            collapsed: HashSet::new(),
        }
    }
}

impl Expansion {
    fn opens_by_default(s: &Snapshot, v: &View, idx: usize) -> bool {
        plan(s, v).items && s.tasks[idx].kind == TaskKind::Checklist
    }

    /// Whether parent `idx` shows its children in view `v`.
    pub fn is_open(&self, s: &Snapshot, v: &View, idx: usize) -> bool {
        let uid = &s.tasks[idx].uid;
        if Self::opens_by_default(s, v, idx) {
            !self.collapsed.contains(uid)
        } else {
            self.expanded.contains(uid)
        }
    }

    /// Expand parent `idx` if it is collapsed in view `v`, else collapse it, in every view.
    pub fn toggle(&mut self, s: &Snapshot, v: &View, idx: usize) {
        let uid = &s.tasks[idx].uid;
        if self.is_open(s, v, idx) {
            self.expanded.remove(uid);
            self.collapsed.insert(uid.clone());
        } else {
            self.open(uid);
        }
    }

    /// Open the parent with this UID in every view.
    pub fn open(&mut self, uid: &str) {
        self.expanded.insert(uid.to_string());
        self.collapsed.remove(uid);
    }
}

/// Whether child `k` of an open parent belongs beneath it in flat view `v`, when it is not a
/// row of its own: completed children in Done, open ones elsewhere (in Today and Next 3 Days
/// only those already started).
fn flat_child(s: &Snapshot, v: &View, c: &Clock, k: usize) -> bool {
    let t = &s.tasks[k];
    match v {
        View::Done => t.status == Status::Completed,
        View::Scheduled => t.status == Status::Open,
        _ => t.status == Status::Open && !c.future_start(t),
    }
}

/// Tree sections are independent of ordinary date buckets: active families stay together.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TreeSection {
    Active,
    Future,
    Completed,
}

/// A task's section. An open child with no due or start date follows its open parent in the
/// view, as checklist items do: it has no date of its own to place it elsewhere.
fn tree_section(s: &Snapshot, c: &Clock, in_view: &[bool], idx: usize) -> TreeSection {
    let t = &s.tasks[idx];
    if t.status == Status::Completed {
        return TreeSection::Completed;
    }
    if c.future_start(t) {
        return TreeSection::Future;
    }
    match s.parent(idx) {
        Some(p)
            if t.due.is_none()
                && t.start.is_none()
                && in_view[p]
                && s.tasks[p].status == Status::Open =>
        {
            tree_section(s, c, in_view, p)
        }
        _ => TreeSection::Active,
    }
}

fn group_of(
    s: &Snapshot,
    v: &View,
    plan: &Plan,
    c: &Clock,
    in_view: &[bool],
    idx: usize,
) -> DateGroup {
    if plan.tree {
        match tree_section(s, c, in_view, idx) {
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

/// One view's placement rules over a snapshot.
struct Layout<'a> {
    s: &'a Snapshot,
    v: &'a View,
    plan: Plan,
    c: Clock,
    in_view: Vec<bool>,
}

impl<'a> Layout<'a> {
    fn new(s: &'a Snapshot, v: &'a View, now: DateTime<Tz>) -> Self {
        let c = Clock::new(now);
        let in_view = (0..s.tasks.len()).map(|i| eligible(s, i, v, &c)).collect();
        Layout {
            s,
            v,
            plan: plan(s, v),
            c,
            in_view,
        }
    }

    fn section(&self, idx: usize) -> TreeSection {
        tree_section(self.s, &self.c, &self.in_view, idx)
    }

    /// Whether task `i` is a row of its own. Tree roots include children whose parent is
    /// absent or in another section; ordinary active date buckets do not split families.
    /// Flat views list every eligible task except checklist items.
    fn is_root(&self, i: usize) -> bool {
        let s = self.s;
        self.in_view[i]
            && if self.plan.tree {
                s.parent(i)
                    .is_none_or(|p| !self.in_view[p] || self.section(p) != self.section(i))
            } else {
                !s.is_item(i)
            }
    }

    /// The children shown beneath `parent` when it is open, in item order. Tree views nest
    /// children in the parent's section; flat views nest those not already rows of their own.
    fn nested(&self, parent: usize) -> Vec<usize> {
        let s = self.s;
        let mut kids: Vec<usize> = s
            .children(parent)
            .iter()
            .copied()
            .filter(|&k| {
                if self.plan.tree {
                    self.in_view[k] && self.section(k) == self.section(parent)
                } else {
                    !self.is_root(k) && flat_child(s, self.v, &self.c, k)
                }
            })
            .collect();
        kids.sort_by(|&a, &b| compare_items(s, self.c.tz, a, b));
        kids
    }

    /// Append `idx` and, when it is open, its nested descendants.
    fn push_family(&self, expansion: &Expansion, idx: usize, depth: u8, out: &mut Vec<Row>) {
        out.push(Row {
            task: idx,
            depth,
            parent_hint: self.s.parent(idx),
        });
        if self.s.children(idx).is_empty() || !expansion.is_open(self.s, self.v, idx) {
            return;
        }
        for k in self.nested(idx) {
            self.push_family(expansion, k, depth.saturating_add(1), out);
        }
    }
}

/// How many of parent `idx`'s children appear beneath it in view `v` when it is open. Zero
/// when they are all rows of their own, or none belongs in the view.
pub fn nested_count(s: &Snapshot, v: &View, now: DateTime<Tz>, idx: usize) -> usize {
    Layout::new(s, v, now).nested(idx).len()
}

/// Rows of view `v` under date-group headers, plus its counts. `expansion` says which parents
/// show their children. The result is deterministic.
pub fn build_view(s: &Snapshot, v: &View, now: DateTime<Tz>, expansion: &Expansion) -> ViewResult {
    let layout = Layout::new(s, v, now);
    let (c, plan) = (&layout.c, &layout.plan);
    let mut top: Vec<usize> = (0..s.tasks.len()).filter(|&i| layout.is_root(i)).collect();
    top.sort_by(|&a, &b| {
        if plan.tree {
            let a_done = s.tasks[a].status == Status::Completed;
            let b_done = s.tasks[b].status == Status::Completed;
            a_done.cmp(&b_done).then_with(|| {
                if a_done {
                    compare(s, c, Key::Completed, false, a, b)
                } else {
                    compare(s, c, plan.key, plan.descending, a, b)
                }
            })
        } else {
            compare(s, c, plan.key, plan.descending, a, b)
        }
    });

    let mut groups: Vec<(DateGroup, Vec<Row>)> = Vec::new();
    for idx in top {
        let g = group_of(s, v, plan, c, &layout.in_view, idx);
        let rows = match groups.iter_mut().find(|(k, _)| *k == g) {
            Some((_, rows)) => rows,
            None => {
                groups.push((g, Vec::new()));
                &mut groups.last_mut().expect("just pushed").1
            }
        };
        layout.push_family(expansion, idx, 0, rows);
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
        counts: counts_with(s, v, c),
    }
}
