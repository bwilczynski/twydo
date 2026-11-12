//! `tdo config init` through the binary: synthetic values, every path in a tempdir.
mod common;

use std::fs;
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
    let o = e.run(&[
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
