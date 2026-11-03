//! The edit card: drawn in place of its row in the Tasks pane (`tasks.rs` reserves the rows
//! and scrolls so it fits), or as a centred overlay with the same lines when the pane is too
//! narrow or too short for it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};
use twydo_core::model::TaskKind;
use unicode_width::UnicodeWidthStr;

use super::{bar_color, dim, flatten, list_name, overlay_rect, pad, theme, truncate};
use crate::app::App;
use crate::form::{EditForm, Field};

/// The narrowest Tasks pane (inside its border) that holds the card in place.
pub(super) const MIN_WIDTH: u16 = 40;
/// The overlay's width when the card falls back to it.
const OVERLAY_WIDTH: u16 = 72;
const LABEL: usize = 8;
const HINTS: &str = " Ctrl-s save · Tab next · Esc cancel ";

fn kind_name(k: TaskKind) -> &'static str {
    match k {
        TaskKind::Task => "Task",
        TaskKind::Project => "Project",
        TaskKind::Checklist => "Checklist",
    }
}

/// Text width inside a card `width` cells wide: borders and one cell of padding each side.
fn text_width(width: u16) -> usize {
    width.saturating_sub(4) as usize
}

/// A field's value in at most `w` cells: focused, with a `▏` cursor and its end in sight;
/// empty and unfocused, the dimmed `placeholder`.
fn value(text: &str, focused: bool, placeholder: &str, w: usize) -> Span<'static> {
    if focused {
        let shown: Vec<char> = format!("{}▏", flatten(text)).chars().collect();
        let tail: String = shown[shown.len().saturating_sub(w)..].iter().collect();
        Span::styled(truncate(&tail, w), theme::accent())
    } else if text.is_empty() {
        Span::styled(truncate(placeholder, w), dim())
    } else {
        Span::raw(truncate(text, w))
    }
}

fn label(name: &str, focused: bool) -> Span<'static> {
    Span::styled(
        pad(name, LABEL),
        if focused { theme::accent() } else { dim() },
    )
}

/// A labelled field, with its preview beside it when it fits, else wrapped below it.
fn field_row(
    out: &mut Vec<Line<'static>>,
    name: &str,
    v: Span<'static>,
    focused: bool,
    preview: Option<String>,
    w: usize,
) {
    let used = LABEL + v.content.width();
    let mut line = vec![label(name, focused), v];
    match preview {
        Some(p) if used + 2 + p.width() <= w => {
            line.push(Span::styled(format!("  {p}"), dim()));
            out.push(Line::from(line));
        }
        Some(p) => {
            out.push(Line::from(line));
            out.extend(
                wrap(&p, w.saturating_sub(LABEL)).into_iter().map(|l| {
                    Line::from(vec![Span::raw(" ".repeat(LABEL)), Span::styled(l, dim())])
                }),
            );
        }
        None => out.push(Line::from(line)),
    }
}

/// Greedy word wrap to `w` cells (a word longer than a line is cut).
fn wrap(text: &str, w: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if !cur.is_empty() && cur.width() + 1 + word.width() > w {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.into_iter().map(|l| truncate(&l, w)).collect()
}

/// The card's lines for text width `w`, one per shown field (plus previews and the error).
fn lines(app: &App, e: &EditForm, w: usize) -> Vec<Line<'static>> {
    let today = app.now.date_naive();
    let work_days = &app.snapshot.config.work_days;
    let fields = e.fields();
    let on = |f: Field| e.focus == f;
    let mut out = vec![Line::from(value(&e.title, on(Field::Title), "", w))];

    let mut chips: Vec<Span> = Vec::new();
    if fields.contains(&Field::List) {
        let name = truncate(&list_name(&app.snapshot, &e.list), w / 2);
        let style = if on(Field::List) {
            theme::accent()
        } else {
            Style::new()
        };
        chips.push(Span::styled(
            "▌",
            Style::new().fg(bar_color(&app.snapshot, &e.list)),
        ));
        chips.push(Span::styled(format!("{name} ▸"), style));
        chips.push(Span::raw("   "));
    }
    if fields.contains(&Field::Type) {
        let kind = kind_name(e.kind);
        chips.push(if on(Field::Type) {
            Span::styled(format!("‹ {kind} ›"), theme::accent())
        } else {
            Span::raw(kind)
        });
    }
    if !chips.is_empty() {
        out.push(Line::from(chips));
    }

    let vw = w.saturating_sub(LABEL);
    for (f, name, text) in [
        (Field::Due, "Due", &e.due),
        (Field::Start, "Start", &e.start),
        (Field::Repeat, "Repeat", &e.repeat),
    ] {
        if !fields.contains(&f) {
            continue;
        }
        let placeholder = if f == Field::Repeat && !e.keeps_rule() {
            "none"
        } else {
            "—"
        };
        // An empty date says nothing beside it; Repeat's preview also hints and keeps.
        let preview = if f == Field::Repeat || !text.is_empty() {
            e.preview(f, today, work_days)
        } else {
            None
        };
        field_row(
            &mut out,
            name,
            value(text, on(f), placeholder, vw),
            on(f),
            preview,
            w,
        );
    }

    let mut notes = e.notes.lines();
    let first = notes.next().unwrap_or("");
    let more = notes.count();
    let mut l = vec![
        label("Notes", on(Field::Notes)),
        value(first, false, "—", vw),
    ];
    if more > 0 {
        l.push(Span::styled(format!(" (+{more} lines)"), dim()));
    }
    if on(Field::Notes) {
        l.push(Span::styled("  (Enter opens $EDITOR)", dim()));
    }
    out.push(Line::from(l));

    if let Some(err) = &e.error {
        out.extend(
            wrap(err, w)
                .into_iter()
                .map(|l| Line::styled(l, theme::overdue())),
        );
    }
    out
}

/// Rows the card takes when drawn `width` cells wide, borders included.
pub(super) fn height(app: &App, e: &EditForm, width: u16) -> u16 {
    lines(app, e, text_width(width)).len() as u16 + 2
}

fn block() -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::accent())
        .padding(Padding::horizontal(1))
        .title_bottom(Span::styled(HINTS, dim()))
}

/// The card in place, at `rect` (from [`height`]).
pub(super) fn render(f: &mut Frame, rect: Rect, app: &App, e: &EditForm) {
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines(app, e, text_width(rect.width))).block(block()),
        rect,
    );
}

/// The fallback: the same card centred over the screen, titled.
pub(super) fn render_overlay(f: &mut Frame, area: Rect, app: &App, e: &EditForm) {
    let width = OVERLAY_WIDTH.min(area.width.saturating_sub(2)).max(1);
    let lines = lines(app, e, text_width(width));
    let rect = overlay_rect(area, width, lines.len() as u16 + 2);
    let title = if e.uid.is_some() {
        " Edit task "
    } else {
        " New task "
    };
    f.render_widget(Clear, rect);
    f.render_widget(Paragraph::new(lines).block(block().title(title)), rect);
}
