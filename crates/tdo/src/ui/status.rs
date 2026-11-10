//! Bottom line: mode, the unreadable-files indicator, key hints (or the message), and the
//! position in the view.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use twydo_daemon::protocol::methods::WireCollState;

use super::{dim, list_name, theme, view_name};
use crate::app::{App, Mode};
use crate::form::Field;

const HINTS: &str = "j/k move  x done  t today  ] +1d  o new  / search  : cmd  ? help";

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let (label, prompt) = match app.mode {
        Mode::Normal => ("NORMAL", None),
        Mode::Search => ("SEARCH", Some(format!("/{}▏", app.input))),
        Mode::Command => ("COMMAND", Some(format!(":{}▏", app.input))),
        Mode::Insert if let Some(row) = &app.new_row => {
            let list = list_name(&app.snapshot, &row.target.collection);
            let hint = match &row.error {
                Some(e) => format!("{e} · Enter save  Esc cancel"),
                None => format!("New task in {list} · Enter save  Tab more  Esc cancel"),
            };
            ("INSERT", Some(hint))
        }
        Mode::Insert if app.list_form.is_some() => (
            "INSERT",
            Some("Tab next action  Ctrl-s save to CalDAV  Esc close".into()),
        ),
        Mode::Insert => {
            let hint = match app.form.as_ref().map(|f| f.focus) {
                Some(Field::Notes) => {
                    "Enter edit in $EDITOR  Tab next field  Ctrl-s save  Esc cancel"
                }
                Some(Field::List) => "Enter pick a list  Tab next field  Ctrl-s save  Esc cancel",
                Some(Field::Type) => {
                    "Space/h/l change type  Tab next field  Enter save  Esc cancel"
                }
                _ => "Tab next field  Enter save  Esc cancel",
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
        // A refused new row's reason.
        (Some(p), _) if app.new_row.as_ref().is_some_and(|r| r.error.is_some()) => {
            Span::styled(p, theme::warning())
        }
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
    let Some(cs) = &app.sync else {
        return Vec::new();
    };
    let Some(s) = &cs.sync else {
        return Vec::new();
    };
    let yellow = theme::warning();
    let mut out = Vec::new();
    // While switched off, always first, so local-only edits are not forgotten.
    if !s.auto {
        out.push(("sync off".into(), yellow));
        out.push((" · ".into(), Style::new()));
    }
    let activity = app.sync_activity.label();
    if let Some(label) = &activity {
        out.push((label.clone(), theme::accent()));
    }
    let missing = s.collections.iter().find(|(_, state)| {
        matches!(state,
        WireCollState::Error { message } if message.trim_end_matches(';').ends_with("Not Found"))
    });
    if activity.is_some() && (missing.is_some() || s.last_error.is_some()) {
        out.push((" · ".into(), Style::new()));
    }
    if let Some((id, _)) = missing {
        out.push((
            format!(
                "list {:?}: Not Found (:forget-list to choose; :errors)",
                list_name(&app.snapshot, id)
            ),
            theme::overdue(),
        ));
    } else if let Some(err) = &s.last_error {
        let retry = s.next_attempt.map(|t| {
            format!(
                "; retry {}",
                t.with_timezone(&app.snapshot.tz()).format("%H:%M")
            )
        });
        let error = if err.starts_with("list discovery failed;") {
            "list discovery failed (:forget-list for other deleted lists; :errors)"
        } else {
            err.as_str()
        };
        out.push((
            format!("sync failed: {error}{}", retry.unwrap_or_default()),
            theme::overdue(),
        ));
    } else if activity.is_none() {
        out.push((
            match s.last_success {
                Some(t) => format!(
                    "synced {}",
                    ago((app.now.with_timezone(&chrono::Utc) - t).num_seconds())
                ),
                None => "not synced yet".into(),
            },
            Style::new(),
        ));
    }
    if cs.unsynced + cs.queued > 0 {
        out.push((
            format!(" · {} pending", cs.unsynced + cs.queued),
            Style::new(),
        ));
    }
    match cs.conflicts.len() {
        0 => {}
        1 => out.push((" · 1 conflict".into(), yellow)),
        n => out.push((format!(" · {n} conflicts"), yellow)),
    }
    for (coll, state) in &s.collections {
        let where_ = match state {
            WireCollState::EmptiedRemote => "on server",
            WireCollState::EmptiedLocal => "here",
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
