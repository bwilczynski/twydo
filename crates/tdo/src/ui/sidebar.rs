//! Left pane: views with `due│total` counts, list groups and lists, then smart lists.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use twydo_core::views::{View, counts};
use unicode_width::UnicodeWidthStr;

use super::{bar_color, pad, pane_block, theme, truncate, view_name};
use crate::app::{App, Pane, SidebarEntry};

/// One sidebar line, with the index into `App::sidebar_items` when it is selectable.
type Entry = (Line<'static>, Option<usize>);

fn divider(label: &str, width: usize) -> Line<'static> {
    let head = format!("─ {label} ");
    let fill = "─".repeat(width.saturating_sub(head.width()));
    Line::styled(format!("{head}{fill}"), theme::accent())
}

fn item_line(app: &App, i: usize, view: &View, width: usize) -> Line<'static> {
    let snap = &app.snapshot;
    let c = counts(snap, view, app.now);
    let count = if c.total == 0 {
        String::new()
    } else if c.due_or_overdue > 0 {
        format!("{}│{}", c.due_or_overdue, c.total)
    } else {
        c.total.to_string()
    };
    let current = *view == app.view;
    let prefix = match view {
        View::List(id) => vec![
            Span::styled("▌", Style::new().fg(bar_color(snap, id))),
            Span::raw(" "),
        ],
        _ if current => vec![Span::raw("◷ ")],
        _ => vec![Span::raw("  ")],
    };
    let name_w = width.saturating_sub(2 + count.width() + 1);
    let name = pad(&truncate(&view_name(snap, view), name_w), name_w);
    let mut spans = prefix;
    spans.push(Span::raw(name));
    spans.push(Span::raw(" "));
    spans.push(Span::raw(count));
    let mut style = Style::new();
    if current {
        style = style.add_modifier(Modifier::BOLD);
    }
    if app.focus == Pane::Sidebar && i == app.sidebar_cursor {
        style = style.patch(theme::selection());
    }
    Line::from(spans).style(style)
}

/// The lines of [`App::sidebar_entries`], in its order: headings become dividers and each
/// view carries its index into `App::sidebar_items`.
fn entries(app: &App, width: usize) -> Vec<Entry> {
    let mut i = 0;
    app.sidebar_entries()
        .into_iter()
        .map(|e| match e {
            SidebarEntry::Heading(label) => (divider(&label, width), None),
            SidebarEntry::Item(view) => {
                let line = item_line(app, i, &view, width);
                i += 1;
                (line, Some(i - 1))
            }
        })
        .collect()
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = pane_block("Views".into(), app, Pane::Sidebar);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let all = entries(app, inner.width as usize);
    let h = inner.height as usize;
    let sel = all
        .iter()
        .position(|(_, i)| *i == Some(app.sidebar_cursor))
        .unwrap_or(0);
    let offset = (sel + 1).saturating_sub(h);
    let lines: Vec<Line> = all
        .into_iter()
        .skip(offset)
        .take(h)
        .map(|(l, _)| l)
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}
