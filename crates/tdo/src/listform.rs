//! Pure list-card input state. Saves, local archive and recovery are separate actions.
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use twydo_core::lists::{ListRequest, ListState};

pub const PALETTE: [&str; 11] = [
    "#3289D2", "#379417", "#6946CD", "#75715B", "#A32124", "#FF8B00", "#55B456", "#FFC0BD",
    "#B08DAA", "#EEDC1B", "#4D4B4B",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListField {
    Name,
    Color,
    Save,
    Archive,
    Delete,
    Reconcile,
    KeepServer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListFormAction {
    None,
    Cancel,
    Save,
    Archive,
    Delete,
    Reconcile,
    KeepServer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListForm {
    pub id: String,
    pub original: Option<ListState>,
    pub name: String,
    pub color: String,
    pub focus: ListField,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub busy: bool,
    pub operation: Option<String>,
    pub server_values: Option<(Option<String>, Option<String>)>,
}

impl ListForm {
    pub fn new(palette_index: usize) -> Self {
        Self {
            id: twydo_core::edit::new_uid(),
            original: None,
            name: String::new(),
            color: PALETTE[palette_index % PALETTE.len()].into(),
            focus: ListField::Name,
            error: None,
            notice: None,
            busy: false,
            operation: None,
            server_values: None,
        }
    }
    pub fn edit(state: ListState) -> Self {
        Self {
            id: state.id.clone(),
            name: state.name.clone(),
            color: state
                .color
                .map(|(r, g, b)| format!("#{r:02X}{g:02X}{b:02X}"))
                .unwrap_or_default(),
            original: Some(state),
            ..Self::new(0)
        }
    }
    pub fn fields(&self) -> Vec<ListField> {
        if self.operation.is_some() {
            return vec![ListField::Reconcile, ListField::KeepServer];
        }
        let mut fields = vec![ListField::Name, ListField::Color, ListField::Save];
        if self.original.is_some() {
            fields.extend([ListField::Archive, ListField::Delete]);
        }
        fields
    }
    pub fn parsed_color(&self) -> Result<Option<(u8, u8, u8)>, String> {
        if self.color.is_empty() && self.original.as_ref().is_some_and(|s| s.color.is_none()) {
            return Ok(None);
        }
        if self.color.len() != 7 {
            return Err("Color: use #RRGGBB or choose a preset with Left/Right".into());
        }
        twydo_core::vdir::parse_color(&self.color)
            .map(Some)
            .ok_or_else(|| "Color: use #RRGGBB".into())
    }
    pub fn request(&self) -> Result<ListRequest, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("Name must not be empty".into());
        }
        let color = self.parsed_color()?;
        if let Some(original) = &self.original {
            if name == original.name && color == original.color {
                return Err("no list changes".into());
            }
            Ok(ListRequest::Edit {
                id: self.id.clone(),
                name: name.into(),
                color,
                expected: original.clone(),
            })
        } else {
            Ok(ListRequest::Create {
                id: self.id.clone(),
                name: name.into(),
                color: color.ok_or_else(|| "Choose a color".to_string())?,
            })
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> ListFormAction {
        if self.busy {
            return ListFormAction::None;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match (key.code, ctrl) {
            (KeyCode::Esc, _) => return ListFormAction::Cancel,
            (KeyCode::Char('s'), true) => return ListFormAction::Save,
            (KeyCode::Tab | KeyCode::Down, _) | (KeyCode::BackTab | KeyCode::Up, _) => {
                let fields = self.fields();
                let current = fields.iter().position(|f| *f == self.focus).unwrap_or(0) as isize;
                let delta = if matches!(key.code, KeyCode::BackTab | KeyCode::Up) {
                    -1
                } else {
                    1
                };
                self.focus = fields[(current + delta).rem_euclid(fields.len() as isize) as usize];
            }
            (KeyCode::Enter, _) => {
                return match self.focus {
                    ListField::Archive => ListFormAction::Archive,
                    ListField::Delete => ListFormAction::Delete,
                    ListField::Reconcile => ListFormAction::Reconcile,
                    ListField::KeepServer => ListFormAction::KeepServer,
                    _ => ListFormAction::Save,
                };
            }
            (KeyCode::Left | KeyCode::Right, false) if self.focus == ListField::Color => {
                let delta = if key.code == KeyCode::Left { -1 } else { 1 };
                let current = PALETTE
                    .iter()
                    .position(|v| v.eq_ignore_ascii_case(&self.color))
                    .map(|i| i as isize)
                    .unwrap_or(if delta > 0 {
                        PALETTE.len() as isize - 1
                    } else {
                        0
                    });
                self.color =
                    PALETTE[(current + delta).rem_euclid(PALETTE.len() as isize) as usize].into();
            }
            _ if self.operation.is_none() => {
                let text = match self.focus {
                    ListField::Name => Some(&mut self.name),
                    ListField::Color => Some(&mut self.color),
                    _ => None,
                };
                if let Some(text) = text {
                    crate::form::edit_line(text, key);
                }
            }
            _ => {}
        }
        ListFormAction::None
    }
    pub fn paste(&mut self, text: &str) {
        if self.busy || self.operation.is_some() {
            return;
        }
        let field = match self.focus {
            ListField::Name => Some(&mut self.name),
            ListField::Color => Some(&mut self.color),
            _ => None,
        };
        if let Some(field) = field {
            field.push_str(&crate::form::one_line(text));
        }
    }
}
