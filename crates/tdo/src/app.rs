//! TUI application state and vim-style keys. No terminal here: the renderer reads the public
//! fields and the terminal loop feeds `handle_key` / `tick`.

mod newrow;

pub use newrow::{NewRow, default_list};

use std::cell::Cell;
use std::collections::HashSet;
use std::time::Duration;

use chrono::DateTime;
use chrono_tz::Tz;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::form::{EditForm, Field, FormAction, NO_CHANGES};
use crate::write::Request;
use twydo_core::conflicts::Conflict;
use twydo_core::dateinput::{DueShift, shift_due};
use twydo_core::edit::{Change, Op};
use twydo_core::model::{Status, TaskKind};
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
    /// The edit form is open.
    Insert,
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
    /// Edit these notes in `$EDITOR`; the loop reports back with [`App::notes_edited`].
    EditNotes(String),
    /// Stop the running sync, then write; the loop reports back with [`App::write_finished`].
    StopSyncAndWrite(Request),
}

/// The list picker, in sidebar order: `m` moves task `uid` to the chosen list; from the
/// card's List field (`uid` = `None`) the choice only sets that field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPicker {
    pub uid: Option<String>,
    pub lists: Vec<String>,
    pub cursor: usize,
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
    /// A success note that clears itself, with the time the loop first saw it. Only clears
    /// `message` while it still holds that text.
    transient: Option<(String, Option<DateTime<Tz>>)>,
    pub show_help: bool,
    /// The `:errors` overlay listing unreadable files.
    pub show_errors: bool,
    /// Sync state for the status line and conflict markers; `None` without `[sync]`.
    pub sync: Option<SyncSummary>,
    /// The `:conflicts` overlay.
    pub show_conflicts: bool,
    pub conflict_cursor: usize,
    /// The `m` list picker, open until its write finishes or Esc.
    pub picker: Option<ListPicker>,
    /// The edit card; open while `mode` is Insert.
    pub form: Option<EditForm>,
    /// The snapshot the form opened on: its save is checked against these bytes, so a sync
    /// that lands meanwhile makes the save stale instead of being overwritten.
    form_base: Option<Snapshot>,
    /// The new row (`o`, `O`, `a`); open while `mode` is Insert.
    pub new_row: Option<NewRow>,
    /// A create sent to the store, to follow the new task after the reload.
    adding: Option<newrow::Adding>,
    /// A task `a` is converting into a parent; its item row opens when that write succeeds.
    converting: Option<String>,
    pub now: DateTime<Tz>,
    /// Half-page rows for Ctrl-d / Ctrl-u.
    pub page: usize,
    pending: Option<char>,
    /// The message text that reports a failed reload (alone or after a write's result); a
    /// successful reload clears the message while it still shows this.
    reload_error: Option<String>,
    details_visible: bool,
    /// Lines the Details pane is scrolled down, for the task `details_uid`.
    details_scroll: u16,
    details_uid: Option<String>,
    /// The furthest the Details pane can scroll; the renderer sets it on each draw.
    pub(crate) details_max: Cell<u16>,
    /// A write a running sync blocked, and the question asking whether to stop the sync.
    stop_prompt: Option<(Request, String)>,
    /// The snapshot that blocked write was made against, kept until its retry finishes, so
    /// the retry is checked against what the user decided on rather than a later reload.
    stop_base: Option<Snapshot>,
    /// A repeating task being completed. Completing keeps its UID on the next occurrence, so
    /// the next reload keeps the cursor's row instead of following that UID to its new date.
    /// Cleared by [`App::write_refused`]: after a refused write the task is still open.
    completing_repeat: Option<String>,
}

/// How long something has run: `45s`, `2m 14s`, `1h 3m`.
pub fn fmt_elapsed(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m {}s", s / 60, s % 60),
        _ => format!("{}h {}m", s / 3600, s % 3600 / 60),
    }
}

/// Terminal width at which the Details pane becomes visible.
pub(crate) const DETAILS_MIN_TERMINAL: u16 = 110;

const RELOAD_FAILED: &str = "reload failed: ";
/// The end of the question a parent's `x` asks before completing its open items.
const CONFIRM_X: &str = "? x to confirm";
/// The message a sync that went well leaves.
pub const SYNC_DONE: &str = "sync done";
/// After declining to stop a sync.
const NOT_SAVED: &str = "not saved; the sync is still running";
/// How long a success message stays.
const MESSAGE_SECS: i64 = 5;

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
            transient: None,
            show_help: false,
            show_errors: false,
            sync: None,
            show_conflicts: false,
            conflict_cursor: 0,
            picker: None,
            form: None,
            form_base: None,
            new_row: None,
            adding: None,
            converting: None,
            now,
            page: 10,
            pending: None,
            reload_error: None,
            details_visible: true,
            details_scroll: 0,
            details_uid: None,
            completing_repeat: None,
            stop_prompt: None,
            stop_base: None,
            details_max: Cell::new(0),
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

    /// Clear a success message [`MESSAGE_SECS`] seconds after the loop first saw it. Other
    /// messages (errors, blocked reasons) stay until Esc or the next message.
    pub fn expire_message(&mut self, now: DateTime<Tz>) {
        let Some((text, seen)) = &mut self.transient else {
            return;
        };
        if self.message.as_ref() != Some(text) {
            self.transient = None;
            return;
        }
        match seen {
            None => *seen = Some(now),
            Some(t) if (now - *t).num_seconds() >= MESSAGE_SECS => {
                self.message = None;
                self.transient = None;
            }
            Some(_) => {}
        }
    }

    /// Store `now`; keep the selection on the same task as the view rebuilds around it.
    /// A pending `d`, `g`, `x` or `c` is dropped when the selected task leaves the view.
    pub fn tick(&mut self, now: DateTime<Tz>) {
        let uid = self.selected_uid();
        self.now = now;
        self.restore(uid.clone());
        if self.selected_uid() != uid {
            self.drop_pending();
        }
    }

    /// Forget a pending `d`, `g`, `x` or `a`; the question of an `x` or `a` goes with it.
    fn drop_pending(&mut self) {
        let asked = match self.pending.take() {
            Some('x') => |m: &str| m.ends_with(CONFIRM_X),
            Some('a') => |m: &str| m == newrow::ASK_ITEMS,
            _ => return,
        };
        if self.message.as_deref().is_some_and(asked) {
            self.message = None;
        }
    }

    /// Swap in a freshly loaded snapshot, keeping the cursor on the same UID if it survives.
    pub fn reload(&mut self, s: Snapshot) {
        let completing = self.completing_repeat.take();
        let uid = self
            .selected_uid()
            .filter(|u| completing.as_ref() != Some(u));
        let side = self.sidebar_items().get(self.sidebar_cursor).cloned();
        self.snapshot = s;
        // The row under a pending `d`, `g` or `x` may have changed: the second key starts afresh.
        self.drop_pending();
        if self
            .reload_error
            .take()
            .is_some_and(|e| self.message.as_ref() == Some(&e))
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
        let text = format!("{RELOAD_FAILED}{e}");
        self.reload_error = Some(text.clone());
        self.message = Some(text);
    }

    /// Store a freshly read sync summary and keep the overlay cursor in range.
    pub fn set_sync(&mut self, s: Option<SyncSummary>) {
        self.sync = s;
        let n = self.conflicts().len();
        self.conflict_cursor = self.conflict_cursor.min(n.saturating_sub(1));
    }

    /// A background sync ended: show its first stderr line, or `sync done`.
    pub fn sync_finished(&mut self, message: String) {
        self.transient = (message == SYNC_DONE).then(|| (message.clone(), None));
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

    /// [`App::handle_key`] with the clock brought up to `now` first, so a date key never
    /// uses a day the minute tick has not caught up with. When that moves the selection to
    /// another task (the selected one left the view) and the key acts on the selected task,
    /// it is dropped: it was meant for a task that is no longer under the cursor, and the new
    /// one has not been drawn yet. Forms and the picker keep their own task, and command,
    /// search, overlay and sidebar keys do not use it, so their input always goes through.
    pub fn key_at(&mut self, k: KeyEvent, now: DateTime<Tz>) -> Outcome {
        let uses_task_cursor = self.mode == Mode::Normal
            && self.stop_prompt.is_none()
            && self.form.is_none()
            && self.new_row.is_none()
            && self.picker.is_none()
            && !self.overlay_open()
            && self.focus != Pane::Sidebar;
        let before = self.selected_uid();
        self.tick(now);
        if uses_task_cursor && self.selected_uid() != before {
            self.message = Some("the view changed; check the selection and try again".into());
            return Outcome::Continue;
        }
        self.handle_key(k)
    }

    /// Lines to scroll the Details pane down: 0 once another task is selected.
    pub fn details_offset(&self) -> u16 {
        if self.details_uid.is_some() && self.details_uid == self.selected_uid() {
            self.details_scroll.min(self.details_max.get())
        } else {
            0
        }
    }

    fn scroll_details(&mut self, delta: i32) {
        let to =
            (i32::from(self.details_offset()) + delta).clamp(0, i32::from(self.details_max.get()));
        self.details_scroll = to as u16;
        self.details_uid = self.selected_uid();
    }

    pub fn handle_key(&mut self, k: KeyEvent) -> Outcome {
        let before = self.selected_uid();
        let out = self.dispatch_key(k);
        if self.selected_uid() != before {
            self.details_scroll = 0;
            self.details_uid = None;
        }
        out
    }

    fn dispatch_key(&mut self, k: KeyEvent) -> Outcome {
        if self.stop_prompt.is_some() {
            return self.stop_prompt_key(k);
        }
        if self.picker.is_some() {
            return self.picker_key(k);
        }
        if self.form.is_some() {
            return self.form_key(k);
        }
        if self.new_row.is_some() {
            return self.row_key(k);
        }
        match self.mode {
            Mode::Normal => self.normal_key(k),
            Mode::Search => {
                self.search_key(k);
                Outcome::Continue
            }
            Mode::Command => self.command_key(k),
            Mode::Insert => Outcome::Continue,
        }
    }

    fn block(&mut self, action: &str) {
        self.message = Some(blocked(action));
    }

    fn normal_key(&mut self, k: KeyEvent) -> Outcome {
        let pending = self.pending.take();
        if pending == Some('a') {
            return self.answer_items(k);
        }
        if pending == Some('x') && !matches!(k.code, KeyCode::Char('x' | ' ')) {
            self.message = None;
        }
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
            (KeyCode::Char('d'), true) if self.focus == Pane::Details => {
                self.scroll_details(half as i32)
            }
            (KeyCode::Char('u'), true) if self.focus == Pane::Details => {
                self.scroll_details(-half as i32)
            }
            (KeyCode::Char('d'), true) => self.move_cursor(half),
            (KeyCode::Char('u'), true) => self.move_cursor(-half),
            (KeyCode::Char('r'), true) if !self.overlay_open() => {
                return Outcome::Write(Request::Redo);
            }
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
            self.open_form(Field::Title);
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
        // An overlay hides the task list: keys that write or edit must not reach it.
        if self.overlay_open() && "xtT[]{}duimroOac ".contains(c) {
            return Outcome::Continue;
        }
        if let Some(op) = self.task_write(c, pending) {
            let req = Request::Apply(op);
            self.note_repeat_completion(&req);
            return Outcome::Write(req);
        }
        // Details is a page, not a list: movement keys scroll it.
        if self.focus == Pane::Details {
            let delta = match c {
                'j' => Some(1),
                'k' => Some(-1),
                'G' => Some(i32::from(u16::MAX)),
                'g' if pending == Some('g') => Some(-i32::from(u16::MAX)),
                _ => None,
            };
            if let Some(d) = delta {
                self.scroll_details(d);
                return Outcome::Continue;
            }
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
            'o' | 'O' if self.focus != Pane::Sidebar => self.open_new_row(c == 'o'),
            'a' if self.focus != Pane::Sidebar => self.add_items(),
            'i' => self.open_form(Field::Title),
            'J' | 'K' => self.block("move task"),
            'r' => self.open_form(Field::Repeat),
            'm' => self.open_picker(),
            _ => {}
        }
        Outcome::Continue
    }

    /// The write a key makes on the selected task: none outside the task panes, on an empty
    /// view, or for other keys.
    fn task_write(&mut self, c: char, pending: Option<char>) -> Option<Op> {
        if self.focus == Pane::Sidebar {
            return None;
        }
        let idx = self.selected()?;
        let task = &self.snapshot.tasks[idx];
        let uid = task.uid.clone();
        let shift = match c {
            'x' | ' ' if task.status == Status::Completed => return Some(Op::Reopen { uid }),
            'x' | ' ' => {
                let open = self.snapshot.open_items(idx);
                if open > 0 && pending != Some('x') {
                    self.message = Some(format!(
                        "complete \"{}\" and its {open} open {}{CONFIRM_X}",
                        task.summary,
                        if open == 1 { "item" } else { "items" }
                    ));
                    self.pending = Some('x');
                    return None;
                }
                return Some(Op::Complete { uid });
            }
            'd' if pending == Some('d') => return Some(Op::Delete { uid }),
            'c' if pending != Some('c') => {
                self.pending = Some('c');
                return None;
            }
            't' | 'p' | 'c' if pending == Some('c') => {
                let (kind, name) = match c {
                    't' => (TaskKind::Task, "task"),
                    'p' => (TaskKind::Project, "project"),
                    _ => (TaskKind::Checklist, "checklist"),
                };
                // The type the file declares: an untyped parent shows as a project.
                if twydo_core::views::declared_kind(task) == kind {
                    self.message = Some(format!("\"{}\" is already a {name}", task.summary));
                    return None;
                }
                return Some(Op::Save {
                    uid,
                    changes: vec![Change::Kind(kind)],
                    to: None,
                });
            }
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

    /// The card for task `idx` of the current snapshot.
    fn card_for(&self, idx: usize) -> EditForm {
        let s = &self.snapshot;
        let item_of = s.parent(idx).map(|p| s.tasks[p].kind);
        EditForm::open(&s.tasks[idx], item_of, s.tz())
    }

    /// Open the edit card on the selected task with `focus`.
    fn open_form(&mut self, focus: Field) {
        if self.focus == Pane::Sidebar {
            return;
        }
        let Some(i) = self.selected() else { return };
        let mut form = self.card_for(i);
        if !form.fields().contains(&focus) {
            self.message = Some("repeat: checklist items have no dates or repeats".into());
            return;
        }
        form.focus = focus;
        self.form = Some(form);
        self.form_base = Some(self.snapshot.clone());
        self.mode = Mode::Insert;
    }

    fn close_form(&mut self) {
        self.form = None;
        self.form_base = None;
        self.mode = Mode::Normal;
    }

    /// The snapshot an [`Outcome::Write`] applies against: the one the open card was built
    /// from when it edits a task, else the current one. A new task (row or card) writes no
    /// file that exists, so nothing can go stale under it; it is checked against the tasks as
    /// they are now, so a parent deleted or closed meanwhile refuses the item.
    pub fn write_snapshot(&self) -> &Snapshot {
        if let Some(base) = &self.stop_base {
            return base;
        }
        match (&self.form, &self.form_base) {
            (Some(f), Some(base)) if f.uid.is_some() => base,
            _ => &self.snapshot,
        }
    }

    /// After a refused save: if the task changed on disk since the form opened, rebuild the
    /// form from the current version so the next save cannot overwrite what the user has not
    /// seen. When the task is gone the form closes and the error becomes the message.
    fn refresh_form(&mut self, error: String) {
        let (Some(form), Some(base)) = (&self.form, &self.form_base) else {
            return;
        };
        // A new task has nothing on disk to change under it.
        let Some(uid) = form.uid.clone() else {
            self.form.as_mut().expect("form is open").error = Some(error);
            return;
        };
        let old = base.find(&uid).map(|i| &base.tasks[i]);
        let Some(now_idx) = self.snapshot.find(&uid) else {
            self.close_form();
            self.message = Some(error);
            return;
        };
        let task = &self.snapshot.tasks[now_idx];
        let changed = old.is_none_or(|o| {
            o.path != task.path || base.source(&o.path) != self.snapshot.source(&task.path)
        });
        let focus = form.focus;
        let mut form = if changed {
            let mut f = self.card_for(now_idx);
            if f.fields().contains(&focus) {
                f.focus = focus;
            }
            f.error = Some(format!("{error}; the form now shows the saved version"));
            self.form_base = Some(self.snapshot.clone());
            f
        } else {
            let mut f = form.clone();
            f.error = Some(error);
            f
        };
        std::mem::swap(self.form.as_mut().expect("form is open"), &mut form);
    }

    fn form_key(&mut self, k: KeyEvent) -> Outcome {
        let today = self.now.date_naive();
        let work_days = self.snapshot.config.work_days.clone();
        let Some(form) = self.form.as_mut() else {
            return Outcome::Continue;
        };
        match form.key(k) {
            FormAction::None => {}
            FormAction::EditNotes => return Outcome::EditNotes(form.notes.clone()),
            FormAction::PickList => {
                let current = form.list.clone();
                self.open_list_picker(None, &current);
            }
            FormAction::Cancel => self.close_form(),
            FormAction::Save => match form.save(today, &work_days) {
                Err(e) if e == NO_CHANGES => {
                    self.close_form();
                    self.message = Some(e);
                }
                Err(e) => form.error = Some(e),
                Ok(op) => {
                    form.error = None;
                    self.note_create(&op);
                    return Outcome::Write(Request::Apply(op));
                }
            },
        }
        Outcome::Continue
    }

    /// Help, `:errors` or `:conflicts` is drawn over the task list.
    fn overlay_open(&self) -> bool {
        self.show_help || self.show_errors || self.show_conflicts
    }

    fn open_picker(&mut self) {
        if self.focus == Pane::Sidebar {
            return;
        }
        let Some(i) = self.selected() else { return };
        let task = &self.snapshot.tasks[i];
        let (uid, current) = (task.uid.clone(), task.collection.clone());
        self.open_list_picker(Some(uid), &current);
    }

    /// The picker over every sidebar list but `current`.
    fn open_list_picker(&mut self, uid: Option<String>, current: &str) {
        let lists: Vec<String> = self
            .sidebar_items()
            .into_iter()
            .filter_map(|v| match v {
                View::List(id) if id != current => Some(id),
                _ => None,
            })
            .collect();
        if lists.is_empty() {
            self.message = Some("no other list to move to".into());
            return;
        }
        self.picker = Some(ListPicker {
            uid,
            lists,
            cursor: 0,
        });
    }

    fn picker_key(&mut self, k: KeyEvent) -> Outcome {
        let Some(p) = self.picker.as_mut() else {
            return Outcome::Continue;
        };
        let last = p.lists.len().saturating_sub(1);
        match k.code {
            KeyCode::Esc => self.picker = None,
            KeyCode::Char('j') | KeyCode::Down => p.cursor = (p.cursor + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => p.cursor = p.cursor.saturating_sub(1),
            KeyCode::Enter => {
                let Some(to) = p.lists.get(p.cursor).cloned() else {
                    return Outcome::Continue;
                };
                match p.uid.clone() {
                    Some(uid) => return Outcome::Write(Request::Apply(Op::Move { uid, to })),
                    None => {
                        self.picker = None;
                        if let Some(form) = self.form.as_mut() {
                            form.set_list(to);
                        }
                    }
                }
            }
            _ => {}
        }
        Outcome::Continue
    }

    /// Pasted text (bracketed paste). It only ever fills a form field: a paste never runs
    /// as keys, so a pasted newline cannot save, complete or delete anything.
    pub fn paste(&mut self, text: &str) -> Outcome {
        if let Some(form) = self.form.as_mut() {
            form.paste(text);
        } else {
            self.row_paste(text);
        }
        Outcome::Continue
    }

    /// The editor from [`Outcome::EditNotes`] closed: take its text, or keep the notes and
    /// say why not.
    pub fn notes_edited(&mut self, result: Result<String, String>) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        match result {
            Ok(notes) => {
                form.set_notes(notes);
                form.error = None;
            }
            Err(e) => form.error = Some(format!("notes: {e}")),
        }
    }

    /// Remember a write that completes a repeating task, for the reload that follows it.
    fn note_repeat_completion(&mut self, req: &Request) {
        self.completing_repeat = match req {
            Request::Apply(Op::Complete { uid })
                if self
                    .snapshot
                    .tasks
                    .iter()
                    .any(|t| &t.uid == uid && t.rrule.is_some()) =>
            {
                Some(uid.clone())
            }
            _ => None,
        };
    }

    /// The write just made was refused. Call it before the reload that follows, so that
    /// reload keeps the cursor on the selected task, which nothing has changed.
    pub fn write_refused(&mut self) {
        self.completing_repeat = None;
    }

    /// A write found the vdir busy. With a duration: keep `req` with `base`, the snapshot it
    /// was made against (taken before any reload), and ask the stop question. Without (nothing
    /// the TUI can stop): the plain busy message, as a refused write.
    pub fn write_busy(&mut self, req: Request, running_for: Option<Duration>, base: Snapshot) {
        let Some(d) = running_for else {
            self.write_finished(Err(twydo_core::store::WriteError::Busy.to_string()));
            return;
        };
        let question = format!(
            "sync running for {} — stop it and save? (y/n)",
            fmt_elapsed(d)
        );
        self.transient = None;
        self.message = Some(question.clone());
        self.stop_prompt = Some((req, question));
        self.stop_base = Some(base);
    }

    /// The open stop question, if any; the status line shows it over any mode's hints.
    pub fn stop_question(&self) -> Option<&str> {
        self.stop_prompt.as_ref().map(|(_, q)| q.as_str())
    }

    /// `y` stops the sync and writes; `n` or Esc gives up the write; other keys do nothing.
    fn stop_prompt_key(&mut self, k: KeyEvent) -> Outcome {
        match k.code {
            KeyCode::Char('y') => {
                let (req, _) = self.stop_prompt.take().expect("asking");
                self.note_repeat_completion(&req);
                Outcome::StopSyncAndWrite(req)
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.stop_prompt = None;
                self.write_finished(Err(NOT_SAVED.into()));
                Outcome::Continue
            }
            _ => {
                // Keep the question in sight if something replaced it meanwhile.
                self.message = self.stop_prompt.as_ref().map(|(_, q)| q.clone());
                Outcome::Continue
            }
        }
    }

    /// A write requested by [`Outcome::Write`] ended: show its result.
    pub fn write_finished(&mut self, result: Result<String, String>) {
        self.picker = None;
        self.stop_base = None;
        let adding = self.adding.take();
        let converting = self.converting.take();
        // A refused edit keeps the form (and its typing, unless the task changed meanwhile),
        // with the reason in it; a refused row keeps its text too.
        if let Err(e) = &result {
            if self.form.is_some() {
                self.refresh_form(e.clone());
                return;
            }
            if self.row_refused(e) {
                return;
            }
        }
        if self.form.is_some() {
            self.close_form();
        }
        // Inside a parent, the saved row opens the next one (the `Enter` chain).
        match self.new_row.as_mut() {
            Some(row) if result.is_ok() && row.target.parent.is_some() => {
                row.title.clear();
                row.error = None;
            }
            _ => self.close_row(),
        }
        let ok = result.is_ok();
        // The loop reloads before reporting: a failed reload must stay visible, so it
        // follows the result and the message does not clear itself.
        let reload_error = self
            .reload_error
            .take()
            .filter(|e| self.message.as_ref() == Some(e));
        let clears = result.is_ok() && reload_error.is_none();
        let mut text = match result {
            Ok(m) | Err(m) => m,
        };
        if let Some(e) = reload_error {
            text = format!("{text} · {e}");
            self.reload_error = Some(text.clone());
        }
        self.transient = clears.then(|| (text.clone(), None));
        self.message = Some(text);
        if ok && let Some(added) = adding {
            self.follow_created(added);
        }
        if ok && let Some(uid) = converting {
            self.converted(&uid);
        }
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
