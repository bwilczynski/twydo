//! The daemon's service unit: its names, its rendered text, a stable path to `tdo` and the
//! commands that drive the service manager (a systemd user service on Linux, a launchd agent
//! on macOS). Everything but [`manager`]'s commands is pure.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use twydo_daemon::paths;

pub mod launchd;
pub mod systemd;

/// The first line of every rendered unit, so installed files say where they came from.
pub const HEADER: &str = "Written by tdo daemon install; reference copy in contrib/";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Systemd,
    Launchd,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::Launchd
        } else {
            Platform::Systemd
        }
    }
}

/// Where a daemon's unit lives and what it is called.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    pub platform: Platform,
    /// `twydo.service` or `twydo-<id>.service` (systemd); `app.twydo.daemon[.<id>]` (launchd).
    pub name: String,
    pub file: PathBuf,
    pub socket: PathBuf,
    pub default_socket: bool,
}

/// What a unit file is rendered from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub exe: PathBuf,
    pub socket: PathBuf,
    pub default_socket: bool,
    pub vdir: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub log: PathBuf,
}

type Env<'a> = &'a dyn Fn(&str) -> Option<OsString>;

fn non_empty(env: Env, k: &str) -> Option<OsString> {
    env(k).filter(|v| !v.is_empty())
}

/// The unit that serves `socket`. Paths come from `XDG_CONFIG_HOME` / `HOME` through `env`.
pub fn unit_for(platform: Platform, socket: &Path, env: Env) -> Unit {
    // SAFETY: geteuid has no preconditions and cannot fail.
    let uid = unsafe { libc::geteuid() };
    let default = paths::socket_path_from(
        None,
        &|k| if k == "TWYDO_SOCKET" { None } else { env(k) },
        uid,
    );
    let default_socket = socket == default;
    let id = paths::socket_id(socket);
    let home = || PathBuf::from(non_empty(env, "HOME").unwrap_or_default());
    let (name, file) = match platform {
        Platform::Systemd => {
            let name = if default_socket {
                "twydo.service".to_string()
            } else {
                format!("twydo-{id}.service")
            };
            let base = match non_empty(env, "XDG_CONFIG_HOME") {
                Some(c) => PathBuf::from(c),
                None => home().join(".config"),
            };
            let file = base.join("systemd").join("user").join(&name);
            (name, file)
        }
        Platform::Launchd => {
            let name = if default_socket {
                "app.twydo.daemon".to_string()
            } else {
                format!("app.twydo.daemon.{id}")
            };
            let file = home()
                .join("Library")
                .join("LaunchAgents")
                .join(format!("{name}.plist"));
            (name, file)
        }
    };
    Unit {
        platform,
        name,
        file,
        socket: socket.to_path_buf(),
        default_socket,
    }
}

pub fn unit_for_current(socket: &Path) -> Unit {
    unit_for(Platform::current(), socket, &|k| std::env::var_os(k))
}

/// The unit text (systemd) or plist XML (launchd).
pub fn render(unit: &Unit, spec: &Spec) -> String {
    match unit.platform {
        Platform::Systemd => systemd::render(unit, spec),
        Platform::Launchd => launchd::render(unit, spec),
    }
}

/// A path to `tdo` that survives `mise` upgrades: the first `PATH` entry holding `tdo` that is
/// not a versioned install directory (`.../installs/...`), else `<mise_shims>/tdo`.
pub fn stable_exe(path_var: Option<&OsStr>, mise_shims: &Path) -> Result<PathBuf, String> {
    let mut versioned: Option<PathBuf> = None;
    for dir in std::env::split_paths(path_var.unwrap_or_default()) {
        let candidate = dir.join("tdo");
        if !candidate.is_file() {
            continue;
        }
        if dir.components().any(|c| c.as_os_str() == "installs") {
            versioned.get_or_insert(candidate);
            continue;
        }
        return Ok(candidate);
    }
    let shim = mise_shims.join("tdo");
    if shim.is_file() {
        return Ok(shim);
    }
    let found = versioned
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "nothing on PATH".to_string());
    Err(format!(
        "no stable path to tdo (found only {found}); run 'mise reshim' or pass --exe <path>"
    ))
}

/// The environment a unit carries: `PATH` plus the XDG variables that are set (a service
/// manager's own environment is bare).
pub fn service_env(env: Env) -> Vec<(String, String)> {
    ["PATH", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_DATA_HOME"]
        .iter()
        .filter_map(|k| {
            non_empty(env, k).map(|v| (k.to_string(), v.to_string_lossy().into_owned()))
        })
        .collect()
}

/// What the manager says about a unit; `None` = it did not say.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnitState {
    pub enabled: Option<bool>,
    pub active: Option<bool>,
}

pub trait Manager {
    fn reload(&self) -> io::Result<()>;
    fn enable_now(&self, u: &Unit) -> io::Result<()>;
    fn start(&self, u: &Unit) -> io::Result<()>;
    fn stop(&self, u: &Unit) -> io::Result<()>;
    fn disable_now(&self, u: &Unit) -> io::Result<()>;
    fn state(&self, u: &Unit) -> UnitState;
    /// Disables the old sync timer if one is enabled; returns the name it disabled.
    fn disable_old_timer(&self) -> io::Result<Option<String>>;
}

/// The real manager for `p`, running `systemctl` or `launchctl` from `PATH`.
pub fn manager(p: Platform) -> Box<dyn Manager> {
    match p {
        Platform::Systemd => Box::new(systemd::Systemd::default()),
        Platform::Launchd => Box::new(launchd::Launchd::default()),
    }
}

/// A finished command: success flag and trimmed stdout.
pub(crate) struct Ran {
    pub ok: bool,
    pub stdout: String,
}

/// How long a service manager command may take before it is killed.
pub const MANAGER_TIMEOUT: Duration = Duration::from_secs(10);

fn command_line(program: &Path, args: &[&str]) -> String {
    std::iter::once(program.display().to_string())
        .chain(args.iter().map(|a| a.to_string()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Runs `program args` for at most `timeout`; failing to start is an error, and so is a
/// timeout (the child's process group is killed and reaped). A non-zero exit is reported in
/// `Ran` along with its stderr in the second field.
pub(crate) fn run(program: &Path, args: &[&str], timeout: Duration) -> io::Result<(Ran, String)> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        let done = done_tx.clone();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            let _ = done.send(());
            buf
        })
    };
    let out = drain(child.stdout.take().map(|p| Box::new(p) as _));
    let err = drain(child.stderr.take().map(|p| Box::new(p) as _));
    drop(done_tx);
    // The deadline covers the child and everything holding its output: a descendant that
    // outlives the leader keeps the pipes open, and must not extend the bound.
    let deadline = Instant::now() + timeout;
    let mut status = None;
    let mut drained = 0;
    let finished = loop {
        if status.is_none() {
            status = child.try_wait()?;
        }
        while done_rx.try_recv().is_ok() {
            drained += 1;
        }
        if status.is_some() && drained == 2 {
            break status;
        }
        if Instant::now() >= deadline {
            break None;
        }
        if drained < 2 {
            let _ = done_rx
                .recv_timeout(Duration::from_millis(10))
                .map(|_| drained += 1);
        } else {
            // Both pipes are closed (every sender is gone): wait for the leader without spinning.
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    let Some(status) = finished else {
        // SAFETY: killpg of the child's own group (it was spawned with process_group(0)); the
        // group outlives its leader while a descendant is alive.
        unsafe { libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL) };
        let _ = child.kill();
        let _ = child.wait();
        // Not joined: a descendant outside the group may still hold the pipes, and the drain
        // threads end by themselves when they close.
        drop((out, err));
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "{}: timed out after {} s",
                command_line(program, args),
                timeout.as_secs_f64().ceil() as u64
            ),
        ));
    };
    let ran = Ran {
        ok: status.success(),
        stdout: String::from_utf8_lossy(&out.join().unwrap_or_default())
            .trim()
            .to_string(),
    };
    let stderr = String::from_utf8_lossy(&err.join().unwrap_or_default())
        .trim()
        .to_string();
    Ok((ran, stderr))
}

/// Runs a command and turns a non-zero exit into `io::Error::other("<cmd>: <stderr>")`.
pub(crate) fn run_ok(program: &Path, args: &[&str], timeout: Duration) -> io::Result<()> {
    let (ran, stderr) = run(program, args, timeout)?;
    if ran.ok {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{}: {stderr}",
            command_line(program, args)
        )))
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{Duration, Instant};

    /// Writes an executable `sh` script and waits until it can be executed: a script written
    /// while another test thread forks can briefly fail with "Text file busy" (ETXTBSY). The
    /// script answers `__probe` by exiting at once (same approach as the integration tests'
    /// `common::write_script`).
    pub fn write_script(path: &Path, body: &str) {
        fs::write(
            path,
            format!("#!/bin/sh\n[ \"$1\" = __probe ] && exit 0\n{body}\n"),
        )
        .unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match Command::new(path).arg("__probe").status() {
                Ok(st) => {
                    assert!(st.success(), "probe of {} failed", path.display());
                    return;
                }
                Err(e) if e.raw_os_error() == Some(26) && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("cannot run {}: {e}", path.display()),
            }
        }
    }

    /// A fake program that appends its arguments to `<dir>/log`, prints `stdout` and exits
    /// with `code` (stderr `boom` when non-zero).
    pub fn fake(dir: &Path, name: &str, stdout: &str, code: i32) -> PathBuf {
        let path = dir.join(name);
        write_script(
            &path,
            &format!(
                "echo \"$@\" >> '{}'\necho '{stdout}'\n[ {code} -ne 0 ] && echo boom >&2\nexit {code}",
                dir.join("log").display()
            ),
        );
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn env_of(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |k| {
            vars.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| OsString::from(*v))
        }
    }

    #[test]
    fn unit_file_follows_xdg_config_home() {
        let sock = Path::new("/run/u/twydo/tdo.sock");
        let e = env_of(&[("XDG_CONFIG_HOME", "/c"), ("XDG_RUNTIME_DIR", "/run/u")]);
        let u = unit_for(Platform::Systemd, sock, &e);
        assert_eq!(u.file, PathBuf::from("/c/systemd/user/twydo.service"));
        let e = env_of(&[("HOME", "/h"), ("XDG_RUNTIME_DIR", "/run/u")]);
        let u = unit_for(Platform::Systemd, sock, &e);
        assert_eq!(
            u.file,
            PathBuf::from("/h/.config/systemd/user/twydo.service")
        );
        assert!(u.default_socket);
        let u = unit_for(Platform::Launchd, sock, &e);
        assert_eq!(
            u.file,
            PathBuf::from("/h/Library/LaunchAgents/app.twydo.daemon.plist")
        );
        assert_eq!(u.name, "app.twydo.daemon");
    }

    #[test]
    fn stable_exe_skips_versioned_installs() {
        let dir = tempfile::tempdir().unwrap();
        let ver = dir.path().join("m/installs/twydo/0.3.0");
        let stable = dir.path().join("usr/local/bin");
        let shims = dir.path().join("shims");
        for d in [&ver, &stable, &shims] {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(ver.join("tdo"), "").unwrap();
        fs::write(stable.join("tdo"), "").unwrap();
        let both = std::env::join_paths([&ver, &stable]).unwrap();
        assert_eq!(stable_exe(Some(&both), &shims).unwrap(), stable.join("tdo"));
        let only = std::env::join_paths([&ver]).unwrap();
        let err = stable_exe(Some(&only), &shims).unwrap_err();
        assert_eq!(
            err,
            format!(
                "no stable path to tdo (found only {}); run 'mise reshim' or pass --exe <path>",
                ver.join("tdo").display()
            )
        );
        fs::write(shims.join("tdo"), "").unwrap();
        assert_eq!(stable_exe(Some(&only), &shims).unwrap(), shims.join("tdo"));
    }

    #[test]
    fn service_env_keeps_path_and_set_xdg_vars() {
        let e = env_of(&[
            ("XDG_DATA_HOME", "/d"),
            ("PATH", "/usr/bin"),
            ("XDG_CONFIG_HOME", "/c"),
            ("HOME", "/h"),
        ]);
        assert_eq!(
            service_env(&e),
            vec![
                ("PATH".to_string(), "/usr/bin".to_string()),
                ("XDG_CONFIG_HOME".to_string(), "/c".to_string()),
                ("XDG_DATA_HOME".to_string(), "/d".to_string()),
            ]
        );
    }

    #[test]
    fn manager_runs_systemctl_with_user() {
        let dir = tempfile::tempdir().unwrap();
        let prog = testutil::fake(dir.path(), "systemctl", "", 0);
        let m = systemd::Systemd {
            program: prog,
            unit_dir: PathBuf::new(),
            timeout: MANAGER_TIMEOUT,
        };
        let u = unit_for(
            Platform::Systemd,
            Path::new("/s"),
            &env_of(&[("HOME", "/h")]),
        );
        let u = Unit {
            name: "twydo.service".into(),
            ..u
        };
        m.enable_now(&u).unwrap();
        let log = fs::read_to_string(dir.path().join("log")).unwrap();
        assert_eq!(log.trim(), "--user enable --now twydo.service");

        let bad = tempfile::tempdir().unwrap();
        let prog = testutil::fake(bad.path(), "systemctl", "", 1);
        let err = systemd::Systemd {
            program: prog,
            unit_dir: PathBuf::new(),
            timeout: MANAGER_TIMEOUT,
        }
        .enable_now(&u)
        .unwrap_err();
        assert!(err.to_string().contains("boom"), "{err}");
    }
}
