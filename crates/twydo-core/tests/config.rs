use std::path::{Path, PathBuf};

use chrono::{NaiveTime, Weekday};
use twydo_core::config::{Config, SortField, SyncConfig, WriteMode, expand_tilde, state_dir_from};

/// A `[sync]` table with the three required keys plus `extra`.
fn sync_toml(extra: &str) -> String {
    format!("vdir = \"/x\"\n[sync]\n{CONN}{extra}")
}

const CONN: &str =
    "url = \"https://dav.example.test/a/\"\nusername = \"alice\"\npassword_command = [\"true\"]\n";

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
    let c = parse(&sync_toml("")).unwrap();
    assert_eq!(
        c.sync,
        Some(SyncConfig {
            url: "https://dav.example.test/a/".into(),
            username: "alice".into(),
            password_command: vec!["true".into()],
            ca_file: None,
            command: "vdirsyncer".into(),
            list_python: None,
            metasync: true,
            batch_size: 5,
            max_wait_secs: 60,
            timeout_secs: 300,
            max_backoff_secs: 1800,
            retention_days: 30,
        })
    );
}

#[test]
fn sync_unknown_key_is_error() {
    let e = parse(&sync_toml("bogus_key = 1\n"))
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
    let s = parse(&sync_toml("")).unwrap().sync.unwrap();
    assert_eq!((s.batch_size, s.max_wait_secs), (5, 60));
}

#[test]
fn zero_batch_size_is_refused() {
    let e = parse(&sync_toml("batch_size = 0\n")).unwrap_err();
    assert!(e.to_string().contains("batch_size"), "{e}");
}

#[test]
fn zero_max_wait_is_refused() {
    let e = parse(&sync_toml("max_wait_secs = 0\n")).unwrap_err();
    assert!(e.to_string().contains("max_wait_secs"), "{e}");
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

#[test]
fn sync_ca_file_expands_tilde() {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty());
    let Some(home) = home else {
        eprintln!("skipping: HOME not set");
        return;
    };
    let c = parse(&sync_toml("ca_file = \"~/ca.pem\"\n")).unwrap();
    assert_eq!(
        c.sync.unwrap().ca_file,
        Some(Path::new(&home).join("ca.pem"))
    );
    let c = parse(&sync_toml("ca_file = \"/abs/ca.pem\"\n")).unwrap();
    assert_eq!(c.sync.unwrap().ca_file, Some(PathBuf::from("/abs/ca.pem")));
}

#[test]
fn sync_connection_keys_parse() {
    let c = parse(
        "vdir = \"/x\"\n[sync]\nurl = \"https://dav.example.test/a/\"\nusername = \"alice\"\npassword_command = [\"pass\", \"show\", \"x\"]\n",
    )
    .unwrap();
    let s = c.sync.unwrap();
    assert_eq!(s.url, "https://dav.example.test/a/");
    assert_eq!(s.username, "alice");
    assert_eq!(s.password_command, ["pass", "show", "x"]);
    assert_eq!(s.ca_file, None);
}

#[test]
fn removed_sync_keys_are_named_errors() {
    for (key, value) in [
        ("pair", "\"twydo\""),
        ("local_storage", "\"twydo_local\""),
        ("vdirsyncer_config", "\"/x/vdirsyncer\""),
        ("after_edit", "true"),
    ] {
        let e = parse(&sync_toml(&format!("{key} = {value}\n"))).unwrap_err();
        assert!(
            matches!(e, twydo_core::config::ConfigError::Invalid { .. }),
            "{e:?}"
        );
        let text = e.to_string();
        let want = format!(
            "[sync] {key} is no longer supported: Twydo generates the vdirsyncer config; run `tdo config init`"
        );
        assert!(text.ends_with(&want), "{text}");
    }
}

#[test]
fn missing_connection_keys_are_named_errors() {
    let msg = |toml: &str| parse(toml).unwrap_err().to_string();
    let required = |key: &str| format!("[sync] {key} is required; run `tdo config init`");
    assert!(msg("vdir = \"/x\"\n[sync]\n").ends_with(&required("url")));
    assert!(
        msg("vdir = \"/x\"\n[sync]\nurl = \"https://dav.example.test/a/\"\n")
            .ends_with(&required("username"))
    );
    assert!(
        msg("vdir = \"/x\"\n[sync]\nurl = \"https://dav.example.test/a/\"\nusername = \"alice\"\npassword_command = []\n")
            .ends_with(&required("password_command"))
    );
    for blank in ["[\"\"]", "[\" \", \"x\"]"] {
        let toml = format!(
            "vdir = \"/x\"\n[sync]\nurl = \"https://dav.example.test/a/\"\nusername = \"alice\"\npassword_command = {blank}\n"
        );
        assert!(
            msg(&toml).ends_with(&required("password_command")),
            "{blank}"
        );
    }
}

#[test]
fn no_sync_table_means_sync_off() {
    assert_eq!(parse("vdir = \"/x\"\n").unwrap().sync, None);
}

#[test]
fn warnings_are_empty() {
    assert!(parse(&sync_toml("")).unwrap().warnings().is_empty());
}

#[test]
fn unset_list_python_is_absent_on_the_wire() {
    // Released clients read `list_python` as a string; null would not parse.
    let c = parse(&sync_toml("")).unwrap();
    let v = serde_json::to_value(c.sync.as_ref().unwrap()).unwrap();
    assert!(v.get("list_python").is_none(), "{v}");
    let c = parse(&sync_toml("list_python = \"/usr/bin/python\"\n")).unwrap();
    let v = serde_json::to_value(c.sync.as_ref().unwrap()).unwrap();
    assert_eq!(v["list_python"], "/usr/bin/python");
}
