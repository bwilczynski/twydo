//! `tdo daemon install`, `uninstall`, `status` and `schema`, against a fake `systemctl` that
//! only logs. No real service manager or user unit directory is touched: the fake is first on
//! the child's `PATH`, and `HOME` and `XDG_CONFIG_HOME` are tempdirs.
mod common;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::process::{Command, Output};

use common::{TestEnv, fake_service_manager};
use twydo_daemon::paths;

struct Setup {
    env: TestEnv,
    log: PathBuf,
    path: String,
}

impl Setup {
    /// `tdo_dir_name` is the directory (under the env root) holding the `tdo` symlink.
    fn with_tdo_dir(tdo_dir_name: &str) -> Setup {
        let env = TestEnv::new("");
        let (bin, log) = fake_service_manager(env.root.path());
        let tdo_dir = env.root.path().join(tdo_dir_name);
        fs::create_dir_all(&tdo_dir).unwrap();
        symlink(common::tdo_exe(), tdo_dir.join("tdo")).unwrap();
        let path = format!("{}:{}:/usr/bin:/bin", bin.display(), tdo_dir.display());
        Setup { env, log, path }
    }

    fn new() -> Setup {
        Setup::with_tdo_dir("tdo-link")
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = self.env.tdo_cmd();
        c.env("PATH", &self.path)
            .env("TDO_BIN", common::tdo_exe())
            .env_remove("MISE_DATA_DIR")
            .arg("daemon")
            .args(args);
        c
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd(args).output().unwrap()
    }

    fn unit_name(&self) -> String {
        format!("twydo-{}.service", paths::socket_id(&self.env.socket()))
    }

    fn unit_file(&self) -> PathBuf {
        self.env
            .root
            .path()
            .join("config/systemd/user")
            .join(self.unit_name())
    }

    fn manager_log(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn install_writes_the_unit_and_enables_it() {
    let s = Setup::new();
    let o = s.run(&["install"]);
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    let name = s.unit_name();
    assert!(
        out(&o).contains(&format!("installed {name}")),
        "{}",
        out(&o)
    );
    let unit = fs::read_to_string(s.unit_file()).unwrap();
    let link = s.env.root.path().join("tdo-link/tdo");
    assert!(unit.contains(&link.display().to_string()), "{unit}");
    assert!(unit.contains("--socket"), "{unit}");
    assert!(unit.contains("XDG_CONFIG_HOME"), "{unit}");
    let log = s.manager_log();
    let reload = log.find("--user daemon-reload").expect(&log);
    let enable = log
        .find(&format!("--user enable --now {name}"))
        .expect(&log);
    assert!(reload < enable, "{log}");
}

#[test]
fn install_is_idempotent() {
    let s = Setup::new();
    assert_eq!(s.run(&["install"]).status.code(), Some(0));
    let first = fs::read(s.unit_file()).unwrap();
    let o = s.run(&["install"]);
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    assert_eq!(fs::read(s.unit_file()).unwrap(), first);
    assert!(
        out(&o).contains(&format!(
            "{} was already installed; rewrote it",
            s.unit_name()
        )),
        "{}",
        out(&o)
    );
}

#[test]
fn reinstall_stops_the_loaded_unit_before_enabling_it() {
    let s = Setup::new();
    let run = |s: &Setup| {
        let mut c = s.cmd(&["install"]);
        c.env("FAKE_STRICT_LOADED", "1");
        c.output().unwrap()
    };
    let o = run(&s);
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    let o = run(&s);
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    let log = s.manager_log();
    let name = s.unit_name();
    let stop = log.find(&format!("--user stop {name}")).expect(&log);
    let enable = log
        .rfind(&format!("--user enable --now {name}"))
        .expect(&log);
    assert!(stop < enable, "{log}");
}

#[test]
fn install_stops_a_detached_daemon_first() {
    let s = Setup::new();
    let o = s.run(&["--detach"]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    let before = s.env.connect(&s.env.socket());
    drop(before);
    let old = s.env.info().epoch;
    let o = s.run(&["install"]);
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    // The fake `enable` starts a fresh daemon; the old one would have kept its epoch.
    let new = s.env.info().epoch;
    assert_ne!(old, new, "the detached daemon was not stopped");
}

#[test]
fn install_disables_the_old_timer() {
    let s = Setup::new();
    let dir = s.env.root.path().join("config/systemd/user");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("twydo-sync.timer"), "[Timer]\n").unwrap();
    let o = s.run(&["install"]);
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    assert!(
        s.manager_log()
            .contains("--user disable --now twydo-sync.timer"),
        "{}",
        s.manager_log()
    );
    assert!(
        out(&o).contains("disabled twydo-sync.timer: the daemon syncs now"),
        "{}",
        out(&o)
    );
    assert!(dir.join("twydo-sync.timer").exists());
}

#[test]
fn install_refuses_a_versioned_exe() {
    let s = Setup::with_tdo_dir("installs/twydo/0.1.0");
    let o = s.run(&["install"]);
    assert_eq!(o.status.code(), Some(2), "{}{}", out(&o), err(&o));
    assert!(err(&o).contains("no stable path to tdo"), "{}", err(&o));
    assert!(!s.unit_file().exists());
}

#[test]
fn uninstall_disables_and_removes() {
    let s = Setup::new();
    let o = s.run(&["uninstall"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(out(&o).contains("no unit installed"), "{}", out(&o));

    assert_eq!(s.run(&["install"]).status.code(), Some(0));
    let o = s.run(&["uninstall"]);
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    let log = s.manager_log();
    let name = s.unit_name();
    let disable = log
        .find(&format!("--user disable --now {name}"))
        .expect(&log);
    assert!(!s.unit_file().exists());
    assert!(log[disable..].contains("--user daemon-reload"), "{log}");
}

#[test]
fn status_reports_unit_and_daemon_without_starting_one() {
    let s = Setup::new();
    let o = s.run(&["status"]);
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    let text = out(&o);
    assert!(
        text.contains("unit: not installed; daemon won't start at login; run tdo daemon install"),
        "{text}"
    );
    assert!(text.contains("daemon: not running"), "{text}");
    assert!(!s.env.socket().exists(), "status started a daemon");

    assert_eq!(s.run(&["install"]).status.code(), Some(0));
    let o = s.run(&["status"]);
    let text = out(&o);
    assert!(
        text.contains(&format!(
            "unit: installed ({}), enabled",
            s.unit_file().display()
        )),
        "{text}"
    );
    let info = s.env.info();
    assert!(
        text.contains(&format!(
            "daemon: running, version {}, pid {}, vdir {}",
            info.version,
            info.pid,
            info.vdir.display()
        )),
        "{text}"
    );
}

#[test]
fn schema_prints_the_document() {
    let s = Setup::new();
    let o = s.run(&["schema"]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    let printed: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../schema/tdo.openrpc.json");
    let checked_in: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(printed, checked_in);
}

// ---- starting and stopping through an installed unit ----

impl Setup {
    /// A unit file in place, as `install` leaves it, without running the manager.
    fn put_unit_file(&self) {
        let file = self.unit_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, "[Service]\n").unwrap();
    }

    fn ping(&self, cmd: &mut Command) -> Output {
        cmd.arg("daemon").arg("ping").output().unwrap()
    }
}

#[test]
fn a_client_starts_the_daemon_through_the_unit() {
    let s = Setup::new();
    s.put_unit_file();
    let mut c = s.env.tdo_cmd();
    c.env("PATH", &s.path).env("TDO_BIN", common::tdo_exe());
    let o = s.ping(&mut c);
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    assert_eq!(out(&o).trim(), s.env.info().epoch);
    let log = s.manager_log();
    assert!(
        log.contains(&format!("--user start {}", s.unit_name())),
        "{log}"
    );
}

#[test]
fn a_failing_manager_falls_back_to_the_detached_start() {
    let s = Setup::new();
    s.put_unit_file();
    let mut c = s.env.tdo_cmd();
    c.env("PATH", &s.path)
        .env("TDO_BIN", common::tdo_exe())
        .env("FAKE_START_FAILS", "1");
    let o = s.ping(&mut c);
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    assert_eq!(out(&o).trim(), s.env.info().epoch);
    assert!(
        s.manager_log()
            .contains(&format!("--user start {}", s.unit_name())),
        "{}",
        s.manager_log()
    );
}

#[test]
fn stop_goes_through_the_manager_when_a_unit_is_installed() {
    let s = Setup::new();
    s.put_unit_file();
    let mut c = s.env.tdo_cmd();
    c.env("PATH", &s.path).env("TDO_BIN", common::tdo_exe());
    assert_eq!(s.ping(&mut c).status.code(), Some(0));
    let o = s.run(&["stop"]);
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    let log = s.manager_log();
    assert!(
        log.contains(&format!("--user stop {}", s.unit_name())),
        "{log}"
    );
    assert!(twydo_daemon::client::Client::connect_raw(&s.env.socket()).is_err());
}

#[test]
fn stop_under_a_unit_stops_a_detached_daemon() {
    let s = Setup::new();
    s.put_unit_file();
    let o = s.run(&["--detach"]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    drop(s.env.connect(&s.env.socket()));
    let mut c = s.env.tdo_cmd();
    c.env("PATH", &s.path)
        .env("TDO_BIN", common::tdo_exe())
        .env("FAKE_STOP_NOOP", "1")
        .args(["daemon", "stop"]);
    let o = c.output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    assert!(twydo_daemon::client::Client::connect_raw(&s.env.socket()).is_err());
}

#[test]
fn stop_under_a_unit_survives_a_failing_manager() {
    let s = Setup::new();
    s.put_unit_file();
    let o = s.run(&["--detach"]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    drop(s.env.connect(&s.env.socket()));
    let mut c = s.env.tdo_cmd();
    c.env("PATH", &s.path)
        .env("TDO_BIN", common::tdo_exe())
        .env("FAKE_STOP_FAILS", "1")
        .args(["daemon", "stop"]);
    let o = c.output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    assert!(twydo_daemon::client::Client::connect_raw(&s.env.socket()).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn a_unit_start_that_never_answers_names_the_journal() {
    let s = Setup::new();
    s.put_unit_file();
    let mut c = s.env.tdo_cmd();
    c.env("PATH", &s.path)
        .env("TDO_BIN", common::tdo_exe())
        .env("FAKE_START_NOOP", "1");
    let o = s.ping(&mut c);
    assert_ne!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    assert!(
        err(&o).contains("journalctl --user -u twydo-"),
        "{}",
        err(&o)
    );
}

#[test]
fn stop_under_a_unit_with_nothing_running() {
    let s = Setup::new();
    s.put_unit_file();
    let o = s.run(&["stop"]);
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    assert_eq!(out(&o).trim(), "no daemon is running");
    assert!(
        s.manager_log()
            .contains(&format!("--user stop {}", s.unit_name())),
        "{}",
        s.manager_log()
    );
}

#[test]
fn stop_under_a_unit_with_nothing_running_and_a_failing_manager() {
    let s = Setup::new();
    s.put_unit_file();
    let mut c = s.cmd(&["stop"]);
    c.env("FAKE_STOP_FAILS", "1");
    let o = c.output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    assert_eq!(out(&o).trim(), "no daemon is running");
}
