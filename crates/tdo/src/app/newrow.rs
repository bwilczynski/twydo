//! The new row (`o`, `O`, `a`): a title typed straight into the list. `Enter` saves it as one
//! [`Op::Create`]; inside a project or checklist the next item's row opens after the save
//! (the `Enter` chain). `Tab` turns the row into the card, `Esc` drops it unwritten.

use std::collections::HashSet;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use twydo_core::edit::{Change, NewTask, Op, Placement};
use twydo_core::model::TaskKind;
use twydo_core::views::{Snapshot, View};

use super::{App, Mode, Outcome};
use crate::form::{EditForm, NewTarget, edit_line, one_line};
use crate::write::Request;

/// The question `a` asks on a plain task.
pub(super) const ASK_ITEMS: &str = "Add items: make it a (p)roject or (c)hecklist?";

/// The row being typed: where the task goes and where the row is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRow {
    /// None for append; explicit sibling placement for o/O.
    pub insertion: Option<(String, Placement)>,
    pub target: NewTarget,
    pub title: String,
    /// Drawn below `anchor` (`o`, `a`, the chain), else above it (`O`).
    pub below: bool,
    /// UID of the row it is drawn next to; `None` (or not in the view): at the top.
    pub anchor: Option<String>,
    /// Why the last save was refused; typing clears it.
    pub error: Option<String>,
}

/// A create on its way to the store, to find the new task in the reload that follows.
#[derive(Debug, Clone)]
pub(super) struct Adding {
    task: NewTask,
    /// UIDs loaded before the write.
    before: HashSet<String>,
}

/// The list new tasks go to outside a list: the config's `default_list` when that list
/// exists, else the list named Inbox, else the first list. Empty when there is no list.
pub fn default_list(snap: &Snapshot) -> String {
    let lists = &snap.collections;
    let configured = snap
        .config
        .default_list
        .as_ref()
        .filter(|id| lists.iter().any(|c| &c.id == *id));
    let inbox = || {
        lists.iter().find(|c| {
            c.display_name
                .as_deref()
                .unwrap_or(&c.id)
                .eq_ignore_ascii_case("inbox")
        })
    };
    configured
        .cloned()
        .or_else(|| inbox().map(|c| c.id.clone()))
        .or_else(|| lists.first().map(|c| c.id.clone()))
        .unwrap_or_default()
}

impl App {
    /// `o` (`below`) / `O`: a new row next to the selected task, as a sibling item when that
    /// task is an item, else a task for the current view (§3.1).
    pub(super) fn open_new_row(&mut self, below: bool) {
        let selected = self.selected();
        let target = match selected.and_then(|i| self.snapshot.parent(i)) {
            Some(p) => self.item_target(p),
            None => NewTarget {
                collection: match &self.view {
                    View::List(id) => id.clone(),
                    _ => default_list(&self.snapshot),
                },
                parent: None,
                parent_kind: None,
                due_today: self.view == View::Today,
            },
        };
        if target.collection.is_empty() {
            self.message = Some("no list to add the task to".into());
            return;
        }
        let anchor = selected.map(|i| self.snapshot.tasks[i].uid.clone());
        let insertion = target.parent.as_ref().and(anchor.clone()).map(|uid| {
            (
                uid,
                if below {
                    Placement::After
                } else {
                    Placement::Before
                },
            )
        });
        self.show_row(target, anchor, below);
        if let Some(row) = &mut self.new_row {
            row.insertion = insertion;
        }
    }

    /// `a`: an item row at the end of the selected parent (or of the selected item's parent);
    /// on a plain task, ask whether to make it a project or a checklist first.
    pub(super) fn add_items(&mut self) {
        let Some(i) = self.selected() else { return };
        if let Some(p) = self.snapshot.parent(i) {
            return self.open_item_row(p);
        }
        if self.snapshot.tasks[i].kind == TaskKind::Task {
            self.message = Some(ASK_ITEMS.into());
            self.pending = Some('a');
            return;
        }
        self.open_item_row(i);
    }

    /// The key after [`ASK_ITEMS`]: `p` / `c` converts (the row opens once that write
    /// succeeds, see [`App::write_finished`]); any other key cancels and does nothing else.
    pub(super) fn answer_items(&mut self, k: KeyEvent) -> Outcome {
        self.message = None;
        let kind = match k.code {
            _ if k.modifiers.contains(KeyModifiers::CONTROL) => return Outcome::Continue,
            KeyCode::Char('p') => TaskKind::Project,
            KeyCode::Char('c') => TaskKind::Checklist,
            _ => return Outcome::Continue,
        };
        let Some(uid) = self.selected_uid() else {
            return Outcome::Continue;
        };
        self.converting = Some(uid.clone());
        Outcome::Write(Request::Apply(Op::Save {
            uid,
            changes: vec![Change::Kind(kind)],
            to: None,
        }))
    }

    /// Where an item of parent `p` goes. In Today it is due today; a checklist item's card
    /// has no Due, so it gets no date (see [`EditForm::new_task`]).
    fn item_target(&self, p: usize) -> NewTarget {
        let parent = &self.snapshot.tasks[p];
        NewTarget {
            collection: parent.collection.clone(),
            parent: Some(parent.uid.clone()),
            parent_kind: Some(parent.kind),
            due_today: self.view == View::Today,
        }
    }

    /// The item row at the end of parent `p`'s items, expanding it.
    fn open_item_row(&mut self, p: usize) {
        let uid = self.snapshot.tasks[p].uid.clone();
        self.expanded.insert(uid);
        let anchor = self.last_item_anchor(p);
        let target = self.item_target(p);
        self.show_row(target, Some(anchor), true);
    }

    fn last_item_anchor(&self, p: usize) -> String {
        self.visible_rows()
            .iter()
            // Detached Future/Done children are not anchors for this parent's item row.
            .rfind(|r| {
                self.snapshot.parent(r.task) == Some(p)
                    && (r.depth > 0 || !matches!(self.view, View::All | View::List(_)))
            })
            .map(|r| self.snapshot.tasks[r.task].uid.clone())
            .unwrap_or_else(|| self.snapshot.tasks[p].uid.clone())
    }

    fn show_row(&mut self, target: NewTarget, anchor: Option<String>, below: bool) {
        self.message = None;
        self.new_row = Some(NewRow {
            insertion: None,
            target,
            title: String::new(),
            below,
            anchor,
            error: None,
        });
        self.mode = Mode::Insert;
    }

    pub(super) fn close_row(&mut self) {
        if self.new_row.take().is_some() && self.form.is_none() {
            self.mode = Mode::Normal;
        }
    }

    pub(super) fn row_key(&mut self, k: KeyEvent) -> Outcome {
        let Some(row) = self.new_row.as_mut() else {
            return Outcome::Continue;
        };
        match k.code {
            KeyCode::Esc => self.close_row(),
            KeyCode::Enter => return self.save_row(),
            KeyCode::Tab => self.row_to_card(),
            _ => {
                if edit_line(&mut row.title, k) {
                    row.error = None;
                }
            }
        }
        Outcome::Continue
    }

    pub(super) fn row_paste(&mut self, text: &str) {
        if let Some(row) = self.new_row.as_mut() {
            row.title.push_str(&one_line(text));
            row.error = None;
        }
    }

    /// `Enter`: an empty title closes the row; otherwise the create, built by the card's
    /// rules so the row and the card write the same task.
    fn save_row(&mut self) -> Outcome {
        let Some(row) = self.new_row.as_ref() else {
            return Outcome::Continue;
        };
        if row.title.trim().is_empty() {
            self.close_row();
            return Outcome::Continue;
        }
        let mut form =
            EditForm::new_task(row.target.clone(), row.title.clone(), self.snapshot.tz());
        form.insertion = row.insertion.clone();
        let saved = form.save(self.now.date_naive(), &self.snapshot.config.work_days);
        let row = self.new_row.as_mut().expect("row is open");
        match saved {
            Ok(op) => {
                row.error = None;
                self.note_create(&op);
                Outcome::Write(Request::Apply(op))
            }
            Err(e) => {
                row.error = Some(e);
                Outcome::Continue
            }
        }
    }

    /// `Tab`: the card for the new task, with the title typed so far, on the next field.
    fn row_to_card(&mut self) {
        let Some(row) = self.new_row.take() else {
            return;
        };
        let mut form = EditForm::new_task(row.target, row.title, self.snapshot.tz());
        form.insertion = row.insertion;
        if let Some(next) = form.fields().get(1) {
            form.focus = *next;
        }
        self.form = Some(form);
        self.form_base = Some(self.snapshot.clone());
        self.mode = Mode::Insert;
    }

    /// Remember a create, so [`App::follow_created`] can find the task it wrote.
    pub(super) fn note_create(&mut self, op: &Op) {
        if let Op::Create(task) | Op::CreateAt { task, .. } = op {
            self.adding = Some(Adding {
                task: task.clone(),
                before: self.snapshot.tasks.iter().map(|t| t.uid.clone()).collect(),
            });
        }
    }

    /// A row save was refused with `e`: keep the row and its text with the reason. Returns
    /// false (the row closes, the caller shows `e`) when there is no row, or when the row's
    /// parent is gone (deleted elsewhere mid-chain), so no item can be saved under it.
    pub(super) fn row_refused(&mut self, e: &str) -> bool {
        let Some(row) = self.new_row.as_mut() else {
            return false;
        };
        let parent_gone = row
            .target
            .parent
            .as_ref()
            .is_some_and(|p| self.snapshot.find(p).is_none());
        if parent_gone {
            self.close_row();
            return false;
        }
        row.error = Some(e.to_string());
        true
    }

    /// After a successful create (the snapshot is already reloaded): put the cursor on the
    /// new task, or say where it went when the view does not show it. An open row (the
    /// `Enter` chain) moves to the end of its parent's visible items.
    pub(super) fn follow_created(&mut self, added: Adding) {
        let t = &added.task;
        let uid = self
            .snapshot
            .tasks
            .iter()
            .find(|n| {
                !added.before.contains(&n.uid)
                    && n.summary == t.summary
                    && n.collection == t.collection
                    && n.parent == t.parent
            })
            .map(|n| n.uid.clone());
        let Some(uid) = uid else { return };
        let shown = self
            .visible_rows()
            .iter()
            .any(|r| self.snapshot.tasks[r.task].uid == uid);
        if shown {
            self.restore(Some(uid.clone()));
        } else {
            let list = crate::ui::list_name(&self.snapshot, &t.collection);
            let text = format!("Added \"{}\" to {list}", t.summary);
            self.transient = Some((text.clone(), None));
            self.message = Some(text);
        }
        let anchor = self
            .new_row
            .as_ref()
            .and_then(|row| row.target.parent.as_deref())
            .and_then(|parent| self.snapshot.find(parent))
            .map(|p| self.last_item_anchor(p))
            .unwrap_or(uid);
        if let Some(row) = self.new_row.as_mut() {
            row.insertion = None;
            row.anchor = Some(anchor);
            row.below = true;
        }
    }

    /// After `a`'s conversion of `uid` succeeded: the item row inside it.
    pub(super) fn converted(&mut self, uid: &str) {
        if let Some(i) = self.snapshot.find(uid) {
            self.open_item_row(i);
        }
    }
}
