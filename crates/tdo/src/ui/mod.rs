//! Rendering: three panes plus a status line, drawn from [`App`] state. Chrome uses only the
//! 16 ANSI colours; true colour appears solely on the `▌` list bars.

mod card;
mod conflicts;
mod details;
mod picker;
mod sidebar;
mod status;
mod tasks;
mod theme;

use theme::muted as dim;

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use twydo_core::views::{Snapshot, View};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, Pane};

const DETAILS_WIDTH: u16 = 36;

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    let [main, bottom] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);
    let sidebar_w = if area.width >= 60 {
        (area.width / 5).clamp(24, 40)
    } else {
        area.width / 3
    };
    // The App decides visibility (in `resize`) so focus never lands on a hidden pane.
    let details_w = if app.details_visible() {
        DETAILS_WIDTH
    } else {
        0
    };
    let [side, mid, right] = Layout::horizontal([
        Constraint::Length(sidebar_w),
        Constraint::Min(0),
        Constraint::Length(details_w),
    ])
    .areas(main);

    sidebar::render(f, side, app);
    let card_in_place = tasks::render(f, mid, app);
    if details_w > 0 {
        details::render(f, right, app);
    }
    status::render(f, bottom, app);
    if app.show_help {
        render_help(f, area);
    }
    if app.show_errors {
        render_errors(f, area, app);
    }
    if app.show_conflicts {
        conflicts::render(f, area, app);
    }
    if let Some(e) = &app.form
        && !card_in_place
    {
        card::render_overlay(f, area, app, e);
    }
    if let Some(p) = &app.picker {
        picker::render(f, area, app, p);
    }
}

/// A centred, cleared rectangle of at most `width` x `height` cells.
fn overlay_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2)).max(1);
    let height = height.min(area.height);
    let [row] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let [rect] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(row);
    rect
}

/// The `:errors` overlay: each unreadable file with the reason it failed to load.
fn render_errors(f: &mut Frame, area: Rect, app: &App) {
    let errors = &app.snapshot.errors;
    let rect = overlay_rect(area, 100, errors.len().max(1) as u16 + 2);
    let text_w = rect.width.saturating_sub(2) as usize;
    let lines: Vec<Line> = if errors.is_empty() {
        vec![Line::styled("No unreadable files", dim())]
    } else {
        errors
            .iter()
            .map(|(path, msg)| {
                let path = path.display().to_string();
                let msg = format!(": {msg}");
                let path = truncate(&path, text_w);
                let msg = truncate(&msg, text_w.saturating_sub(path.width()));
                Line::from(vec![Span::styled(path, theme::warning()), Span::raw(msg)])
            })
            .collect()
    };
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" Unreadable files "),
        ),
        rect,
    );
}

/// A rounded, titled pane border; the focused pane uses the terminal accent.
pub(crate) fn pane_block(title: String, app: &App, pane: Pane) -> Block<'static> {
    let border = if app.focus == pane {
        theme::accent()
    } else {
        Style::new()
    };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(Span::styled(
            format!(" {title} "),
            border.add_modifier(Modifier::BOLD),
        ))
}

/// The sidebar / status name of a view.
pub(crate) fn view_name(snapshot: &Snapshot, view: &View) -> String {
    match view {
        View::Today => "Today".into(),
        View::All => "All".into(),
        View::Scheduled => "Scheduled".into(),
        View::Done => "Done".into(),
        View::Next3Days => "Next 3 Days".into(),
        View::List(id) => list_name(snapshot, id),
    }
}

pub(crate) fn list_name(snapshot: &Snapshot, id: &str) -> String {
    snapshot
        .collections
        .iter()
        .find(|c| c.id == id)
        .and_then(|c| c.display_name.clone())
        .unwrap_or_else(|| id.to_string())
}

/// The list colour for a `▌` bar, or dark grey when the list has none.
pub(crate) fn bar_color(snapshot: &Snapshot, id: &str) -> Color {
    snapshot
        .collections
        .iter()
        .find(|c| c.id == id)
        .and_then(|c| c.color)
        .map_or(Color::DarkGray, |(r, g, b)| Color::Rgb(r, g, b))
}

/// Replace control characters (tabs, newlines) so a title occupies one cell per char.
fn flatten(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Truncate to at most `width` display cells, ending in `…` when cut.
pub(crate) fn truncate(s: &str, width: usize) -> String {
    let s = flatten(s);
    if s.width() <= width {
        return s;
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Pad with spaces to exactly `width` display cells (never shortens).
pub(crate) fn pad(s: &str, width: usize) -> String {
    let w = s.width();
    format!("{s}{}", " ".repeat(width.saturating_sub(w)))
}

const HELP: &[(&str, &str, bool)] = &[
    ("j/k  gg/G  Ctrl-d/u", "move (in Details: scroll)", false),
    ("h/l  Tab", "focus pane", false),
    ("Enter", "expand/collapse a parent (All, lists)", false),
    (
        "Enter on a leaf, i",
        "edit task in a card (Tab fields, Enter save)",
        false,
    ),
    ("x  Space", "toggle done", false),
    ("o  O", "new task / new item", true),
    ("J  K", "move task down/up", true),
    ("dd", "delete", false),
    ("u  Ctrl-r", "undo / redo", false),
    ("t  T", "due today / tomorrow", false),
    ("]  [  }  {", "due +1/-1 day, +1/-1 week", false),
    ("m", "move to another list", false),
    ("r", "card on the Repeat field", false),
    ("a", "edit alerts", true),
    ("/  n  N", "search in view", false),
    ("1-5", "Today All Scheduled Done Next 3 Days", false),
    (":list <name>  :q", "open list / quit", false),
    (":errors", "list unreadable files", false),
    (
        ":sync  :sync! <list>",
        "sync / apply an emptied list",
        false,
    ),
    (
        ":conflicts",
        "resolve sync conflicts (l local, r remote)",
        false,
    ),
    ("?  Esc", "toggle help / clear", false),
];

fn render_help(f: &mut Frame, area: Rect) {
    let rect = overlay_rect(area, 70, HELP.len() as u16 + 2);
    let lines: Vec<Line> = HELP
        .iter()
        .map(|(keys, what, gated)| {
            let mut spans = vec![
                Span::styled(pad(keys, 21), theme::accent()),
                Span::raw(*what),
            ];
            if *gated {
                spans.push(Span::styled(" (after gate)", dim()));
            }
            Line::from(spans)
        })
        .collect();
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" Help "),
        ),
        rect,
    );
}
