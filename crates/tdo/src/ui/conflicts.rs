//! The `:conflicts` overlay: each unresolved conflict with its kind in words.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use twydo_daemon::protocol::admin::{ConflictView, WireConflictKind, WireConflictState};

use super::{dim, list_name, overlay_rect, truncate};
use crate::app::App;

/// The task's title, or its file name when the task is gone from the vdir (deleted here).
fn title(app: &App, c: &ConflictView) -> String {
    app.snapshot
        .tasks
        .iter()
        .find(|t| t.uid == c.uid)
        .map(|t| t.summary.clone())
        .or_else(|| c.title.clone())
        .unwrap_or_else(|| c.path.rsplit('/').next().unwrap_or(&c.path).to_string())
}

/// The kind in words, for people.
pub fn kind_words(kind: WireConflictKind) -> &'static str {
    match kind {
        WireConflictKind::BothChanged => "changed here and on another device",
        WireConflictKind::LocalDeleteRemoteEdit => "deleted here, edited on another device",
        WireConflictKind::RemoteDeleteLocalEdit => "edited here, deleted on another device",
        WireConflictKind::Journal => "an interrupted change here meets another device's version",
        WireConflictKind::Unknown => "in conflict with another device",
    }
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
            format!("  {}", truncate(kind_words(c.kind), w.saturating_sub(2))),
            Style::new().fg(Color::Yellow),
        )];
        if c.state == WireConflictState::Chosen {
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
