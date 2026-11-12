//! `tdo config check`: synthetic config in temp dirs, fake executables written by the tests.
mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Output;

use common::tdo_bin;

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("vdir")).unwrap();
        fs::create_dir_all(dir.path().join("config/twydo")).unwrap();
        Env { dir }
    }

    fn p(&self, rel: &str) -> std::path::PathBuf {
        self.dir.path().join(rel)
    }

    fn script(&self, name: &str, body: &str) -> String {
        let p = self.p(name);
        fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        p.display().to_string()
    }

    /// Write config.toml with the vdir plus `extra` (a `[sync]` table, say).
    fn config(&self, extra: &str) {
        fs::write(
            self.p("config/twydo/config.toml"),
            format!("vdir = \"{}\"\n{extra}", self.p("vdir").display()),
        )
        .unwrap();
    }

    /// A `[sync]` table with a good fake vdirsyncer and python, and a password command.
    fn sync(&self, password: &str, more: &str) -> String {
        let vd = self.script("vds", "echo 'vdirsyncer, version 0.21.1'");
        let py = self.script("py", "echo 0.21.1");
        format!(
            "[sync]\nurl = \"https://dav.example.test/alice/\"\nusername = \"alice\"\n\
             password_command = [\"{password}\"]\ncommand = \"{vd}\"\nlist_python = \"{py}\"\n{more}"
        )
    }

    fn run(&self) -> Output {
        tdo_bin(&self.p("none.sock"), &self.p("config"), &self.p("state"))
            .args(["config", "check"])
            .output()
            .unwrap()
    }
}

fn text(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn line<'a>(out: &'a str, prefix: &str) -> &'a str {
    out.lines()
        .find(|l| l.starts_with(prefix))
        .unwrap_or_else(|| panic!("no line starting {prefix:?} in:\n{out}"))
}

#[test]
fn sync_off_passes() {
    let e = Env::new();
    e.config("");
    let o = e.run();
    let out = text(&o);
    assert_eq!(o.status.code(), Some(0), "{out}");
    for n in ["✓ config", "✓ vdir", "✓ sync"] {
        line(&out, n);
    }
    let v = line(&out, "– vdirsyncer");
    assert!(v.contains("sync off"), "{v}");
    assert!(line(&out, "– ca file").contains("sync off"));
}

#[test]
fn removed_key_fails_config_step() {
    let e = Env::new();
    e.config("[sync]\npair = \"x\"\n");
    let o = e.run();
    let out = text(&o);
    assert_eq!(o.status.code(), Some(1), "{out}");
    line(&out, "✗ config");
    assert!(out.contains("hint: run tdo config init"), "{out}");
    line(&out, "– vdir");
    assert!(line(&out, "– vdirsyncer").contains("skipped: config failed"));
}

#[test]
fn unsupported_vdirsyncer_version_fails() {
    let e = Env::new();
    let vd = e.script("old", "echo 'vdirsyncer, version 0.20.1'");
    let pw = e.script("pw", "echo s3cret");
    e.config(&e.sync(&pw, "").replace(
        &format!("command = \"{}\"", e.p("vds").display()),
        &format!("command = \"{vd}\""),
    ));
    let o = e.run();
    let out = text(&o);
    assert_eq!(o.status.code(), Some(1), "{out}");
    let l = line(&out, "✗ vdirsyncer");
    assert!(l.contains("0.20.1"), "{l}");
    assert!(out.contains("[sync].command"), "{out}");
    // Independent steps still run.
    line(&out, "✓ password");
}

#[test]
fn missing_vdirsyncer_names_the_key() {
    let e = Env::new();
    e.config(&e.sync("true", "").replace(
        &format!("command = \"{}\"", e.p("vds").display()),
        "command = \"/nonexistent/vdirsyncer\"",
    ));
    let out = text(&e.run());
    assert!(line(&out, "✗ vdirsyncer").contains("not found"), "{out}");
}

#[test]
fn password_failure_hides_stdout() {
    let e = Env::new();
    let pw = e.script("pw", "echo s3cret; echo 'no such entry' >&2; exit 1");
    e.config(&e.sync(&pw, ""));
    let o = e.run();
    let out = text(&o);
    assert_eq!(o.status.code(), Some(1), "{out}");
    line(&out, "✗ password");
    assert!(out.contains("no such entry"), "{out}");
    assert!(!out.contains("s3cret"), "{out}");
    assert!(!String::from_utf8_lossy(&o.stderr).contains("s3cret"));
}

#[test]
fn password_success_hides_secret() {
    let e = Env::new();
    let pw = e.script("pw", "echo s3cret");
    e.config(&e.sync(&pw, ""));
    let o = e.run();
    let out = text(&o);
    assert_eq!(o.status.code(), Some(0), "{out}");
    line(&out, "✓ password");
    assert!(!out.contains("s3cret"), "{out}");
    assert!(!String::from_utf8_lossy(&o.stderr).contains("s3cret"));
}

#[test]
fn check_creates_no_state() {
    let e = Env::new();
    let pw = e.script("pw", "echo s3cret");
    e.config(&e.sync(&pw, ""));
    let out = text(&e.run());
    assert!(
        line(&out, "✓ generated config").contains("written on first sync"),
        "{out}"
    );
    let state = e.p("state");
    assert!(
        !state.exists() || fs::read_dir(&state).unwrap().next().is_none(),
        "state was created"
    );
    assert!(!Path::new(&e.p("state")).join("twydo").exists());
}

#[test]
fn missing_ca_file_fails() {
    let e = Env::new();
    let pw = e.script("pw", "echo s3cret");
    let none = e.p("none.pem");
    e.config(&e.sync(&pw, &format!("ca_file = \"{}\"\n", none.display())));
    let o = e.run();
    let out = text(&o);
    assert_eq!(o.status.code(), Some(1), "{out}");
    line(&out, "✗ ca file");
}

#[test]
fn vdir_failure_skips_only_generated_config() {
    let e = Env::new();
    let pw = e.script("pw", "echo s3cret");
    e.config(&e.sync(&pw, ""));
    fs::remove_dir(e.p("vdir")).unwrap();
    let out = text(&e.run());
    line(&out, "✗ vdir");
    assert!(line(&out, "– generated config").contains("skipped: vdir failed"));
    line(&out, "✓ vdirsyncer");
    line(&out, "✓ password");
}
