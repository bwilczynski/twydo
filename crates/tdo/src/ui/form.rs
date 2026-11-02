//! The `i` overlay: one row per field, date previews, and the last save's error.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};

use super::{dim, flatten, pad, theme, truncate};
use crate::app::App;
use crate::form::{EditForm, Field};

const LABEL: usize = 8;

fn label(name: &str, focused: bool) -> Span<'static> {
    let style = if focused { theme::accent() } else { dim() };
    Span::styled(pad(name, LABEL), style)
}

/// A field row: the label, then the value with a `▏` cursor when focused.
fn row(name: &str, value: &str, focused: bool, width: usize) -> Line<'static> {
    let shown = if focused {
        format!("{}▏", flatten(value))
    } else {
        flatten(value)
    };
    // Keep the end visible while typing: show the tail when the value is too long.
    let w = width.saturating_sub(LABEL);
    let chars: Vec<char> = shown.chars().collect();
    let shown = if focused && chars.len() > w {
        chars[chars.len() - w..].iter().collect()
    } else {
        truncate(&shown, w)
    };
    let value = if focused {
        Span::styled(shown, theme::accent())
    } else {
        Span::raw(shown)
    };
    Line::from(vec![label(name, focused), value])
}

fn preview(e: &EditForm, field: Field, app: &App) -> Option<Line<'static>> {
    let p = e.preview(field, app.now.date_naive())?;
    Some(Line::from(vec![
        Span::raw(" ".repeat(LABEL)),
        Span::styled(p, dim()),
    ]))
}

pub fn render(f: &mut Frame, area: Rect, app: &App, e: &EditForm) {
    let mut lines: Vec<Line> = Vec::new();
    let rect = overlay_size(area);
    let w = rect.width.saturating_sub(2) as usize;
    let focused = |fl: Field| e.focus == fl;

    lines.push(row("Title", &e.title, focused(Field::Title), w));
    lines.push(row("Due", &e.due, focused(Field::Due), w));
    lines.extend(preview(e, Field::Due, app));
    lines.push(row("Start", &e.start, focused(Field::Start), w));
    lines.extend(preview(e, Field::Start, app));
    match &e.repeat {
        Some(n) => {
            let mut l = row("Repeat", n, focused(Field::Repeat), w);
            l.spans
                .push(Span::styled(format!(" {}", e.repeat_unit), dim()));
            l.spans.insert(1, Span::styled("every ", dim()));
            lines.push(l);
        }
        None => lines.push(Line::from(vec![
            label("Repeat", false),
            Span::styled("none", dim()),
        ])),
    }
    let mut notes = e.notes.lines();
    let first = notes.next().unwrap_or("");
    let more = notes.count();
    let mut l = row("Notes", first, false, w);
    if focused(Field::Notes) {
        l.spans[0] = label("Notes", true);
    }
    if more > 0 {
        l.spans
            .push(Span::styled(format!(" (+{more} lines)"), dim()));
    }
    lines.push(l);
    lines.push(Line::raw(""));
    if let Some(err) = &e.error {
        lines.push(Line::styled(err.clone(), theme::overdue()));
    }

    let height = (lines.len() as u16 + 2).min(area.height);
    let rect = super::overlay_rect(area, rect.width, height);
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" Edit task "),
        ),
        rect,
    );
}

fn overlay_size(area: Rect) -> Rect {
    super::overlay_rect(area, 72, area.height)
}
