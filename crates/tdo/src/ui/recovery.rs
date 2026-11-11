//! Preserved-list browser and explicit confirmation overlay.
use super::{dim, overlay_rect};
use crate::app::{App, Confirmation};
use ratatui::style::{Modifier, Style};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, Paragraph, Wrap},
};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    if app.recovery.is_none() && app.confirmation.is_none() {
        return;
    }
    if let Some(confirmation) = &app.confirmation
        && matches!(
            confirmation,
            Confirmation::Forget(_) | Confirmation::Archive { .. } | Confirmation::DeleteList(_)
        )
    {
        let title = if matches!(confirmation, Confirmation::DeleteList(_)) {
            " Delete list "
        } else if matches!(confirmation, Confirmation::Archive { .. }) {
            " Archive list "
        } else {
            " Forget list "
        };
        let question = Paragraph::new(confirmation.question())
            .wrap(Wrap { trim: false })
            .block(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .title(title),
            );
        let width = overlay_rect(area, 110, area.height).width;
        // line_count includes the top/bottom borders, but wraps at the supplied width.
        let height = question
            .line_count(width.saturating_sub(2))
            .min(u16::MAX as usize) as u16;
        let rect = overlay_rect(area, 110, height);
        f.render_widget(Clear, rect);
        f.render_widget(question, rect);
        return;
    }
    let rect = overlay_rect(area, 110, area.height.saturating_sub(4).min(30));
    f.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(" Preserved lists ");
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let question = app.confirmation.as_ref().map(|c| c.question());
    let footer = question.unwrap_or_else(|| {
        if app.recovery.as_ref().is_some_and(|p| p.preview.is_some()) {
            "j/k scroll · r choose destination · Esc back · :recover <backup-id> <destination-list>"
                .into()
        } else {
            "j/k move · Enter preview · r choose destination · Esc close".into()
        }
    });
    let footer_height = if app.confirmation.is_some() { 5 } else { 3 };
    let [content, bottom] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(footer_height)]).areas(inner);
    let mut lines = Vec::new();
    let mut scroll = 0;
    if let Some(panel) = &app.recovery {
        if let Some(preview) = &panel.preview {
            if let Some(b) = panel.backups.get(panel.cursor) {
                lines.push(Line::raw(format!(
                    "{} · {} · {} task(s)",
                    b.id, b.name, b.tasks
                )));
                if b.conflicts {
                    lines.push(Line::raw("Recovery uses the local snapshot. Both conflict versions remain in the backup."));
                }
                lines.push(Line::raw(""));
            }
            for t in &preview.tasks {
                lines.push(Line::raw(format!(
                    "{}{}",
                    if t.parent.is_some() { "  " } else { "" },
                    t.summary
                )));
                if let Some(notes) = &t.description {
                    lines.extend(
                        notes
                            .lines()
                            .map(|s| Line::styled(format!("    {s}"), dim())),
                    );
                }
            }
            for (path, error) in &preview.errors {
                lines.push(Line::raw(format!("Unreadable {}: {error}", path.display())));
            }
            scroll = panel.scroll;
        } else if panel.backups.is_empty() {
            lines.push(Line::raw("No preserved lists"));
        } else {
            for (i, b) in panel.backups.iter().enumerate() {
                let style = if i == panel.cursor {
                    Style::new().add_modifier(Modifier::REVERSED)
                } else {
                    Style::new()
                };
                lines.push(Line::from(Span::styled(
                    format!("{} · {}", b.id, b.name),
                    style,
                )));
                lines.push(Line::styled(
                    format!(
                        "  {} · {} task(s) · {} unsynced operation(s){}",
                        b.created.format("%Y-%m-%d %H:%M"),
                        b.tasks,
                        b.pending,
                        b.destination
                            .as_ref()
                            .map(|d| format!(" · recovery to {d} recorded"))
                            .unwrap_or_default()
                    ),
                    dim(),
                ));
            }
            let row = panel.cursor.saturating_mul(2);
            scroll = row
                .saturating_sub(content.height.saturating_sub(2) as usize)
                .min(u16::MAX as usize) as u16;
        }
    }
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        content,
    );
    f.render_widget(Paragraph::new(footer).wrap(Wrap { trim: false }), bottom);
}
