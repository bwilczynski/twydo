//! Right pane: the selected task's list, dates, repeat, alerts, warnings, its tasks (on a
//! parent) and notes.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Padding, Paragraph, Wrap};
use twydo_core::format::{describe_alarm, relative_date};
use twydo_core::model::Status;
use twydo_core::model::repeat::{describe, parse_rrule};
use twydo_core::views::{View, compare_items};

use super::tasks::glyph;
use super::{dim, list_name, pane_block, theme};
use crate::app::{App, Pane};

fn field(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<8}"), dim()),
        Span::raw(value),
    ])
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = pane_block("Details".into(), app, Pane::Details).padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(idx) = app.selected() else {
        f.render_widget(
            Paragraph::new(Line::styled("No task selected", dim())),
            inner,
        );
        return;
    };
    let t = &app.snapshot.tasks[idx];
    let tz = app.snapshot.tz();
    let yellow = theme::warning();

    let mut lines = vec![
        Line::styled(t.summary.clone(), Style::new().add_modifier(Modifier::BOLD)),
        Line::raw(""),
        field("List", list_name(&app.snapshot, &t.collection)),
    ];
    if let Some(d) = &t.due {
        lines.push(field("Due", relative_date(d, app.now)));
    }
    if let Some(raw) = &t.rrule {
        let text = match parse_rrule(raw) {
            Ok(r) => describe(
                &r,
                t.due.as_ref().map(|d| d.local_date(tz)),
                t.from_completion,
            ),
            Err(_) => "unsupported rule".to_string(),
        };
        lines.push(field("Repeat", text));
    }
    for (i, a) in t.alarms.iter().enumerate() {
        let label = if i == 0 { "Alert" } else { "" };
        lines.push(field(label, describe_alarm(a, t, tz)));
    }
    if let Some(s) = &t.start {
        lines.push(field("Start", relative_date(s, app.now)));
    }
    for w in &t.warnings {
        lines.push(Line::styled(format!("! {w}"), yellow));
    }
    if let Some(b) = &t.blocked {
        lines.push(Line::styled(format!("! {b}"), yellow));
    }
    if let Some(c) = app.conflict_for(&t.uid) {
        lines.push(Line::styled(
            format!("! conflict: {} — :conflicts", c.kind.words()),
            yellow,
        ));
    }
    lines.extend(children(app, idx));
    if let Some(notes) = t.description.as_deref().filter(|n| !n.trim().is_empty()) {
        lines.push(Line::raw(""));
        lines.push(Line::styled("── Notes ──", dim()));
        lines.extend(notes.lines().map(|l| Line::raw(l.to_string())));
    }
    let page = Paragraph::new(lines).wrap(Wrap { trim: false });
    let rows = page.line_count(inner.width);
    app.details_max.set(
        rows.saturating_sub(inner.height as usize)
            .min(u16::MAX as usize) as u16,
    );
    f.render_widget(page.scroll((app.details_offset(), 0)), inner);
}

/// A parent's tasks: the open ones in sibling order, then how many are done, then
/// (in views that nest tasks) how to see them in the list. Nothing for a task without children.
fn children(app: &App, idx: usize) -> Vec<Line<'static>> {
    let snap = &app.snapshot;
    let kids = snap.children(idx);
    if kids.is_empty() {
        return Vec::new();
    }
    let (mut open, done): (Vec<usize>, Vec<usize>) = kids
        .iter()
        .partition(|&&c| !matches!(snap.tasks[c].status, Status::Completed | Status::Cancelled));
    open.sort_by(|&a, &b| compare_items(snap, app.now.timezone(), a, b));
    let mut lines = vec![
        Line::raw(""),
        Line::styled(format!("── Tasks ({} open) ──", open.len()), dim()),
    ];
    for c in open {
        let t = &snap.tasks[c];
        let mut spans = vec![Span::raw(format!("{} {}", glyph(app, c), t.summary))];
        if let Some(d) = &t.due {
            spans.push(Span::styled(
                format!("  {}", relative_date(d, app.now)),
                dim(),
            ));
        }
        lines.push(Line::from(spans));
    }
    if !done.is_empty() {
        lines.push(Line::styled(format!("+ {} completed", done.len()), dim()));
    }
    // Only All and list views nest tasks; Enter does nothing in the date views (#8).
    if matches!(app.view, View::All | View::List(_)) {
        let verb = if app.expanded.contains(&snap.tasks[idx].uid) {
            "collapse"
        } else {
            "expand"
        };
        lines.push(Line::styled(format!("Enter: {verb} in list"), dim()));
    }
    lines
}
