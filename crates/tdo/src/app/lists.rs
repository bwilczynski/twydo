//! List-card actions and results, independent of the terminal and worker threads.
use super::*;
use crate::listform::{ListField, ListForm, ListFormAction};
use std::path::PathBuf;
use twydo_core::lists::{ListRequest, ListResult, ListState};

pub struct CollectionBackups {
    pub backups: Vec<twydo_daemon::protocol::lists::WireDeletePreview>,
    pub cursor: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListAction {
    ExportBackup {
        backup: String,
        destination: PathBuf,
    },
    PreviewDelete {
        id: String,
    },
    Delete(twydo_core::lists::backup::DeletePreview),
    Save(ListRequest),
    Archive {
        expected: ListState,
        archived: bool,
    },
    Reconcile {
        operation: String,
        keep_server: bool,
    },
}

impl ListAction {
    fn belongs_to(&self, form: &ListForm) -> bool {
        match self {
            Self::Save(ListRequest::Create { id, .. } | ListRequest::Edit { id, .. })
            | Self::PreviewDelete { id } => id == &form.id,
            Self::Archive { expected, .. } => expected.id == form.id,
            Self::Delete(preview) => preview.collection == form.id,
            Self::Reconcile { operation, .. } => form.operation.as_ref() == Some(operation),
            Self::ExportBackup { .. } => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListCompletion {
    pub action: ListAction,
    pub result: Result<ListResult, String>,
    pub pending: Option<(String, String)>,
    pub server_values: Option<(Option<String>, Option<String>)>,
}

impl App {
    pub(super) fn open_collection_backups(&mut self) {
        match self.lookup.deleted_backups() {
            Ok(backups) => {
                self.recovery = None;
                self.collection_backups = Some(CollectionBackups { cursor: 0, backups });
            }
            Err(e) => self.message = Some(format!("Cannot read collection backups: {e}")),
        }
    }
    pub(super) fn collection_backup_key(&mut self, key: KeyEvent) -> Outcome {
        let panel = self
            .collection_backups
            .as_mut()
            .expect("collection backups");
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                panel.cursor = (panel.cursor + 1).min(panel.backups.len().saturating_sub(1))
            }
            KeyCode::Char('k') | KeyCode::Up => panel.cursor = panel.cursor.saturating_sub(1),
            KeyCode::Esc => self.collection_backups = None,
            KeyCode::Char(':') => {
                self.mode = Mode::Command;
                self.input.clear();
            }
            KeyCode::Char('q') => return Outcome::Quit,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Outcome::Quit;
            }
            _ => {}
        }
        Outcome::Continue
    }
    fn list_state(&self, id: &str) -> Option<ListState> {
        let list = self.snapshot.collections.iter().find(|c| c.id == id)?;
        Some(ListState {
            id: id.into(),
            name: list.display_name.clone().unwrap_or_else(|| id.into()),
            color: list.color,
            archived: self.snapshot.config.list_prefs(id).archived,
        })
    }

    fn pending_list_card(&mut self) -> bool {
        let record = match self.lookup.pending_list_op() {
            Ok(None) => return false,
            Ok(Some(record)) => record,
            Err(e) => {
                self.message = Some(format!("Cannot inspect pending list save: {e}"));
                return true;
            }
        };
        let Some(id) = record.collection.as_deref() else {
            self.message = Some("Pending list save has no list identity".into());
            return true;
        };
        let mut form = if record.action == "create" {
            ListForm::new(0)
        } else {
            self.list_state(id)
                .map(ListForm::edit)
                .unwrap_or_else(|| ListForm::new(0))
        };
        form.id = id.into();
        if let Some(name) = &record.name {
            form.name = name.clone();
        }
        if let Some(color) = &record.color {
            form.color = color.clone();
        }
        form.operation = Some(record.operation.clone());
        form.focus = ListField::Reconcile;
        form.server_values = (record.observed_name.is_some() || record.observed_color.is_some())
            .then(|| (record.observed_name.clone(), record.observed_color.clone()));
        form.notice = Some("Finish the pending list save before starting another".into());
        self.list_form = Some(form);
        self.mode = Mode::Insert;
        true
    }

    pub(super) fn open_list_card(&mut self, id: Option<&str>) {
        if self.pending_list_card() {
            return;
        }
        let form = match id {
            Some(id) => match self.list_state(id) {
                Some(state) => ListForm::edit(state),
                None => {
                    self.message = Some("List is no longer available".into());
                    return;
                }
            },
            None => ListForm::new(self.snapshot.collections.len()),
        };
        self.list_form = Some(form);
        self.mode = Mode::Insert;
        self.message = None;
    }

    pub(super) fn manage_lists(&mut self) {
        if self.pending_list_card() {
            return;
        }
        let mut lists: Vec<_> = self.snapshot.collections.iter().collect();
        lists.sort_by_key(|c| {
            (
                c.display_name.as_deref().unwrap_or(&c.id).to_lowercase(),
                c.id.clone(),
            )
        });
        if lists.is_empty() {
            self.message = Some("No lists yet; use o in the sidebar to create one".into());
            return;
        }
        self.picker = Some(ListPicker {
            action: ListPickerAction::Manage,
            targets: lists
                .into_iter()
                .map(|c| PickerTarget::List(c.id.clone()))
                .collect(),
            cursor: 0,
            query: String::new(),
            filtering: false,
        });
    }

    pub(super) fn sidebar_list_key(&mut self, c: char, pending: Option<char>) -> Option<Outcome> {
        if c == 'o' {
            self.open_list_card(None);
            return Some(Outcome::Continue);
        }
        if c == 'i' || (c == 'd' && pending == Some('d')) {
            let id = match self.sidebar_items().get(self.sidebar_cursor) {
                Some(View::List(id)) => Some(id.clone()),
                _ => None,
            };
            if let Some(id) = id {
                self.open_list_card(Some(&id));
                if c == 'd'
                    && let Some(form) = self.list_form.as_mut()
                    && form.operation.is_none()
                {
                    form.focus = ListField::Delete;
                    return Some(self.list_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
                }
            } else if c == 'i' {
                self.message = Some("Choose an ordinary list to edit, or use :lists".into());
            }
            return Some(Outcome::Continue);
        }
        None
    }

    pub(super) fn start_list_action(&mut self, action: ListAction) -> Outcome {
        if let Some(form) = self.list_form.as_mut() {
            form.busy = true;
            form.error = None;
            form.notice = None;
        }
        Outcome::ManageList(action)
    }

    pub(super) fn list_key(&mut self, key: KeyEvent) -> Outcome {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Outcome::Quit;
        }
        let action = self.list_form.as_mut().expect("list card").key(key);
        match action {
            ListFormAction::None => {}
            ListFormAction::Cancel => {
                self.list_form = None;
                self.mode = Mode::Normal;
            }
            ListFormAction::Save => {
                let form = self.list_form.as_mut().expect("list card");
                if form.operation.is_some() {
                    form.error = Some("Resolve the pending save first".into());
                    return Outcome::Continue;
                }
                match form.request() {
                    Ok(request) => return self.start_list_action(ListAction::Save(request)),
                    Err(error) => form.error = Some(error),
                }
            }
            ListFormAction::Archive => {
                let Some(state) = self.list_form.as_ref().and_then(|f| f.original.clone()) else {
                    return Outcome::Continue;
                };
                let archived = !state.archived;
                let unfinished = self
                    .snapshot
                    .tasks
                    .iter()
                    .filter(|t| t.collection == state.id && t.status == Status::Open)
                    .count();
                let unreadable = self
                    .snapshot
                    .errors
                    .iter()
                    .filter(|(p, _)| p.starts_with(self.snapshot.config.vdir.join(&state.id)))
                    .count();
                if archived && (unfinished > 0 || unreadable > 0) {
                    self.confirmation = Some(Confirmation::Archive {
                        state,
                        unfinished,
                        unreadable,
                    });
                } else {
                    return self.start_list_action(ListAction::Archive {
                        expected: state,
                        archived,
                    });
                }
            }
            ListFormAction::Delete => {
                let id = self.list_form.as_ref().expect("list card").id.clone();
                return self.start_list_action(ListAction::PreviewDelete { id });
            }
            ListFormAction::Reconcile | ListFormAction::KeepServer => {
                if let Some(id) = self.list_form.as_ref().and_then(|f| f.operation.clone()) {
                    return self.start_list_action(ListAction::Reconcile {
                        operation: id,
                        keep_server: action == ListFormAction::KeepServer,
                    });
                }
            }
        }
        Outcome::Continue
    }

    pub fn list_finished(&mut self, completion: ListCompletion) {
        // A deletion ends the list wherever it came from: a fresh delete or a reconcile of a
        // pending one, with or without its card open.
        if let Ok(ListResult::Deleted {
            collection,
            name,
            backup,
            ..
        }) = &completion.result
        {
            self.snapshot.collections.retain(|c| &c.id != collection);
            self.snapshot.config.lists.remove(collection);
            if self.view == View::List(collection.clone()) {
                self.open(View::Today);
            }
            if self.list_form.as_ref().is_some_and(|f| &f.id == collection) {
                self.list_form = None;
                self.mode = Mode::Normal;
            }
            // Only this list's confirmation: another prompt may be open meanwhile.
            if matches!(&self.confirmation, Some(Confirmation::DeleteList(p)) if &p.collection == collection)
            {
                self.confirmation = None;
            }
            self.message = Some(format!("Deleted {name} from CalDAV; backup {backup} kept"));
            self.sync_sidebar();
            return;
        }
        if matches!(completion.action, ListAction::ExportBackup { .. })
            || !self
                .list_form
                .as_ref()
                .is_some_and(|form| completion.action.belongs_to(form))
        {
            self.message = Some(match completion.result {
                Err(error) => error,
                Ok(ListResult::Exported {
                    backup,
                    destination,
                }) => format!(
                    "Exported collection backup {backup} to {}",
                    destination.display()
                ),
                Ok(_) => "List request finished; use :lists to inspect its current state".into(),
            });
            return;
        }
        match completion.result {
            Ok(ListResult::Exported {
                backup,
                destination,
            }) => {
                self.message = Some(format!(
                    "Exported collection backup {backup} to {}",
                    destination.display()
                ));
            }
            Ok(ListResult::DeletePreview(preview)) => {
                if let Some(form) = self.list_form.as_mut() {
                    form.busy = false;
                }
                self.confirmation = Some(Confirmation::DeleteList(preview));
            }
            Ok(ListResult::Deleted { .. }) => {} // handled above
            Err(error) => {
                if self.list_form.is_none() {
                    self.message = Some(error.clone());
                }
                if let Some(form) = self.list_form.as_mut() {
                    form.busy = false;
                    form.error = Some(error);
                    form.operation = completion
                        .pending
                        .filter(|(_, id)| *id == form.id)
                        .map(|(operation, _)| operation);
                    form.server_values = completion.server_values;
                    if form.operation.is_some() {
                        form.focus = ListField::Reconcile;
                    }
                }
            }
            Ok(ListResult::Saved { state, .. }) => {
                self.snapshot
                    .config
                    .lists
                    .entry(state.id.clone())
                    .or_default()
                    .archived = state.archived;
                if let Some(list) = self
                    .snapshot
                    .collections
                    .iter_mut()
                    .find(|c| c.id == state.id)
                {
                    list.display_name = Some(state.name.clone());
                    list.color = state.color;
                } else {
                    self.snapshot
                        .collections
                        .push(twydo_core::vdir::Collection {
                            id: state.id.clone(),
                            path: self.snapshot.config.vdir.join(&state.id),
                            display_name: Some(state.name.clone()),
                            color: state.color,
                        });
                }
                if state.archived && self.view == View::List(state.id.clone()) {
                    self.open(View::Today);
                }
                let message = match &completion.action {
                    ListAction::Archive { archived, .. } => {
                        if *archived {
                            "Archived on this desktop"
                        } else {
                            "Unarchived on this desktop"
                        }
                    }
                    ListAction::Reconcile {
                        keep_server: true, ..
                    } => "Kept server values",
                    ListAction::Reconcile { .. } => "List save reconciled",
                    ListAction::Save(_) => "List saved",
                    ListAction::PreviewDelete { .. } | ListAction::Delete(_) => {
                        "List request finished"
                    }
                    ListAction::ExportBackup { .. } => "Backup exported",
                };
                if matches!(completion.action, ListAction::Archive { .. }) {
                    if let Some(form) = self.list_form.as_mut() {
                        form.busy = false;
                        form.original = Some(state.clone());
                        form.notice = Some(message.into());
                        form.error = None;
                    }
                } else {
                    self.list_form = None;
                    self.mode = Mode::Normal;
                    if matches!(
                        completion.action,
                        ListAction::Save(ListRequest::Create { .. })
                    ) {
                        self.open(View::List(state.id));
                    }
                }
                self.message = Some(message.into());
                self.sync_sidebar();
            }
        }
    }
}
