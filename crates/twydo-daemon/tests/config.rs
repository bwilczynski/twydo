//! Live `config.toml`: preferences apply and publish with cause `config`; binding changes wait
//! for a restart; a broken file keeps the last good configuration; a save by rename is seen; a
//! `--vdir` override is kept. Synthetic fixtures.

mod common;

use std::fs;
use std::time::{Duration, Instant};

use chrono::Weekday;
use common::{Fixture, TestDaemon, empty, snapshot, wait_event};
use twydo_core::config::WriteMode;
use twydo_daemon::protocol::events::{Cause, Event};
use twydo_daemon::protocol::methods::StatusView;

const BOUND: Duration = Duration::from_secs(2);

fn fx() -> Fixture {
    Fixture::new(&[("c1", "u1", "One", None)])
}

/// The text of a config file naming the fixture's vdir and state directory, then `extra`.
fn config_text(fx: &Fixture, writes: &str, extra: &str) -> String {
    // Top-level keys in `extra` must come first, so they are split off at its first table.
    let (top, tables) = extra
        .split_once('[')
        .map_or((extra, String::new()), |(a, b)| (a, format!("[{b}")));
    format!(
        "vdir = {:?}\nstate_dir = {:?}\n{top}[writes]\nmode = {writes:?}\n{tables}",
        fx.vdir(),
        fx.config().state_dir(),
    )
}

/// Writes the fixture's config file (the one the daemon watches).
fn write_config(fx: &Fixture, text: &str) {
    fs::write(fx.instance().config, text).unwrap();
}

fn status(c: &twydo_daemon::client::Client) -> StatusView {
    c.call("status.get", empty()).unwrap()
}

/// Polls `status.get` until `pred` holds, within `BOUND`.
fn wait_status(c: &twydo_daemon::client::Client, pred: impl Fn(&StatusView) -> bool) -> StatusView {
    let deadline = Instant::now() + BOUND;
    loop {
        let st = status(c);
        if pred(&st) {
            return st;
        }
        assert!(Instant::now() < deadline, "status never matched: {st:?}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn list_preference_change_publishes_cause_config() {
    let fx = fx();
    write_config(&fx, &config_text(&fx, "test", ""));
    let td = TestDaemon::start(&fx);
    let c = td.client();
    c.call::<_, twydo_daemon::protocol::methods::SubscribeResult>("events.subscribe", empty())
        .unwrap();
    assert!(snapshot(&c).config.lists.is_empty());
    write_config(
        &fx,
        &config_text(&fx, "test", "[lists.c1]\nhidden = true\n"),
    );
    let ev = wait_event(&c, BOUND, |e| matches!(e, Event::DataChanged { .. }));
    let Event::DataChanged { cause, .. } = ev else {
        unreachable!()
    };
    assert_eq!(cause, Cause::Config);
    assert!(snapshot(&c).config.lists["c1"].hidden);
}

#[test]
fn binding_change_needs_a_restart() {
    let fx = fx();
    write_config(&fx, &config_text(&fx, "test", ""));
    let td = TestDaemon::start(&fx);
    let c = td.client();
    write_config(&fx, &config_text(&fx, "verified", ""));
    let st = wait_status(&c, |s| !s.config.restart_needed.is_empty());
    assert_eq!(st.config.restart_needed, ["writes"]);
    assert_eq!(st.config.error, None);
    assert_eq!(snapshot(&c).config.writes.mode, WriteMode::Test);
}

#[test]
fn broken_config_keeps_the_last_good_one() {
    let fx = fx();
    write_config(&fx, &config_text(&fx, "test", ""));
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let before = snapshot(&c);
    write_config(&fx, "vdir = \n");
    let st = wait_status(&c, |s| s.config.error.is_some());
    assert!(st.config.restart_needed.is_empty());
    assert_eq!(snapshot(&c), before);
    write_config(&fx, &config_text(&fx, "test", "week_start = \"sun\"\n"));
    wait_status(&c, |s| s.config.error.is_none());
    let deadline = Instant::now() + BOUND;
    while snapshot(&c).config.week_start != Weekday::Sun {
        assert!(Instant::now() < deadline, "week_start never published");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn config_saved_by_rename_is_seen() {
    let fx = fx();
    write_config(&fx, &config_text(&fx, "test", ""));
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let path = fx.instance().config;
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, config_text(&fx, "test", "week_start = \"sun\"\n")).unwrap();
    fs::rename(&tmp, &path).unwrap();
    let deadline = Instant::now() + BOUND;
    while snapshot(&c).config.week_start != Weekday::Sun {
        assert!(Instant::now() < deadline, "rename not seen");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn vdir_flag_override_is_kept() {
    let fx = fx();
    // The daemon starts with `--vdir` (the fixture's instance says so); the file names another.
    write_config(&fx, &config_text(&fx, "test", ""));
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let other = fx.instance().config.with_file_name("elsewhere");
    fs::create_dir_all(&other).unwrap();
    let text = config_text(&fx, "test", "week_start = \"sun\"\n")
        .replace(&format!("{:?}", fx.vdir()), &format!("{other:?}"));
    write_config(&fx, &text);
    let deadline = Instant::now() + BOUND;
    while snapshot(&c).config.week_start != Weekday::Sun {
        assert!(Instant::now() < deadline, "change not applied");
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(status(&c).config.restart_needed.is_empty());
}

#[test]
fn error_clears_when_the_same_good_text_returns() {
    let fx = fx();
    let good = config_text(&fx, "test", "");
    write_config(&fx, &good);
    let td = TestDaemon::start(&fx);
    let c = td.client();
    // Deleted, then written back as it was.
    fs::remove_file(fx.instance().config).unwrap();
    wait_status(&c, |s| s.config.error.is_some());
    write_config(&fx, &good);
    wait_status(&c, |s| s.config.error.is_none());
    // Invalid, then the previous good text again.
    write_config(&fx, "vdir = \n");
    wait_status(&c, |s| s.config.error.is_some());
    write_config(&fx, &good);
    wait_status(&c, |s| s.config.error.is_none());
}

/// Waits until `week_start` is `want` in the published snapshot.
fn wait_week_start(c: &twydo_daemon::client::Client, want: Weekday, what: &str) {
    let deadline = Instant::now() + BOUND;
    while snapshot(c).config.week_start != want {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Final review: `config.toml` is often a link into a dotfiles directory. Edits to the target
/// (in place, and saved by rename next to it) are reloaded with cause `config`, and so is a
/// link retargeted to another file.
#[test]
fn a_symlinked_config_follows_its_target() {
    let fx = fx();
    let path = fx.instance().config;
    let dots = path.parent().unwrap().join("dotfiles");
    fs::create_dir_all(&dots).unwrap();
    let target = dots.join("twydo.toml");
    fs::write(&target, config_text(&fx, "test", "")).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    c.call::<_, twydo_daemon::protocol::methods::SubscribeResult>("events.subscribe", empty())
        .unwrap();

    // Edited in place.
    fs::write(&target, config_text(&fx, "test", "week_start = \"sun\"\n")).unwrap();
    let ev = wait_event(&c, BOUND, |e| matches!(e, Event::DataChanged { .. }));
    let Event::DataChanged { cause, .. } = ev else {
        unreachable!()
    };
    assert_eq!(cause, Cause::Config);
    wait_week_start(
        &c,
        Weekday::Sun,
        "an in-place edit of the target was not seen",
    );

    // Saved by rename over the target.
    let tmp = dots.join("twydo.toml.tmp");
    fs::write(&tmp, config_text(&fx, "test", "week_start = \"tue\"\n")).unwrap();
    fs::rename(&tmp, &target).unwrap();
    wait_week_start(
        &c,
        Weekday::Tue,
        "a save by rename of the target was not seen",
    );

    // The link retargeted to a file in another directory; edits there are seen too.
    let other_dir = path.parent().unwrap().join("elsewhere");
    fs::create_dir_all(&other_dir).unwrap();
    let other = other_dir.join("other.toml");
    fs::write(&other, config_text(&fx, "test", "week_start = \"wed\"\n")).unwrap();
    let link_tmp = path.with_extension("link.tmp");
    std::os::unix::fs::symlink(&other, &link_tmp).unwrap();
    fs::rename(&link_tmp, &path).unwrap();
    wait_week_start(&c, Weekday::Wed, "a retargeted link was not seen");
    fs::write(&other, config_text(&fx, "test", "week_start = \"thu\"\n")).unwrap();
    wait_week_start(&c, Weekday::Thu, "an edit of the new target was not seen");
}
