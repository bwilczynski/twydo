//! The sidebar order and view names shared by the TUI and the daemon.

use std::path::PathBuf;

use twydo_core::config::{Config, Group, ListPrefs};
use twydo_core::vdir::{Collection, LoadReport};
use twydo_core::views::{SidebarEntry, Snapshot, View, sidebar, view_name};

fn snap(hide_inbox: bool) -> Snapshot {
    let collections = [("inbox", Some("Inbox")), ("work", None), ("home", None)]
        .iter()
        .map(|(id, name)| Collection {
            id: id.to_string(),
            path: PathBuf::from(id),
            display_name: name.map(str::to_string),
            color: None,
        })
        .collect();
    let mut cfg = Config::with_vdir(PathBuf::from("unused"));
    cfg.groups = vec![Group {
        name: "G".into(),
        lists: vec!["home".into()],
    }];
    if hide_inbox {
        cfg.lists.insert(
            "inbox".into(),
            ListPrefs {
                hidden: true,
                ..ListPrefs::default()
            },
        );
    }
    Snapshot::new(
        LoadReport {
            collections,
            ..Default::default()
        },
        cfg,
    )
}

fn list(id: &str) -> SidebarEntry {
    SidebarEntry::Item(View::List(id.into()))
}

#[test]
fn sidebar_order() {
    let item = SidebarEntry::Item;
    let head = |s: &str| SidebarEntry::Heading(s.into());
    assert_eq!(
        sidebar(&snap(false)),
        vec![
            list("inbox"),
            item(View::Today),
            item(View::All),
            item(View::Scheduled),
            item(View::Done),
            head("G"),
            list("home"),
            head("Lists"),
            list("work"),
            head("Smart lists"),
            item(View::Next3Days),
        ]
    );
}

#[test]
fn hidden_inbox_left_out() {
    let entries = sidebar(&snap(true));
    assert_eq!(entries[0], SidebarEntry::Item(View::Today));
    assert!(!entries.contains(&list("inbox")));
}

#[test]
fn view_names() {
    let s = snap(false);
    let n = |v| view_name(&s, &v);
    assert_eq!(n(View::Today), "Today");
    assert_eq!(n(View::All), "All");
    assert_eq!(n(View::Scheduled), "Scheduled");
    assert_eq!(n(View::Done), "Done");
    assert_eq!(n(View::Next3Days), "Next 3 Days");
    assert_eq!(n(View::List("work".into())), "work");
    assert_eq!(n(View::List("inbox".into())), "Inbox");
}
