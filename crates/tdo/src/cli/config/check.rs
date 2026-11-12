//! `tdo config check`: whether this machine is ready to sync. Checks only this machine; it
//! never contacts the server and never writes (no state is bound, no file created).

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use twydo_core::config::Config;
use twydo_core::store::existing_vdir_state;
use twydo_core::sync::vdirsyncer;

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const PASSWORD_TIMEOUT: Duration = Duration::from_secs(60);
const INIT_HINT: &str = "run tdo config init";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pass,
    Fail,
    Skip,
}

#[derive(Debug)]
pub struct Step {
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
    pub hint: Option<String>,
}

#[derive(Debug)]
pub struct Report {
    pub steps: Vec<Step>,
}

impl Report {
    pub fn failed(&self) -> bool {
        self.steps.iter().any(|s| s.status == Status::Fail)
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        for s in &self.steps {
            let mark = match s.status {
                Status::Pass => '✓',
                Status::Fail => '✗',
                Status::Skip => '–',
            };
            out.push_str(&format!("{mark} {:<17} {}\n", s.name, s.detail));
            if let Some(h) = &s.hint {
                // The detail column: mark, space, 17 name columns, space.
                out.push_str(&format!("{:20}hint: {h}\n", ""));
            }
        }
        out
    }
}

const NAMES: [&str; 8] = [
    "config",
    "vdir",
    "sync",
    "vdirsyncer",
    "list helper",
    "password",
    "generated config",
    "ca file",
];

struct Builder {
    steps: Vec<Step>,
    failed: Option<&'static str>,
}

impl Builder {
    fn push(&mut self, name: &'static str, status: Status, detail: String, hint: Option<String>) {
        if status == Status::Fail && self.failed.is_none() {
            self.failed = Some(name);
        }
        self.steps.push(Step {
            name,
            status,
            detail,
            hint,
        });
    }

    fn pass(&mut self, name: &'static str, detail: impl Into<String>) {
        self.push(name, Status::Pass, detail.into(), None);
    }

    fn fail(&mut self, name: &'static str, detail: impl Into<String>, hint: impl Into<String>) {
        self.push(name, Status::Fail, detail.into(), Some(hint.into()));
    }

    fn skip(&mut self, name: &'static str, detail: impl Into<String>) {
        self.push(name, Status::Skip, detail.into(), None);
    }

    /// Skip every step not yet reported with the reason.
    fn skip_rest(&mut self, detail: &str) {
        for name in NAMES.iter().skip(self.steps.len()) {
            self.skip(name, detail);
        }
    }

    /// True (after skipping the rest) when an earlier step failed.
    fn blocked(&mut self) -> bool {
        match self.failed {
            Some(f) => {
                self.skip_rest(&format!("skipped: {f} failed"));
                true
            }
            None => false,
        }
    }

    fn done(self) -> Report {
        Report { steps: self.steps }
    }
}

pub fn run(path: &Path) -> Report {
    let mut b = Builder {
        steps: Vec::new(),
        failed: None,
    };

    let cfg = match Config::load(path) {
        Ok(c) => {
            b.pass("config", path.display().to_string());
            c
        }
        Err(e) => {
            b.fail("config", e.to_string(), INIT_HINT);
            b.blocked();
            return b.done();
        }
    };

    if cfg.vdir.is_dir() {
        b.pass("vdir", cfg.vdir.display().to_string());
    } else {
        b.fail(
            "vdir",
            format!("{} is not a directory", cfg.vdir.display()),
            "create it, or rerun tdo config init",
        );
    }
    if b.blocked() {
        return b.done();
    }

    let Some(sync) = cfg.sync.as_ref() else {
        b.pass("sync", "sync off");
        b.skip_rest("sync off");
        return b.done();
    };
    b.pass("sync", "[sync] present");

    // vdirsyncer
    let step = "vdirsyncer";
    match probe(&sync.command, &["--version"], |out| {
        version_of(out, "version ")
    }) {
        Ok(v) if v.starts_with("0.21.") => b.pass(step, format!("{} {v}", sync.command)),
        Ok(v) => b.fail(
            step,
            format!("unsupported version {v} (need 0.21.x)"),
            "install vdirsyncer 0.21.x or set [sync].command",
        ),
        Err(e) => b.fail(step, e, "install vdirsyncer 0.21.x or set [sync].command"),
    }
    if b.blocked() {
        return b.done();
    }

    // list helper
    let step = "list helper";
    match probe(
        &sync.list_python,
        &["-c", "import vdirsyncer; print(vdirsyncer.__version__)"],
        |out| out.lines().next().map(|l| l.trim().to_string()),
    ) {
        Ok(v) if v.starts_with("0.21.") => {
            b.pass(step, format!("{} vdirsyncer {v}", sync.list_python))
        }
        Ok(v) => b.fail(
            step,
            format!("unsupported vdirsyncer version {v} (need 0.21.x)"),
            "install vdirsyncer 0.21.x for the interpreter or set [sync].list_python",
        ),
        Err(e) => b.fail(
            step,
            e,
            "install vdirsyncer 0.21.x for the interpreter or set [sync].list_python",
        ),
    }
    if b.blocked() {
        return b.done();
    }

    // password
    match run_password(&sync.password_command) {
        Ok(()) => b.pass("password", "password_command printed a password"),
        Err(e) => b.fail(
            "password",
            e,
            "check [sync].password_command by running it yourself",
        ),
    }
    if b.blocked() {
        return b.done();
    }

    // generated config
    match existing_vdir_state(&cfg.state_dir(), &cfg.vdir) {
        Ok(Some(vstate)) => b.pass(
            "generated config",
            vdirsyncer::config_path(&vstate).display().to_string(),
        ),
        Ok(None) => b.pass("generated config", "written on first sync"),
        Err(e) => b.fail(
            "generated config",
            e.to_string(),
            "check the state directory",
        ),
    }
    if b.blocked() {
        return b.done();
    }

    // ca file
    match &sync.ca_file {
        None => b.skip("ca file", "not set"),
        Some(p) => match std::fs::File::open(p) {
            Ok(_) if p.is_file() => b.pass("ca file", p.display().to_string()),
            Ok(_) => b.fail(
                "ca file",
                format!("{} is not a file", p.display()),
                "set [sync].ca_file to a PEM file",
            ),
            Err(e) => b.fail(
                "ca file",
                format!("{}: {e}", p.display()),
                "set [sync].ca_file to a readable PEM file",
            ),
        },
    }
    b.done()
}

/// The text after `marker` up to the next whitespace, when it looks like `d.d.d`.
fn version_of(out: &str, marker: &str) -> Option<String> {
    let rest = &out[out.find(marker)? + marker.len()..];
    let v = rest.split_whitespace().next()?;
    let mut parts = v.split('.');
    let ok = (0..3).all(|_| {
        parts
            .next()
            .is_some_and(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()))
    });
    ok.then(|| v.to_string())
}

struct Output {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

/// Run `cmd` with `args`, killing its process group after `timeout`. Stdin is inherited only
/// when `inherit_stdin`.
fn run_with_timeout(
    cmd: &str,
    args: &[&str],
    inherit_stdin: bool,
    timeout: Duration,
) -> Result<Output, String> {
    use std::os::unix::process::CommandExt;
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(if inherit_stdin {
            Stdio::inherit()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("{cmd}: not found")
            } else {
                format!("{cmd}: {e}")
            }
        })?;
    let pgid = child.id() as i32;
    let out_h = drain(child.stdout.take());
    let err_h = drain(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() >= deadline => {
                // SAFETY: signalling the process group this function created.
                unsafe { libc::kill(-pgid, libc::SIGKILL) };
                let _ = child.wait();
                return Err(format!("{cmd}: timed out after {} s", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("{cmd}: {e}")),
        }
    };
    Ok(Output {
        status,
        stdout: out_h.join().unwrap_or_default(),
        stderr: err_h.join().unwrap_or_default(),
    })
}

fn drain<R: Read + Send + 'static>(r: Option<R>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut r) = r {
            let _ = r.read_to_end(&mut buf);
        }
        String::from_utf8_lossy(&buf).into_owned()
    })
}

/// Run a version probe and parse its output; the error is the step's detail.
fn probe(
    cmd: &str,
    args: &[&str],
    parse: impl Fn(&str) -> Option<String>,
) -> Result<String, String> {
    let out = run_with_timeout(cmd, args, false, PROBE_TIMEOUT)?;
    if !out.status.success() {
        return Err(format!("{cmd} {}", describe_failure(&out)));
    }
    parse(&out.stdout)
        .or_else(|| parse(&out.stderr))
        .ok_or_else(|| format!("{cmd}: could not read a version from its output"))
}

fn describe_failure(out: &Output) -> String {
    let last = out
        .stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string());
    match last {
        Some(l) => format!("{}: {l}", out.status),
        None => out.status.to_string(),
    }
}

/// Run the password command; its stdout is read but never reported.
fn run_password(cmd: &[String]) -> Result<(), String> {
    let (prog, args) = cmd
        .split_first()
        .ok_or_else(|| "password_command is empty".to_string())?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = run_with_timeout(prog, &args, true, PASSWORD_TIMEOUT)?;
    if !out.status.success() {
        return Err(describe_failure(&out));
    }
    match out.stdout.lines().next() {
        Some(l) if !l.is_empty() => Ok(()),
        _ => Err("password_command printed no password".to_string()),
    }
}
