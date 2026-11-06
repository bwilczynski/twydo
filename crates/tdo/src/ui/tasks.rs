//! Middle pane: the current view's tasks under date-group headers.

use chrono::NaiveDate;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Padding, Paragraph};
use twydo_core::format::relative_date;
use twydo_core::model::repeat::{parse_rrule, short_hint};
use twydo_core::model::{Status, Task, TaskKind};
use twydo_core::views::{DateGroup, Row, View, build_view, date_group};
use unicode_width::UnicodeWidthStr;

use super::{bar_color, card, dim, pad, pane_block, theme, truncate, view_name};
use crate::app::{App, NewRow, Pane};

const MAX_HINT: usize = 12;
const MAX_DATE: usize = 16;
/// Narrowest title column worth keeping the hint column beside.
const MIN_TITLE: usize = 12;

fn pane_title(app: &App) -> String {
    match &app.view {
        View::Today => format!("Today · {}", app.now.format("%A %-d %b")),
        v => view_name(&app.snapshot, v),
    }
}

fn group_title(g: DateGroup, today: NaiveDate) -> Option<String> {
    Some(match g {
        DateGroup::Overdue => "OVERDUE".to_string(),
        DateGroup::InProgress => "IN PROGRESS".into(),
        DateGroup::LaterToday => "LATER TODAY".into(),
        DateGroup::Today => "TODAY".into(),
        DateGroup::Tomorrow => "TOMORROW".into(),
        DateGroup::DayAfterTomorrow => "DAY AFTER TOMORROW".into(),
        DateGroup::Next7Days => "NEXT 7 DAYS".into(),
        DateGroup::Next2Weeks => "NEXT 2 WEEKS".into(),
        DateGroup::Next30Days => "NEXT 30 DAYS".into(),
        DateGroup::Future => "FUTURE".into(),
        DateGroup::Done => "DONE".into(),
        DateGroup::NoDueDate => "NO DUE DATE".into(),
        DateGroup::Scheduled => "SCHEDULED".into(),
        DateGroup::CompletedOn(d) if d == today => "TODAY".into(),
        DateGroup::CompletedOn(d) if d.succ_opt() == Some(today) => "YESTERDAY".into(),
        DateGroup::CompletedOn(d) => d.format("%a %-d %b").to_string(),
        DateGroup::Ungrouped => return None,
    })
}

pub(super) fn glyph(app: &App, idx: usize) -> String {
    let t = &app.snapshot.tasks[idx];
    let n = app.snapshot.children(idx).len();
    if n > 0 {
        format!("[{n}]")
    } else if t.status == Status::Completed {
        "✓".into()
    } else if t.status == Status::Cancelled {
        "✗".into()
    } else if t.rrule.is_some() {
        "↺".into()
    } else {
        "○".into()
    }
}

/// The repeat hint, or `project` / `checklist`, or nothing.
fn hint(t: &Task) -> String {
    if let Some(r) = t.rrule.as_deref().and_then(|r| parse_rrule(r).ok()) {
        return short_hint(&r);
    }
    match t.kind {
        TaskKind::Project => "project".into(),
        TaskKind::Checklist => "checklist".into(),
        TaskKind::Task => String::new(),
    }
}

fn date_text(app: &App, t: &Task) -> String {
    t.due
        .as_ref()
        .or(t.start.as_ref())
        .map(|d| relative_date(d, app.now))
        .unwrap_or_default()
}

/// Overdue by the task's own dates, wherever its row sits (nested, ungrouped).
fn is_overdue(app: &App, idx: usize) -> bool {
    app.snapshot.tasks[idx].status == Status::Open
        && date_group(&app.snapshot, idx, app.now, false) == DateGroup::Overdue
}

/// A task carrying load warnings, a reason edits stay off, or an unresolved conflict: marked `!` before the title.
fn flagged(app: &App, t: &Task) -> bool {
    !t.warnings.is_empty() || t.blocked.is_some() || app.conflict_for(&t.uid).is_some()
}

enum Item {
    Header(String, bool, usize),
    Blank,
    Row(Row),
}

/// Group headers and rows in display order, restricted to the rows `App::visible_rows` keeps.
fn items(app: &App) -> Vec<Item> {
    let visible = app.visible_rows();
    let result = build_view(&app.snapshot, &app.view, app.now, &app.expanded);
    let today = app.now.date_naive();
    let mut next = 0;
    let mut out = Vec::new();
    for (g, rows) in result.groups {
        let kept: Vec<Row> = rows
            .into_iter()
            .filter(|r| {
                let hit = visible.get(next) == Some(r);
                next += usize::from(hit);
                hit
            })
            .collect();
        if kept.is_empty() {
            continue;
        }
        if let Some(title) = group_title(g, today) {
            if !out.is_empty() {
                out.push(Item::Blank);
            }
            out.push(Item::Header(title, g == DateGroup::Overdue, kept.len()));
        }
        out.extend(kept.into_iter().map(Item::Row));
    }
    out
}

/// Draw the pane; with the edit card open, in place of its task's row when it fits, and with
/// a new row open, that row next to its anchor (at the top when the anchor is not shown).
/// Returns whether the card was drawn here (else the caller draws it as an overlay).
pub fn render(f: &mut Frame, area: Rect, app: &App) -> bool {
    let block = pane_block(pane_title(app), app, Pane::Tasks).padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return false;
    }
    let card = app.form.as_ref().and_then(|e| {
        let h = card::height(app, e, inner.width);
        (inner.width >= card::MIN_WIDTH && h <= inner.height).then_some((e, h as usize))
    });
    let width = inner.width as usize;
    let items = items(app);
    let new_row = app.new_row.as_ref();
    if items.is_empty()
        && let Some(row) = new_row
    {
        let line = new_row_line(app, row, 0, width, (1, 0));
        f.render_widget(Paragraph::new(line), inner);
        return false;
    }
    if items.is_empty() {
        // A new task's card in an empty view sits at the top.
        if let Some((e, h)) = card.filter(|(e, _)| e.uid.is_none()) {
            card::render(
                f,
                Rect {
                    height: h as u16,
                    ..inner
                },
                app,
                e,
            );
            return true;
        }
        f.render_widget(Paragraph::new(Line::styled("No tasks", dim())), inner);
        return false;
    }

    // Column widths are shared by every row so dates line up.
    let task_rows: Vec<&Row> = items
        .iter()
        .filter_map(|i| if let Item::Row(r) = i { Some(r) } else { None })
        .collect();
    let col = |f: &dyn Fn(&Row) -> String, cap: usize| {
        task_rows
            .iter()
            .map(|r| f(r).width())
            .max()
            .unwrap_or(0)
            .min(cap)
    };
    let glyph_w = col(&|r| glyph(app, r.task), 5);
    let mut hint_w = col(&|r| hint(&app.snapshot.tasks[r.task]), MAX_HINT);
    let date_w = col(&|r| date_text(app, &app.snapshot.tasks[r.task]), MAX_DATE);
    let max_depth = task_rows
        .iter()
        .map(|r| r.depth as usize * 2)
        .max()
        .unwrap_or(0);
    let mark_w = if task_rows
        .iter()
        .any(|r| flagged(app, &app.snapshot.tasks[r.task]))
    {
        2
    } else {
        0
    };
    let left_fixed = 2 + max_depth + glyph_w + 1 + mark_w;
    let right =
        |h: usize| (if h > 0 { h + 1 } else { 0 }) + if date_w > 0 { date_w + 1 } else { 0 };
    if width < left_fixed + right(hint_w) + MIN_TITLE {
        hint_w = 0;
    }

    let mut lines: Vec<Line> = Vec::new();
    let mut card_at: Option<usize> = None;
    let mut row_at: Option<usize> = None;
    let mut selected_line = 0;
    let mut row_no = 0;
    for item in &items {
        match item {
            Item::Blank => lines.push(Line::raw("")),
            Item::Header(title, overdue, n) => {
                let count = n.to_string();
                let style = if *overdue {
                    theme::overdue().add_modifier(Modifier::BOLD)
                } else {
                    Style::new().add_modifier(Modifier::BOLD)
                };
                let title = truncate(title, width.saturating_sub(count.width() + 1));
                let gap = width.saturating_sub(title.width() + count.width());
                lines.push(Line::from(vec![
                    Span::styled(title, style),
                    Span::raw(" ".repeat(gap)),
                    Span::styled(count, dim()),
                ]));
            }
            Item::Row(r) => {
                // While a new row is open it is the selection, not the cursor's row.
                let selected = row_no == app.cursor && new_row.is_none();
                if selected {
                    selected_line = lines.len();
                }
                row_no += 1;
                let uid = &app.snapshot.tasks[r.task].uid;
                let anchored = new_row.filter(|n| n.anchor.as_ref() == Some(uid));
                let new_line = |below: bool| {
                    anchored.filter(|n| n.below == below).map(|n| {
                        let into = n.target.parent.as_ref() == Some(uid);
                        let depth = r.depth as usize + usize::from(into);
                        new_row_line(app, n, depth, width, (glyph_w, mark_w))
                    })
                };
                if let Some(l) = new_line(false) {
                    row_at = Some(lines.len());
                    lines.push(l);
                }
                // The card takes its task's row; a new task's card opens below the cursor.
                let card_here = card.filter(|(e, _)| e.uid.as_ref() == Some(uid));
                if card_here.is_none() {
                    lines.push(row_line(
                        app,
                        r,
                        selected,
                        width,
                        (glyph_w, mark_w, hint_w, date_w),
                    ));
                }
                let new_here = card.filter(|(e, _)| e.uid.is_none() && selected);
                if let Some((_, h)) = card_here.or(new_here) {
                    card_at = Some(lines.len());
                    lines.extend(std::iter::repeat_n(Line::raw(""), h));
                }
                if let Some(l) = new_line(true) {
                    row_at = Some(lines.len());
                    lines.push(l);
                }
            }
        }
    }
    // A row whose anchor the view does not show goes on top.
    if let Some(n) = new_row {
        selected_line = row_at.unwrap_or_else(|| {
            lines.insert(0, new_row_line(app, n, 0, width, (glyph_w, mark_w)));
            0
        });
    }

    let h = inner.height as usize;
    // Scroll so the card is whole (it fits: `card` checked its height), else the cursor row.
    let (last, first) = match (card, card_at) {
        (Some((_, ch)), Some(at)) => (at + ch, at),
        _ => (selected_line + 1, selected_line),
    };
    let mut offset = last.saturating_sub(h);
    // Keep the group header above the first row visible when it fits.
    if offset > 0 && offset == first && first >= 1 && last - first < h {
        offset -= 1;
    }
    let visible: Vec<Line> = lines.into_iter().skip(offset).take(h).collect();
    f.render_widget(Paragraph::new(visible), inner);
    let (Some((e, ch)), Some(at)) = (card, card_at) else {
        return false;
    };
    let y = inner.y + (at - offset) as u16;
    card::render(
        f,
        Rect {
            y,
            height: ch as u16,
            ..inner
        },
        app,
        e,
    );
    true
}

/// The new row: the target list's bar, an open glyph, and the typed title with the cursor,
/// lined up with the rows' titles. Highlighted as the selection.
fn new_row_line(
    app: &App,
    row: &NewRow,
    depth: usize,
    width: usize,
    (glyph_w, mark_w): (usize, usize),
) -> Line<'static> {
    let indent = depth * 2;
    let glyph_w = glyph_w.max(1);
    let title_w = width.saturating_sub(2 + indent + glyph_w + 1 + mark_w);
    let title = tail(&format!("{}▏", row.title), title_w);
    let filler = title_w.saturating_sub(title.width());
    let spans = vec![
        Span::styled("›", theme::accent()),
        Span::styled(
            "▌",
            Style::new().fg(bar_color(&app.snapshot, &row.target.collection)),
        ),
        Span::raw(" ".repeat(indent)),
        Span::raw(pad("○", glyph_w)),
        Span::raw(" ".repeat(1 + mark_w)),
        Span::raw(title),
        Span::raw(" ".repeat(filler)),
    ];
    Line::from(spans).style(theme::selection())
}

/// The end of `s` that fits in `width` cells, so the typing cursor stays in view.
fn tail(s: &str, width: usize) -> String {
    let mut out: Vec<char> = Vec::new();
    let mut used = 0;
    for c in s.chars().rev() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w > width {
            break;
        }
        used += w;
        out.push(c);
    }
    out.into_iter().rev().collect()
}

/// Follows the title of a task whose edit waits in the write queue.
const QUEUED_MARKER: &str = " (queued)";

fn row_line(
    app: &App,
    r: &Row,
    selected: bool,
    width: usize,
    (glyph_w, mark_w, hint_w, date_w): (usize, usize, usize, usize),
) -> Line<'static> {
    let t = &app.snapshot.tasks[r.task];
    let overdue = is_overdue(app, r.task);
    let indent = r.depth as usize * 2;
    let left = 2 + indent + glyph_w + 1 + mark_w;
    let right = (if hint_w > 0 { hint_w + 1 } else { 0 }) + if date_w > 0 { date_w + 1 } else { 0 };
    let title_w = width.saturating_sub(left + right);

    // Keep parent context for independently displayed Today rows and detached tree roots.
    let parent_hint = match (&app.view, r.parent_hint) {
        (View::Today, Some(p)) => Some(format!("  ↳ {}", app.snapshot.tasks[p].summary)),
        (View::All | View::List(_), Some(p)) if r.depth == 0 => {
            Some(format!("  ↳ {}", app.snapshot.tasks[p].summary))
        }
        _ => None,
    };
    let budget = parent_hint
        .as_ref()
        .map_or(0, |s| truncate(s, title_w / 3).width());
    // A queued edit shows as saved, with a muted marker until a sync lets it be written.
    let marker = if app.snapshot.is_pending(r.task) {
        QUEUED_MARKER
    } else {
        ""
    };
    let marker_w = marker.width();
    let title = truncate(&t.summary, title_w.saturating_sub(budget + marker_w));
    let remaining = title_w.saturating_sub(title.width() + marker_w);
    let suffix = parent_hint
        .map(|s| truncate(&s, remaining))
        .unwrap_or_default();
    let filler = remaining.saturating_sub(suffix.width());

    let text = if overdue {
        theme::overdue()
    } else {
        Style::new()
    };
    let mut spans = vec![
        Span::styled(
            if selected { "›" } else { " " },
            if app.focus == Pane::Tasks {
                theme::accent()
            } else {
                dim()
            },
        ),
        Span::styled(
            "▌",
            Style::new().fg(bar_color(&app.snapshot, &t.collection)),
        ),
        Span::raw(" ".repeat(indent)),
        Span::styled(pad(&glyph(app, r.task), glyph_w), text),
        Span::raw(" "),
    ];
    if mark_w > 0 {
        let mark = if flagged(app, t) { "!" } else { " " };
        spans.push(Span::styled(
            pad(mark, mark_w),
            theme::warning().add_modifier(Modifier::DIM),
        ));
    }
    spans.extend([
        Span::styled(title, text),
        Span::styled(marker, theme::muted()),
        Span::styled(suffix, dim()),
        Span::raw(" ".repeat(filler)),
    ]);
    if hint_w > 0 {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            pad(&truncate(&hint(t), hint_w), hint_w),
            dim(),
        ));
    }
    if date_w > 0 {
        let d = truncate(&date_text(app, t), date_w);
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("{}{d}", " ".repeat(date_w - d.width())),
            text,
        ));
    }
    if selected && app.focus == Pane::Tasks {
        for s in &mut spans {
            s.style = s.style.remove_modifier(Modifier::DIM);
        }
        Line::from(spans).style(theme::selection())
    } else {
        Line::from(spans)
    }
}
