//! The `m` overlay: the lists a task can move to, each with its colour bar.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use super::{bar_color, dim, list_name, overlay_rect, theme, truncate};
use crate::app::{App, ListPicker};

pub fn render(f: &mut Frame, area: Rect, app: &App, p: &ListPicker) {
    let rect = overlay_rect(area, 40, p.lists.len() as u16 + 4);
    let w = rect.width.saturating_sub(4) as usize;
    // Rows left for lists after the borders, the blank line and the hint; scroll so the
    // cursor is always among them.
    let rows = (rect.height as usize).saturating_sub(4).max(1);
    let skip = (p.cursor + 1).saturating_sub(rows);
    let mut lines: Vec<Line> = p
        .lists
        .iter()
        .enumerate()
        .skip(skip)
        .take(rows)
        .map(|(i, id)| {
            let name = Span::raw(truncate(&list_name(&app.snapshot, id), w));
            let bar = Span::styled("▌ ", Style::new().fg(bar_color(&app.snapshot, id)));
            let line = Line::from(vec![bar, name]);
            if i == p.cursor {
                line.style(theme::selection())
            } else {
                line
            }
        })
        .collect();
    lines.push(Line::raw(""));
    lines.push(Line::styled("j/k move  Enter move here  Esc cancel", dim()));
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" Move to "),
        ),
        rect,
    );
}
