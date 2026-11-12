//! `tdo config init` through the binary: synthetic values, every path in a tempdir.
mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Output, Stdio};

use common::tdo_bin;

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("home")).unwrap();
        Env { dir }
    }

    fn cfg(&self) -> std::path::PathBuf {
        self.dir.path().join("config/twydo/config.toml")
    }

    /// A directory of fake `vdirsyncer`, `tdo` and `python3` executables, so the check that
    /// `init` ends with does not depend on what the host has installed.
    fn fake_bin(&self) -> std::path::PathBuf {
        let bin = self.dir.path().join("fakebin");
        fs::create_dir_all(&bin).unwrap();
        for (name, body) in [
            ("vdirsyncer", "echo 'vdirsyncer, version 0.21.0'"),
            ("python3", "echo 0.21.0"),
            ("tdo", ""),
        ] {
            let p = bin.join(name);
            fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        }
        bin
    }

    fn run_with_fakes(&self, args: &[&str]) -> Output {
        let d = self.dir.path();
        let path = format!("{}:/usr/bin:/bin", self.fake_bin().display());
        let mut c: Command = tdo_bin(&d.join("no-socket"), &d.join("config"), &d.join("state"));
        c.env("HOME", d.join("home"))
            .env("PATH", path)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn run(&self, args: &[&str]) -> Output {
        let d = self.dir.path();
        let mut c: Command = tdo_bin(&d.join("no-socket"), &d.join("config"), &d.join("state"));
        c.env("HOME", d.join("home"))
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
}

#[test]
fn non_interactive_missing_url_writes_nothing() {
    let e = Env::new();
    let o = e.run(&[
        "config",
        "init",
        "--username",
        "alice",
        "--password-command",
        "true",
    ]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("--url"));
    assert!(!e.cfg().exists());
}

#[test]
fn init_runs_check() {
    let e = Env::new();
    let v = e.dir.path().join("v");
    let o = e.run(&["--vdir", v.to_str().unwrap(), "config", "init", "--no-sync"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("sync off"), "{out}");
    assert!(String::from_utf8_lossy(&o.stderr).contains("wrote "));
    assert!(v.is_dir());
    assert!(fs::read_to_string(e.cfg()).unwrap().contains("vdir = "));
}

#[test]
fn non_interactive_with_flags_writes_sync() {
    let e = Env::new();
    let o = e.run_with_fakes(&[
        "config",
        "init",
        "--url",
        "https://dav.example.test/a/",
        "--username",
        "alice",
        "--password-command",
        "echo pw",
    ]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let t = fs::read_to_string(e.cfg()).unwrap();
    assert!(t.contains("vdir = \"~/.local/share/twydo/vdir\""), "{t}");
    assert!(t.contains("password_command = [\"echo\", \"pw\"]"), "{t}");
    assert!(e.dir.path().join("home/.local/share/twydo/vdir").is_dir());
}

#[test]
fn empty_flag_values_are_rejected_and_write_nothing() {
    for (flag, args) in [
        (
            "--url",
            [
                "--url",
                "",
                "--username",
                "alice",
                "--password-command",
                "true",
            ],
        ),
        (
            "--username",
            [
                "--url",
                "https://dav.example.test/a/",
                "--username",
                " ",
                "--password-command",
                "true",
            ],
        ),
        (
            "--password-command",
            [
                "--url",
                "https://dav.example.test/a/",
                "--username",
                "alice",
                "--password-command",
                "",
            ],
        ),
    ] {
        let e = Env::new();
        let mut full = vec!["config", "init"];
        full.extend(args);
        let o = e.run(&full);
        assert_eq!(o.status.code(), Some(2), "{flag}");
        assert!(
            String::from_utf8_lossy(&o.stderr).contains(flag),
            "{flag}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        assert!(!e.cfg().exists(), "{flag}");
    }
}
