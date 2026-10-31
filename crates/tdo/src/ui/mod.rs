//! Rendering: three panes plus a status line, drawn from [`App`] state. Chrome uses only the
//! 16 ANSI colours; true colour appears solely on the `▌` list bars.

mod details;
mod sidebar;
mod status;
mod tasks;

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use twydo_core::views::{Snapshot, View};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, Pane};

const SIDEBAR_WIDTH: u16 = 20;
const DETAILS_WIDTH: u16 = 36;
/// The details pane appears from this terminal width.
const DETAILS_MIN_TERMINAL: u16 = 110;

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    let [main, bottom] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);
    let sidebar_w = if area.width >= 60 {
        SIDEBAR_WIDTH
    } else {
        area.width / 3
    };
    let details_w = if area.width >= DETAILS_MIN_TERMINAL {
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
    tasks::render(f, mid, app);
    if details_w > 0 {
        details::render(f, right, app);
    }
    status::render(f, bottom, app);
    if app.show_help {
        render_help(f, area);
    }
}

/// A rounded, titled pane border; the focused pane is bold cyan.
pub(crate) fn pane_block(title: String, app: &App, pane: Pane) -> Block<'static> {
    let border = if app.focus == pane {
        Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::new()
    };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(Span::styled(
            format!(" {title} "),
            Style::new().add_modifier(Modifier::BOLD),
        ))
}

pub(crate) fn dim() -> Style {
    Style::new().add_modifier(Modifier::DIM)
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
    ("j/k  gg/G  Ctrl-d/u", "move", false),
    ("h/l  Tab", "focus pane", false),
    ("Enter", "expand/collapse a parent", false),
    ("Enter on a leaf, i", "edit task", true),
    ("x  Space", "toggle done", true),
    ("o  O", "new task / new item", true),
    ("J  K", "move task down/up", true),
    ("dd", "delete", true),
    ("u  Ctrl-r", "undo / redo", true),
    ("t  T", "due today / tomorrow", true),
    ("]  [  }  {", "due +1/-1 day, +1/-1 week", true),
    ("r  a  m", "edit repeat / alerts / move list", true),
    ("/  n  N", "search in view", false),
    ("1-5", "Today All Scheduled Done Next 3 Days", false),
    (":list <name>  :q", "open list / quit", false),
    (":sync  :conflicts", "sync / conflicts", true),
    ("?  Esc", "toggle help / clear", false),
];

fn render_help(f: &mut Frame, area: Rect) {
    let width = 70.min(area.width.saturating_sub(2)).max(1);
    let height = (HELP.len() as u16 + 2).min(area.height);
    let [row] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let [rect] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(row);
    let lines: Vec<Line> = HELP
        .iter()
        .map(|(keys, what, gated)| {
            let mut spans = vec![
                Span::styled(
                    pad(keys, 21),
                    Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ),
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
