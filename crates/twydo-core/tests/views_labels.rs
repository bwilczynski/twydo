//! Inbox resolution and date-group labels shared by the TUI and the daemon's views.

use std::path::PathBuf;

use chrono::NaiveDate;
use twydo_core::config::Config;
use twydo_core::vdir::{Collection, LoadReport};
use twydo_core::views::{DateGroup, Snapshot, default_list, group_label, inbox_list};

fn snap(lists: &[(&str, Option<&str>)], default: Option<&str>) -> Snapshot {
    let collections = lists
        .iter()
        .map(|(id, name)| Collection {
            id: id.to_string(),
            path: PathBuf::from(id),
            display_name: name.map(str::to_string),
            color: None,
        })
        .collect();
    let mut cfg = Config::with_vdir(PathBuf::from("unused"));
    cfg.default_list = default.map(str::to_string);
    Snapshot::new(
        LoadReport {
            collections,
            ..Default::default()
        },
        cfg,
    )
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

#[test]
fn group_labels_match_tui_wording() {
    let today = d(2026, 11, 17);
    let l = |g| group_label(g, today);
    assert_eq!(l(DateGroup::Overdue).as_deref(), Some("OVERDUE"));
    assert_eq!(l(DateGroup::InProgress).as_deref(), Some("IN PROGRESS"));
    assert_eq!(l(DateGroup::LaterToday).as_deref(), Some("LATER TODAY"));
    assert_eq!(l(DateGroup::Today).as_deref(), Some("TODAY"));
    assert_eq!(l(DateGroup::Tomorrow).as_deref(), Some("TOMORROW"));
    assert_eq!(
        l(DateGroup::DayAfterTomorrow).as_deref(),
        Some("DAY AFTER TOMORROW")
    );
    assert_eq!(l(DateGroup::Next7Days).as_deref(), Some("NEXT 7 DAYS"));
    assert_eq!(l(DateGroup::Next2Weeks).as_deref(), Some("NEXT 2 WEEKS"));
    assert_eq!(l(DateGroup::Next30Days).as_deref(), Some("NEXT 30 DAYS"));
    assert_eq!(l(DateGroup::Future).as_deref(), Some("FUTURE"));
    assert_eq!(l(DateGroup::Done).as_deref(), Some("DONE"));
    assert_eq!(l(DateGroup::NoDueDate).as_deref(), Some("NO DUE DATE"));
    assert_eq!(l(DateGroup::Scheduled).as_deref(), Some("SCHEDULED"));
    assert_eq!(l(DateGroup::CompletedOn(today)).as_deref(), Some("TODAY"));
    assert_eq!(
        l(DateGroup::CompletedOn(d(2026, 11, 16))).as_deref(),
        Some("YESTERDAY")
    );
    assert_eq!(
        l(DateGroup::CompletedOn(d(2026, 11, 10))).as_deref(),
        Some("Tue 10 Nov")
    );
    assert_eq!(l(DateGroup::Ungrouped), None);
}

#[test]
fn inbox_prefers_configured_default_list() {
    let s = snap(&[("inbox", None), ("personal", None)], Some("personal"));
    assert_eq!(inbox_list(&s).as_deref(), Some("personal"));
}

#[test]
fn inbox_falls_back_to_list_named_inbox_case_insensitive() {
    let s = snap(&[("a", None), ("b", Some("InBox"))], Some("gone"));
    assert_eq!(inbox_list(&s).as_deref(), Some("b"));
    assert_eq!(inbox_list(&snap(&[("a", None)], None)), None);
}

#[test]
fn default_list_falls_back_to_first_list() {
    let s = snap(&[("bills", None), ("home", None)], None);
    assert_eq!(default_list(&s), "bills");
}

#[test]
fn default_list_empty_without_lists() {
    assert_eq!(default_list(&snap(&[], None)), "");
}
