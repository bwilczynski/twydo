//! Semantic styles built from terminal defaults and the terminal's 16-color palette.
//! Never copy RGB values from a desktop theme: palette changes should apply live.

use ratatui::style::{Color, Modifier, Style};

pub(super) fn accent() -> Style {
    Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD)
}

pub(super) fn muted() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}

/// The active row. Callers drop DIM from the row's spans: dim text on bright-black is
/// unreadable in palettes that use bright-black for muted text.
pub(super) fn selection() -> Style {
    Style::new()
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD)
}

/// Sidebar section headings: accent while the sidebar has focus, muted otherwise, so they
/// never compete with the focused pane's border.
pub(super) fn heading(focused: bool) -> Style {
    if focused {
        accent()
    } else {
        Style::new().fg(Color::Blue).add_modifier(Modifier::DIM)
    }
}

pub(super) fn overdue() -> Style {
    Style::new().fg(Color::Red)
}

pub(super) fn warning() -> Style {
    Style::new().fg(Color::Yellow)
}

/// Reversed blue: blue fill with the terminal's own background as text, readable in any palette.
pub(super) fn badge() -> Style {
    Style::new()
        .fg(Color::Blue)
        .add_modifier(Modifier::REVERSED | Modifier::BOLD)
}
