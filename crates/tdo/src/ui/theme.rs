//! Semantic styles built from terminal defaults and the terminal's 16-color palette.
//! Never copy RGB values from a desktop theme: palette changes should apply live.

use ratatui::style::{Color, Modifier, Style};

pub(super) fn accent() -> Style {
    Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD)
}

pub(super) fn muted() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}

pub(super) fn selection() -> Style {
    Style::new()
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD)
}

pub(super) fn overdue() -> Style {
    Style::new().fg(Color::Red)
}

pub(super) fn warning() -> Style {
    Style::new().fg(Color::Yellow)
}

pub(super) fn badge() -> Style {
    Style::new()
        .fg(Color::Black)
        .bg(Color::Blue)
        .add_modifier(Modifier::BOLD)
}
