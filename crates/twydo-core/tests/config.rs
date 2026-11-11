use std::path::{Path, PathBuf};

use chrono::{NaiveTime, Weekday};
use twydo_core::config::{Config, SortField, SyncConfig, WriteMode, expand_tilde, state_dir_from};

fn parse(s: &str) -> Result<Config, twydo_core::config::ConfigError> {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("config.toml");
    std::fs::write(&p, s).unwrap();
    Config::load(&p)
}

#[test]
fn config_defaults_and_unknown_key() {
    let c = parse("vdir = \"/x\"").unwrap();
    assert_eq!(c.vdir, PathBuf::from("/x"));
    assert_eq!(c.timezone, None);
    assert_eq!(c.week_start, Weekday::Mon);
    assert_eq!(c.morning_time, NaiveTime::from_hms_opt(9, 0, 0).unwrap());
    assert_eq!(c.default_list, None);
    assert!(c.lists.is_empty() && c.groups.is_empty());
    let p = c.list_prefs("anything");
    assert!(p.show_in_all && p.show_in_today && p.show_in_done);
    assert!(!p.archived && !p.hidden);
    assert_eq!(p.sort.field, SortField::Due);
    assert!(!p.sort.descending);

    let e = parse("vdri = \"/x\"").unwrap_err().to_string();
    assert!(e.contains("vdri"), "{e}");
}

#[test]
fn config_full_example() {
    let c = parse(
        r#"
vdir = "/x"
timezone = "America/New_York"
week_start = "Sunday"
morning_time = "07:30"
default_list = "inbox"

[lists.bills]
hidden = true
show_in_today = false
sort = { field = "title", descending = true }

[[groups]]
name = "Home"
lists = ["inbox", "personal"]
"#,
    )
    .unwrap();
    assert_eq!(c.week_start, Weekday::Sun);
    assert_eq!(c.morning_time, NaiveTime::from_hms_opt(7, 30, 0).unwrap());
    assert_eq!(c.tz(), chrono_tz::America::New_York);
    let b = c.list_prefs("bills");
    assert!(b.hidden && !b.show_in_today && b.show_in_all);
    assert_eq!(b.sort.field, SortField::Title);
    assert!(b.sort.descending);
    assert_eq!(c.groups[0].lists, ["inbox", "personal"]);
}

#[test]
fn config_rejects_bad_values() {
    assert!(parse("vdir = \"/x\"\nweek_start = \"funday\"").is_err());
    assert!(parse("vdir = \"/x\"\nmorning_time = \"25:00\"").is_err());
    assert!(parse("vdir = \"/x\"\ntimezone = \"Mars/Base\"").is_err());
    assert!(parse("vdir = \"/x\"\n[lists.a]\nsort = { field = \"x\" }").is_err());
    assert!(parse("").is_err());
}

#[test]
fn config_path_honours_xdg() {
    let home = Path::new("/home/u");
    assert_eq!(
        Config::path_from(Some(Path::new("/tmp/c")), home),
        PathBuf::from("/tmp/c/twydo/config.toml")
    );
    assert_eq!(
        Config::path_from(None, home),
        PathBuf::from("/home/u/.config/twydo/config.toml")
    );
    assert_eq!(
        Config::path_from(Some(Path::new("")), home),
        PathBuf::from("/home/u/.config/twydo/config.toml")
    );
}

#[test]
fn config_expands_tilde() {
    let home = Path::new("/home/u");
    assert_eq!(
        expand_tilde(Path::new("~/v/t"), home),
        PathBuf::from("/home/u/v/t")
    );
    assert_eq!(expand_tilde(Path::new("~"), home), PathBuf::from("/home/u"));
    assert_eq!(
        expand_tilde(Path::new("/abs/~/x"), home),
        PathBuf::from("/abs/~/x")
    );
    assert_eq!(
        expand_tilde(Path::new("~bob/x"), home),
        PathBuf::from("~bob/x")
    );
}

#[test]
fn writes_mode_defaults_to_verified() {
    let c = parse("vdir = \"/x\"").unwrap();
    assert_eq!(c.writes.mode, WriteMode::Verified);
    assert_eq!(c.state_dir, None);
}

#[test]
fn writes_mode_test_parses() {
    let c = parse("vdir = \"/x\"\n[writes]\nmode = \"test\"").unwrap();
    assert_eq!(c.writes.mode, WriteMode::Test);
}

#[test]
fn writes_mode_rejects_unknown() {
    assert!(parse("vdir = \"/x\"\n[writes]\nmode = \"yolo\"").is_err());
    assert!(parse("vdir = \"/x\"\n[writes]\nbogus = 1").is_err());
}

#[test]
fn state_dir_from_xdg_and_home() {
    let h = Path::new("/h");
    assert_eq!(
        state_dir_from(Some(Path::new("/x")), h),
        PathBuf::from("/x/twydo")
    );
    let d = PathBuf::from("/h/.local/state/twydo");
    assert_eq!(state_dir_from(None, h), d);
    assert_eq!(state_dir_from(Some(Path::new("")), h), d);
    let mut c = Config::with_vdir("/v".into());
    c.state_dir = Some("/s".into());
    assert_eq!(c.state_dir(), PathBuf::from("/s"));
}

#[test]
fn sync_absent_means_off() {
    let c = parse("vdir = \"/x\"").unwrap();
    assert_eq!(c.sync, None);
}

#[test]
fn sync_defaults() {
    let c = parse("vdir = \"/x\"\n[sync]\n").unwrap();
    assert_eq!(
        c.sync,
        Some(SyncConfig {
            pair: "twydo".into(),
            local_storage: "twydo_local".into(),
            command: "vdirsyncer".into(),
            list_python: "python3".into(),
            vdirsyncer_config: None,
            metasync: true,
            batch_size: 5,
            max_wait_secs: 60,
            after_edit: None,
            timeout_secs: 300,
            max_backoff_secs: 1800,
            retention_days: 30,
        })
    );
}

#[test]
fn sync_vdirsyncer_config_expands_tilde() {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty());
    let Some(home) = home else {
        eprintln!("skipping: HOME not set");
        return;
    };
    let c = parse("vdir = \"/x\"\n[sync]\nvdirsyncer_config = \"~/vs/config\"\n").unwrap();
    assert_eq!(
        c.sync.unwrap().vdirsyncer_config,
        Some(Path::new(&home).join("vs/config"))
    );
    let c = parse("vdir = \"/x\"\n[sync]\nvdirsyncer_config = \"/abs/config\"\n").unwrap();
    assert_eq!(
        c.sync.unwrap().vdirsyncer_config,
        Some(PathBuf::from("/abs/config"))
    );
}

#[test]
fn sync_unknown_key_is_error() {
    let e = parse("vdir = \"/x\"\n[sync]\nbogus_key = 1\n")
        .unwrap_err()
        .to_string();
    assert!(e.contains("bogus_key"), "{e}");
}

#[test]
fn work_days_default_and_custom() {
    use Weekday::*;
    let c = parse("vdir = \"/x\"").unwrap();
    assert_eq!(c.work_days, vec![Mon, Tue, Wed, Thu, Fri]);
    let c = parse("vdir = \"/x\"\nwork_days = [\"sun\", \"Monday\", \"tue\"]").unwrap();
    assert_eq!(c.work_days, vec![Sun, Mon, Tue]);
    assert!(parse("vdir = \"/x\"\nwork_days = []").is_err());
    assert!(parse("vdir = \"/x\"\nwork_days = [\"funday\"]").is_err());
}

#[test]
fn sync_defaults_batch_and_wait() {
    let s = parse("vdir = \"/x\"\n[sync]\n").unwrap().sync.unwrap();
    assert_eq!((s.batch_size, s.max_wait_secs), (5, 60));
}

#[test]
fn zero_batch_size_is_refused() {
    let e = parse("vdir = \"/x\"\n[sync]\nbatch_size = 0\n").unwrap_err();
    assert!(e.to_string().contains("batch_size"), "{e}");
}

#[test]
fn zero_max_wait_is_refused() {
    let e = parse("vdir = \"/x\"\n[sync]\nmax_wait_secs = 0\n").unwrap_err();
    assert!(e.to_string().contains("max_wait_secs"), "{e}");
}

#[test]
fn after_edit_false_still_loads_with_a_warning() {
    let c = parse("vdir = \"/x\"\n[sync]\nafter_edit = false\n").unwrap();
    assert_eq!(
        c.warnings(),
        vec![
            "[sync] after_edit is no longer used: edits sync in batches (batch_size, max_wait_secs); use `tdo sync off` to keep syncs manual"
        ]
    );
    assert!(
        parse("vdir = \"/x\"\n[sync]\n")
            .unwrap()
            .warnings()
            .is_empty()
    );
}

#[test]
fn the_daemon_table_defaults_to_warning_and_reads_the_switch() {
    assert!(parse("vdir = \"/x\"").unwrap().daemon.unit_warning);
    let c = parse("vdir = \"/x\"\n[daemon]\nunit_warning = false\n").unwrap();
    assert!(!c.daemon.unit_warning);
    let e = parse("vdir = \"/x\"\n[daemon]\nfoo = 1\n")
        .unwrap_err()
        .to_string();
    assert!(e.contains("foo"), "{e}");
}
