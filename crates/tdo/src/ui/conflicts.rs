//! The `:conflicts` overlay: each unresolved conflict with its kind in words.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use twydo_core::conflicts::{Conflict, ConflictState};

use super::{dim, list_name, overlay_rect, truncate};
use crate::app::App;

/// The task's title, or its file name when the task is gone from the vdir (deleted here).
fn title(app: &App, c: &Conflict) -> String {
    app.snapshot
        .tasks
        .iter()
        .find(|t| t.uid == c.uid)
        .map(|t| t.summary.clone())
        .unwrap_or_else(|| c.path.rsplit('/').next().unwrap_or(&c.path).to_string())
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let conflicts = app.conflicts();
    let rect = overlay_rect(area, 100, (conflicts.len() * 2).max(1) as u16 + 4);
    let w = rect.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();
    if conflicts.is_empty() {
        lines.push(Line::styled("No unresolved conflicts", dim()));
    }
    for (i, c) in conflicts.iter().enumerate() {
        let sel = if i == app.conflict_cursor {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
        };
        let head = format!(
            "{} · {}",
            list_name(&app.snapshot, &c.collection),
            title(app, c)
        );
        lines.push(Line::from(Span::styled(truncate(&head, w), sel)));
        let mut spans = vec![Span::styled(
            format!("  {}", truncate(c.kind.words(), w.saturating_sub(2))),
            Style::new().fg(Color::Yellow),
        )];
        if c.state == ConflictState::Chosen {
            spans.push(Span::styled("  waiting for sync", dim()));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "j/k move  l keep local  r keep remote  c keep current  Esc close",
        dim(),
    ));
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" Conflicts "),
        ),
        rect,
    );
}
