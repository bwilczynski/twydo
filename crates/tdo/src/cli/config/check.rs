//! `tdo config check`: whether this machine is ready to sync. Checks only this machine; it
//! never contacts the server and never writes (no state is bound, no file created).

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use twydo_core::config::Config;
use twydo_core::store::existing_vdir_state;
use twydo_core::sync::{list_python, vdirsyncer};

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

const NAMES: [&str; 9] = [
    "config",
    "vdir",
    "sync",
    "vdirsyncer",
    "tdo on PATH",
    "list helper",
    "password",
    "generated config",
    "ca file",
];

struct Builder {
    steps: Vec<Step>,
}

impl Builder {
    fn push(&mut self, name: &'static str, status: Status, detail: String, hint: Option<String>) {
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

    fn done(self) -> Report {
        Report { steps: self.steps }
    }
}

pub fn run(path: &Path) -> Report {
    let mut b = Builder { steps: Vec::new() };

    let cfg = match Config::load(path) {
        Ok(c) => {
            b.pass("config", path.display().to_string());
            c
        }
        Err(e) => {
            b.fail("config", e.to_string(), INIT_HINT);
            b.skip_rest("skipped: config failed");
            return b.done();
        }
    };

    let vdir_ok = cfg.vdir.is_dir();
    if vdir_ok {
        b.pass("vdir", cfg.vdir.display().to_string());
    } else {
        b.fail(
            "vdir",
            format!("{} is not a directory", cfg.vdir.display()),
            "create it, or rerun tdo config init",
        );
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

    // tdo on PATH: the conflict hook runs `tdo` by name
    match twydo_core::which::on_path("tdo") {
        Some(p) => b.pass("tdo on PATH", p.display().to_string()),
        None => b.fail(
            "tdo on PATH",
            "no executable named tdo on PATH",
            "the conflict hook runs `tdo` by name: put it on the PATH the daemon and vdirsyncer run with",
        ),
    }

    // list helper
    let step = "list helper";
    let python = list_python::resolve(sync);
    let interpreter = match python.source {
        list_python::Source::Configured => python.program.clone(),
        _ => python.to_string(),
    };
    // `-I` as the helper runs it: a user site or PYTHONPATH must not make the check pass.
    match probe(
        &python.program,
        &[
            "-I",
            "-c",
            "import vdirsyncer; print(vdirsyncer.__version__)",
        ],
        |out| out.lines().next().map(|l| l.trim().to_string()),
    ) {
        Ok(v) if v.starts_with("0.21.") => b.pass(step, format!("{interpreter} vdirsyncer {v}")),
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

    // password
    match run_password(&sync.password_command) {
        Ok(()) => b.pass("password", "password_command printed a password"),
        Err(e) => {
            let hint = if e == MULTILINE_PASSWORD {
                "vdirsyncer uses the whole output as the password; print only the password (e.g. [\"sh\", \"-c\", \"pass show x | head -n1\"])"
            } else {
                "run the command yourself and check it prints only the password"
            };
            b.fail("password", e, hint)
        }
    }

    // generated config (depends on the vdir)
    if !vdir_ok {
        b.skip("generated config", "skipped: vdir failed");
    } else {
        match existing_vdir_state(&cfg.state_dir(), &cfg.vdir) {
            Ok(Some(vstate)) => match state_writable(&vstate) {
                Ok(()) => b.pass(
                    "generated config",
                    vdirsyncer::config_path(&vstate).display().to_string(),
                ),
                Err(e) => b.fail(
                    "generated config",
                    e,
                    "make the state directory writable by this user",
                ),
            },
            Ok(None) => b.pass("generated config", "written on first sync"),
            Err(e) => b.fail(
                "generated config",
                e.to_string(),
                "check the state directory",
            ),
        }
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
    // Version probes get their own process group so a timeout kills what a wrapper started.
    // A command that may read the terminal stays in ours: a background group would be
    // stopped by SIGTTIN on its first tty read and would not see Ctrl-C.
    let own_group = !inherit_stdin;
    let mut command = Command::new(cmd);
    command.args(args);
    if own_group {
        command.process_group(0);
    }
    let mut child = command
        .stdin(if inherit_stdin {
            Stdio::inherit()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("{cmd}: not found")
            } else {
                format!("{cmd}: {e}")
            }
        })?;
    let pgid = child.id() as i32;
    let out = Drain::spawn(child.stdout.take());
    let err = Drain::spawn(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() >= deadline => {
                if own_group {
                    // SAFETY: signalling the process group this function created.
                    unsafe { libc::kill(-pgid, libc::SIGKILL) };
                } else {
                    let _ = child.kill();
                }
                let _ = child.wait();
                return Err(format!("{cmd}: timed out after {} s", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{cmd}: {e}"));
            }
        }
    };
    // A grandchild (an agent the command started) may keep the pipes open, so EOF may never
    // come: wait for it within a short grace period, then use whatever was read so far.
    let grace = Instant::now() + Duration::from_secs(2);
    Ok(Output {
        status,
        stdout: out.collect(grace),
        stderr: err.collect(grace),
    })
}

/// A pipe read in the background into a shared buffer, so a partial read stays available
/// when the pipe never reaches EOF.
struct Drain {
    buf: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    eof: std::sync::mpsc::Receiver<()>,
}

impl Drain {
    fn spawn<R: Read + Send + 'static>(r: Option<R>) -> Self {
        let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (tx, eof) = std::sync::mpsc::channel();
        let shared = buf.clone();
        std::thread::spawn(move || {
            if let Some(mut r) = r {
                let mut chunk = [0u8; 4096];
                while let Ok(n) = r.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    shared.lock().unwrap().extend_from_slice(&chunk[..n]);
                }
            }
            let _ = tx.send(());
        });
        Drain { buf, eof }
    }

    /// Wait for EOF until `deadline`, then return everything read so far.
    fn collect(&self, deadline: Instant) -> String {
        let _ = self
            .eof
            .recv_timeout(deadline.saturating_duration_since(Instant::now()));
        String::from_utf8_lossy(&self.buf.lock().unwrap()).into_owned()
    }
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

/// Whether the vdir state (and its `vdirsyncer` subdirectory, if present) is a writable
/// directory. Uses `access(2)`, so nothing is written.
fn state_writable(vstate: &Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    let sub = vstate.join("vdirsyncer");
    let mut dirs = vec![vstate.to_path_buf()];
    if sub.exists() {
        dirs.push(sub);
    }
    for d in dirs {
        let md = std::fs::metadata(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        if !md.is_dir() {
            return Err(format!("{} is not a directory", d.display()));
        }
        let c = std::ffi::CString::new(d.as_os_str().as_bytes())
            .map_err(|e| format!("{}: {e}", d.display()))?;
        // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
        if unsafe { libc::access(c.as_ptr(), libc::W_OK) } != 0 {
            return Err(format!(
                "{}: {}",
                d.display(),
                std::io::Error::last_os_error()
            ));
        }
    }
    Ok(())
}

const MULTILINE_PASSWORD: &str = "password_command printed more than one line";

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
    // vdirsyncer 0.21 uses the whole stdout minus leading and trailing newlines.
    let pw = out.stdout.trim_matches('\n');
    if pw.is_empty() {
        Err("password_command printed no password".to_string())
    } else if pw.contains('\n') {
        Err(MULTILINE_PASSWORD.to_string())
    } else {
        Ok(())
    }
}
