//! List selection for movement, editing, recovery and retirement.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use twydo_core::sync::status::CollState;

use super::{bar_color, dim, overlay_rect, theme, truncate};
use crate::app::{App, ListPicker, ListPickerAction};

pub fn render(f: &mut Frame, area: Rect, app: &App, p: &ListPicker) {
    let (title, hint) = match p.action {
        ListPickerAction::Move(_) => (" Move to ", "j/k move  Enter move here  Esc cancel"),
        ListPickerAction::SetList => (" Choose list ", "j/k move  Enter select  Esc cancel"),
        ListPickerAction::Recover(_) => (" Recover into ", "j/k move  Enter select  Esc cancel"),
        ListPickerAction::Forget => (" Forget list ", "j/k move  Enter select  Esc cancel"),
    };
    let targets = p.visible_targets(&app.snapshot);
    let names: Vec<_> = targets
        .iter()
        .map(|target| target.label(&app.snapshot))
        .collect();
    let suffixes: Vec<_> = targets
        .iter()
        .zip(&names)
        .map(|(target, name)| {
            let id = target.collection();
            let mut suffix = if names.iter().filter(|other| *other == name).count() > 1 {
                format!(" ({id})")
            } else {
                String::new()
            };
            if p.action == ListPickerAction::Forget
                && app.sync.as_ref().is_some_and(|sync| {
                    matches!(sync.status.collections.get(id), Some(CollState::Error { message })
                        if message.trim_end_matches(';').ends_with("Not Found"))
                })
            {
                suffix.push_str(" · Not Found");
            }
            suffix
        })
        .collect();
    let width = names
        .iter()
        .zip(&suffixes)
        .map(|(name, suffix)| {
            name.chars()
                .count()
                .saturating_add(suffix.chars().count())
                .saturating_add(6)
        })
        .max()
        .unwrap_or(40)
        .clamp(40, 110) as u16;
    let rect = overlay_rect(
        area,
        width,
        targets.len().saturating_add(4).min(u16::MAX as usize) as u16,
    );
    let w = rect.width.saturating_sub(4) as usize;
    // Rows left for lists after the borders, the blank line and the hint; scroll so the
    // cursor is always among them.
    let rows = (rect.height as usize).saturating_sub(4).max(1);
    let skip = (p.cursor + 1).saturating_sub(rows);
    let mut lines: Vec<Line> = targets
        .iter()
        .enumerate()
        .skip(skip)
        .take(rows)
        .map(|(i, target)| {
            let id = target.collection();
            let suffix = &suffixes[i];
            let label = format!(
                "{}{}",
                truncate(&names[i], w.saturating_sub(suffix.chars().count())),
                suffix
            );
            let name = Span::raw(label);
            let bar = Span::styled("▌ ", Style::new().fg(bar_color(&app.snapshot, id)));
            let line = Line::from(vec![bar, name]);
            if i == p.cursor {
                line.style(theme::selection())
            } else {
                line
            }
        })
        .collect();
    lines.push(Line::raw(format!("Filter: {}", p.query)));
    lines.push(Line::styled(hint, dim()));
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(title),
        ),
        rect,
    );
}
