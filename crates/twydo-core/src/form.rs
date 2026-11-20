//! The edit form's model: one field per editable task property, the card's previews and the
//! one [`Op`] a save writes (see [`Form::save`]). Pure state, shared by the TUI card
//! (`tdo::form::EditForm` adds focus, keys and the discard question) and the daemon's
//! `tasks.form`.

use crate::alertinput;
use crate::dateinput::{self, input_text};
use crate::edit::{Change, Due, NewTask, Op, Placement};
use crate::model::repeat::parse_rrule;
use crate::model::{AlertOffset, CustomAlert, Task, TaskKind};
use crate::repeatinput::{self, EXAMPLES, RepeatSpec};
use chrono::{NaiveDate, Weekday};
use chrono_tz::Tz;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The card's fields, in Tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Title,
    List,
    Type,
    Due,
    Start,
    Repeat,
    Alert,
    Notes,
}

impl Field {
    /// The field's name in messages, lower case.
    pub fn name(self) -> &'static str {
        match self {
            Field::Title => "title",
            Field::List => "list",
            Field::Type => "type",
            Field::Due => "due",
            Field::Start => "start",
            Field::Repeat => "repeat",
            Field::Alert => "alert",
            Field::Notes => "notes",
        }
    }
}

/// What [`Form::rebase`] reports besides the card, each list in Tab order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rebased {
    /// Typed fields the saved version changed as well: saving replaces its value.
    pub overlap: Vec<Field>,
    /// Typed fields the saved version's card cannot take: their typing is lost.
    pub dropped: Vec<Field>,
}

/// Why typing into Alert does nothing on a task whose alerts Twydo can't write back.
pub const ALERT_READ_ONLY: &str = "this task has alerts Twydo can't edit";

/// What [`Form::save`] returns when nothing would be written.
pub const NO_CHANGES: &str = "no changes";

/// Where a new task goes: built by the new row, which knows the view it was opened in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTarget {
    pub collection: String,
    /// UID of the checklist or project the task becomes an item of.
    pub parent: Option<String>,
    /// That parent's kind: it decides the card's fields (see [`Form::fields`]).
    pub parent_kind: Option<TaskKind>,
    /// Start with Due = today (a new task in Today).
    pub due_today: bool,
}

/// The text of every field, as typed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FormValues {
    pub title: String,
    /// Collection id.
    pub list: String,
    pub kind: TaskKind,
    pub due: String,
    pub start: String,
    /// Typed repeat rule (`repeatinput` syntax); empty for none.
    pub repeat: String,
    /// Typed alert offset (`alertinput` syntax); empty for none.
    pub alert: String,
    pub notes: String,
}

/// The card's state without the UI's: values, what they started as and what the task stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    /// Anchored sibling placement, retained when the input row becomes a card.
    pub insertion: Option<(String, Placement)>,
    /// The task being edited; `None` for a new task.
    pub uid: Option<String>,
    pub title: String,
    /// Collection id.
    pub list: String,
    pub kind: TaskKind,
    pub due: String,
    pub start: String,
    /// Typed repeat rule (`repeatinput` syntax); empty for none.
    pub repeat: String,
    /// Typed alert offset (`alertinput` syntax); empty for none.
    pub alert: String,
    pub notes: String,
    /// The kind of the parent this task is an item of, which decides the fields: a checklist
    /// item has only Title and Notes, and no item has List.
    pub item_of: Option<TaskKind>,
    /// A new task's parent UID.
    parent: Option<String>,
    /// The task's repeat as stored, to tell an unchanged rule typed differently from a change.
    stored_repeat: Option<RepeatSpec>,
    /// The stored rule cannot be written in the field's syntax: the field starts empty and
    /// the rule stays as it is unless something is typed.
    kept_rule: bool,
    /// The type the task's file declares (an untyped parent shows as a project but is a
    /// task): choosing it writes nothing.
    declared: TaskKind,
    /// The task's alert as stored; `Unsupported` makes the Alert field read-only.
    stored_alert: CustomAlert,
    initial: FormValues,
}

impl Form {
    /// The card for an existing task; `item_of` is its parent's kind, if it is an item.
    pub fn open(task: &Task, item_of: Option<TaskKind>, tz: Tz) -> Form {
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
        let values = FormValues {
            title: task.summary.clone(),
            list: task.collection.clone(),
            kind: task.kind,
            due: date(&task.due),
            start: date(&task.start),
            repeat: text.unwrap_or_default(),
            alert: match task.custom_alert {
                CustomAlert::One(o) => alertinput::format(o),
                CustomAlert::AtDue => alertinput::AT_DUE.to_string(),
                CustomAlert::None | CustomAlert::Unsupported => String::new(),
            },
            notes: task.description.clone().unwrap_or_default(),
        };
        let mut f = Form::with(Some(task.uid.clone()), None, values, stored_repeat);
        f.item_of = item_of;
        f.kept_rule = kept_rule;
        f.declared = crate::views::declared_kind(task);
        f.stored_alert = task.custom_alert;
        f
    }

    /// The card for a task not written yet, starting with `title`. Due starts as `today` when
    /// the target asks for it; a checklist item has no Due, so it is never written.
    pub fn new_task(target: NewTarget, title: String, _tz: Tz) -> Form {
        let values = FormValues {
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
            alert: String::new(),
            notes: String::new(),
        };
        let mut f = Form::with(None, target.parent, values, None);
        f.item_of = target.parent_kind;
        f
    }

    fn with(
        uid: Option<String>,
        parent: Option<String>,
        v: FormValues,
        stored_repeat: Option<RepeatSpec>,
    ) -> Form {
        Form {
            uid,
            title: v.title.clone(),
            list: v.list.clone(),
            kind: v.kind,
            due: v.due.clone(),
            start: v.start.clone(),
            repeat: v.repeat.clone(),
            alert: v.alert.clone(),
            notes: v.notes.clone(),
            insertion: None,
            item_of: None,
            parent,
            stored_repeat,
            kept_rule: false,
            declared: v.kind,
            stored_alert: CustomAlert::None,
            initial: v,
        }
    }

    /// The fields in display and Tab order. A checklist item has no dates or repeat of its
    /// own (2Do 01 §1); an item lives in its parent's list, so it has no List. A new task
    /// has no Alert: alerts are set on an existing task.
    pub fn fields(&self) -> Vec<Field> {
        let mut fields = self.layout();
        // A checklist's own alert was never captured on the phone.
        if self.uid.is_none() || self.declared == TaskKind::Checklist {
            fields.retain(|f| *f != Field::Alert);
        }
        fields
    }

    fn layout(&self) -> Vec<Field> {
        match self.item_of {
            Some(TaskKind::Checklist) => vec![Field::Title, Field::Notes],
            Some(_) => vec![
                Field::Title,
                Field::Type,
                Field::Due,
                Field::Start,
                Field::Repeat,
                Field::Alert,
                Field::Notes,
            ],
            None => vec![
                Field::Title,
                Field::List,
                Field::Type,
                Field::Due,
                Field::Start,
                Field::Repeat,
                Field::Alert,
                Field::Notes,
            ],
        }
    }

    /// The stored rule stays because the field cannot show it (see [`Form::open`]).
    pub fn keeps_rule(&self) -> bool {
        self.kept_rule && self.repeat == self.initial.repeat
    }

    /// Whether `Esc` would lose anything: any field differs from when the card opened, or, on a
    /// new task's card, the title (typed in the new row) is non-empty.
    pub fn is_dirty(&self) -> bool {
        if self.uid.is_none() && !self.title.trim().is_empty() {
            return true;
        }
        self.values() != self.initial
    }

    /// The field values as they stand now.
    pub fn values(&self) -> FormValues {
        FormValues {
            title: self.title.clone(),
            list: self.list.clone(),
            kind: self.kind,
            due: self.due.clone(),
            start: self.start.clone(),
            repeat: self.repeat.clone(),
            alert: self.alert.clone(),
            notes: self.notes.clone(),
        }
    }

    /// This card's typing carried onto `onto`, the card for the task's saved version (after a
    /// save refused because the task changed meanwhile). Each field typed here (one that
    /// differs from when this card opened) keeps its typed value; the others show the saved
    /// one. `onto` keeps the saved version as its starting values, so a save writes only the
    /// typed fields and `Esc` still asks before discarding them. What could not be carried, and
    /// what the saved version changed too, is reported in [`Rebased`].
    pub fn rebase(&self, mut onto: Form) -> (Form, Rebased) {
        let (now, mine) = (self.values(), &self.initial);
        let theirs = onto.initial.clone();
        let fields = onto.fields();
        let mut out = Rebased::default();
        macro_rules! carry {
            ($field:expr, $m:ident) => {
                if now.$m != mine.$m {
                    // The saved version has no such field (the task became an item, say), or
                    // an Alert Twydo can't edit, or a repeat rule the field cannot show, which
                    // an empty field keeps rather than clears.
                    let lost = !fields.contains(&$field)
                        || ($field == Field::Alert
                            && onto.stored_alert == CustomAlert::Unsupported)
                        || ($field == Field::Repeat && onto.kept_rule && now.repeat.is_empty());
                    if lost {
                        out.dropped.push($field);
                    } else {
                        if theirs.$m != mine.$m && theirs.$m != now.$m {
                            out.overlap.push($field);
                        }
                        onto.$m = now.$m.clone();
                    }
                }
            };
        }
        carry!(Field::Title, title);
        carry!(Field::List, list);
        carry!(Field::Type, kind);
        carry!(Field::Due, due);
        carry!(Field::Start, start);
        carry!(Field::Repeat, repeat);
        carry!(Field::Alert, alert);
        carry!(Field::Notes, notes);
        (onto, out)
    }

    /// The parsed Due field's date; an unreadable one counts as empty (its preview says why).
    fn due_date(&self, today: NaiveDate) -> Option<NaiveDate> {
        dateinput::parse(&self.due, today)
            .ok()
            .flatten()
            .map(|d| d.date)
    }

    /// The line beside a field: `→ Fri 6 Nov 09:00` for Due and Start; for Repeat the rule in
    /// 2Do's words, the examples while the field is empty and `focused`, or the parse error.
    pub fn preview(
        &self,
        f: Field,
        today: NaiveDate,
        work_days: &[Weekday],
        focused: bool,
    ) -> Option<String> {
        let text = match f {
            Field::Due => &self.due,
            Field::Start => &self.start,
            Field::Repeat => return self.repeat_preview(today, work_days, focused),
            Field::Alert => return self.alert_preview(focused),
            _ => return None,
        };
        Some(match dateinput::parse(text, today) {
            Ok(None) => "→ none".into(),
            Ok(Some(d)) => format!("→ {}", dateinput::describe(&d, today)),
            Err(e) => e,
        })
    }

    fn repeat_preview(
        &self,
        today: NaiveDate,
        work_days: &[Weekday],
        focused: bool,
    ) -> Option<String> {
        let due = self.due_date(today);
        if self.keeps_rule() {
            let spec = self.stored_repeat.as_ref()?;
            return Some(format!("→ {} (kept)", repeatinput::describe(spec, due)));
        }
        // Saving an emptied field on a repeating task removes the rule: say so.
        let removes = self.stored_repeat.is_some();
        if self.repeat.trim().is_empty() && !removes {
            return focused.then(|| EXAMPLES.to_string());
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

    fn alert_preview(&self, focused: bool) -> Option<String> {
        if self.stored_alert == CustomAlert::Unsupported {
            return Some(format!("→ {ALERT_READ_ONLY}"));
        }
        let removes = matches!(self.stored_alert, CustomAlert::One(_) | CustomAlert::AtDue);
        if self.alert.trim().is_empty() && !removes {
            return focused.then(|| alertinput::EXAMPLES.to_string());
        }
        if alertinput::is_at_due(&self.alert) && self.stored_alert == CustomAlert::AtDue {
            return Some("→ at the due time (2Do's automatic alert)".into());
        }
        Some(match alertinput::parse(&self.alert) {
            Ok(None) if removes => "→ none (removes the alert)".into(),
            Ok(None) => "→ none".into(),
            Ok(Some(o)) => format!("→ {} before due", describe_offset(o)),
            Err(e) => e,
        })
    }

    /// The alert to write, if the field changed it: `Some(None)` removes it.
    fn alert_change(&self) -> Result<Option<Option<AlertOffset>>, String> {
        if self.alert == self.initial.alert || self.stored_alert == CustomAlert::Unsupported {
            return Ok(None);
        }
        if alertinput::is_at_due(&self.alert) && self.stored_alert == CustomAlert::AtDue {
            return Ok(None);
        }
        let parsed = alertinput::parse(&self.alert).map_err(|e| format!("alert: {e}"))?;
        if parsed.is_none() && self.stored_alert == CustomAlert::AtDue {
            return Ok(Some(None));
        }
        let stored = match self.stored_alert {
            CustomAlert::One(o) => Some(o),
            _ => None,
        };
        Ok((parsed != stored).then_some(parsed))
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
        if self.kind != i.kind && self.kind != self.declared {
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
        if let Some(a) = self.alert_change()? {
            out.push(Change::Alert(a));
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
            return self
                .create(today, work_days)
                .map(|task| match &self.insertion {
                    Some((anchor, placement)) => Op::CreateAt {
                        task,
                        anchor: anchor.clone(),
                        placement: *placement,
                    },
                    None => Op::Create(task),
                });
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

    /// Replace the typed text of every field; what the card opened with stays.
    pub fn set_values(&mut self, v: FormValues) {
        self.title = v.title;
        self.list = v.list;
        self.kind = v.kind;
        self.due = v.due;
        self.start = v.start;
        self.repeat = v.repeat;
        self.alert = v.alert;
        self.notes = v.notes;
    }

    /// What the card opened with.
    pub fn initial(&self) -> &FormValues {
        &self.initial
    }

    /// This form with `initial` as its starting values: what "the form was opened with" means
    /// when the daemon rebuilds an older form for [`Form::rebase`].
    pub fn with_initial(mut self, initial: FormValues) -> Form {
        self.initial = initial;
        self
    }

    /// Whether Alert can be typed into: not when the task has alerts Twydo can't write back.
    pub fn alert_editable(&self) -> bool {
        self.stored_alert != CustomAlert::Unsupported
    }

    /// The text that typing into field `f` changes, if any.
    pub fn text_mut(&mut self, f: Field) -> Option<&mut String> {
        match f {
            Field::Title => Some(&mut self.title),
            Field::Due => Some(&mut self.due),
            Field::Start => Some(&mut self.start),
            Field::Repeat => Some(&mut self.repeat),
            Field::Alert if !self.alert_editable() => None,
            Field::Alert => Some(&mut self.alert),
            Field::List | Field::Type | Field::Notes => None,
        }
    }

    /// Task → Project → Checklist, and round again.
    pub fn cycle_kind(&mut self, delta: isize) {
        const ORDER: [TaskKind; 3] = [TaskKind::Task, TaskKind::Project, TaskKind::Checklist];
        let at = ORDER.iter().position(|k| *k == self.kind).unwrap_or(0) as isize;
        self.kind = ORDER[(at + delta).rem_euclid(3) as usize];
    }

    /// The lines beside the shown fields among Due, Start, Repeat and Alert, as the card draws
    /// them: dates only when their text is non-empty, Repeat and Alert always (their previews
    /// hint), all as if focused.
    pub fn previews(&self, today: NaiveDate, work_days: &[Weekday]) -> Vec<(Field, String)> {
        let shown = self.fields();
        [Field::Due, Field::Start, Field::Repeat, Field::Alert]
            .into_iter()
            .filter(|f| shown.contains(f))
            .filter(|f| match f {
                Field::Due => !self.due.is_empty(),
                Field::Start => !self.start.is_empty(),
                _ => true,
            })
            .filter_map(|f| Some((f, self.preview(f, today, work_days, true)?)))
            .collect()
    }

    /// The notes as edited in `$EDITOR`.
    pub fn set_notes(&mut self, notes: String) {
        self.notes = notes;
    }

    /// The list picker's choice (a collection id).
    pub fn set_list(&mut self, list: String) {
        self.list = list;
    }
}

/// `15 min`, `1 h 30 min`, `4 h`, `1 d`: the offset as the Details pane words it.
fn describe_offset(o: AlertOffset) -> String {
    let m = o.minutes();
    let (d, h, min) = (m / 1440, m % 1440 / 60, m % 60);
    let mut parts = Vec::new();
    if d > 0 {
        parts.push(format!("{d} d"));
    }
    if h > 0 {
        parts.push(format!("{h} h"));
    }
    if min > 0 {
        parts.push(format!("{min} min"));
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use std::path::PathBuf;

    const NY: Tz = chrono_tz::America::New_York;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 4).unwrap()
    }

    fn task(summary: &str) -> Task {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/sample-vdir");
        crate::vdir::load(&root)
            .unwrap()
            .tasks
            .into_iter()
            .find(|t| t.summary == summary)
            .unwrap()
    }

    #[test]
    fn values_round_trip() {
        let mut f = Form::open(&task("Buy stamps"), None, NY);
        let before = f.initial().clone();
        let mut v = f.values();
        v.title = "Buy more stamps".into();
        f.set_values(v.clone());
        assert_eq!(f.values(), v);
        assert_eq!(f.initial(), &before);
        assert!(f.is_dirty());
        match f.save(today(), &[]).unwrap() {
            Op::Save { changes, to, .. } => {
                assert_eq!(changes, vec![Change::Summary("Buy more stamps".into())]);
                assert_eq!(to, None);
            }
            other => panic!("expected a save, got {other:?}"),
        }
    }

    #[test]
    fn with_initial_then_rebase_reports_overlap() {
        let cur = Form::open(&task("Buy stamps"), None, NY);
        let mut old = cur.initial().clone();
        old.title = "Buy stamp".into();
        let mut mine = cur.clone().with_initial(old);
        let mut v = mine.values();
        v.title = "Buy postage".into();
        mine.set_values(v);
        let (onto, rebased) = mine.rebase(cur);
        assert_eq!(rebased.overlap, vec![Field::Title]);
        assert!(rebased.dropped.is_empty());
        assert_eq!(onto.title, "Buy postage");
    }

    #[test]
    fn previews_follow_the_card_rule() {
        let mut t = task("Buy stamps");
        t.due = None;
        let mut f = Form::open(&t, None, NY);
        let p = f.previews(today(), &[]);
        assert!(p.iter().all(|(field, _)| *field != Field::Due));
        let repeat = p.iter().find(|(field, _)| *field == Field::Repeat);
        assert_eq!(repeat.map(|(_, s)| s.as_str()), Some(repeatinput::EXAMPLES));
        let mut v = f.values();
        v.due = "tomorrow".into();
        f.set_values(v);
        let p = f.previews(today(), &[]);
        let due = p.iter().find(|(field, _)| *field == Field::Due).unwrap();
        assert!(due.1.starts_with("→ "), "{}", due.1);
    }

    #[test]
    fn field_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(Field::Due).unwrap(),
            serde_json::json!("due")
        );
    }

    #[test]
    fn examples_show_only_when_focused() {
        let mut t = task("Buy stamps");
        t.due = None;
        let f = Form::open(&t, None, NY);
        assert_eq!(f.preview(Field::Repeat, today(), &[], false), None);
        assert_eq!(
            f.preview(Field::Repeat, today(), &[], true).as_deref(),
            Some(repeatinput::EXAMPLES)
        );
    }

    #[test]
    fn unsupported_alert_is_not_editable() {
        let mut t = task("Buy stamps");
        assert!(Form::open(&t, None, NY).alert_editable());
        t.custom_alert = CustomAlert::Unsupported;
        let f = Form::open(&t, None, NY);
        assert!(!f.alert_editable());
    }
}
