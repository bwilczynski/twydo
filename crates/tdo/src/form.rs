//! The `i` edit form: one text field per editable task property. Pure state; the app turns
//! [`FormAction::Save`] into an edit operation and [`FormAction::EditNotes`] into `$EDITOR`.

use chrono::NaiveDate;
use chrono_tz::Tz;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use twydo_core::dateinput::{describe, input_text, parse};
use twydo_core::edit::Change;
use twydo_core::model::Task;
use twydo_core::model::repeat::{Freq, parse_rrule};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Title,
    Due,
    Start,
    Repeat,
    Notes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormAction {
    None,
    Cancel,
    Save,
    EditNotes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Values {
    title: String,
    due: String,
    start: String,
    repeat: Option<String>,
    notes: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditForm {
    pub uid: String,
    pub focus: Field,
    pub title: String,
    pub due: String,
    pub start: String,
    /// `None`: the task has no parseable RRULE (shown as "none", not focusable).
    pub repeat: Option<String>,
    /// `days`, `weeks`, `months` or `years`.
    pub repeat_unit: &'static str,
    pub notes: String,
    /// Why the last save was refused.
    pub error: Option<String>,
    initial: Values,
}

impl EditForm {
    pub fn open(task: &Task, tz: Tz) -> EditForm {
        let rule = task.rrule.as_deref().and_then(|r| parse_rrule(r).ok());
        let values = Values {
            title: task.summary.clone(),
            due: task
                .due
                .as_ref()
                .map_or_else(String::new, |v| input_text(v, tz)),
            start: task
                .start
                .as_ref()
                .map_or_else(String::new, |v| input_text(v, tz)),
            repeat: rule.as_ref().map(|r| r.interval.to_string()),
            notes: task.description.clone().unwrap_or_default(),
        };
        let repeat_unit = match rule.map(|r| r.freq) {
            Some(Freq::Daily) => "days",
            Some(Freq::Weekly) => "weeks",
            Some(Freq::Yearly) => "years",
            _ => "months",
        };
        EditForm {
            uid: task.uid.clone(),
            focus: Field::Title,
            title: values.title.clone(),
            due: values.due.clone(),
            start: values.start.clone(),
            repeat: values.repeat.clone(),
            repeat_unit,
            notes: values.notes.clone(),
            error: None,
            initial: values,
        }
    }

    /// The fields in display and Tab order; Repeat only when the task repeats.
    pub fn fields(&self) -> Vec<Field> {
        let mut f = vec![Field::Title, Field::Due, Field::Start];
        if self.repeat.is_some() {
            f.push(Field::Repeat);
        }
        f.push(Field::Notes);
        f
    }

    fn step(&mut self, delta: isize) {
        let fields = self.fields();
        let at = fields.iter().position(|f| *f == self.focus).unwrap_or(0) as isize;
        let n = fields.len() as isize;
        self.focus = fields[(at + delta).rem_euclid(n) as usize];
    }

    fn text_mut(&mut self) -> Option<&mut String> {
        match self.focus {
            Field::Title => Some(&mut self.title),
            Field::Due => Some(&mut self.due),
            Field::Start => Some(&mut self.start),
            Field::Repeat => self.repeat.as_mut(),
            Field::Notes => None,
        }
    }

    pub fn key(&mut self, k: KeyEvent) -> FormAction {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match (k.code, ctrl) {
            (KeyCode::Esc, _) => return FormAction::Cancel,
            (KeyCode::Char('s'), true) => return FormAction::Save,
            (KeyCode::Enter, _) if self.focus == Field::Notes => return FormAction::EditNotes,
            (KeyCode::Enter, _) => return FormAction::Save,
            (KeyCode::Tab | KeyCode::Down, _) => self.step(1),
            (KeyCode::BackTab | KeyCode::Up, _) => self.step(-1),
            (KeyCode::Char('u'), true) => {
                if let Some(t) = self.text_mut() {
                    t.clear();
                }
            }
            (KeyCode::Char('w'), true) => {
                if let Some(t) = self.text_mut() {
                    let kept = t.trim_end().trim_end_matches(|c: char| !c.is_whitespace());
                    let kept = kept.trim_end().len();
                    t.truncate(kept);
                }
            }
            (KeyCode::Backspace, _) => {
                if let Some(t) = self.text_mut() {
                    t.pop();
                }
            }
            (KeyCode::Char(c), false) => {
                let digits_only = self.focus == Field::Repeat;
                if let Some(t) = self.text_mut()
                    && (!digits_only || c.is_ascii_digit())
                {
                    t.push(c);
                }
            }
            _ => {}
        }
        FormAction::None
    }

    /// `→ Fri 6 Nov 09:00`, `→ none` or the parse error, for Due and Start.
    pub fn preview(&self, f: Field, today: NaiveDate) -> Option<String> {
        let text = match f {
            Field::Due => &self.due,
            Field::Start => &self.start,
            _ => return None,
        };
        Some(match parse(text, today) {
            Ok(None) => "→ none".into(),
            Ok(Some(d)) => format!("→ {}", describe(&d, today)),
            Err(e) => e,
        })
    }

    /// The changes for the fields whose text differs from when the form opened.
    pub fn changes(&self, today: NaiveDate) -> Result<Vec<Change>, String> {
        let i = &self.initial;
        let mut out = Vec::new();
        if self.title.trim().is_empty() {
            return Err("title must not be empty".into());
        }
        if self.title != i.title {
            out.push(Change::Summary(self.title.clone()));
        }
        if self.due != i.due {
            let d = parse(&self.due, today).map_err(|e| format!("due: {e}"))?;
            out.push(Change::Due(d.map(|d| d.due())));
        }
        if self.start != i.start {
            let d = parse(&self.start, today).map_err(|e| format!("start: {e}"))?;
            if d.is_some_and(|d| d.time.is_some()) {
                return Err("start: start dates have no time".into());
            }
            out.push(Change::Start(d.map(|d| d.date)));
        }
        if self.repeat != i.repeat
            && let Some(r) = &self.repeat
        {
            match r.parse::<u32>() {
                Ok(n) if n >= 1 => out.push(Change::RepeatInterval(n)),
                _ => return Err(format!("repeat: enter a number of {}", self.repeat_unit)),
            }
        }
        if self.notes != i.notes {
            out.push(Change::Description(
                (!self.notes.is_empty()).then(|| self.notes.clone()),
            ));
        }
        Ok(out)
    }

    pub fn set_notes(&mut self, notes: String) {
        self.notes = notes;
    }
}
