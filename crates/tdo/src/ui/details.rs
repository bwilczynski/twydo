//! Right pane: the selected task's list, dates, repeat, alerts, warnings and notes.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Padding, Paragraph, Wrap};
use twydo_core::format::{describe_alarm, relative_date};
use twydo_core::model::repeat::{describe, parse_rrule};

use super::{dim, list_name, pane_block};
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
    let yellow = Style::new().fg(Color::Yellow);

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
    if let Some(notes) = t.description.as_deref().filter(|n| !n.trim().is_empty()) {
        lines.push(Line::raw(""));
        lines.push(Line::styled("── Notes ──", dim()));
        lines.extend(notes.lines().map(|l| Line::raw(l.to_string())));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
