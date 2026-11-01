//! Bottom line: mode, the unreadable-files indicator, key hints (or the message), and the
//! position in the view.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use twydo_core::sync::status::CollState;

use super::{dim, list_name, view_name};
use crate::app::{App, Mode};

const HINTS: &str = "j/k move  x done  t today  ] +1d  o new  / search  : cmd  ? help";

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let (label, prompt) = match app.mode {
        Mode::Normal => ("NORMAL", None),
        Mode::Search => ("SEARCH", Some(format!("/{}▏", app.input))),
        Mode::Command => ("COMMAND", Some(format!(":{}▏", app.input))),
    };
    let rows = app.visible_rows().len();
    let pos = format!(
        "{}·{}/{}",
        view_name(&app.snapshot, &app.view),
        if rows == 0 { 0 } else { app.cursor + 1 },
        rows
    );

    let mut spans = vec![
        Span::styled(
            format!(" {label} "),
            Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD),
        ),
        Span::raw(" "),
    ];
    // Persistent while any file fails to load; `:errors` lists them.
    let unreadable = unreadable(app);
    if let Some(u) = &unreadable {
        spans.push(Span::styled(
            u.clone(),
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw("  "));
    }
    let sync = sync_segment(app);
    let sync_w: usize = sync.iter().map(|(t, _)| t.width()).sum();
    if !sync.is_empty() {
        spans.extend(sync.into_iter().map(|(t, s)| Span::styled(t, s)));
        spans.push(Span::raw("  "));
    }
    match (&prompt, &app.message) {
        (Some(p), _) => spans.push(Span::raw(p.clone())),
        (None, Some(m)) => spans.push(Span::styled(m.clone(), Style::new().fg(Color::Yellow))),
        (None, None) => spans.push(Span::styled(HINTS, dim())),
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);

    // The position goes on top at the right edge when there is room for it.
    let w = pos.width() as u16;
    let left_used: usize = 10
        + unreadable.as_ref().map_or(0, |u| u.width() + 2)
        + if sync_w > 0 { sync_w + 2 } else { 0 }
        + match (&prompt, &app.message) {
            (Some(p), _) => p.width(),
            (None, Some(m)) => m.width(),
            (None, None) => HINTS.width(),
        };
    if area.width > w && left_used + (w as usize) < area.width as usize {
        let r = Rect::new(area.x + area.width - w, area.y, w, area.height.min(1));
        f.render_widget(Paragraph::new(pos).alignment(Alignment::Right), r);
    }
}

/// `! N file(s) unreadable`, or nothing when every file loaded.
fn unreadable(app: &App) -> Option<String> {
    match app.snapshot.errors.len() {
        0 => None,
        1 => Some("! 1 file unreadable".into()),
        n => Some(format!("! {n} files unreadable")),
    }
}

/// How long ago, coarsely: `just now`, `3m ago`, `2h ago`, `4d ago`.
fn ago(secs: i64) -> String {
    match secs {
        ..60 => "just now".into(),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86400 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86400),
    }
}

/// The sync part of the line, styled; empty without `[sync]`.
fn sync_segment(app: &App) -> Vec<(String, Style)> {
    let Some(s) = &app.sync else {
        return Vec::new();
    };
    let yellow = Style::new().fg(Color::Yellow);
    let mut out = Vec::new();
    if let Some(err) = &s.status.last_error {
        let retry = s.status.next_attempt.map(|t| {
            format!(
                "; retry {}",
                t.with_timezone(&app.snapshot.tz()).format("%H:%M")
            )
        });
        out.push((
            format!("sync failed: {err}{}", retry.unwrap_or_default()),
            Style::new().fg(Color::Red),
        ));
    } else {
        out.push((
            match s.status.last_success {
                Some(t) => format!(
                    "synced {}",
                    ago((app.now.with_timezone(&chrono::Utc) - t).num_seconds())
                ),
                None => "not synced yet".into(),
            },
            Style::new(),
        ));
    }
    if s.pending > 0 {
        out.push((format!(" · {} pending", s.pending), Style::new()));
    }
    match s.conflicts.len() {
        0 => {}
        1 => out.push((" · 1 conflict".into(), yellow)),
        n => out.push((format!(" · {n} conflicts"), yellow)),
    }
    for (coll, state) in &s.status.collections {
        let where_ = match state {
            CollState::EmptiedRemote => "on server",
            CollState::EmptiedLocal => "here",
            _ => continue,
        };
        let name = list_name(&app.snapshot, coll);
        out.push((
            format!(" · \"{name}\" emptied {where_} (:sync! {name})"),
            yellow,
        ));
    }
    out
}
