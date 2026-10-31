//! Left pane: views with `due│total` counts, then list groups and lists.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use twydo_core::views::{View, counts};
use unicode_width::UnicodeWidthStr;

use super::{bar_color, dim, pad, pane_block, truncate, view_name};
use crate::app::{App, Pane};

/// One sidebar line, with the index into `App::sidebar_items` when it is selectable.
type Entry = (Line<'static>, Option<usize>);

fn divider(label: &str, width: usize) -> Line<'static> {
    let head = format!("─ {label} ");
    let fill = "─".repeat(width.saturating_sub(head.width()));
    Line::styled(format!("{head}{fill}"), dim())
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
        style = style.add_modifier(Modifier::REVERSED);
    }
    Line::from(spans).style(style)
}

fn entries(app: &App, width: usize) -> Vec<Entry> {
    let items = app.sidebar_items();
    let mut out: Vec<Entry> = Vec::new();
    let add = |out: &mut Vec<Entry>, i: usize| {
        out.push((item_line(app, i, &items[i], width), Some(i)));
    };
    for i in 0..4.min(items.len()) {
        add(&mut out, i);
    }

    // Lists: each configured group in order, then everything else under "Lists".
    let list_index = |id: &str| items.iter().position(|v| *v == View::List(id.to_string()));
    let mut placed: Vec<usize> = Vec::new();
    for g in &app.snapshot.config.groups {
        let idx: Vec<usize> = g
            .lists
            .iter()
            .filter_map(|id| list_index(id))
            .filter(|i| !placed.contains(i))
            .collect();
        if idx.is_empty() {
            continue;
        }
        out.push((divider(&g.name, width), None));
        for i in idx {
            placed.push(i);
            add(&mut out, i);
        }
    }
    let rest: Vec<usize> = (5..items.len()).filter(|i| !placed.contains(i)).collect();
    if !rest.is_empty() {
        out.push((divider("Lists", width), None));
        for i in rest {
            add(&mut out, i);
        }
    }
    if items.len() > 4 {
        out.push((divider("Smart lists", width), None));
        add(&mut out, 4);
    }
    out
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
