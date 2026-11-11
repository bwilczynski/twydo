//! Manual list-retirement confirmation and preserved-list browsing.
use super::*;
use twydo_daemon::protocol::lists::{ForgetPreview, RetiredBackupView};

pub struct RecoveryPanel {
    pub backups: Vec<RetiredBackupView>,
    pub cursor: usize,
    pub preview: Option<Snapshot>,
    pub scroll: u16,
}

pub enum Confirmation {
    DeleteList(twydo_core::lists::backup::DeletePreview),
    Archive {
        state: twydo_core::lists::ListState,
        unfinished: usize,
        unreadable: usize,
    },
    Forget(ForgetPreview),
    Reparent {
        op: Box<Op>,
    },
    Recover {
        backup: String,
        to: String,
        question: String,
    },
}

impl Confirmation {
    pub fn question(&self) -> String {
        match self {
            Self::DeleteList(p) => format!(
                "Delete {:?} ({}) from CalDAV and ALL DEVICES? Backup {} preserves {} remote resource(s) and {} local file(s). This action is currently refused until server-enforced concurrency protection is demonstrated. y attempt · n/Esc cancel",
                p.name, p.collection, p.backup, p.resources, p.local_files
            ),
            Self::Archive {
                state,
                unfinished,
                unreadable,
            } => format!(
                "Archive {:?} on this desktop? {unfinished} unfinished task(s){}. Task data stays intact. y confirm · n/Esc cancel",
                state.name,
                if *unreadable > 0 {
                    format!(", {unreadable} unreadable resource(s)")
                } else {
                    String::new()
                }
            ),
            Self::Reparent { .. } => {
                "Move into checklist and remove this task’s dates? y confirm · n/Esc cancel".into()
            }
            Self::Forget(p) => format!(
                "Did you delete {:?} ({}) on another device? Preserve {} task(s), including {} unsynced operation(s), then remove its local copy? y confirm · n/Esc cancel",
                p.name, p.collection, p.tasks, p.pending
            ),
            Self::Recover { question, .. } => question.clone(),
        }
    }
}

impl App {
    pub(super) fn forget_command(&mut self, arg: &str) {
        if arg.is_empty() {
            let mut collections: Vec<_> = self.snapshot.collections.iter().collect();
            collections.sort_by_key(|c| {
                (
                    c.display_name.as_deref().unwrap_or(&c.id).to_lowercase(),
                    c.id.clone(),
                )
            });
            let lists: Vec<_> = collections.into_iter().map(|c| c.id.clone()).collect();
            if lists.is_empty() {
                self.message = Some("no lists to forget".into());
                return;
            }
            self.recovery = None;
            self.message = None;
            self.picker = Some(ListPicker {
                action: ListPickerAction::Forget,
                targets: lists.into_iter().map(PickerTarget::List).collect(),
                query: String::new(),
                filtering: false,
                cursor: 0,
            });
            return;
        }
        match self.lookup.forget_preview(arg) {
            Ok(p) => {
                self.recovery = None;
                self.confirmation = Some(Confirmation::Forget(p));
            }
            Err(e) => self.message = Some(format!("cannot preserve list: {e}")),
        }
    }

    /// The directory of the list called `name`: its id, or else its unique display name.
    fn collection_id(&self, name: &str) -> Result<String, String> {
        let plain = !name.is_empty()
            && std::path::Path::new(name).components().count() == 1
            && std::path::Path::new(name)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)));
        let lists = &self.snapshot.collections;
        if plain && lists.iter().any(|c| c.id == name) {
            return Ok(name.to_string());
        }
        let named: Vec<_> = lists
            .iter()
            .filter(|c| c.display_name.as_deref() == Some(name))
            .collect();
        match named.as_slice() {
            [c] => Ok(c.id.clone()),
            [] => Err(format!("no list {name:?}")),
            _ => Err(format!(
                "list name {name:?} is ambiguous; use its collection id"
            )),
        }
    }

    pub(super) fn recover_command(&mut self, arg: &str) {
        if arg.is_empty() {
            match self.lookup.retired() {
                Ok(backups) => {
                    self.recovery = Some(RecoveryPanel {
                        backups,
                        cursor: 0,
                        preview: None,
                        scroll: 0,
                    });
                    self.message = None;
                }
                Err(e) => self.message = Some(format!("cannot read preserved lists: {e}")),
            }
            return;
        }
        let Some((id, destination)) = arg.split_once(char::is_whitespace) else {
            self.message = Some("usage: :recover <backup-id> <destination-list>".into());
            return;
        };
        match self.collection_id(destination.trim()) {
            Ok(to) => self.confirm_recovery(id, &to),
            Err(e) => self.message = Some(e.to_string()),
        }
    }

    pub(super) fn confirm_recovery(&mut self, id: &str, to: &str) {
        let result = self.lookup.retired().and_then(|backups| {
            let b = backups
                .into_iter()
                .find(|b| b.id == id)
                .ok_or_else(|| format!("no preserved list {id:?}"))?;
            let preview = self.lookup.retired_preview(id)?;
            Ok((b, preview))
        });
        match result {
            Ok((b, preview)) => {
                let destination = self
                    .snapshot
                    .collections
                    .iter()
                    .find(|c| c.id == to)
                    .and_then(|c| c.display_name.as_deref())
                    .unwrap_or(to);
                let question = format!(
                    "Recover {} task(s) from {:?} into {:?}? Restore the local snapshot; historical conflict alternatives stay in the backup. y confirm · n/Esc cancel",
                    b.tasks, b.name, destination
                );
                self.recovery = Some(RecoveryPanel {
                    backups: vec![b],
                    cursor: 0,
                    preview: Some(preview),
                    scroll: 0,
                });
                self.confirmation = Some(Confirmation::Recover {
                    backup: id.into(),
                    to: to.into(),
                    question,
                });
            }
            Err(e) => self.message = Some(format!("cannot recover: {e}")),
        }
    }

    pub(super) fn confirmation_key(&mut self, k: KeyEvent) -> Outcome {
        match k.code {
            KeyCode::Char('j') | KeyCode::Down if self.recovery.is_some() => {
                let p = self.recovery.as_mut().unwrap();
                p.scroll = p.scroll.saturating_add(1);
                Outcome::Continue
            }
            KeyCode::Char('k') | KeyCode::Up if self.recovery.is_some() => {
                let p = self.recovery.as_mut().unwrap();
                p.scroll = p.scroll.saturating_sub(1);
                Outcome::Continue
            }
            KeyCode::Char('y') if k.modifiers.is_empty() => {
                let confirmation = self.confirmation.take().expect("confirmation");
                self.recovery = None;
                match confirmation {
                    Confirmation::DeleteList(preview) => {
                        self.start_list_action(ListAction::Delete(preview))
                    }
                    Confirmation::Archive { state, .. } => {
                        self.start_list_action(ListAction::Archive {
                            expected: state,
                            archived: true,
                        })
                    }
                    Confirmation::Reparent { op } => Outcome::Write(Request::Apply(*op)),
                    Confirmation::Forget(p) => Outcome::ForgetList(p),
                    Confirmation::Recover { backup, to, .. } => {
                        Outcome::Write(Request::Recover { backup, to })
                    }
                }
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                if matches!(self.confirmation, Some(Confirmation::Reparent { .. })) {
                    self.picker = None;
                }
                self.confirmation = None;
                self.message = Some("cancelled".into());
                Outcome::Continue
            }
            _ => Outcome::Continue,
        }
    }

    pub(super) fn recovery_key(&mut self, k: KeyEvent) -> Outcome {
        if k.code == KeyCode::Char(':') {
            self.mode = Mode::Command;
            self.input.clear();
            return Outcome::Continue;
        }
        let panel = self.recovery.as_mut().expect("recovery panel");
        match k.code {
            KeyCode::Esc if panel.preview.is_some() => {
                panel.preview = None;
                panel.scroll = 0;
            }
            KeyCode::Esc => self.recovery = None,
            KeyCode::Char('j') | KeyCode::Down if panel.preview.is_some() => {
                panel.scroll = panel.scroll.saturating_add(1)
            }
            KeyCode::Char('k') | KeyCode::Up if panel.preview.is_some() => {
                panel.scroll = panel.scroll.saturating_sub(1)
            }
            KeyCode::Char('j') | KeyCode::Down => {
                panel.cursor = (panel.cursor + 1).min(panel.backups.len().saturating_sub(1))
            }
            KeyCode::Char('k') | KeyCode::Up => panel.cursor = panel.cursor.saturating_sub(1),
            KeyCode::Enter => {
                if let Some(b) = panel.backups.get(panel.cursor) {
                    match self.lookup.retired_preview(&b.id) {
                        Ok(s) => {
                            panel.preview = Some(s);
                            panel.scroll = 0;
                        }
                        Err(e) => self.message = Some(format!("cannot preview: {e}")),
                    }
                }
            }
            KeyCode::Char('r') => {
                if let Some(b) = panel.backups.get(panel.cursor) {
                    let id = b.id.clone();
                    self.open_list_picker(None, "");
                    if let Some(picker) = self.picker.as_mut() {
                        picker.action = ListPickerAction::Recover(id);
                    }
                }
            }
            _ => {}
        }
        Outcome::Continue
    }
}
