//! The systemd user service: rendering and `systemctl --user` commands.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use super::{MANAGER_TIMEOUT, Manager, Spec, Unit, UnitState, run, run_ok};
use twydo_daemon::EXIT_ALREADY_RUNNING;

const OLD_TIMER: &str = "twydo-sync.timer";

/// One `ExecStart` argument per systemd.service(5) "Command lines": `%` and `$` doubled, and
/// double-quoted (with `\` and `"` escaped) when it holds whitespace, quotes or a backslash.
fn exec_arg(a: &str) -> String {
    let a = a.replace('%', "%%").replace('$', "$$");
    if a.is_empty()
        || a.contains(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | ';'))
    {
        let e = a
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n");
        format!("\"{e}\"")
    } else {
        a
    }
}

/// The inside of an `Environment="..."` assignment: `\` and `"` escaped, `%` doubled.
fn env_value(v: &str) -> String {
    v.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
        .replace('\n', "\\n")
}

pub fn render(_unit: &Unit, spec: &Spec) -> String {
    let mut args = vec![spec.exe.display().to_string(), "daemon".to_string()];
    if !spec.default_socket {
        args.push("--socket".to_string());
        args.push(spec.socket.display().to_string());
    }
    if let Some(v) = &spec.vdir {
        args.push("--vdir".to_string());
        args.push(v.display().to_string());
    }
    let exec = args
        .iter()
        .map(|a| exec_arg(a))
        .collect::<Vec<_>>()
        .join(" ");
    let mut out = String::new();
    out.push_str(&format!("# {}\n", super::HEADER));
    out.push_str("[Unit]\nDescription=Twydo daemon\n\n[Service]\n");
    out.push_str(&format!("ExecStart={exec}\n"));
    for (k, v) in &spec.env {
        out.push_str(&format!(
            "Environment=\"{}={}\"\n",
            env_value(k),
            env_value(v)
        ));
    }
    out.push_str("Restart=always\nRestartSec=1\n");
    out.push_str(&format!(
        "RestartPreventExitStatus={EXIT_ALREADY_RUNNING}\n"
    ));
    out.push_str("TimeoutStopSec=90\n\n[Install]\nWantedBy=default.target\n");
    out
}

pub struct Systemd {
    pub program: PathBuf,
    /// The user unit directory, where the old sync timer would live.
    pub unit_dir: PathBuf,
    pub timeout: Duration,
}

impl Default for Systemd {
    fn default() -> Self {
        Systemd {
            program: "systemctl".into(),
            unit_dir: super::unit_for(
                super::Platform::Systemd,
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

impl Systemd {
    fn ctl(&self, args: &[&str]) -> io::Result<()> {
        let mut full = vec!["--user"];
        full.extend_from_slice(args);
        run_ok(&self.program, &full, self.timeout)
    }

    /// `is-enabled` / `is-active`: the verdict word, if the manager printed one we know.
    fn verdict(&self, verb: &str, name: &str, yes: &[&str], no: &[&str]) -> Option<bool> {
        let (ran, _) = run(&self.program, &["--user", verb, name], self.timeout).ok()?;
        let word = ran.stdout.lines().next().unwrap_or("").trim().to_string();
        if yes.contains(&word.as_str()) {
            Some(true)
        } else if no.contains(&word.as_str()) {
            Some(false)
        } else {
            None
        }
    }
}

impl Manager for Systemd {
    fn reload(&self) -> io::Result<()> {
        self.ctl(&["daemon-reload"])
    }
    fn enable_now(&self, u: &Unit) -> io::Result<()> {
        self.ctl(&["enable", "--now", &u.name])
    }
    fn start(&self, u: &Unit) -> io::Result<()> {
        self.ctl(&["start", &u.name])
    }
    fn stop(&self, u: &Unit) -> io::Result<()> {
        self.ctl(&["stop", &u.name])
    }
    fn disable_now(&self, u: &Unit) -> io::Result<()> {
        self.ctl(&["disable", "--now", &u.name])
    }
    fn state(&self, u: &Unit) -> UnitState {
        UnitState {
            enabled: self.verdict(
                "is-enabled",
                &u.name,
                &["enabled", "enabled-runtime", "static", "alias", "linked"],
                &["disabled", "masked", "linked-runtime", "not-found"],
            ),
            active: self.verdict(
                "is-active",
                &u.name,
                &["active", "activating", "reloading"],
                &["inactive", "failed", "deactivating"],
            ),
        }
    }
    fn disable_old_timer(&self) -> io::Result<Option<String>> {
        if !self.unit_dir.join(OLD_TIMER).exists() {
            return Ok(None);
        }
        self.ctl(&["disable", "--now", OLD_TIMER])?;
        Ok(Some(OLD_TIMER.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::Systemd;

    #[test]
    fn old_timer_disabled_when_its_file_exists() {
        let d = tempfile::tempdir().unwrap();
        let prog = testutil::fake(d.path(), "systemctl", "", 0);
        let m = Systemd {
            program: prog,
            unit_dir: d.path().join("units"),
            timeout: MANAGER_TIMEOUT,
        };
        std::fs::create_dir_all(&m.unit_dir).unwrap();
        std::fs::write(m.unit_dir.join("twydo-sync.timer"), "").unwrap();
        assert_eq!(
            m.disable_old_timer().unwrap().as_deref(),
            Some("twydo-sync.timer")
        );
        let log = std::fs::read_to_string(d.path().join("log")).unwrap();
        assert_eq!(log.trim(), "--user disable --now twydo-sync.timer");
    }

    #[test]
    fn a_stuck_manager_command_times_out() {
        let d = tempfile::tempdir().unwrap();
        let prog = d.path().join("systemctl");
        testutil::write_script(&prog, "sleep 30");
        let m = Systemd {
            program: prog,
            unit_dir: PathBuf::new(),
            timeout: std::time::Duration::from_millis(300),
        };
        let u = unit_for(Platform::Systemd, Path::new("/s"), &|_| None);
        let t = std::time::Instant::now();
        let err = m.stop(&u).unwrap_err();
        assert!(
            t.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            t.elapsed()
        );
        assert!(err.to_string().contains("timed out after"), "{err}");
        assert!(err.to_string().contains("systemctl"), "{err}");
    }

    #[test]
    fn a_descendant_holding_the_pipes_still_times_out() {
        let d = tempfile::tempdir().unwrap();
        let prog = d.path().join("systemctl");
        let pidfile = d.path().join("pid");
        testutil::write_script(
            &prog,
            &format!("sleep 30 &\necho $! > {}\nexit 0", pidfile.display()),
        );
        let m = Systemd {
            program: prog,
            unit_dir: PathBuf::new(),
            timeout: std::time::Duration::from_millis(300),
        };
        let u = unit_for(Platform::Systemd, Path::new("/s"), &|_| None);
        let t = std::time::Instant::now();
        let err = m.stop(&u).unwrap_err();
        assert!(
            t.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            t.elapsed()
        );
        assert!(err.to_string().contains("timed out after"), "{err}");
        let pid: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // The background sleep is dead (killed with the group); allow the reaping to land.
        let gone = (0..50).any(|_| {
            // SAFETY: signal 0 only probes whether the process exists.
            let alive = unsafe { libc::kill(pid, 0) } == 0
                && !std::fs::read_to_string(format!("/proc/{pid}/stat"))
                    .map(|s| s.contains(") Z"))
                    .unwrap_or(true);
            if alive {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            !alive
        });
        assert!(gone, "the background process {pid} is still running");
    }

    #[test]
    fn a_leader_that_closes_its_output_does_not_spin() {
        fn cpu() -> f64 {
            let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
            // The loop under test runs on this thread; per-thread keeps parallel tests out.
            #[cfg(target_os = "linux")]
            let who = libc::RUSAGE_THREAD;
            #[cfg(not(target_os = "linux"))]
            let who = libc::RUSAGE_SELF;
            // SAFETY: ru is a valid rusage out-pointer.
            unsafe { libc::getrusage(who, &mut ru) };
            let t = |v: libc::timeval| v.tv_sec as f64 + v.tv_usec as f64 / 1e6;
            t(ru.ru_utime) + t(ru.ru_stime)
        }
        let d = tempfile::tempdir().unwrap();
        let prog = d.path().join("systemctl");
        testutil::write_script(&prog, "exec >&- 2>&-\nsleep 1");
        let before = cpu();
        let t = std::time::Instant::now();
        let (ran, _) = crate::service::run(&prog, &[], std::time::Duration::from_secs(5)).unwrap();
        let spent = cpu() - before;
        assert!(ran.ok);
        assert!(
            t.elapsed() < std::time::Duration::from_secs(3),
            "{:?}",
            t.elapsed()
        );
        assert!(spent < 0.5, "burned {spent} s of cpu waiting");
    }

    #[test]
    fn no_old_timer_does_nothing() {
        let d = tempfile::tempdir().unwrap();
        let prog = testutil::fake(d.path(), "systemctl", "", 0);
        let m = Systemd {
            program: prog,
            unit_dir: d.path().join("units"),
            timeout: MANAGER_TIMEOUT,
        };
        assert_eq!(m.disable_old_timer().unwrap(), None);
        assert!(!d.path().join("log").exists());
    }
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    fn unit(socket: &str, default: bool) -> Unit {
        let env = |k: &str| match k {
            "HOME" => Some(OsString::from("/h")),
            _ => None,
        };
        let mut u = unit_for(Platform::Systemd, Path::new(socket), &env);
        u.default_socket = default;
        u
    }

    fn spec(socket: &str, default: bool, vdir: Option<&str>) -> Spec {
        Spec {
            exe: PathBuf::from("/home/u/.local/share/mise/shims/tdo"),
            socket: PathBuf::from(socket),
            default_socket: default,
            vdir: vdir.map(PathBuf::from),
            env: vec![
                ("PATH".into(), "/usr/bin".into()),
                ("XDG_CONFIG_HOME".into(), "/home/u/cfg".into()),
            ],
            log: PathBuf::from("/l"),
        }
    }

    #[test]
    fn systemd_unit_for_the_default_socket() {
        let text = render(
            &unit("/run/u/tdo.sock", true),
            &spec("/run/u/tdo.sock", true, None),
        );
        let lines: Vec<&str> = text.lines().collect();
        for want in [
            "ExecStart=/home/u/.local/share/mise/shims/tdo daemon",
            "Environment=\"PATH=/usr/bin\"",
            "Environment=\"XDG_CONFIG_HOME=/home/u/cfg\"",
            "Restart=always",
            "RestartSec=1",
            "RestartPreventExitStatus=10",
            "TimeoutStopSec=90",
            "WantedBy=default.target",
        ] {
            assert!(lines.contains(&want), "missing {want}\n{text}");
        }
        assert!(!text.contains("--socket"));
    }

    #[test]
    fn systemd_unit_escapes_arguments_and_environment() {
        let mut sp = spec(
            "/run/t/s.sock",
            false,
            Some("/v/Task Files/50% \"q\" \\b $x"),
        );
        sp.env = vec![("K".into(), "a b\"c\\d%e".into())];
        let text = render(&unit("/run/t/s.sock", false), &sp);
        assert!(
            text.lines().any(|l| l
                == "ExecStart=/home/u/.local/share/mise/shims/tdo daemon --socket /run/t/s.sock --vdir \"/v/Task Files/50%% \\\"q\\\" \\\\b $$x\""),
            "{text}"
        );
        assert!(
            text.lines()
                .any(|l| l == "Environment=\"K=a b\\\"c\\\\d%%e\""),
            "{text}"
        );
    }

    #[test]
    fn systemd_unit_for_another_socket_names_it() {
        let sock = Path::new("/run/t/s.sock");
        let u = unit_for(Platform::Systemd, sock, &|_| None);
        assert_eq!(
            u.name,
            format!("twydo-{}.service", twydo_daemon::paths::socket_id(sock))
        );
        let text = render(&u, &spec("/run/t/s.sock", false, Some("/v")));
        assert!(
            text.lines().any(|l| l.starts_with("ExecStart=")
                && l.ends_with("daemon --socket /run/t/s.sock --vdir /v")),
            "{text}"
        );
    }
}
