//! TUI rendering into a `TestBackend`. All content is synthetic.

use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, TimeZone};
use chrono_tz::Tz;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use tdo::app::{App, Mode};
use tdo::ui::draw;
use twydo_core::config::Config;
use twydo_core::views::{Snapshot, View, counts};

const NY: Tz = chrono_tz::America::New_York;

fn now() -> DateTime<Tz> {
    NY.with_ymd_and_hms(2026, 11, 4, 10, 0, 0).unwrap()
}

fn snapshot_at(root: &std::path::Path) -> Snapshot {
    let report = twydo_core::vdir::load(root).unwrap();
    let mut cfg = Config::with_vdir(PathBuf::from("unused"));
    cfg.timezone = Some("America/New_York".into());
    Snapshot::new(report, cfg)
}

fn app() -> App {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../twydo-core/tests/data/sample-vdir");
    App::new(snapshot_at(&root), now())
}

fn render(app: &App, w: u16, h: u16) -> Buffer {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| draw(f, app)).unwrap();
    term.backend().buffer().clone()
}

fn line(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol())
        .collect::<String>()
}

fn text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| line(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Position of the first cell of `needle` inside the task pane (columns 20..84 at width 120).
fn find_in_tasks(buf: &Buffer, needle: &str) -> (u16, u16) {
    for y in 0..buf.area.height {
        let l: Vec<&str> = (20..84).map(|x| buf[(x, y)].symbol()).collect();
        let joined = l.concat();
        if let Some(b) = joined.find(needle) {
            return (20 + joined[..b].chars().count() as u16, y);
        }
    }
    panic!("{needle:?} not in task pane:\n{}", text(buf));
}

#[test]
fn renders_three_panes_wide() {
    let t = text(&render(&app(), 120, 30));
    assert!(t.contains("Views"), "{t}");
    assert!(t.contains("Details"), "{t}");
    assert!(t.contains("Today · Wednesday 4 Nov"), "{t}");
    assert!(t.contains("OVERDUE"), "{t}");
    assert!(t.contains("NORMAL"), "{t}");
}

#[test]
fn details_hidden_when_narrow() {
    let t = text(&render(&app(), 100, 30));
    assert!(!t.contains("Details"), "{t}");
    assert!(t.contains("Views"), "{t}");
}

#[test]
fn details_pane_shows_the_selected_task() {
    let mut a = app();
    a.view = View::All;
    let rows = a.visible_rows();
    a.cursor = rows
        .iter()
        .position(|r| a.snapshot.tasks[r.task].summary == "Pay water bill")
        .unwrap();
    let t = text(&render(&a, 120, 30));
    assert!(t.contains("15 min before due"), "{t}");
    assert!(t.contains("List"), "{t}");
    assert!(t.contains("Bills"), "{t}");
    assert!(t.contains("Fri 09:00"), "{t}");
}

#[test]
fn only_ansi_chrome() {
    let mut a = app();
    a.show_help = false;
    for (view, w) in [(View::Today, 120), (View::All, 80), (View::Done, 120)] {
        a.view = view;
        let buf = render(&a, w, 30);
        let mut bars = 0;
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let c = &buf[(x, y)];
                if matches!(c.fg, Color::Rgb(..)) || matches!(c.bg, Color::Rgb(..)) {
                    assert_eq!(c.symbol(), "▌", "non-bar cell at {x},{y} is true colour");
                    bars += 1;
                }
            }
        }
        assert!(bars > 0, "expected coloured bars");
    }
    // Also with the help overlay open.
    a.show_help = true;
    let buf = render(&a, 120, 30);
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let c = &buf[(x, y)];
            if matches!(c.fg, Color::Rgb(..)) || matches!(c.bg, Color::Rgb(..)) {
                assert_eq!(c.symbol(), "▌");
            }
        }
    }
}

#[test]
fn overdue_row_is_red() {
    let buf = render(&app(), 120, 30);
    let (x, y) = find_in_tasks(&buf, "Renew library books");
    assert_eq!(buf[(x, y)].fg, Color::Red);
    let (x, y) = find_in_tasks(&buf, "OVERDUE");
    assert_eq!(buf[(x, y)].fg, Color::Red);
}

#[test]
fn only_overdue_is_red() {
    let mut a = app();
    a.view = View::All;
    let buf = render(&a, 120, 30);
    let (x, y) = find_in_tasks(&buf, "Call the dentist");
    assert_ne!(buf[(x, y)].fg, Color::Red);
}

#[test]
fn glyphs_and_counts() {
    let mut a = app();
    a.view = View::All;
    let t = text(&render(&a, 120, 40));
    assert!(t.contains('↺'), "{t}");
    assert!(t.contains('○'), "{t}");
    assert!(t.contains("[3]"), "{t}");
    assert!(t.contains("[2]"), "{t}");
    assert!(t.contains("monthly"), "{t}");
    assert!(t.contains("project"), "{t}");
    assert!(t.contains("checklist"), "{t}");
    for v in [View::Today, View::All, View::Scheduled] {
        let c = counts(&a.snapshot, &v, now());
        let want = if c.due_or_overdue > 0 {
            format!("{}│{}", c.due_or_overdue, c.total)
        } else {
            c.total.to_string()
        };
        assert!(t.contains(&want), "missing {want} in {t}");
    }
    let today = counts(&a.snapshot, &View::Today, now());
    assert!(today.due_or_overdue > 0);
}

#[test]
fn selected_row_is_reversed() {
    let mut a = app();
    a.view = View::All;
    a.cursor = 1;
    let buf = render(&a, 120, 30);
    let rows = a.visible_rows();
    let sel = &a.snapshot.tasks[rows[1].task].summary;
    let (x, y) = find_in_tasks(&buf, sel);
    assert!(buf[(x, y)].modifier.contains(Modifier::REVERSED));
    let other = &a.snapshot.tasks[rows[0].task].summary;
    let (x, y) = find_in_tasks(&buf, other);
    assert!(!buf[(x, y)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn modes_help_and_messages_render() {
    let mut a = app();
    a.view = View::All;
    a.mode = Mode::Search;
    a.input = "water".into();
    a.search = Some("water".into());
    let t = text(&render(&a, 120, 30));
    assert!(t.contains("SEARCH"), "{t}");
    assert!(t.contains("/water"), "{t}");
    assert!(t.contains("Pay water bill"), "{t}");
    assert!(!t.contains("Renew library books"), "{t}");

    let mut a = app();
    a.message = Some("toggle done: not enabled until the phase-0 compatibility gate passes".into());
    let t = text(&render(&a, 120, 30));
    assert!(t.contains("not enabled until"), "{t}");

    let mut a = app();
    a.show_help = true;
    let t = text(&render(&a, 120, 40));
    assert!(t.contains("Help"), "{t}");
    assert!(t.contains("(after gate)"), "{t}");
    assert!(t.contains("toggle done"), "{t}");
}

#[test]
fn tiny_terminal_does_not_panic() {
    let mut a = app();
    for (w, h) in [(20, 5), (1, 1), (10, 3), (0, 0), (3, 40), (200, 2), (60, 8)] {
        for view in [View::Today, View::All, View::Done] {
            a.view = view;
            a.show_help = false;
            render(&a, w, h);
            a.show_help = true;
            render(&a, w, h);
        }
    }
    a.show_help = false;
    a.mode = Mode::Command;
    a.input = "list x".into();
    render(&a, 20, 5);
}

#[test]
fn wide_titles_truncate_by_display_width() {
    let dir = tempfile::tempdir().unwrap();
    let col = dir.path().join("wide");
    fs::create_dir(&col).unwrap();
    fs::write(col.join("color"), "#3B82F6\n").unwrap();
    let titles = [
        "漢字かな交じり文漢字かな交じり文漢字かな交じり文漢字かな交じり文漢字かな交じり文",
        "🎉🎉🎉 party planning 🎉🎉🎉 with a very long title that keeps going and going on",
        "short",
        "混ぜ mixed 絵文字😀 title that is also rather long for an eighty column terminal ok",
    ];
    let dues = ["20261103", "20261104", "20261105", "20261106"];
    for (i, (t, d)) in titles.iter().zip(dues).enumerate() {
        fs::write(
            col.join(format!("t{i}.ics")),
            format!(
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\nUID:w-{i}\r\nSUMMARY:{t}\r\nDUE;VALUE=DATE:{d}\r\nSTATUS:NEEDS-ACTION\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
            ),
        )
        .unwrap();
    }
    let mut a = App::new(snapshot_at(dir.path()), now());
    a.view = View::All;
    let buf = render(&a, 80, 20);
    let right = buf.area.width - 1;
    let mut ends = Vec::new();
    for y in 0..buf.area.height {
        let l = line(&buf, y);
        let in_tasks: String = (20..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
        if !in_tasks.contains('▌') {
            continue;
        }
        assert_eq!(buf[(right, y)].symbol(), "│", "row overflows: {l}");
        let end = (20..right)
            .rev()
            .find(|&x| !buf[(x, y)].symbol().trim().is_empty())
            .unwrap();
        ends.push(end);
    }
    assert_eq!(ends.len(), 4, "{}", text(&buf));
    assert!(ends.windows(2).all(|w| w[0] == w[1]), "{ends:?}");
}
