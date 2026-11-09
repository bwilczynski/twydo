//! Shared fixtures: a vdir of synthetic tasks in a tempdir.
#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;

use twydo_core::config::{
    Config, Group, ListPrefs, Sort, SortField, SyncConfig, WriteMode, Writes,
};
use twydo_core::vdir;
use twydo_core::views::Snapshot;

pub struct Fixture {
    tmp: tempfile::TempDir,
}

/// A synthetic VTODO resource.
pub fn todo_ics(uid: &str, summary: &str, parent: Option<&str>) -> String {
    let related = parent
        .map(|p| format!("RELATED-TO;RELTYPE=PARENT:{p}\r\n"))
        .unwrap_or_default();
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\nUID:{uid}\r\n\
         SUMMARY:{summary}\r\n{related}END:VTODO\r\nEND:VCALENDAR\r\n"
    )
}

impl Fixture {
    /// Tasks as `(collection, uid, summary, parent uid)`.
    pub fn new(tasks: &[(&str, &str, &str, Option<&str>)]) -> Fixture {
        let fx = Fixture {
            tmp: tempfile::tempdir().unwrap(),
        };
        fs::create_dir_all(fx.vdir()).unwrap();
        for (collection, uid, summary, parent) in tasks {
            fx.write(
                &format!("{collection}/{uid}.ics"),
                &todo_ics(uid, summary, *parent),
            );
        }
        fx
    }

    pub fn vdir(&self) -> PathBuf {
        self.tmp.path().join("vdir")
    }

    pub fn config(&self) -> Config {
        let mut c = Config::with_vdir(self.vdir());
        c.state_dir = Some(self.tmp.path().join("state"));
        c.writes.mode = WriteMode::Test;
        c
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot::new(vdir::load(&self.vdir()).unwrap(), self.config())
    }

    /// Write `text` at `rel` (relative to the vdir), as a change from outside.
    pub fn write(&self, rel: &str, text: &str) {
        let p = self.vdir().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    pub fn remove(&self, rel: &str) {
        fs::remove_file(self.vdir().join(rel)).unwrap();
    }
}

/// A configuration with every table set (synthetic values).
pub fn full_config() -> Config {
    let mut c = Config::with_vdir(PathBuf::from("/synthetic/vdir"));
    c.timezone = Some("Europe/Berlin".into());
    c.week_start = chrono::Weekday::Sun;
    c.work_days = vec![chrono::Weekday::Mon, chrono::Weekday::Wed];
    c.morning_time = chrono::NaiveTime::from_hms_opt(7, 30, 0).unwrap();
    c.default_list = Some("inbox".into());
    c.lists.insert(
        "inbox".into(),
        ListPrefs {
            archived: true,
            hidden: true,
            show_in_all: false,
            show_in_today: false,
            show_in_done: false,
            sort: Sort {
                field: SortField::Title,
                descending: true,
            },
        },
    );
    c.groups = vec![Group {
        name: "Synthetic group".into(),
        lists: vec!["inbox".into()],
    }];
    c.writes = Writes {
        mode: WriteMode::Test,
    };
    c.state_dir = Some(PathBuf::from("/synthetic/state"));
    c.sync = Some(SyncConfig {
        vdirsyncer_config: Some(PathBuf::from("/synthetic/vdirsyncer.conf")),
        after_edit: Some(true),
        ..SyncConfig::default()
    });
    c
}
