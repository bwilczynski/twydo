//! The edit card (`i`, `r`, `Enter` on a leaf): one field per editable task property. The
//! model is [`twydo_core::form::Form`]; this wrapper adds the TUI's state: the focused field,
//! the last refusal and the discard question. The app turns [`FormAction::Save`] into one
//! [`twydo_core::edit::Op`] (see [`Form::save`]), [`FormAction::PickList`] into the list picker and
//! [`FormAction::EditNotes`] into `$EDITOR`. `Esc` on a card with changes asks before
//! discarding them (see [`EditForm::discarding`]).

use std::ops::{Deref, DerefMut};

use chrono::{NaiveDate, Weekday};
use chrono_tz::Tz;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use twydo_core::form::Form;
pub use twydo_core::form::{ALERT_READ_ONLY, Field, NO_CHANGES, NewTarget, Rebased};
use twydo_core::model::{Task, TaskKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormAction {
    None,
    Cancel,
    Save,
    EditNotes,
    /// `Enter` on List: open the list picker; its choice comes back through
    /// [`Form::set_list`].
    PickList,
}

/// The card: the form model plus what the terminal needs to drive it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditForm {
    pub form: Form,
    pub focus: Field,
    /// Why the last save was refused.
    pub error: Option<String>,
    /// `Esc` was pressed with changes: the card asks whether to discard them. `y` cancels,
    /// `n` or `Esc` goes back to editing, every other key is ignored.
    pub discarding: bool,
}

impl Deref for EditForm {
    type Target = Form;
    fn deref(&self) -> &Form {
        &self.form
    }
}

impl DerefMut for EditForm {
    fn deref_mut(&mut self) -> &mut Form {
        &mut self.form
    }
}

impl EditForm {
    fn wrap(form: Form) -> EditForm {
        EditForm {
            form,
            focus: Field::Title,
            error: None,
            discarding: false,
        }
    }

    /// The card for an existing task; `item_of` is its parent's kind, if it is an item.
    pub fn open(task: &Task, item_of: Option<TaskKind>, tz: Tz) -> EditForm {
        EditForm::wrap(Form::open(task, item_of, tz))
    }

    /// The card for a task not written yet, starting with `title` (see [`Form::new_task`]).
    pub fn new_task(target: NewTarget, title: String, tz: Tz) -> EditForm {
        EditForm::wrap(Form::new_task(target, title, tz))
    }

    /// This card's typing carried onto `onto` (see [`Form::rebase`]); the result starts with
    /// the default focus and no error.
    pub fn rebase(&self, onto: EditForm) -> (EditForm, Rebased) {
        let (form, rebased) = self.form.rebase(onto.form);
        (EditForm::wrap(form), rebased)
    }

    /// The line beside a field (see [`Form::preview`]); the examples show on the focused one.
    pub fn preview(&self, f: Field, today: NaiveDate, work_days: &[Weekday]) -> Option<String> {
        self.form.preview(f, today, work_days, self.focus == f)
    }

    /// Whether `Esc` would lose anything (see [`Form::is_dirty`]).
    pub fn is_dirty(&self) -> bool {
        self.form.is_dirty()
    }

    /// A key while the discard question is shown.
    fn discard_key(&mut self, k: KeyEvent) -> FormAction {
        // Only a bare or shifted letter answers: Alt-y must not discard.
        let plain = (k.modifiers - KeyModifiers::SHIFT).is_empty();
        match k.code {
            KeyCode::Char('y' | 'Y') if plain => return FormAction::Cancel,
            KeyCode::Char('n' | 'N') if plain => self.discarding = false,
            KeyCode::Esc => self.discarding = false,
            _ => {}
        }
        FormAction::None
    }

    fn step(&mut self, delta: isize) {
        let fields = self.fields();
        let at = fields.iter().position(|f| *f == self.focus).unwrap_or(0) as isize;
        let n = fields.len() as isize;
        self.focus = fields[(at + delta).rem_euclid(n) as usize];
    }

    pub fn key(&mut self, k: KeyEvent) -> FormAction {
        if self.discarding {
            return self.discard_key(k);
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match (k.code, ctrl) {
            (KeyCode::Esc, _) if self.is_dirty() => self.discarding = true,
            (KeyCode::Esc, _) => return FormAction::Cancel,
            (KeyCode::Char('s'), true) => return FormAction::Save,
            (KeyCode::Enter, _) if self.focus == Field::Notes => return FormAction::EditNotes,
            (KeyCode::Enter, _) if self.focus == Field::List => return FormAction::PickList,
            (KeyCode::Enter, _) => return FormAction::Save,
            (KeyCode::Tab | KeyCode::Down, _) => self.step(1),
            (KeyCode::BackTab | KeyCode::Up, _) => self.step(-1),
            (KeyCode::Char(' ' | 'l') | KeyCode::Right, false) if self.focus == Field::Type => {
                self.cycle_kind(1)
            }
            (KeyCode::Char('h') | KeyCode::Left, false) if self.focus == Field::Type => {
                self.cycle_kind(-1)
            }
            _ if self.focus == Field::Alert && !self.alert_editable() => {
                self.error = Some(ALERT_READ_ONLY.into());
            }
            _ => {
                let focus = self.focus;
                if let Some(t) = self.form.text_mut(focus) {
                    edit_line(t, k);
                }
            }
        }
        FormAction::None
    }

    /// Insert pasted text into the focused text field as one line: line breaks become
    /// spaces, a trailing one is dropped. Notes take no paste (they are edited in `$EDITOR`).
    pub fn paste(&mut self, text: &str) {
        if self.discarding {
            return;
        }
        let focus = self.focus;
        if let Some(t) = self.form.text_mut(focus) {
            t.push_str(&one_line(text));
        }
    }
}

/// The line-editing keys shared by the card's text fields and the new row: typing,
/// `Backspace`, `Ctrl-u` clears, `Ctrl-w` deletes the last word. Returns whether `k` was one.
pub(crate) fn edit_line(t: &mut String, k: KeyEvent) -> bool {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    match (k.code, ctrl) {
        (KeyCode::Char('u'), true) => t.clear(),
        (KeyCode::Char('w'), true) => {
            let kept = t.trim_end().trim_end_matches(|c: char| !c.is_whitespace());
            let kept = kept.trim_end().len();
            t.truncate(kept);
        }
        (KeyCode::Backspace, _) => {
            t.pop();
        }
        (KeyCode::Char(c), false) => t.push(c),
        _ => return false,
    }
    true
}

/// Pasted text as one line: line breaks become spaces, a trailing one is dropped, other
/// control characters become spaces.
pub(crate) fn one_line(text: &str) -> String {
    let line: String = text
        .split('\n')
        .map(|l| l.trim_end_matches('\r'))
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    line.trim_end().to_string()
}
