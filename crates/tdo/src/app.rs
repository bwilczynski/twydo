//! TUI application state and vim-style keys. No terminal here: the renderer reads the public
//! fields and the terminal loop feeds `handle_key` / `tick`.

use std::collections::HashSet;

use chrono::DateTime;
use chrono_tz::Tz;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use twydo_core::views::{Row, Snapshot, View, build_view};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Sidebar,
    Tasks,
    Details,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Search,
    Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    Quit,
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
    pub now: DateTime<Tz>,
    /// Half-page rows for Ctrl-d / Ctrl-u.
    pub page: usize,
    pending: Option<char>,
}

fn blocked(action: &str) -> String {
    format!("{action}: not enabled until the phase-0 compatibility gate passes")
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
            now,
            page: 10,
            pending: None,
        }
    }

    /// The sidebar entries: the five built-in views, then visible lists (configured groups
    /// first, in order; the rest by display name).
    pub fn sidebar_items(&self) -> Vec<View> {
        let s = &self.snapshot;
        let shown = |id: &str| {
            let p = s.config.list_prefs(id);
            !p.archived && !p.hidden
        };
        let name = |c: &twydo_core::vdir::Collection| {
            c.display_name.clone().unwrap_or_else(|| c.id.clone())
        };
        let mut ids: Vec<&str> = Vec::new();
        for g in &s.config.groups {
            for l in &g.lists {
                let exists = s.collections.iter().any(|c| &c.id == l);
                if exists && shown(l) && !ids.contains(&l.as_str()) {
                    ids.push(l);
                }
            }
        }
        let mut rest: Vec<_> = s
            .collections
            .iter()
            .filter(|c| shown(&c.id) && !ids.contains(&c.id.as_str()))
            .collect();
        rest.sort_by_key(|c| (name(c).to_lowercase(), c.id.clone()));
        ids.extend(rest.iter().map(|c| c.id.as_str()));

        let mut out = vec![
            View::Today,
            View::All,
            View::Scheduled,
            View::Done,
            View::Next3Days,
        ];
        out.extend(ids.into_iter().map(|id| View::List(id.to_string())));
        out
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
        self.snapshot = s;
        if let View::List(id) = &self.view
            && !self.snapshot.collections.iter().any(|c| &c.id == id)
        {
            self.view = View::Today;
        }
        self.sync_sidebar();
        self.restore(uid);
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

    fn move_cursor(&mut self, delta: isize) {
        let len = self.visible_rows().len();
        let max = len.saturating_sub(1) as isize;
        self.cursor = (self.cursor as isize + delta).clamp(0, max) as usize;
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
        let half = self.page.max(1) as isize;
        match (k.code, ctrl) {
            (KeyCode::Char('c'), true) => return Outcome::Quit,
            (KeyCode::Char('d'), true) => self.move_cursor(half),
            (KeyCode::Char('u'), true) => self.move_cursor(-half),
            (KeyCode::Char('r'), true) => self.block("redo"),
            (_, true) => {}
            (KeyCode::Esc, _) => {
                self.message = None;
                self.show_help = false;
                self.search = None;
                self.cursor = self.cursor.min(self.visible_rows().len().saturating_sub(1));
            }
            (KeyCode::Tab, _) => {
                self.focus = match self.focus {
                    Pane::Sidebar => Pane::Tasks,
                    Pane::Tasks => Pane::Details,
                    Pane::Details => Pane::Sidebar,
                }
            }
            (KeyCode::Enter, _) => match self.focus {
                Pane::Sidebar => self.open_sidebar(),
                _ => self.enter_on_task(),
            },
            (KeyCode::Char(c), _) => self.normal_char(c, pending),
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

    fn normal_char(&mut self, c: char, pending: Option<char>) {
        if self.focus == Pane::Sidebar && matches!(c, 'j' | 'k' | 'l') {
            let last = self.sidebar_items().len().saturating_sub(1);
            match c {
                'j' => self.sidebar_cursor = (self.sidebar_cursor + 1).min(last),
                'k' => self.sidebar_cursor = self.sidebar_cursor.saturating_sub(1),
                _ => self.open_sidebar(),
            }
            return;
        }
        match c {
            'j' => self.move_cursor(1),
            'k' => self.move_cursor(-1),
            'g' if pending == Some('g') => self.cursor = 0,
            'g' => self.pending = Some('g'),
            'G' => self.cursor = self.visible_rows().len().saturating_sub(1),
            'h' => {
                self.focus = match self.focus {
                    Pane::Details => Pane::Tasks,
                    _ => Pane::Sidebar,
                }
            }
            'l' => {
                self.focus = match self.focus {
                    Pane::Sidebar => Pane::Tasks,
                    _ => Pane::Details,
                }
            }
            '1'..='5' => {
                let v = self.sidebar_items()[c as usize - '1' as usize].clone();
                self.open(v);
            }
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
            'd' if pending == Some('d') => self.block("delete"),
            'd' => self.pending = Some('d'),
            'x' | ' ' => self.block("toggle done"),
            'o' => self.block("new task"),
            'O' => self.block("new item"),
            'i' => self.block("edit task"),
            'J' | 'K' => self.block("move task"),
            'u' => self.block("undo"),
            't' => self.block("due today"),
            'T' => self.block("due tomorrow"),
            ']' => self.block("due +1 day"),
            '[' => self.block("due -1 day"),
            '}' => self.block("due +1 week"),
            '{' => self.block("due -1 week"),
            'r' => self.block("edit repeat"),
            'a' => self.block("edit alerts"),
            'm' => self.block("move to list"),
            _ => {}
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
            "sync" => self.block("sync"),
            "conflicts" => self.block("conflicts"),
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
