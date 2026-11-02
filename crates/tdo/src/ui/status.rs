//! Bottom line: mode, the unreadable-files indicator, key hints (or the message), and the
//! position in the view.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use twydo_core::sync::status::CollState;

use super::{dim, list_name, theme, view_name};
use crate::app::{App, Mode};
use crate::form::Field;

const HINTS: &str = "j/k move  x done  t today  ] +1d  o new  / search  : cmd  ? help";

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let (label, prompt) = match app.mode {
        Mode::Normal => ("NORMAL", None),
        Mode::Search => ("SEARCH", Some(format!("/{}▏", app.input))),
        Mode::Command => ("COMMAND", Some(format!(":{}▏", app.input))),
        Mode::Insert => {
            let notes = app.form.as_ref().is_some_and(|f| f.focus == Field::Notes);
            let hint = if notes {
                "Enter edit in $EDITOR  Tab next field  Ctrl-s save  Esc cancel"
            } else {
                "Tab next field  Enter save  Esc cancel"
            };
            ("INSERT", Some(hint.to_string()))
        }
    };
    let rows = app.visible_rows().len();
    let pos = format!(
        "{}·{}/{}",
        view_name(&app.snapshot, &app.view),
        if rows == 0 { 0 } else { app.cursor + 1 },
        rows
    );

    let badge = vec![
        Span::styled(format!(" {label} "), theme::badge()),
        Span::raw(" "),
    ];
    let mut rest = Vec::new();
    // Persistent while any file fails to load; `:errors` lists them.
    if let Some(u) = unreadable(app) {
        rest.push(Span::styled(
            u,
            theme::warning().add_modifier(Modifier::BOLD),
        ));
        rest.push(Span::raw("  "));
    }
    let sync = sync_segment(app);
    if !sync.is_empty() {
        rest.extend(sync.into_iter().map(|(t, s)| Span::styled(t, s)));
        rest.push(Span::raw("  "));
    }
    rest.push(match (prompt, &app.message) {
        (Some(p), _) => Span::raw(p),
        (None, Some(m)) => Span::styled(m.clone(), theme::warning()),
        (None, None) => Span::styled(HINTS, dim()),
    });
    // The focus label is optional: it only goes in when everything else still fits.
    let width = |spans: &[Span]| spans.iter().map(|s| s.content.width()).sum::<usize>();
    let focus = Span::styled(format!("{} · ", app.focus.name()), theme::accent());
    let mut spans = badge;
    if width(&spans) + focus.content.width() + width(&rest) <= area.width as usize {
        spans.push(focus);
    }
    spans.extend(rest);
    let left_used = width(&spans);
    f.render_widget(Paragraph::new(Line::from(spans)), area);

    // The position goes on top at the right edge when there is room for it.
    let w = pos.width() as u16;
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
    let yellow = theme::warning();
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
            theme::overdue(),
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
