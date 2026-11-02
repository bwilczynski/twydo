//! TUI application state and vim-style keys. No terminal here: the renderer reads the public
//! fields and the terminal loop feeds `handle_key` / `tick`.

use std::collections::HashSet;

use chrono::DateTime;
use chrono_tz::Tz;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::write::Request;
use twydo_core::conflicts::Conflict;
use twydo_core::dateinput::{DueShift, shift_due};
use twydo_core::edit::{Change, Op};
use twydo_core::model::Status;
use twydo_core::sync::SyncSummary;
use twydo_core::sync::resolve::Keep;
use twydo_core::views::{Row, Snapshot, View, build_view};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Sidebar,
    Tasks,
    Details,
}

impl Pane {
    /// The name shown in the status line.
    pub fn name(self) -> &'static str {
        match self {
            Pane::Sidebar => "Sidebar",
            Pane::Tasks => "Tasks",
            Pane::Details => "Details",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Search,
    Command,
}

/// One sidebar line: a group heading, or a selectable view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarEntry {
    Heading(String),
    Item(View),
}

/// The views behind the number keys 1-5.
const NUMBERED_VIEWS: [View; 5] = [
    View::Today,
    View::All,
    View::Scheduled,
    View::Done,
    View::Next3Days,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    Quit,
    /// Run `tdo sync`, with `--force-delete <list>` when given.
    Sync {
        force_delete: Option<String>,
    },
    /// Keep one version of conflict `id`.
    Resolve {
        id: String,
        keep: Keep,
    },
    /// Write to the task store; the loop reports back with [`App::write_finished`].
    Write(Request),
}

pub struct App {
    pub snapshot: Snapshot,
    pub view: View,
    pub focus: Pane,
    pub cursor: usize,
    pub sidebar_cursor: usize,
    pub expanded: HashSet<String>,
    pub mode: Mode,
    /// The text being typed in Search or Command mode.
    pub input: String,
    /// The active search filter (live while typing, kept after Enter).
    pub search: Option<String>,
    pub message: Option<String>,
    pub show_help: bool,
    /// The `:errors` overlay listing unreadable files.
    pub show_errors: bool,
    /// Sync state for the status line and conflict markers; `None` without `[sync]`.
    pub sync: Option<SyncSummary>,
    /// The `:conflicts` overlay.
    pub show_conflicts: bool,
    pub conflict_cursor: usize,
    pub now: DateTime<Tz>,
    /// Half-page rows for Ctrl-d / Ctrl-u.
    pub page: usize,
    pending: Option<char>,
    details_visible: bool,
}

/// Terminal width at which the Details pane becomes visible.
pub(crate) const DETAILS_MIN_TERMINAL: u16 = 110;

const RELOAD_FAILED: &str = "reload failed: ";

fn blocked(action: &str) -> String {
    format!("{action}: not available yet")
}

impl App {
    pub fn new(snapshot: Snapshot, now: DateTime<Tz>) -> App {
        App {
            snapshot,
            view: View::Today,
            focus: Pane::Tasks,
            cursor: 0,
            sidebar_cursor: 0,
            expanded: HashSet::new(),
            mode: Mode::Normal,
            input: String::new(),
            search: None,
            message: None,
            show_help: false,
            show_errors: false,
            sync: None,
            show_conflicts: false,
            conflict_cursor: 0,
            now,
            page: 10,
            pending: None,
            details_visible: true,
        }
    }

    /// Whether the Details pane is drawn; set by [`App::resize`], which must run before drawing.
    pub fn details_visible(&self) -> bool {
        self.details_visible
    }

    /// Keep keyboard focus on a visible pane and paging in step with terminal size.
    pub fn resize(&mut self, width: u16, height: u16) {
        self.details_visible = width >= DETAILS_MIN_TERMINAL;
        self.page = (height.saturating_sub(3) / 2).max(1) as usize;
        if !self.details_visible && self.focus == Pane::Details {
            self.focus = Pane::Tasks;
        }
    }

    /// The sidebar in display order: the four main views, then visible lists (each configured
    /// group under its heading, in order; the rest by display name under "Lists"), then the
    /// smart lists. The renderer draws exactly this, so navigation follows what is shown.
    pub fn sidebar_entries(&self) -> Vec<SidebarEntry> {
        let s = &self.snapshot;
        let shown = |id: &str| {
            let p = s.config.list_prefs(id);
            !p.archived && !p.hidden && s.collections.iter().any(|c| c.id == id)
        };
        let name = |c: &twydo_core::vdir::Collection| {
            c.display_name.clone().unwrap_or_else(|| c.id.clone())
        };
        let mut out: Vec<SidebarEntry> = [View::Today, View::All, View::Scheduled, View::Done]
            .into_iter()
            .map(SidebarEntry::Item)
            .collect();

        let mut placed: Vec<&str> = Vec::new();
        for g in &s.config.groups {
            let mut lists: Vec<&str> = Vec::new();
            for l in &g.lists {
                if shown(l) && !placed.contains(&l.as_str()) && !lists.contains(&l.as_str()) {
                    lists.push(l);
                }
            }
            if lists.is_empty() {
                continue;
            }
            out.push(SidebarEntry::Heading(g.name.clone()));
            for l in lists {
                placed.push(l);
                out.push(SidebarEntry::Item(View::List(l.to_string())));
            }
        }
        let mut rest: Vec<_> = s
            .collections
            .iter()
            .filter(|c| shown(&c.id) && !placed.contains(&c.id.as_str()))
            .collect();
        rest.sort_by_key(|c| (name(c).to_lowercase(), c.id.clone()));
        if !rest.is_empty() {
            out.push(SidebarEntry::Heading("Lists".into()));
            out.extend(
                rest.iter()
                    .map(|c| SidebarEntry::Item(View::List(c.id.clone()))),
            );
        }

        out.push(SidebarEntry::Heading("Smart lists".into()));
        out.push(SidebarEntry::Item(View::Next3Days));
        out
    }

    /// The selectable sidebar entries, in display order; `sidebar_cursor` indexes this.
    pub fn sidebar_items(&self) -> Vec<View> {
        self.sidebar_entries()
            .into_iter()
            .filter_map(|e| match e {
                SidebarEntry::Item(v) => Some(v),
                SidebarEntry::Heading(_) => None,
            })
            .collect()
    }

    /// The current view's rows in group order, search filter applied. Headers are not rows.
    pub fn visible_rows(&self) -> Vec<Row> {
        let result = build_view(&self.snapshot, &self.view, self.now, &self.expanded);
        let needle = self.search.as_ref().map(|q| q.to_lowercase());
        result
            .groups
            .into_iter()
            .flat_map(|(_, rows)| rows)
            .filter(|r| match &needle {
                None => true,
                Some(q) if q.is_empty() => true,
                Some(q) => {
                    let t = &self.snapshot.tasks[r.task];
                    t.summary.to_lowercase().contains(q)
                        || t.description
                            .as_ref()
                            .is_some_and(|d| d.to_lowercase().contains(q))
                }
            })
            .collect()
    }

    /// Task index of the row under the cursor.
    pub fn selected(&self) -> Option<usize> {
        self.visible_rows().get(self.cursor).map(|r| r.task)
    }

    fn selected_uid(&self) -> Option<String> {
        self.selected().map(|i| self.snapshot.tasks[i].uid.clone())
    }

    /// Put the cursor back on `uid` if it is still visible, else clamp it.
    fn restore(&mut self, uid: Option<String>) {
        let rows = self.visible_rows();
        if let Some(uid) = uid
            && let Some(p) = rows
                .iter()
                .position(|r| self.snapshot.tasks[r.task].uid == uid)
        {
            self.cursor = p;
        }
        self.cursor = self.cursor.min(rows.len().saturating_sub(1));
    }

    /// Store `now`; keep the selection on the same task as the view rebuilds around it.
    pub fn tick(&mut self, now: DateTime<Tz>) {
        let uid = self.selected_uid();
        self.now = now;
        self.restore(uid);
    }

    /// Swap in a freshly loaded snapshot, keeping the cursor on the same UID if it survives.
    pub fn reload(&mut self, s: Snapshot) {
        let uid = self.selected_uid();
        let side = self.sidebar_items().get(self.sidebar_cursor).cloned();
        self.snapshot = s;
        if self
            .message
            .as_deref()
            .is_some_and(|m| m.starts_with(RELOAD_FAILED))
        {
            self.message = None;
        }
        if let View::List(id) = &self.view
            && !self.snapshot.collections.iter().any(|c| &c.id == id)
        {
            self.view = View::Today;
        }
        // Keep the sidebar cursor on the same entry; fall back to the open view if it is gone.
        match side.and_then(|v| self.sidebar_items().iter().position(|x| *x == v)) {
            Some(p) => self.sidebar_cursor = p,
            None => self.sync_sidebar(),
        }
        self.restore(uid);
    }

    /// Report a failed reload; the message stays until a reload succeeds or Esc.
    pub fn reload_failed(&mut self, e: impl std::fmt::Display) {
        self.message = Some(format!("{RELOAD_FAILED}{e}"));
    }

    /// Store a freshly read sync summary and keep the overlay cursor in range.
    pub fn set_sync(&mut self, s: Option<SyncSummary>) {
        self.sync = s;
        let n = self.conflicts().len();
        self.conflict_cursor = self.conflict_cursor.min(n.saturating_sub(1));
    }

    /// A background sync ended: show its first stderr line, or `sync done`.
    pub fn sync_finished(&mut self, message: String) {
        self.message = Some(message);
    }

    /// The unresolved conflicts, as of the last summary.
    pub fn conflicts(&self) -> &[Conflict] {
        self.sync.as_ref().map_or(&[], |s| &s.conflicts)
    }

    /// The unresolved conflict on the task `uid`, if any.
    pub fn conflict_for(&self, uid: &str) -> Option<&Conflict> {
        self.conflicts().iter().find(|c| c.uid == uid)
    }

    fn overlay_key(&mut self, k: KeyEvent) -> Option<Outcome> {
        let n = self.conflicts().len();
        match k.code {
            KeyCode::Esc => self.show_conflicts = false,
            KeyCode::Char('j') => {
                self.conflict_cursor = (self.conflict_cursor + 1).min(n.saturating_sub(1))
            }
            KeyCode::Char('k') => self.conflict_cursor = self.conflict_cursor.saturating_sub(1),
            KeyCode::Char(c @ ('l' | 'r' | 'c')) => {
                let keep = match c {
                    'l' => Keep::Local,
                    'r' => Keep::Remote,
                    _ => Keep::Current,
                };
                if let Some(c) = self.conflicts().get(self.conflict_cursor) {
                    return Some(Outcome::Resolve {
                        id: c.id.clone(),
                        keep,
                    });
                }
            }
            _ => return None,
        }
        Some(Outcome::Continue)
    }

    fn sync_sidebar(&mut self) {
        let items = self.sidebar_items();
        self.sidebar_cursor = items
            .iter()
            .position(|v| *v == self.view)
            .unwrap_or(0)
            .min(items.len().saturating_sub(1));
    }

    fn open(&mut self, view: View) {
        self.view = view;
        self.cursor = 0;
        self.search = None;
        self.sync_sidebar();
    }

    /// Move the cursor of the focused list: the sidebar's, or the task rows'.
    fn move_cursor(&mut self, delta: isize) {
        let sidebar = self.focus == Pane::Sidebar;
        let len = if sidebar {
            self.sidebar_items().len()
        } else {
            self.visible_rows().len()
        };
        let cursor = if sidebar {
            &mut self.sidebar_cursor
        } else {
            &mut self.cursor
        };
        let max = len.saturating_sub(1) as isize;
        *cursor = (*cursor as isize).saturating_add(delta).clamp(0, max) as usize;
    }

    pub fn handle_key(&mut self, k: KeyEvent) -> Outcome {
        match self.mode {
            Mode::Normal => self.normal_key(k),
            Mode::Search => {
                self.search_key(k);
                Outcome::Continue
            }
            Mode::Command => self.command_key(k),
        }
    }

    fn block(&mut self, action: &str) {
        self.message = Some(blocked(action));
    }

    fn normal_key(&mut self, k: KeyEvent) -> Outcome {
        let pending = self.pending.take();
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if self.show_conflicts
            && !ctrl
            && let Some(o) = self.overlay_key(k)
        {
            return o;
        }
        let half = self.page.max(1) as isize;
        match (k.code, ctrl) {
            (KeyCode::Char('c'), true) => return Outcome::Quit,
            (KeyCode::Char('d'), true) => self.move_cursor(half),
            (KeyCode::Char('u'), true) => self.move_cursor(-half),
            (KeyCode::Char('r'), true) => return Outcome::Write(Request::Redo),
            (_, true) => {}
            (KeyCode::Esc, _) => {
                self.message = None;
                self.show_help = false;
                self.show_errors = false;
                self.show_conflicts = false;
                self.search = None;
                self.cursor = self.cursor.min(self.visible_rows().len().saturating_sub(1));
            }
            (KeyCode::Tab, _) => {
                self.focus = match self.focus {
                    Pane::Sidebar => Pane::Tasks,
                    Pane::Tasks if self.details_visible => Pane::Details,
                    Pane::Tasks => Pane::Sidebar,
                    Pane::Details => Pane::Sidebar,
                }
            }
            (KeyCode::Enter, _) => match self.focus {
                Pane::Sidebar => self.open_sidebar(),
                _ => self.enter_on_task(),
            },
            (KeyCode::Char(c), _) => return self.normal_char(c, pending),
            _ => {}
        }
        Outcome::Continue
    }

    fn open_sidebar(&mut self) {
        if let Some(v) = self.sidebar_items().get(self.sidebar_cursor).cloned() {
            self.open(v);
            self.focus = Pane::Tasks;
        }
    }

    fn enter_on_task(&mut self) {
        let Some(i) = self.selected() else { return };
        if self.snapshot.children(i).is_empty() {
            self.block("edit task");
            return;
        }
        let uid = self.snapshot.tasks[i].uid.clone();
        if !self.expanded.remove(&uid) {
            self.expanded.insert(uid);
        }
    }

    fn normal_char(&mut self, c: char, pending: Option<char>) -> Outcome {
        if self.focus == Pane::Sidebar && c == 'l' {
            self.open_sidebar();
            return Outcome::Continue;
        }
        if let Some(op) = self.task_write(c, pending) {
            return Outcome::Write(Request::Apply(op));
        }
        match c {
            'j' => self.move_cursor(1),
            'k' => self.move_cursor(-1),
            'g' if pending == Some('g') => self.move_cursor(isize::MIN),
            'g' => self.pending = Some('g'),
            'G' => self.move_cursor(isize::MAX),
            'h' => {
                self.focus = match self.focus {
                    Pane::Details => Pane::Tasks,
                    _ => Pane::Sidebar,
                }
            }
            'l' => {
                self.focus = match self.focus {
                    Pane::Sidebar => Pane::Tasks,
                    _ if self.details_visible => Pane::Details,
                    _ => Pane::Tasks,
                }
            }
            '1'..='5' => self.open(NUMBERED_VIEWS[c as usize - '1' as usize].clone()),
            '/' => {
                self.mode = Mode::Search;
                self.input.clear();
                self.search = None;
            }
            'n' => self.step_match(1),
            'N' => self.step_match(-1),
            ':' => {
                self.mode = Mode::Command;
                self.input.clear();
            }
            '?' => self.show_help = !self.show_help,
            'd' if pending != Some('d') => self.pending = Some('d'),
            'u' => return Outcome::Write(Request::Undo),
            'o' => self.block("new task"),
            'O' => self.block("new item"),
            'i' => self.block("edit task"),
            'J' | 'K' => self.block("move task"),
            'r' => self.block("edit repeat"),
            'a' => self.block("edit alerts"),
            'm' => self.block("move to list"),
            _ => {}
        }
        Outcome::Continue
    }

    /// The write a key makes on the selected task: none outside the task panes, on an empty
    /// view, or for other keys.
    fn task_write(&self, c: char, pending: Option<char>) -> Option<Op> {
        if self.focus == Pane::Sidebar {
            return None;
        }
        let task = &self.snapshot.tasks[self.selected()?];
        let uid = task.uid.clone();
        let shift = match c {
            'x' | ' ' if task.status == Status::Completed => return Some(Op::Reopen { uid }),
            'x' | ' ' => return Some(Op::Complete { uid }),
            'd' if pending == Some('d') => return Some(Op::Delete { uid }),
            't' => DueShift::Today,
            'T' => DueShift::Tomorrow,
            ']' => DueShift::Days(1),
            '[' => DueShift::Days(-1),
            '}' => DueShift::Days(7),
            '{' => DueShift::Days(-7),
            _ => return None,
        };
        let due = shift_due(
            task.due.as_ref(),
            self.snapshot.tz(),
            self.now.date_naive(),
            shift,
        );
        Some(Op::Edit {
            uid,
            changes: vec![Change::Due(Some(due))],
        })
    }

    /// A write requested by [`Outcome::Write`] ended: show its result.
    pub fn write_finished(&mut self, result: Result<String, String>) {
        self.message = Some(match result {
            Ok(m) | Err(m) => m,
        });
    }

    /// With the filter applied every visible row is a match, so n/N cycle through the rows.
    fn step_match(&mut self, dir: isize) {
        let len = self.visible_rows().len() as isize;
        if len == 0 {
            return;
        }
        self.cursor = (self.cursor as isize + dir).rem_euclid(len) as usize;
    }

    fn search_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.input.clear();
                self.search = None;
            }
            KeyCode::Enter => {
                self.mode = Mode::Normal;
                self.input.clear();
            }
            KeyCode::Backspace => {
                self.input.pop();
                self.search = Some(self.input.clone());
            }
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.push(c);
                self.search = Some(self.input.clone());
            }
            _ => return,
        }
        self.cursor = 0;
    }

    fn command_key(&mut self, k: KeyEvent) -> Outcome {
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.input.clear();
            }
            KeyCode::Backspace => {
                if self.input.pop().is_none() {
                    self.mode = Mode::Normal;
                }
            }
            KeyCode::Enter => {
                let line = std::mem::take(&mut self.input);
                self.mode = Mode::Normal;
                return self.run_command(line.trim());
            }
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => self.input.push(c),
            _ => {}
        }
        Outcome::Continue
    }

    fn run_command(&mut self, line: &str) -> Outcome {
        let (cmd, arg) = match line.split_once(char::is_whitespace) {
            Some((c, a)) => (c, a.trim()),
            None => (line, ""),
        };
        match cmd {
            "" => {}
            "q" | "quit" => return Outcome::Quit,
            "sync" | "sync!" => {
                if cmd == "sync!" && arg.is_empty() {
                    self.message = Some("usage: :sync! <list>".into());
                } else {
                    self.message = Some("syncing…".into());
                    let force_delete = (cmd == "sync!").then(|| arg.to_string());
                    return Outcome::Sync { force_delete };
                }
            }
            "conflicts" => self.show_conflicts = !self.show_conflicts,
            "errors" => self.show_errors = !self.show_errors,
            "list" => {
                let want = arg.to_lowercase();
                let found = self.snapshot.collections.iter().find(|c| {
                    c.display_name
                        .as_deref()
                        .is_some_and(|n| n.to_lowercase() == want)
                        || c.id.to_lowercase() == want
                });
                match found.map(|c| c.id.clone()) {
                    Some(id) => self.open(View::List(id)),
                    None => self.message = Some(format!("unknown list: {arg}")),
                }
            }
            other => self.message = Some(format!("unknown command: {other}")),
        }
        Outcome::Continue
    }
}
