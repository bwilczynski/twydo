//! Manual list-retirement confirmation and preserved-list browsing.
use super::*;
use twydo_core::retired::{self, Backup, Preview};

pub struct RecoveryPanel {
    pub backups: Vec<Backup>,
    pub cursor: usize,
    pub preview: Option<Snapshot>,
    pub scroll: u16,
}

pub enum Confirmation {
    Forget(Preview),
    Recover {
        backup: String,
        to: String,
        question: String,
    },
}

impl Confirmation {
    pub fn question(&self) -> String {
        match self {
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
            self.message = Some("usage: :forget-list <list name or collection id>".into());
            return;
        }
        match retired::preview(&self.snapshot.config, arg) {
            Ok(p) => {
                self.recovery = None;
                self.confirmation = Some(Confirmation::Forget(p));
            }
            Err(e) => self.message = Some(format!("cannot preserve list: {e}")),
        }
    }

    pub(super) fn recover_command(&mut self, arg: &str) {
        if arg.is_empty() {
            match retired::list(&self.snapshot.config) {
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
        match retired::collection_id(&self.snapshot.config, destination.trim()) {
            Ok(to) => self.confirm_recovery(id, &to),
            Err(e) => self.message = Some(e.to_string()),
        }
    }

    pub(super) fn confirm_recovery(&mut self, id: &str, to: &str) {
        let cfg = &self.snapshot.config;
        let result = retired::get(cfg, id).and_then(|b| {
            let preview = retired::snapshot(cfg, id)?;
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
                    Confirmation::Forget(p) => Outcome::ForgetList(p),
                    Confirmation::Recover { backup, to, .. } => {
                        Outcome::Write(Request::Recover { backup, to })
                    }
                }
            }
            KeyCode::Char('n') | KeyCode::Esc => {
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
                    match retired::snapshot(&self.snapshot.config, &b.id) {
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
                    self.recovery_target = Some(b.id.clone());
                    self.open_list_picker(None, "");
                }
            }
            _ => {}
        }
        Outcome::Continue
    }
}
