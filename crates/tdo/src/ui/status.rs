//! Bottom line: mode, key hints (or the message), and the position in the view.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::{dim, view_name};
use crate::app::{App, Mode};

const HINTS: &str = "j/k move  x done  t today  ] +1d  o new  / search  : cmd  ? help";

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let (label, prompt) = match app.mode {
        Mode::Normal => ("NORMAL", None),
        Mode::Search => ("SEARCH", Some(format!("/{}▏", app.input))),
        Mode::Command => ("COMMAND", Some(format!(":{}▏", app.input))),
    };
    let rows = app.visible_rows().len();
    let pos = format!(
        "{}·{}/{}",
        view_name(&app.snapshot, &app.view),
        if rows == 0 { 0 } else { app.cursor + 1 },
        rows
    );

    let mut spans = vec![
        Span::styled(
            format!(" {label} "),
            Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD),
        ),
        Span::raw(" "),
    ];
    match (&prompt, &app.message) {
        (Some(p), _) => spans.push(Span::raw(p.clone())),
        (None, Some(m)) => spans.push(Span::styled(m.clone(), Style::new().fg(Color::Yellow))),
        (None, None) => spans.push(Span::styled(HINTS, dim())),
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);

    // The position goes on top at the right edge when there is room for it.
    let w = pos.width() as u16;
    let left_used: usize = 10
        + match (&prompt, &app.message) {
            (Some(p), _) => p.width(),
            (None, Some(m)) => m.width(),
            (None, None) => HINTS.width(),
        };
    if area.width > w && left_used + (w as usize) < area.width as usize {
        let r = Rect::new(area.x + area.width - w, area.y, w, area.height.min(1));
        f.render_widget(Paragraph::new(pos).alignment(Alignment::Right), r);
    }
}
