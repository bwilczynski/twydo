//! The edit card (`i`, `r`, `Enter` on a leaf): one field per editable task property. Pure
//! state; the app turns [`FormAction::Save`] into one [`Op`] (see [`EditForm::save`]),
//! [`FormAction::PickList`] into the list picker and [`FormAction::EditNotes`] into `$EDITOR`.

use chrono::{NaiveDate, Weekday};
use chrono_tz::Tz;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use twydo_core::dateinput::{self, input_text};
use twydo_core::edit::{Change, Due, NewTask, Op};
use twydo_core::model::repeat::parse_rrule;
use twydo_core::model::{Task, TaskKind};
use twydo_core::repeatinput::{self, EXAMPLES, RepeatSpec};

/// The card's fields, in Tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Title,
    List,
    Type,
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
    /// `Enter` on List: open the list picker; its choice comes back through
    /// [`EditForm::set_list`].
    PickList,
}

/// What [`EditForm::save`] returns when nothing would be written.
pub const NO_CHANGES: &str = "no changes";

/// Where a new task goes: built by the new row, which knows the view it was opened in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTarget {
    pub collection: String,
    /// UID of the checklist or project the task becomes an item of.
    pub parent: Option<String>,
    /// That parent's kind: it decides the card's fields (see [`EditForm::fields`]).
    pub parent_kind: Option<TaskKind>,
    /// Start with Due = today (a new task in Today).
    pub due_today: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Values {
    title: String,
    list: String,
    kind: TaskKind,
    due: String,
    start: String,
    repeat: String,
    notes: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditForm {
    /// The task being edited; `None` for a new task.
    pub uid: Option<String>,
    pub focus: Field,
    pub title: String,
    /// Collection id.
    pub list: String,
    pub kind: TaskKind,
    pub due: String,
    pub start: String,
    /// Typed repeat rule (`repeatinput` syntax); empty for none.
    pub repeat: String,
    pub notes: String,
    /// The kind of the parent this task is an item of, which decides the fields: a checklist
    /// item has only Title and Notes, and no item has List.
    pub item_of: Option<TaskKind>,
    /// Why the last save was refused.
    pub error: Option<String>,
    /// A new task's parent UID.
    parent: Option<String>,
    /// The task's repeat as stored, to tell an unchanged rule typed differently from a change.
    stored_repeat: Option<RepeatSpec>,
    /// The stored rule cannot be written in the field's syntax: the field starts empty and
    /// the rule stays as it is unless something is typed.
    kept_rule: bool,
    initial: Values,
}

impl EditForm {
    /// The card for an existing task; `item_of` is its parent's kind, if it is an item.
    pub fn open(task: &Task, item_of: Option<TaskKind>, tz: Tz) -> EditForm {
        let stored_repeat = task
            .rrule
            .as_deref()
            .and_then(|r| parse_rrule(r).ok())
            .map(|rule| RepeatSpec {
                rule,
                from_completion: task.from_completion,
            });
        let text = task
            .rrule
            .as_deref()
            .and_then(|r| repeatinput::to_text(r, task.from_completion));
        let kept_rule = stored_repeat.is_some() && text.is_none();
        let date = |v: &Option<_>| v.as_ref().map_or_else(String::new, |v| input_text(v, tz));
        let values = Values {
            title: task.summary.clone(),
            list: task.collection.clone(),
            kind: task.kind,
            due: date(&task.due),
            start: date(&task.start),
            repeat: text.unwrap_or_default(),
            notes: task.description.clone().unwrap_or_default(),
        };
        let mut f = EditForm::with(Some(task.uid.clone()), None, values, stored_repeat);
        f.item_of = item_of;
        f.kept_rule = kept_rule;
        f
    }

    /// The card for a task not written yet, starting with `title`. Due starts as `today` when
    /// the target asks for it; a checklist item has no Due, so it is never written.
    pub fn new_task(target: NewTarget, title: String, _tz: Tz) -> EditForm {
        let values = Values {
            title,
            list: target.collection,
            kind: TaskKind::Task,
            due: if target.due_today {
                "today".into()
            } else {
                String::new()
            },
            start: String::new(),
            repeat: String::new(),
            notes: String::new(),
        };
        let mut f = EditForm::with(None, target.parent, values, None);
        f.item_of = target.parent_kind;
        f
    }

    fn with(
        uid: Option<String>,
        parent: Option<String>,
        v: Values,
        stored_repeat: Option<RepeatSpec>,
    ) -> EditForm {
        EditForm {
            uid,
            focus: Field::Title,
            title: v.title.clone(),
            list: v.list.clone(),
            kind: v.kind,
            due: v.due.clone(),
            start: v.start.clone(),
            repeat: v.repeat.clone(),
            notes: v.notes.clone(),
            item_of: None,
            error: None,
            parent,
            stored_repeat,
            kept_rule: false,
            initial: v,
        }
    }

    /// The fields in display and Tab order. A checklist item has no dates or repeat of its
    /// own (2Do 01 §1); an item lives in its parent's list, so it has no List.
    pub fn fields(&self) -> Vec<Field> {
        match self.item_of {
            Some(TaskKind::Checklist) => vec![Field::Title, Field::Notes],
            Some(_) => vec![
                Field::Title,
                Field::Type,
                Field::Due,
                Field::Start,
                Field::Repeat,
                Field::Notes,
            ],
            None => vec![
                Field::Title,
                Field::List,
                Field::Type,
                Field::Due,
                Field::Start,
                Field::Repeat,
                Field::Notes,
            ],
        }
    }

    /// The stored rule stays because the field cannot show it (see [`EditForm::open`]).
    pub fn keeps_rule(&self) -> bool {
        self.kept_rule && self.repeat == self.initial.repeat
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
            Field::Repeat => Some(&mut self.repeat),
            Field::List | Field::Type | Field::Notes => None,
        }
    }

    /// Task → Project → Checklist, and round again.
    fn cycle_kind(&mut self, delta: isize) {
        const ORDER: [TaskKind; 3] = [TaskKind::Task, TaskKind::Project, TaskKind::Checklist];
        let at = ORDER.iter().position(|k| *k == self.kind).unwrap_or(0) as isize;
        self.kind = ORDER[(at + delta).rem_euclid(3) as usize];
    }

    pub fn key(&mut self, k: KeyEvent) -> FormAction {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match (k.code, ctrl) {
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
            _ => {
                if let Some(t) = self.text_mut() {
                    edit_line(t, k);
                }
            }
        }
        FormAction::None
    }

    /// The parsed Due field's date; an unreadable one counts as empty (its preview says why).
    fn due_date(&self, today: NaiveDate) -> Option<NaiveDate> {
        dateinput::parse(&self.due, today)
            .ok()
            .flatten()
            .map(|d| d.date)
    }

    /// The line beside a field: `→ Fri 6 Nov 09:00` for Due and Start; for Repeat the rule in
    /// 2Do's words, the examples while the field is empty and focused, or the parse error.
    pub fn preview(&self, f: Field, today: NaiveDate, work_days: &[Weekday]) -> Option<String> {
        let text = match f {
            Field::Due => &self.due,
            Field::Start => &self.start,
            Field::Repeat => return self.repeat_preview(today, work_days),
            _ => return None,
        };
        Some(match dateinput::parse(text, today) {
            Ok(None) => "→ none".into(),
            Ok(Some(d)) => format!("→ {}", dateinput::describe(&d, today)),
            Err(e) => e,
        })
    }

    fn repeat_preview(&self, today: NaiveDate, work_days: &[Weekday]) -> Option<String> {
        let due = self.due_date(today);
        if self.keeps_rule() {
            let spec = self.stored_repeat.as_ref()?;
            return Some(format!("→ {} (kept)", repeatinput::describe(spec, due)));
        }
        // Saving an emptied field on a repeating task removes the rule: say so.
        let removes = self.stored_repeat.is_some();
        if self.repeat.trim().is_empty() && !removes {
            return (self.focus == Field::Repeat).then(|| EXAMPLES.to_string());
        }
        Some(match repeatinput::parse(&self.repeat, today, work_days) {
            Ok(None) if removes => "→ none (removes the repeat)".into(),
            Ok(None) => "→ none".into(),
            Ok(Some(spec)) => {
                let mut p = format!("→ {}", repeatinput::describe(&spec, due.or(Some(today))));
                if due.is_none() {
                    p.push_str(" · due today");
                }
                p
            }
            Err(e) => e,
        })
    }

    /// The repeat to write, if the field changed it: `Some(None)` removes the rule.
    fn repeat_change(
        &self,
        today: NaiveDate,
        work_days: &[Weekday],
    ) -> Result<Option<Option<RepeatSpec>>, String> {
        if self.repeat == self.initial.repeat {
            return Ok(None);
        }
        let spec = repeatinput::parse(&self.repeat, today, work_days)
            .map_err(|e| format!("repeat: {e}"))?;
        Ok((spec != self.stored_repeat).then_some(spec))
    }

    fn checked_title(&self) -> Result<&str, String> {
        let t = self.title.trim();
        if t.is_empty() {
            return Err("title must not be empty".into());
        }
        Ok(t)
    }

    fn parsed_due(&self, today: NaiveDate) -> Result<Option<Due>, String> {
        let d = dateinput::parse(&self.due, today).map_err(|e| format!("due: {e}"))?;
        Ok(d.map(|d| d.due()))
    }

    fn parsed_start(&self, today: NaiveDate) -> Result<Option<NaiveDate>, String> {
        let d = dateinput::parse(&self.start, today).map_err(|e| format!("start: {e}"))?;
        if d.is_some_and(|d| d.time.is_some()) {
            return Err("start: start dates have no time".into());
        }
        Ok(d.map(|d| d.date))
    }

    /// The changes for the fields whose text differs from when the card opened, in field
    /// order. Setting a repeat on a task left without a due date makes it due today, as 2Do
    /// does; that `Due` comes before the `Repeat`.
    pub fn changes(&self, today: NaiveDate, work_days: &[Weekday]) -> Result<Vec<Change>, String> {
        let i = &self.initial;
        let mut out = Vec::new();
        let title = self.checked_title()?;
        if title != i.title.trim() {
            out.push(Change::Summary(title.to_string()));
        }
        if self.kind != i.kind {
            out.push(Change::Kind(self.kind));
        }
        let due_edited = self.due != i.due;
        let due = self.parsed_due(today)?;
        let repeat = self.repeat_change(today, work_days)?;
        let has_due = if due_edited {
            due.is_some()
        } else {
            !i.due.is_empty()
        };
        if matches!(repeat, Some(Some(_))) && !has_due {
            out.push(Change::Due(Some(Due::Date(today))));
        } else if due_edited {
            out.push(Change::Due(due));
        }
        if self.start != i.start {
            out.push(Change::Start(self.parsed_start(today)?));
        }
        if let Some(r) = repeat {
            out.push(Change::Repeat(r));
        }
        if self.notes != i.notes {
            out.push(Change::Description(
                (!self.notes.is_empty()).then(|| self.notes.clone()),
            ));
        }
        Ok(out)
    }

    /// The one operation this card writes: [`Op::Create`] for a new task, else [`Op::Save`]
    /// with the changed fields and, when List changed, the move. `Err(`[`NO_CHANGES`]`)` when
    /// nothing would be written.
    pub fn save(&self, today: NaiveDate, work_days: &[Weekday]) -> Result<Op, String> {
        let Some(uid) = &self.uid else {
            return self.create(today, work_days).map(Op::Create);
        };
        let changes = self.changes(today, work_days)?;
        let to = (self.list != self.initial.list).then(|| self.list.clone());
        if changes.is_empty() && to.is_none() {
            return Err(NO_CHANGES.into());
        }
        Ok(Op::Save {
            uid: uid.clone(),
            changes,
            to,
        })
    }

    /// The new task from the fields the card shows; a hidden field is never written (a
    /// checklist item gets no dates or repeat, whatever its Due started as).
    fn create(&self, today: NaiveDate, work_days: &[Weekday]) -> Result<NewTask, String> {
        let summary = self.checked_title()?.to_string();
        let shown = self.fields();
        let mut due = match shown.contains(&Field::Due) {
            true => self.parsed_due(today)?,
            false => None,
        };
        let start = match shown.contains(&Field::Start) {
            true => self.parsed_start(today)?,
            false => None,
        };
        let repeat = match shown.contains(&Field::Repeat) {
            true => repeatinput::parse(&self.repeat, today, work_days)
                .map_err(|e| format!("repeat: {e}"))?,
            false => None,
        };
        if repeat.is_some() && due.is_none() {
            due = Some(Due::Date(today));
        }
        let kind = match shown.contains(&Field::Type) {
            true => self.kind,
            false => TaskKind::Task,
        };
        Ok(NewTask {
            collection: self.list.clone(),
            summary,
            description: (!self.notes.is_empty()).then(|| self.notes.clone()),
            due,
            start,
            kind,
            parent: self.parent.clone(),
            repeat,
        })
    }

    /// Insert pasted text into the focused text field as one line: line breaks become
    /// spaces, a trailing one is dropped. Notes take no paste (they are edited in `$EDITOR`).
    pub fn paste(&mut self, text: &str) {
        if let Some(t) = self.text_mut() {
            t.push_str(&one_line(text));
        }
    }

    pub fn set_notes(&mut self, notes: String) {
        self.notes = notes;
    }

    /// The list picker's choice (a collection id).
    pub fn set_list(&mut self, list: String) {
        self.list = list;
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
