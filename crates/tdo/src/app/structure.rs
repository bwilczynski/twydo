//! Thin structure routing; the writer's gate remains the admission authority.
use super::*;
impl App {
    pub(super) fn eligible_structure_parent(&self, i: usize) -> bool {
        let t = &self.snapshot.tasks[i];
        t.status == Status::Open
            && t.parent.is_none()
            && t.meta.format == twydo_core::model::MetaFormat::Json
            && t.meta
                .raw
                .as_deref()
                .and_then(twydo_core::model::meta_edit::MetaJson::parse)
                .is_some_and(|m| matches!(m.get_integer("TaskType"), Some(1 | 2)))
            && matches!(t.kind, TaskKind::Checklist | TaskKind::Project)
            && t.blocked.is_none()
            && self.snapshot.children(i).iter().all(|&c| {
                self.snapshot.tasks[c].kind == TaskKind::Task
                    && self.snapshot.children(c).is_empty()
            })
    }
    pub(super) fn structure_key(&mut self, indent: bool) -> Outcome {
        if self.focus != Pane::Tasks || !matches!(self.view, View::List(_)) {
            self.message = Some(">>/<< require the Tasks pane in a list view".into());
            return Outcome::Continue;
        }
        let Some(i) = self.selected() else {
            return Outcome::Continue;
        };
        let t = &self.snapshot.tasks[i];
        if t.status != Status::Open
            || t.kind != TaskKind::Task
            || !self.snapshot.children(i).is_empty()
        {
            self.message = Some("indent/outdent needs one open plain leaf".into());
            return Outcome::Continue;
        }
        let (uid, collection) = (t.uid.clone(), t.collection.clone());
        let parent = if indent {
            let rows = self.visible_rows();
            let Some(p) = rows[..self.cursor].iter().rev().find(|r| {
                self.snapshot.tasks[r.task].collection == collection
                    && self.eligible_structure_parent(r.task)
            }) else {
                self.message = Some("no eligible parent above this task in this list view".into());
                return Outcome::Continue;
            };
            Some(self.snapshot.tasks[p.task].uid.clone())
        } else {
            if self.snapshot.parent(i).is_none() {
                self.message = Some("this task has no parent to outdent from".into());
                return Outcome::Continue;
            }
            None
        };
        if parent.as_deref() == t.parent.as_deref() {
            self.message = Some("already in this parent; use J/K to reorder".into());
            return Outcome::Continue;
        }
        self.request_reparent(uid, collection, parent)
    }
    pub(super) fn request_reparent(
        &mut self,
        uid: String,
        collection: String,
        parent: Option<String>,
    ) -> Outcome {
        let op = Op::Reparent {
            uid: uid.clone(),
            collection,
            parent: parent.clone(),
        };
        let checklist = parent
            .as_deref()
            .and_then(|p| self.snapshot.find(p))
            .is_some_and(|p| self.snapshot.tasks[p].kind == TaskKind::Checklist);
        if checklist && let Some(i) = self.snapshot.find(&uid) {
            let t = &self.snapshot.tasks[i];
            // Refuse unsupported cleanup immediately; this probe performs no writes.
            if let Some(cal) = self
                .snapshot
                .source(&t.path)
                .and_then(|b| twydo_core::ical::parse(b).ok())
                && let Err(e) = twydo_core::edit::reparented_copy(
                    &cal,
                    "candidate-ui-probe",
                    parent.as_deref(),
                    true,
                    self.now.to_utc(),
                )
            {
                self.message = Some(e.to_string());
                return Outcome::Continue;
            }
            if t.due.is_some() || t.start.is_some() {
                self.confirmation = Some(Confirmation::Reparent { op: Box::new(op) });
                return Outcome::Continue;
            }
        }
        Outcome::Write(Request::Apply(op))
    }
}
