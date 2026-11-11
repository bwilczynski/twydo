//! The launchd agent: rendering and `launchctl` commands.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use super::{MANAGER_TIMEOUT, Manager, Spec, Unit, UnitState, run, run_ok};

const OLD_LABEL: &str = "app.twydo.sync";

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn render(unit: &Unit, spec: &Spec) -> String {
    let mut args = vec![
        spec.exe.display().to_string(),
        "daemon".to_string(),
        "--socket".to_string(),
        spec.socket.display().to_string(),
    ];
    if let Some(v) = &spec.vdir {
        args.push("--vdir".to_string());
        args.push(v.display().to_string());
    }
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!("<!-- {} -->\n", super::HEADER));
    out.push_str("<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n");
    out.push_str("<plist version=\"1.0\">\n<dict>\n");
    out.push_str(&format!(
        "  <key>Label</key>\n  <string>{}</string>\n",
        esc(&unit.name)
    ));
    out.push_str("  <key>ProgramArguments</key>\n  <array>\n");
    for a in &args {
        out.push_str(&format!("    <string>{}</string>\n", esc(a)));
    }
    out.push_str("  </array>\n");
    out.push_str("  <key>EnvironmentVariables</key>\n  <dict>\n");
    for (k, v) in &spec.env {
        out.push_str(&format!(
            "    <key>{}</key>\n    <string>{}</string>\n",
            esc(k),
            esc(v)
        ));
    }
    out.push_str("  </dict>\n");
    out.push_str("  <key>RunAtLoad</key>\n  <true/>\n");
    out.push_str("  <key>KeepAlive</key>\n  <true/>\n");
    out.push_str("  <key>ExitTimeOut</key>\n  <integer>90</integer>\n");
    out.push_str(&format!(
        "  <key>StandardErrorPath</key>\n  <string>{}</string>\n",
        esc(&spec.log.display().to_string())
    ));
    out.push_str("</dict>\n</plist>\n");
    out
}

pub struct Launchd {
    pub program: PathBuf,
    pub uid: u32,
    /// The LaunchAgents directory, where the old sync plist would live.
    pub agents_dir: PathBuf,
    pub timeout: Duration,
}

impl Default for Launchd {
    fn default() -> Self {
        Launchd {
            program: "launchctl".into(),
            // SAFETY: getuid has no preconditions and cannot fail.
            uid: unsafe { libc::getuid() },
            agents_dir: super::unit_for(
                super::Platform::Launchd,
                &twydo_daemon::paths::default_socket(),
                &|k| std::env::var_os(k),
            )
            .file
            .parent()
            .map(PathBuf::from)
            .unwrap_or_default(),
            timeout: MANAGER_TIMEOUT,
        }
    }
}

impl Launchd {
    fn domain(&self) -> String {
        format!("gui/{}", self.uid)
    }
    fn target(&self, label: &str) -> String {
        format!("gui/{}/{label}", self.uid)
    }
}

impl Manager for Launchd {
    fn reload(&self) -> io::Result<()> {
        Ok(())
    }
    fn enable_now(&self, u: &Unit) -> io::Result<()> {
        run_ok(
            &self.program,
            &["bootstrap", &self.domain(), &u.file.display().to_string()],
            self.timeout,
        )
    }
    fn start(&self, u: &Unit) -> io::Result<()> {
        // After a bootout the agent is unloaded; load it again. Any failure here (such as
        // "already loaded") is ignored: kickstart reports the real problem.
        let _ = self.enable_now(u);
        run_ok(
            &self.program,
            &["kickstart", &self.target(&u.name)],
            self.timeout,
        )
    }
    fn stop(&self, u: &Unit) -> io::Result<()> {
        run_ok(
            &self.program,
            &["bootout", &self.target(&u.name)],
            self.timeout,
        )
    }
    fn disable_now(&self, u: &Unit) -> io::Result<()> {
        run_ok(
            &self.program,
            &["bootout", &self.target(&u.name)],
            self.timeout,
        )
    }
    fn state(&self, u: &Unit) -> UnitState {
        match run(
            &self.program,
            &["print", &self.target(&u.name)],
            self.timeout,
        ) {
            Ok((ran, _)) if ran.ok => UnitState {
                enabled: Some(true),
                active: Some(ran.stdout.contains("state = running")),
            },
            Ok(_) => UnitState {
                enabled: Some(false),
                active: Some(false),
            },
            Err(_) => UnitState::default(),
        }
    }
    fn disable_old_timer(&self) -> io::Result<Option<String>> {
        if !self.agents_dir.join(format!("{OLD_LABEL}.plist")).exists() {
            return Ok(None);
        }
        let target = self.target(OLD_LABEL);
        // Not loaded is fine; the disable below is what stops the next login loading it.
        let _ = run(&self.program, &["bootout", &target], self.timeout);
        run_ok(&self.program, &["disable", &target], self.timeout)?;
        Ok(Some(OLD_LABEL.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::Launchd;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn launchd_unit() -> Unit {
        let mut u = unit_for(Platform::Launchd, Path::new("/s"), &|_| None);
        u.name = "app.twydo.daemon".into();
        u.file = PathBuf::from("/h/app.twydo.daemon.plist");
        u
    }

    fn lc(dir: &Path, program: PathBuf) -> Launchd {
        Launchd {
            program,
            uid: 501,
            agents_dir: dir.join("agents"),
            timeout: MANAGER_TIMEOUT,
        }
    }

    fn log(dir: &Path) -> Vec<String> {
        fs::read_to_string(dir.join("log"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    #[test]
    fn launchd_stop_boots_out() {
        let d = tempfile::tempdir().unwrap();
        let m = lc(d.path(), testutil::fake(d.path(), "launchctl", "", 0));
        m.stop(&launchd_unit()).unwrap();
        assert_eq!(log(d.path()), ["bootout gui/501/app.twydo.daemon"]);
    }

    #[test]
    fn launchd_start_bootstraps_then_kickstarts() {
        let d = tempfile::tempdir().unwrap();
        let m = lc(d.path(), testutil::fake(d.path(), "launchctl", "", 0));
        m.start(&launchd_unit()).unwrap();
        assert_eq!(
            log(d.path()),
            [
                "bootstrap gui/501 /h/app.twydo.daemon.plist",
                "kickstart gui/501/app.twydo.daemon"
            ]
        );
        // A failing bootstrap (already loaded) still kickstarts.
        let d = tempfile::tempdir().unwrap();
        let prog = d.path().join("launchctl");
        testutil::write_script(
            &prog,
            &format!(
                "echo \"$@\" >> '{}'\n[ \"$1\" = bootstrap ] && exit 5\nexit 0",
                d.path().join("log").display()
            ),
        );
        lc(d.path(), prog).start(&launchd_unit()).unwrap();
        assert_eq!(log(d.path()).len(), 2);
        assert!(log(d.path())[1].starts_with("kickstart"));
    }

    #[test]
    fn old_timer_disabled_when_its_file_exists() {
        let d = tempfile::tempdir().unwrap();
        let m = lc(d.path(), testutil::fake(d.path(), "launchctl", "", 0));
        fs::create_dir_all(&m.agents_dir).unwrap();
        fs::write(m.agents_dir.join("app.twydo.sync.plist"), "").unwrap();
        assert_eq!(
            m.disable_old_timer().unwrap().as_deref(),
            Some("app.twydo.sync")
        );
        assert_eq!(
            log(d.path()),
            [
                "bootout gui/501/app.twydo.sync",
                "disable gui/501/app.twydo.sync"
            ]
        );
    }

    #[test]
    fn no_old_timer_does_nothing() {
        let d = tempfile::tempdir().unwrap();
        let m = lc(d.path(), testutil::fake(d.path(), "launchctl", "", 0));
        assert_eq!(m.disable_old_timer().unwrap(), None);
        assert!(log(d.path()).is_empty());
    }

    #[test]
    fn launchd_plist_always_names_the_socket() {
        let u = unit_for(
            Platform::Launchd,
            Path::new("/t/twydo/tdo.sock"),
            &|k| match k {
                "HOME" => Some("/h".into()),
                "TMPDIR" | "XDG_RUNTIME_DIR" => Some("/t".into()),
                _ => None,
            },
        );
        let spec = Spec {
            exe: PathBuf::from("/bin/tdo"),
            socket: PathBuf::from("/s/a&b/tdo.sock"),
            default_socket: true,
            vdir: None,
            env: vec![("PATH".into(), "/usr/bin".into())],
            log: PathBuf::from("/l/daemon.log"),
        };
        let text = render(&u, &spec);
        for want in [
            "<string>app.twydo.daemon</string>",
            "<string>--socket</string>",
            "<string>/s/a&amp;b/tdo.sock</string>",
            "<key>RunAtLoad</key>",
            "<key>KeepAlive</key>",
            "<key>ExitTimeOut</key>\n  <integer>90</integer>",
            "<key>StandardErrorPath</key>\n  <string>/l/daemon.log</string>",
            "<key>PATH</key>\n    <string>/usr/bin</string>",
        ] {
            assert!(text.contains(want), "missing {want}\n{text}");
        }
    }
}
