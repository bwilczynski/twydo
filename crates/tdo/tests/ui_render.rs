//! TUI rendering into a `TestBackend`. All content is synthetic.

use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, TimeZone};
use chrono_tz::Tz;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier};
use tdo::app::{App, Mode, Pane};
use tdo::ui::draw;
use twydo_core::config::{Config, ListPrefs, Sort, SortField};
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

/// Draw like the event loop does: size the App first, then render.
fn render(app: &mut App, w: u16, h: u16) -> Buffer {
    app.resize(w, h);
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

/// Position of text inside the task pane, discovered from the rendered borders.
fn find_in_tasks(buf: &Buffer, needle: &str) -> (u16, u16) {
    let start = (1..buf.area.width)
        .find(|&x| buf[(x, 0)].symbol() == "╭")
        .unwrap();
    let end = (start + 1..buf.area.width)
        .find(|&x| buf[(x, 0)].symbol() == "╭")
        .unwrap_or(buf.area.width);
    for y in 0..buf.area.height {
        let l: Vec<&str> = (start..end).map(|x| buf[(x, y)].symbol()).collect();
        let joined = l.concat();
        if let Some(b) = joined.find(needle) {
            return (start + joined[..b].chars().count() as u16, y);
        }
    }
    panic!("{needle:?} not in task pane:\n{}", text(buf));
}

#[test]
fn renders_three_panes_wide() {
    let t = text(&render(&mut app(), 120, 30));
    assert!(t.contains("Views"), "{t}");
    assert!(t.contains("Details"), "{t}");
    assert!(t.contains("Today · Wednesday 4 Nov"), "{t}");
    assert!(t.contains("OVERDUE"), "{t}");
    assert!(t.contains("NORMAL"), "{t}");
}

#[test]
fn details_hidden_when_narrow() {
    let t = text(&render(&mut app(), 100, 30));
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
    let t = text(&render(&mut a, 120, 30));
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
        let buf = render(&mut a, w, 30);
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
    let buf = render(&mut a, 120, 30);
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
    let buf = render(&mut app(), 120, 30);
    let (x, y) = find_in_tasks(&buf, "Renew library books");
    assert_eq!(buf[(x, y)].fg, Color::Red);
    let (x, y) = find_in_tasks(&buf, "OVERDUE");
    assert_eq!(buf[(x, y)].fg, Color::Red);
}

#[test]
fn only_overdue_is_red() {
    let mut a = app();
    a.view = View::All;
    let buf = render(&mut a, 120, 30);
    let (x, y) = find_in_tasks(&buf, "Call the dentist");
    assert_ne!(buf[(x, y)].fg, Color::Red);
}

#[test]
fn glyphs_and_counts() {
    let mut a = app();
    a.view = View::All;
    let t = text(&render(&mut a, 120, 40));
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
fn selected_row_has_palette_background_without_reversing_colors() {
    let mut a = app();
    a.view = View::All;
    a.cursor = 1;
    let buf = render(&mut a, 120, 30);
    let rows = a.visible_rows();
    let sel = &a.snapshot.tasks[rows[1].task].summary;
    let (x, y) = find_in_tasks(&buf, sel);
    assert!(!buf[(x, y)].modifier.contains(Modifier::REVERSED));
    assert_eq!(buf[(x, y)].bg, Color::DarkGray);
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
    let t = text(&render(&mut a, 120, 30));
    assert!(t.contains("SEARCH"), "{t}");
    assert!(t.contains("/water"), "{t}");
    assert!(t.contains("Pay water bill"), "{t}");
    assert!(!t.contains("Renew library books"), "{t}");

    let mut a = app();
    a.message = Some("toggle done: not available yet".into());
    let t = text(&render(&mut a, 120, 30));
    assert!(t.contains("not available yet"), "{t}");

    let mut a = app();
    a.show_help = true;
    let t = text(&render(&mut a, 120, 40));
    assert!(t.contains("Help"), "{t}");
    assert!(t.contains("reorder nested items"), "{t}");
    assert!(t.contains("toggle done"), "{t}");
}

#[test]
fn help_lists_new_keys() {
    let mut a = app();
    a.show_help = true;
    let t = text(&render(&mut a, 120, 40));
    for key in ["o  O", "a ", "ct  cp  cc", "r "] {
        assert!(t.contains(key), "help lacks {key:?}: {t}");
    }
    assert!(t.contains("new task below / above"), "{t}");
    assert!(t.contains("add items to a project or checklist"), "{t}");
    assert!(t.contains("change type"), "{t}");
}

#[test]
fn tiny_terminal_does_not_panic() {
    let mut a = app();
    for (w, h) in [(20, 5), (1, 1), (10, 3), (0, 0), (3, 40), (200, 2), (60, 8)] {
        for view in [View::Today, View::All, View::Done] {
            a.view = view;
            a.show_help = false;
            render(&mut a, w, h);
            a.show_help = true;
            render(&mut a, w, h);
            a.show_help = false;
            a.show_errors = true;
            render(&mut a, w, h);
            a.show_errors = false;
        }
    }
    a.show_help = false;
    a.mode = Mode::Command;
    a.input = "list x".into();
    render(&mut a, 20, 5);
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
    let buf = render(&mut a, 80, 20);
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

fn sidebar_lines(buf: &Buffer) -> Vec<String> {
    (0..buf.area.height)
        .map(|y| (1..19).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect()
}

#[test]
fn sidebar_renders_in_navigation_order() {
    let mut a = app();
    let buf = render(&mut a, 120, 30);
    let lines = sidebar_lines(&buf);
    let mut last = None;
    for v in a.sidebar_items() {
        let name = match &v {
            View::Today => "Today".to_string(),
            View::All => "All".to_string(),
            View::Scheduled => "Scheduled".to_string(),
            View::Done => "Done".to_string(),
            View::Next3Days => "Next 3 Days".to_string(),
            View::List(id) => {
                let mut c = id.chars();
                c.next().unwrap().to_uppercase().chain(c).collect()
            }
        };
        let y = lines
            .iter()
            .position(|l| l.trim_start_matches(['▌', '◷', ' ']).starts_with(&name))
            .unwrap_or_else(|| panic!("{name} not in sidebar:\n{}", text(&buf)));
        assert!(
            last.is_none_or(|p| y > p),
            "{name} out of order:\n{}",
            text(&buf)
        );
        last = Some(y);
    }
}

fn vdir_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let col = dir.path().join("synthetic");
    fs::create_dir(&col).unwrap();
    for (name, body) in files {
        fs::write(col.join(name), body).unwrap();
    }
    dir
}

#[test]
fn unreadable_files_indicator_and_errors_overlay() {
    let dir = vdir_with(&[
        (
            "ok.ics",
            "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:ok\r\nSUMMARY:Fine task\r\nEND:VTODO\r\nEND:VCALENDAR\r\n",
        ),
        ("broken.ics", "this is not ical"),
    ]);
    let mut a = App::new(snapshot_at(dir.path()), now());
    a.view = View::All;
    let buf = render(&mut a, 120, 30);
    let t = text(&buf);
    assert!(t.contains("! 1 file unreadable"), "{t}");
    let y = buf.area.height - 1;
    let x = (0..buf.area.width)
        .find(|&x| buf[(x, y)].symbol() == "!")
        .unwrap();
    assert_eq!(buf[(x, y)].fg, Color::Yellow);
    assert!(!t.contains("broken.ics"), "{t}");

    a.show_errors = true;
    let t = text(&render(&mut a, 120, 30));
    assert!(t.contains("broken.ics"), "{t}");
    assert!(t.contains("Unreadable files"), "{t}");
}

#[test]
fn rows_with_warnings_or_blocks_are_marked() {
    let dir = vdir_with(&[
        (
            "ok.ics",
            "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:ok\r\nSUMMARY:Fine task\r\nEND:VTODO\r\nEND:VCALENDAR\r\n",
        ),
        (
            "warn.ics",
            "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:warn\r\nSUMMARY:Odd task\r\nX-2DOAPP-METADATA:@tasktype:folder\r\nEND:VTODO\r\nEND:VCALENDAR\r\n",
        ),
        (
            "blocked.ics",
            "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:blk\r\nSUMMARY:Stuck task\r\nRRULE:FREQ=HOURLY\r\nEND:VTODO\r\nEND:VCALENDAR\r\n",
        ),
    ]);
    let mut a = App::new(snapshot_at(dir.path()), now());
    a.view = View::All;
    // Below 110 columns there is no details pane, so the marker is the only sign.
    let buf = render(&mut a, 80, 20);
    let row = |title: &str| {
        (0..buf.area.height)
            .map(|y| line(&buf, y))
            .find(|l| l.contains(title))
            .unwrap_or_else(|| panic!("{title} missing:\n{}", text(&buf)))
    };
    assert!(row("Odd task").contains("! Odd task"), "{}", text(&buf));
    assert!(row("Stuck task").contains("! Stuck task"), "{}", text(&buf));
    assert!(!row("Fine task").contains('!'), "{}", text(&buf));
    let (x, y) = (0..buf.area.height)
        .find_map(|y| {
            let l = line(&buf, y);
            l.contains("Odd task").then(|| {
                let b = l.find("! Odd").unwrap();
                (l[..b].chars().count() as u16, y)
            })
        })
        .unwrap();
    assert_eq!(buf[(x, y)].fg, Color::Yellow);
}

#[test]
fn overdue_red_is_decided_per_row() {
    // Nested: at Nov 10 the project child "Collect receipts" (due Nov 8) is overdue while its
    // undated project sits under NO DUE DATE.
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../twydo-core/tests/data/sample-vdir");
    let later = NY.with_ymd_and_hms(2026, 11, 10, 10, 0, 0).unwrap();
    let mut a = App::new(snapshot_at(&root), later);
    a.view = View::All;
    a.expanded.insert("sv-bills-4".into());
    let buf = render(&mut a, 120, 40);
    let (x, y) = find_in_tasks(&buf, "Collect receipts");
    assert_eq!(buf[(x, y)].fg, Color::Red, "{}", text(&buf));
    let (x, y) = find_in_tasks(&buf, "Prepare tax return");
    assert_ne!(buf[(x, y)].fg, Color::Red);

    // Ungrouped: a title-sorted list has no OVERDUE header, but its overdue row is red.
    let mut snap = snapshot_at(&root);
    snap.config.lists.insert(
        "inbox".into(),
        ListPrefs {
            sort: Sort {
                field: SortField::Title,
                descending: false,
            },
            ..ListPrefs::default()
        },
    );
    let mut a = App::new(snap, now());
    a.view = View::List("inbox".into());
    let buf = render(&mut a, 120, 30);
    assert!(!text(&buf).contains("OVERDUE"), "{}", text(&buf));
    let (x, y) = find_in_tasks(&buf, "Renew library books");
    assert_eq!(buf[(x, y)].fg, Color::Red);
    let (x, y) = find_in_tasks(&buf, "Call the dentist");
    assert_ne!(buf[(x, y)].fg, Color::Red);

    // Completed tasks are not overdue, whatever their due date.
    let mut a = app();
    a.view = View::Done;
    let buf = render(&mut a, 120, 30);
    let (x, y) = find_in_tasks(&buf, "Buy stamps");
    assert_ne!(buf[(x, y)].fg, Color::Red);
}

fn summary(
    status: twydo_core::sync::status::SyncStatus,
    pending: usize,
    conflicts: Vec<twydo_core::conflicts::Conflict>,
) -> twydo_core::sync::SyncSummary {
    twydo_core::sync::SyncSummary {
        status,
        pending,
        conflicts,
        off: false,
        queued: 0,
    }
}

fn conflict_on(uid: &str) -> twydo_core::conflicts::Conflict {
    use twydo_core::conflicts::*;
    Conflict {
        id: "c1".into(),
        uid: uid.into(),
        collection: "x".into(),
        path: "x/y.ics".into(),
        kind: ConflictKind::BothChanged,
        held: true,
        vdir_holds: Side::Local,
        detected: chrono::Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap(),
        state: ConflictState::Open,
        choice: None,
        resolved: None,
        local: None,
        remote: None,
    }
}

fn utc(h: u32, m: u32) -> chrono::DateTime<chrono::Utc> {
    chrono::Utc.with_ymd_and_hms(2026, 11, 4, h, m, 0).unwrap()
}

fn status_line(buf: &Buffer) -> String {
    line(buf, buf.area.height - 1)
}

#[test]
fn status_counts_queued_edits_as_pending() {
    let mut a = app();
    let st = twydo_core::sync::status::SyncStatus {
        last_success: Some(utc(14, 57)),
        ..Default::default()
    };
    let mut s = summary(st, 1, vec![]);
    s.queued = 2;
    a.set_sync(Some(s));
    let buf = render(&mut a, 140, 20);
    let l = status_line(&buf);
    assert!(l.contains("synced 3m ago · 3 pending"), "{l}");
}

#[test]
fn a_queued_edit_is_marked_on_its_row() {
    let mut a = app();
    let first = row_title(&a, 0);
    let second = row_title(&a, 1);
    let uid = a.snapshot.tasks[a.visible_rows()[0].task].uid.clone();
    let marked = a.snapshot.clone().with_pending([uid].into());
    a.reload(marked);
    let buf = render(&mut a, 120, 30);
    let (x, y) = find_in_tasks(&buf, &first);
    let row: String = (x..buf.area.width).map(|c| buf[(c, y)].symbol()).collect();
    assert!(row.starts_with(&format!("{first} (queued)")), "{row}");
    let (x, y) = find_in_tasks(&buf, &second);
    let row: String = (x..buf.area.width).map(|c| buf[(c, y)].symbol()).collect();
    assert!(!row.contains("(queued)"), "{row}");
}

#[test]
fn status_shows_sync_off_with_the_pending_count() {
    let mut a = app();
    let st = twydo_core::sync::status::SyncStatus {
        last_success: Some(utc(14, 57)),
        ..Default::default()
    };
    let mut s = summary(st, 3, vec![]);
    s.off = true;
    a.set_sync(Some(s.clone()));
    let buf = render(&mut a, 140, 20);
    let l = status_line(&buf);
    assert!(l.contains("sync off · synced 3m ago · 3 pending"), "{l}");
    let x = l.find("sync off").unwrap();
    let x = l[..x].chars().count() as u16;
    assert_eq!(buf[(x, buf.area.height - 1)].fg, Color::Yellow);

    s.pending = 0;
    a.set_sync(Some(s));
    assert!(status_line(&render(&mut a, 140, 20)).contains("sync off"));
    a.set_sync(Some(summary(Default::default(), 0, vec![])));
    assert!(!status_line(&render(&mut a, 140, 20)).contains("sync off"));
}

#[test]
fn status_shows_synced_ago_pending_and_conflicts() {
    let mut a = app();
    // now() is 10:00 New York = 15:00 UTC.
    let st = twydo_core::sync::status::SyncStatus {
        last_success: Some(utc(14, 57)),
        ..Default::default()
    };
    a.set_sync(Some(summary(st, 2, vec![conflict_on("nope")])));
    let buf = render(&mut a, 140, 20);
    let l = status_line(&buf);
    assert!(l.contains("synced 3m ago · 2 pending · 1 conflict"), "{l}");
    let x = l.find("1 conflict").unwrap();
    let x = l[..x].chars().count() as u16;
    assert_eq!(buf[(x, buf.area.height - 1)].fg, Color::Yellow);

    a.set_sync(Some(summary(Default::default(), 0, vec![])));
    assert!(status_line(&render(&mut a, 140, 20)).contains("not synced yet"));
    let st = twydo_core::sync::status::SyncStatus {
        last_success: Some(utc(15, 0)),
        ..Default::default()
    };
    a.set_sync(Some(summary(st, 0, vec![])));
    assert!(status_line(&render(&mut a, 140, 20)).contains("just now"));
}

#[test]
fn status_shows_emptied_list() {
    use twydo_core::sync::status::CollState;
    let mut a = app();
    let mut st = twydo_core::sync::status::SyncStatus {
        last_success: Some(utc(14, 0)),
        ..Default::default()
    };
    st.collections
        .insert("groceries".into(), CollState::EmptiedRemote);
    a.set_sync(Some(summary(st, 0, vec![])));
    let l = status_line(&render(&mut a, 140, 20));
    assert!(
        l.contains("\"groceries\" emptied on server (:sync! groceries)"),
        "{l}"
    );
}

#[test]
fn status_shows_failure_with_retry_time() {
    let mut a = app();
    let st = twydo_core::sync::status::SyncStatus {
        last_success: Some(utc(14, 0)),
        failures: 1,
        last_error: Some("timed out".into()),
        next_attempt: Some(utc(15, 30)),
        ..Default::default()
    };
    a.set_sync(Some(summary(st, 0, vec![])));
    let buf = render(&mut a, 140, 20);
    let l = status_line(&buf);
    assert!(l.contains("sync failed: timed out; retry 10:30"), "{l}");
    let x = l.find("sync failed").unwrap();
    assert_eq!(
        buf[(l[..x].chars().count() as u16, buf.area.height - 1)].fg,
        Color::Red
    );
}

fn select(a: &mut App, title: &str) -> String {
    a.view = View::All;
    let rows = a.visible_rows();
    a.cursor = rows
        .iter()
        .position(|r| a.snapshot.tasks[r.task].summary == title)
        .unwrap();
    a.snapshot.tasks[a.selected().unwrap()].uid.clone()
}

#[test]
fn conflicted_row_has_marker() {
    let mut a = app();
    let uid = select(&mut a, "Pay water bill");
    a.set_sync(Some(summary(
        Default::default(),
        0,
        vec![conflict_on(&uid)],
    )));
    let buf = render(&mut a, 80, 20);
    let l = (0..buf.area.height)
        .map(|y| line(&buf, y))
        .find(|l| l.contains("Pay water bill"))
        .unwrap();
    assert!(l.contains("! Pay water bill"), "{l}");
}

#[test]
fn details_show_conflict_line() {
    let mut a = app();
    let uid = select(&mut a, "Pay water bill");
    a.set_sync(Some(summary(
        Default::default(),
        0,
        vec![conflict_on(&uid)],
    )));
    let t = text(&render(&mut a, 120, 30));
    assert!(t.contains("! conflict: changed here"), "{t}");
    assert!(t.contains(":conflicts"), "{t}");
}

#[test]
fn conflicts_overlay_lists_kind_in_words() {
    let mut a = app();
    let uid = select(&mut a, "Pay water bill");
    let mut c2 = conflict_on("other");
    c2.id = "c2".into();
    c2.state = twydo_core::conflicts::ConflictState::Chosen;
    c2.kind = twydo_core::conflicts::ConflictKind::LocalDeleteRemoteEdit;
    a.set_sync(Some(summary(
        Default::default(),
        0,
        vec![conflict_on(&uid), c2],
    )));
    a.show_conflicts = true;
    let t = text(&render(&mut a, 120, 30));
    assert!(t.contains("Pay water bill"), "{t}");
    assert!(t.contains("changed here and on another device"), "{t}");
    assert!(t.contains("deleted here, edited on another device"), "{t}");
    assert!(t.contains("waiting for sync"), "{t}");
}

#[test]
fn sidebar_uses_wide_terminal_space_for_list_names() {
    let mut a = app();
    a.snapshot.collections[0].display_name = Some("家計と暮らしの計画".into());
    let narrow = render(&mut a, 80, 30);
    let wide = render(&mut a, 200, 30);
    let sidebar_end = |buf: &Buffer| {
        (1..buf.area.width)
            .find(|&x| buf[(x, 0)].symbol() == "╭")
            .unwrap()
    };
    assert!(sidebar_end(&wide) > sidebar_end(&narrow));
    assert!(sidebar_end(&narrow) >= 24);
    assert!(sidebar_end(&wide) <= 40);
    assert!(text(&wide).replace(' ', "").contains("家計と暮らしの計画"));
    // At wide sizes, the count still has room beside the full name.
    let y = (0..wide.area.height)
        .find(|&y| {
            line(&wide, y)
                .replace(' ', "")
                .contains("家計と暮らしの計画")
        })
        .unwrap();
    assert!(
        line(&wide, y)
            .chars()
            .take(sidebar_end(&wide) as usize)
            .any(|c| c.is_ascii_digit())
    );
}

#[test]
fn focus_keys_move_the_active_style_between_panes() {
    let mut a = app();
    // Issue #2: l puts focus in Details; the first h must visibly restore Tasks.
    for (key, pane) in [
        (KeyCode::Char('l'), Pane::Details),
        (KeyCode::Char('h'), Pane::Tasks),
        (KeyCode::Char('h'), Pane::Sidebar),
        (KeyCode::Tab, Pane::Tasks),
        (KeyCode::Tab, Pane::Details),
    ] {
        a.handle_key(KeyEvent::new(key, KeyModifiers::NONE));
        assert_eq!(a.focus, pane);
        let buf = render(&mut a, 160, 30);
        let (x, y) = find_in_tasks(&buf, "Renew library books");
        assert!(!buf[(x, y)].modifier.contains(Modifier::REVERSED));
        assert_eq!(
            buf[(x, y)].bg,
            if pane == Pane::Tasks {
                Color::DarkGray
            } else {
                Color::Reset
            }
        );
        assert_eq!(buf[(x, y)].fg, Color::Red);
        let marker = (0..x)
            .find(|&col| buf[(col, y)].symbol() == "›")
            .expect("selected task marker");
        assert_eq!(
            buf[(marker, y)].modifier.contains(Modifier::DIM),
            pane != Pane::Tasks
        );
        let bar = (marker + 1..x)
            .find(|&col| buf[(col, y)].symbol() == "▌")
            .unwrap();
        assert!(matches!(buf[(bar, y)].fg, Color::Rgb(..)));
        let starts: Vec<_> = (0..buf.area.width)
            .filter(|&col| buf[(col, 0)].symbol() == "╭")
            .collect();
        let focused = match pane {
            Pane::Sidebar => 0,
            Pane::Tasks => 1,
            Pane::Details => 2,
        };
        for (i, &col) in starts.iter().enumerate() {
            assert_eq!(
                buf[(col, 0)].fg,
                if i == focused {
                    Color::Blue
                } else {
                    Color::Reset
                }
            );
        }
        assert!(line(&buf, 29).contains(pane.name()));
    }
}

#[test]
fn sidebar_active_style_matches_tasks_and_disappears_on_leaving() {
    let mut a = app();
    a.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
    // Find the cursor's row by its text rather than assuming the sidebar layout.
    let name = view_name_at_cursor(&a);
    let buf = render(&mut a, 160, 30);
    let (x, y) = find_in_sidebar(&buf, name);
    assert_eq!(buf[(x, y)].bg, Color::DarkGray);
    assert!(!buf[(x, y)].modifier.contains(Modifier::REVERSED));
    a.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    let buf = render(&mut a, 160, 30);
    assert_eq!(find_in_sidebar(&buf, name), (x, y));
    assert_eq!(buf[(x, y)].bg, Color::Reset);
}

fn view_name_at_cursor(a: &App) -> &'static str {
    match &a.sidebar_items()[a.sidebar_cursor] {
        View::Today => "Today",
        View::All => "All",
        View::Scheduled => "Scheduled",
        View::Done => "Done",
        View::Next3Days => "Next 3 Days",
        View::List(id) => panic!("cursor unexpectedly on list {id}"),
    }
}

/// Position of text inside the sidebar, which ends where the task pane's border starts.
fn find_in_sidebar(buf: &Buffer, needle: &str) -> (u16, u16) {
    let end = (1..buf.area.width)
        .find(|&x| buf[(x, 0)].symbol() == "╭")
        .unwrap();
    for y in 1..buf.area.height {
        let joined: String = (0..end).map(|x| buf[(x, y)].symbol()).collect();
        if let Some(b) = joined.find(needle) {
            return (joined[..b].chars().count() as u16, y);
        }
    }
    panic!("{needle:?} not in sidebar:\n{}", text(buf));
}

#[test]
fn focus_label_does_not_overwrite_search_or_commands() {
    let mut a = app();
    for mode in [Mode::Search, Mode::Command] {
        a.mode = mode;
        a.input = "a long synthetic prompt that must remain readable".into();
        for width in [80, 120, 160] {
            let buf = render(&mut a, width, 30);
            assert!(line(&buf, 29).contains(&a.input));
            assert!(line(&buf, 29).contains("Tasks · "));
        }
        // Badge + focus label + prompt exceed 66 columns: the label must go so the
        // prompt stays whole.
        let buf = render(&mut a, 66, 30);
        assert!(line(&buf, 29).contains(&a.input), "{}", line(&buf, 29));
        assert!(!line(&buf, 29).contains("Tasks · "));
    }
}

#[test]
fn picker_overlay_lists_names() {
    let mut a = app();
    a.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    let names: Vec<String> = a
        .picker
        .as_ref()
        .unwrap()
        .targets
        .iter()
        .filter_map(|target| match target {
            tdo::app::PickerTarget::List(id) => Some(id),
            _ => None,
        })
        .map(|id| {
            a.snapshot
                .collections
                .iter()
                .find(|c| &c.id == id)
                .and_then(|c| c.display_name.clone())
                .unwrap_or_else(|| id.clone())
        })
        .collect();
    let buf = render(&mut a, 80, 24);
    let t = text(&buf);
    assert!(t.contains(" Move to "), "{t}");
    for n in names {
        assert!(t.contains(&format!("▌ {n}")), "{n} missing:\n{t}");
    }
}

fn row_title(a: &App, row: usize) -> String {
    a.snapshot.tasks[a.visible_rows()[row].task].summary.clone()
}

#[test]
fn card_renders_in_place_below_row() {
    let mut a = app();
    let (first, second) = (row_title(&a, 0), row_title(&a, 1));
    let (_, below_before) = find_in_tasks(&render(&mut a, 120, 30), &second);
    a.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    let buf = render(&mut a, 120, 30);
    let t = text(&buf);
    assert!(t.contains("Details"), "Details still drawn:\n{t}");
    assert!(!t.contains(" Edit task "), "no overlay:\n{t}");
    let (_, title) = find_in_tasks(&buf, &format!("{first}▏"));
    let mut ys = vec![title];
    for label in ["Due", "Start", "Repeat", "Notes", "Ctrl-s save"] {
        ys.push(find_in_tasks(&buf, label).1);
    }
    let (_, below) = find_in_tasks(&buf, &second);
    ys.push(below);
    assert!(ys.windows(2).all(|w| w[0] < w[1]), "{ys:?}\n{t}");
    assert!(below >= below_before + 6, "rows below shift down:\n{t}");
    assert!(find_in_tasks(&buf, "→ ").1 > title, "preview:\n{t}");
}

#[test]
fn card_falls_back_to_overlay_under_60_cols() {
    let mut a = app();
    a.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    let t = text(&render(&mut a, 59, 30));
    assert!(t.contains(" Edit task "), "{t}");
    for label in ["Due", "Start", "Repeat", "Notes", "Ctrl-s save"] {
        assert!(t.contains(label), "{label} missing:\n{t}");
    }
    // A pane too short for the card falls back too; tiny terminals never panic.
    let t = text(&render(&mut a, 120, 8));
    assert!(t.contains(" Edit task "), "{t}");
    for (w, h) in [(20, 5), (1, 1), (0, 0), (3, 40), (200, 2), (60, 8)] {
        render(&mut a, w, h);
    }
}

#[test]
fn status_shows_insert_mode() {
    let mut a = app();
    a.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    let buf = render(&mut a, 120, 30);
    let last = line(&buf, 29);
    assert!(last.contains(" INSERT "), "{last}");
    assert!(last.contains("Esc cancel"), "{last}");
}

/// With more lists than fit, the picker scrolls so the selected destination stays visible.
#[test]
fn picker_scrolls_to_keep_selection_visible() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..30 {
        let list = dir.path().join(format!("list-{i:02}"));
        fs::create_dir_all(&list).unwrap();
        fs::write(list.join("displayname"), format!("list-{i:02}")).unwrap();
    }
    let task = dir.path().join("list-00/t.ics");
    fs::write(
        &task,
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\nBEGIN:VTODO\r\nUID:t1\r\n\
         SUMMARY:Synthetic\r\nDUE;VALUE=DATE:20261104\r\nEND:VTODO\r\nEND:VCALENDAR\r\n",
    )
    .unwrap();
    let mut a = App::new(snapshot_at(dir.path()), now());
    a.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    assert_eq!(a.picker.as_ref().unwrap().targets.len(), 29);
    for _ in 0..40 {
        a.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    }
    // The picker's first row is the line under its " Move to " title.
    let first_row = |buf: &Buffer| {
        let y = (0..buf.area.height)
            .find(|y| line(buf, *y).contains(" Move to "))
            .unwrap();
        let l = line(buf, y + 1);
        let x = l.find("│▌").map(|i| i + "│".len()).unwrap_or(0);
        l[x..].split('│').next().unwrap().trim().to_string()
    };
    let buf = render(&mut a, 80, 24);
    assert!(
        text(&buf).contains("▌ list-29"),
        "selected list not visible"
    );
    assert_ne!(
        first_row(&buf),
        "▌ list-01",
        "top should have scrolled away"
    );
    for _ in 0..40 {
        a.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
    }
    assert_eq!(first_row(&render(&mut a, 80, 24)), "▌ list-01");
}

fn todo(uid: &str, summary: &str, extra: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\n\
         UID:{uid}\r\nSUMMARY:{summary}\r\n{extra}END:VTODO\r\nEND:VCALENDAR\r\n"
    )
}

/// A project with two open children (one repeating) and one completed child.
fn project_vdir() -> tempfile::TempDir {
    let parent = todo(
        "p",
        "Synthetic project",
        "X-2DOAPP-METADATA:@tasktype:project\r\nSTATUS:NEEDS-ACTION\r\n",
    );
    let later = todo(
        "c1",
        "Later child",
        "RELATED-TO:p\r\nDUE;VALUE=DATE:20261120\r\nRRULE:FREQ=MONTHLY\r\nSTATUS:NEEDS-ACTION\r\n",
    );
    let sooner = todo(
        "c2",
        "Sooner child",
        "RELATED-TO:p\r\nDUE;VALUE=DATE:20261105\r\nSTATUS:NEEDS-ACTION\r\n",
    );
    let done = todo(
        "c3",
        "Finished child",
        "RELATED-TO:p\r\nSTATUS:COMPLETED\r\nCOMPLETED:20261101T120000Z\r\n",
    );
    vdir_with(&[
        ("p.ics", &parent),
        ("c1.ics", &later),
        ("c2.ics", &sooner),
        ("c3.ics", &done),
    ])
}

fn details_lines(buf: &Buffer) -> Vec<String> {
    let start = (1..buf.area.width)
        .rev()
        .find(|&x| buf[(x, 0)].symbol() == "╭")
        .unwrap();
    (0..buf.area.height)
        .map(|y| {
            (start..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn details_and_task_list_share_captured_checklist_and_project_item_order() {
    for (scenario, title) in [
        ("issue35-checklist-order", "Order 35"),
        ("issue35-project-order", "Project Order 35"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let list = dir.path().join("test");
        fs::create_dir(&list).unwrap();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/phase0/v1")
            .join(scenario);
        for step in ["000-created", "010-appended"] {
            for entry in fs::read_dir(root.join(step)).unwrap() {
                let p = entry.unwrap().path();
                if p.extension().is_some_and(|ext| ext == "ics") {
                    fs::copy(&p, list.join(p.file_name().unwrap())).unwrap();
                }
            }
        }
        let mut a = App::new(snapshot_at(dir.path()), now());
        let parent = select(&mut a, title);
        a.expanded.insert(parent);
        let buf = render(&mut a, 160, 30);
        let details = details_lines(&buf);
        let mut previous = 0;
        for title in ["Zulu", "Alpha", "Mike", "Bravo"] {
            let row = details
                .iter()
                .position(|line| line.contains(title))
                .unwrap();
            assert!(row > previous, "{scenario}: {}", details.join("\n"));
            previous = row;
        }
        let list_order: Vec<_> = ["Zulu", "Alpha", "Mike", "Bravo"]
            .into_iter()
            .map(|title| find_in_tasks(&buf, title).1)
            .collect();
        assert!(list_order.windows(2).all(|pair| pair[0] < pair[1]));
    }
}

#[test]
fn details_list_a_projects_open_tasks_and_completed_count() {
    let dir = project_vdir();
    let mut a = App::new(snapshot_at(dir.path()), now());
    select(&mut a, "Synthetic project");
    let lines = details_lines(&render(&mut a, 140, 30));
    let at = |needle: &str| {
        lines
            .iter()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} not in details:\n{}", lines.join("\n")))
    };
    let heading = at("Tasks (2 open)");
    let sooner = at("○ Sooner child");
    let later = at("↺ Later child");
    // These legacy items have no position or creation time, so UID breaks the tie.
    assert!(heading < later && later < sooner, "{}", lines.join("\n"));
    assert!(lines[sooner].contains("Tomorrow"), "{}", lines[sooner]);
    assert!(at("+ 1 completed") > sooner);
    assert!(!lines.iter().any(|l| l.contains("Finished child")));
    at("Enter: expand in list");
}

#[test]
fn details_of_an_expanded_project_offer_collapse() {
    let dir = project_vdir();
    let mut a = App::new(snapshot_at(dir.path()), now());
    let uid = select(&mut a, "Synthetic project");
    a.expanded.insert(uid);
    let t = details_lines(&render(&mut a, 140, 30)).join("\n");
    assert!(t.contains("Enter: collapse in list"), "{t}");
}

#[test]
fn details_of_a_leaf_task_have_no_task_list() {
    let mut a = app();
    select(&mut a, "Pay water bill");
    let t = details_lines(&render(&mut a, 140, 30)).join("\n");
    assert!(!t.contains("Tasks ("), "{t}");
    assert!(!t.contains("Enter:"), "{t}");
}

#[test]
fn details_in_a_flat_view_list_tasks_without_the_enter_hint() {
    let parent = todo(
        "p",
        "Synthetic project",
        "X-2DOAPP-METADATA:@tasktype:project\r\nDUE;VALUE=DATE:20261104\r\nSTATUS:NEEDS-ACTION\r\n",
    );
    let child = todo(
        "c",
        "Undated child",
        "RELATED-TO:p\r\nSTATUS:NEEDS-ACTION\r\n",
    );
    let dir = vdir_with(&[("p.ics", &parent), ("c.ics", &child)]);
    let mut a = App::new(snapshot_at(dir.path()), now());
    assert_eq!(a.view, View::Today);
    let t = details_lines(&render(&mut a, 140, 30)).join("\n");
    assert!(t.contains("Synthetic project"), "{t}");
    assert!(t.contains("Undated child"), "{t}");
    assert!(!t.contains("Enter:"), "{t}");
}

/// Two tasks due today; the first ("Long notes task") has 40 lines of notes.
fn long_notes_app() -> (tempfile::TempDir, App) {
    let notes = (1..=40)
        .map(|i| format!("Note line {i:02}"))
        .collect::<Vec<_>>()
        .join("\\n");
    let long = todo(
        "n",
        "Long notes task",
        &format!("DUE;VALUE=DATE:20261104\r\nDESCRIPTION:{notes}\r\nSTATUS:NEEDS-ACTION\r\n"),
    );
    let other = todo(
        "s",
        "Second task",
        "DUE;VALUE=DATE:20261104\r\nSTATUS:NEEDS-ACTION\r\n",
    );
    let dir = vdir_with(&[("n.ics", &long), ("s.ics", &other)]);
    let mut a = App::new(snapshot_at(dir.path()), now());
    render(&mut a, 140, 20);
    a.focus = Pane::Details;
    (dir, a)
}

fn press(a: &mut App, c: char) {
    a.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
}

fn details_text(a: &mut App) -> String {
    details_lines(&render(a, 140, 20)).join("\n")
}

#[test]
fn jk_in_details_scroll_the_pane_not_the_task_list() {
    let (_dir, mut a) = long_notes_app();
    assert!(details_text(&mut a).contains("Long notes task"));
    for _ in 0..5 {
        press(&mut a, 'j');
    }
    let t = details_text(&mut a);
    assert!(!t.contains("Long notes task"), "{t}");
    assert!(t.contains("Note line 05"), "{t}");
    assert_eq!(a.cursor, 0, "the selected task stays");
    for _ in 0..5 {
        press(&mut a, 'k');
    }
    assert!(details_text(&mut a).contains("Long notes task"));
}

#[test]
fn details_scroll_stops_at_the_last_line() {
    let (_dir, mut a) = long_notes_app();
    press(&mut a, 'G');
    let t = details_text(&mut a);
    assert!(t.contains("Note line 40"), "{t}");
    press(&mut a, 'k');
    let t = details_text(&mut a);
    assert!(
        !t.contains("Note line 40"),
        "one line up from the bottom:\n{t}"
    );
    press(&mut a, 'g');
    press(&mut a, 'g');
    assert!(details_text(&mut a).contains("Long notes task"));
}

#[test]
fn details_page_keys_scroll_by_half_a_page() {
    let (_dir, mut a) = long_notes_app();
    a.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
    let t = details_text(&mut a);
    assert!(!t.contains("Long notes task"), "{t}");
    assert_eq!(a.cursor, 0);
    a.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert!(details_text(&mut a).contains("Long notes task"));
}

#[test]
fn another_task_opens_details_at_the_top() {
    let (_dir, mut a) = long_notes_app();
    for _ in 0..5 {
        press(&mut a, 'j');
    }
    press(&mut a, 'h');
    press(&mut a, 'j');
    press(&mut a, 'k');
    assert!(details_text(&mut a).contains("Long notes task"));
}

#[test]
fn new_row_draws_under_cursor() {
    let k = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
    let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    let mut a = app();
    let (first, second) = (row_title(&a, 0), row_title(&a, 1));
    let buf = render(&mut a, 120, 30);
    let ((title_x, y_first), (_, y_second)) =
        (find_in_tasks(&buf, &first), find_in_tasks(&buf, &second));
    let selected_bg = buf[(title_x, y_first)].style().bg;
    a.handle_key(k('o'));
    for c in "Plan the garden".chars() {
        a.handle_key(k(c));
    }
    let buf = render(&mut a, 120, 30);
    let t = text(&buf);
    let (x, y) = find_in_tasks(&buf, "Plan the garden▏");
    assert_eq!(y, y_first + 1, "below the cursor row:\n{t}");
    assert_eq!(find_in_tasks(&buf, &first).1, y_first, "{t}");
    assert_eq!(
        find_in_tasks(&buf, &second).1,
        y_second + 1,
        "rows below shift:\n{t}"
    );
    assert_eq!(x, title_x, "the title lines up with the rows' titles:\n{t}");
    // The row is the selection: highlighted, with the marker; the cursor row is not.
    assert_eq!(buf[(x, y)].style().bg, selected_bg, "{t}");
    assert_ne!(buf[(title_x, y_first)].style().bg, selected_bg, "{t}");
    assert!(line(&buf, y).contains('›'), "{t}");
    assert!(!line(&buf, y_first).contains('›'), "{t}");
    assert!(t.contains("Details"), "{t}");

    // `O`: above the cursor row.
    a.handle_key(esc);
    a.handle_key(KeyEvent::new(KeyCode::Char('O'), KeyModifiers::SHIFT));
    a.handle_key(k('z'));
    let buf = render(&mut a, 120, 30);
    let t = text(&buf);
    assert_eq!(find_in_tasks(&buf, "z▏").1, y_first, "{t}");
    assert_eq!(find_in_tasks(&buf, &first).1, y_first + 1, "{t}");

    // An empty view: the row is drawn at the top instead of "No tasks".
    a.handle_key(esc);
    a.view = View::Done;
    a.search = Some("nothing matches this".into());
    a.handle_key(k('o'));
    let t = text(&render(&mut a, 120, 30));
    assert!(t.contains("▏"), "{t}");
    assert!(!t.contains("No tasks"), "{t}");
}

#[test]
fn all_shows_future_and_done_sections_while_today_stays_focused() {
    let mut a = app();
    a.view = View::All;
    let buf = render(&mut a, 120, 60);
    let (_, future_y) = find_in_tasks(&buf, "FUTURE");
    let (_, done_y) = find_in_tasks(&buf, "DONE");
    let (_, stamps_y) = find_in_tasks(&buf, "Buy stamps");
    assert!(done_y > future_y && stamps_y > done_y);
    a.view = View::Today;
    let buf = render(&mut a, 120, 60);
    let t = text(&buf);
    assert!(!t.contains("Buy stamps"));
    assert!(!t.contains("FUTURE"));
}

#[test]
fn detached_children_show_parent_context_in_all_and_ordinary_lists() {
    let base = app().snapshot;
    let mut tasks = base.tasks.clone();
    let p = tasks
        .iter_mut()
        .find(|t| t.summary == "Prepare tax return")
        .unwrap();
    p.summary = "Parent".into();
    let uid = p.uid.clone();
    let mut children = tasks
        .iter_mut()
        .filter(|t| t.parent.as_deref() == Some(&uid));
    let done = children.next().unwrap();
    done.summary = "Done child".into();
    done.status = twydo_core::model::Status::Completed;
    done.completed = Some(twydo_core::ical::value::IcalDateTime::Utc(now().to_utc()));
    let future = children.next().unwrap();
    future.summary = "Future child".into();
    future.start = Some(twydo_core::ical::value::IcalDateTime::Date(
        chrono::NaiveDate::from_ymd_opt(2026, 12, 1).unwrap(),
    ));
    let snapshot = Snapshot::new(
        twydo_core::vdir::LoadReport {
            tasks,
            collections: base.collections.clone(),
            ..Default::default()
        },
        base.config.clone(),
    );
    for view in [View::All, View::List("bills".into())] {
        let mut a = App::new(snapshot.clone(), now());
        a.view = view;
        let buf = render(&mut a, 180, 60);
        for title in ["Done child", "Future child"] {
            let (_, y) = find_in_tasks(&buf, title);
            assert!(line(&buf, y).contains("↳ Parent"), "{}", line(&buf, y));
        }
    }
}

#[test]
fn syncing_replaces_last_sync_age_and_preserves_diagnostics() {
    use std::time::{Duration, Instant};
    let mut a = app();
    let mut s = summary(Default::default(), 2, vec![conflict_on("nope")]);
    s.off = true;
    s.status.last_error = Some("connection refused".into());
    a.set_sync(Some(s));
    let t = Instant::now();
    a.sync_activity.observe(true, t);
    a.message = Some("saved".into());
    let first = status_line(&render(&mut a, 220, 20));
    assert!(first.contains("⠋ Syncing…"), "{first}");
    assert!(first.contains("sync off"));
    assert!(first.contains("connection refused"));
    assert!(first.contains("2 pending · 1 conflict"));
    a.sync_activity
        .observe(true, t + Duration::from_millis(250));
    let second = status_line(&render(&mut a, 220, 20));
    assert!(second.contains("⠙ Syncing…"), "{second}");

    a.sync_activity.observe(false, t + Duration::from_secs(1));
    let failed = status_line(&render(&mut a, 220, 20));
    assert!(!failed.contains("Syncing") && !failed.contains("Sync complete"));
    a.set_sync(Some(summary(
        twydo_core::sync::status::SyncStatus {
            last_success: Some(utc(15, 0)),
            ..Default::default()
        },
        0,
        vec![],
    )));
    a.sync_activity.observe(true, t + Duration::from_secs(2));
    let running = status_line(&render(&mut a, 140, 20));
    assert!(running.contains("Syncing…"), "{running}");
    assert!(!running.contains("synced"), "{running}");
    assert!(!running.contains("Sync complete"), "{running}");
    a.sync_activity.observe(false, t + Duration::from_secs(3));
    let finished = status_line(&render(&mut a, 140, 20));
    assert!(finished.contains("synced just now"), "{finished}");
    assert!(
        !finished.contains("Syncing") && !finished.contains("Sync complete"),
        "{finished}"
    );
    a.tick(now() + chrono::Duration::minutes(1));
    assert!(status_line(&render(&mut a, 140, 20)).contains("synced 1m ago"));
    assert_eq!(a.message.as_deref(), Some("saved"));
}
