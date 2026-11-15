//! The generated vdirsyncer config: Twydo owns the CalDAV connection settings and renders
//! the file vdirsyncer reads from `[sync]`.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::SyncError;
use super::hook::{HOOK_LOG_ENV, VDIR_ENV};
use super::running;
use crate::config::{Config, SyncConfig};
use crate::store::fsio::{VdirLock, write_atomic};

pub const PAIR: &str = "twydo";
pub const LOCAL_STORAGE: &str = "twydo_local";
pub const REMOTE_STORAGE: &str = "twydo_remote";
/// Test-only seam: when set, the remote storage is a filesystem directory instead of CalDAV.
/// The daemon and CLI processes both generate the config, so they must see the same value.
pub const TEST_REMOTE_ENV: &str = "TWYDO_TEST_REMOTE_DIR";

/// Grace for the stderr reader threads after the child is gone (a grandchild may still
/// hold the pipe).
const DRAIN_GRACE: Duration = Duration::from_secs(1);
/// How often a running child is polled for exit.
const POLL: Duration = Duration::from_millis(25);

/// What a caller lets vdirsyncer's runner do about the child's life: record it, and ask
/// whether to stop.
pub(crate) trait Control {
    /// The child (a process group leader) is running.
    fn started(&self, _pid: u32) {}
    /// The child is gone.
    fn finished(&self) {}
    /// Whether the child must not start, or must be killed now.
    fn stop_requested(&self) -> bool {
        false
    }
}

/// For callers with no running record and no stop requests.
// Used by list operations outside a sync run (a later task).
#[allow(dead_code)]
pub(crate) struct Uncontrolled;

impl Control for Uncontrolled {}

pub(crate) enum Outcome {
    Exited(ExitStatus),
    TimedOut,
    /// Killed on request (`sync/stop`).
    Stopped,
    SpawnFailed(String),
}

pub(crate) struct Ran {
    pub(crate) stderr: String,
    pub(crate) outcome: Outcome,
}

/// Starts a thread that reads `r` to the end and sends what it read.
fn reader<R: Read + Send + 'static>(mut r: R) -> Result<mpsc::Receiver<String>, SyncError> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("sync-output".into())
        .spawn(move || {
            let mut buf = Vec::new();
            let _ = r.read_to_end(&mut buf);
            let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
        })
        .map_err(|e| SyncError::Spawn(format!("cannot start an output reader: {e}")))?;
    Ok(rx)
}

/// Kills `child` and, on Unix, its whole process group (it leads one: see
/// [`Vdirsyncer::run`]), so a wrapper's vdirsyncer cannot keep writing the vdir after the
/// runner gives up its shared lock. No libc here: the group goes through `kill(1)` (see
/// [`running::kill_group`]); the direct kill is the fallback.
fn kill_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    running::kill_group(child.id());
    let _ = child.kill();
}

/// Every vdirsyncer invocation: the only place that knows its command line.
pub(crate) struct Vdirsyncer<'a> {
    pub sync: &'a SyncConfig,
    pub config: &'a Path,
    pub vdir: &'a Path,
    pub state: &'a Path,
    /// Set as `TWYDO_SYNC_HOOKLOG` for the conflict hook; none outside a sync run.
    pub hook_log: Option<&'a Path>,
}

impl Vdirsyncer<'_> {
    /// `vdirsyncer discover <pair>`.
    pub(crate) fn discover(&self, c: &dyn Control) -> Result<Ran, SyncError> {
        self.run(&["discover".into(), PAIR.to_string()], c)
    }

    /// `vdirsyncer sync <pair>`, or with `force` `sync --force-delete <pair>/<collection>…`.
    pub(crate) fn sync(&self, force: &[String], c: &dyn Control) -> Result<Ran, SyncError> {
        let mut args = vec!["sync".to_string()];
        if force.is_empty() {
            args.push(PAIR.to_string());
        } else {
            args.push("--force-delete".into());
            args.extend(force.iter().map(|f| format!("{PAIR}/{f}")));
        }
        self.run(&args, c)
    }

    /// `vdirsyncer metasync <pair>`.
    pub(crate) fn metasync(&self, c: &dyn Control) -> Result<Ran, SyncError> {
        self.run(&["metasync".into(), PAIR.to_string()], c)
    }

    /// Runs `<command> args…` with `VDIRSYNCER_CONFIG`, the hook log (when there is one) and
    /// the vdir in the environment. Its stdin is a shared lock on the vdir of its own (the
    /// lock file is empty, so it reads EOF as from null): the kernel keeps that lock until
    /// the child and whatever inherited it exit, so a child orphaned by a killed runner
    /// (Ctrl-C) still keeps writers and the next run's recovery waiting. stdout is drained
    /// and dropped; stderr is returned. The child is killed after `timeout_secs`, or when a
    /// stop is requested (see [`Control`]); with a stop already pending it is not started.
    fn run(&self, args: &[String], c: &dyn Control) -> Result<Ran, SyncError> {
        let sync = self.sync;
        if c.stop_requested() {
            return Ok(Ran {
                stderr: String::new(),
                outcome: Outcome::Stopped,
            });
        }
        let lock = VdirLock::shared_for_child(self.state)?;
        let mut cmd = Command::new(&sync.command);
        cmd.args(args);
        if let Some(log) = self.hook_log {
            cmd.env(HOOK_LOG_ENV, log);
        }
        cmd.env(VDIR_ENV, self.vdir)
            .stdin(Stdio::from(lock))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.env("VDIRSYNCER_CONFIG", self.config);
        // Its own process group, so a timeout also kills what a wrapper script started. A
        // Ctrl-C at the terminal therefore reaches only tdo; the stdin lock covers that.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let mut child = match cmd.spawn() {
            Ok(ch) => ch,
            Err(e) => {
                return Ok(Ran {
                    stderr: String::new(),
                    outcome: Outcome::SpawnFailed(format!("cannot run {}: {e}", sync.command)),
                });
            }
        };
        c.started(child.id());
        let readers = (
            child.stdout.take().map(reader).transpose(),
            child.stderr.take().map(reader).transpose(),
        );
        let (out_rx, err_rx) = match readers {
            (Ok(o), Ok(e)) => (o, e),
            (Err(err), _) | (_, Err(err)) => {
                kill_tree(&mut child);
                let _ = child.wait();
                return Err(err);
            }
        };
        // A timeout too large for `Instant` is ~forever (a year is plenty).
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(sync.timeout_secs))
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(365 * 24 * 3600));
        let outcome = loop {
            if let Some(st) = child.try_wait()? {
                break Outcome::Exited(st);
            }
            if c.stop_requested() {
                kill_tree(&mut child);
                child.wait()?;
                break Outcome::Stopped;
            }
            if Instant::now() >= deadline {
                kill_tree(&mut child);
                child.wait()?;
                break Outcome::TimedOut;
            }
            std::thread::sleep(POLL);
        };
        c.finished();
        // After a normal exit, wait for the rest of stderr until the deadline: a run whose
        // output is incomplete must not look clean. After a kill, give it a short grace only.
        let wait = match outcome {
            Outcome::Exited(_) => deadline.saturating_duration_since(Instant::now()),
            _ => Duration::ZERO,
        }
        .max(DRAIN_GRACE);
        if let Some(rx) = out_rx {
            let _ = rx.recv_timeout(wait);
        }
        let (stderr, outcome) = match err_rx.map(|rx| rx.recv_timeout(wait)) {
            None => (String::new(), outcome),
            Some(Ok(s)) => (s, outcome),
            Some(Err(_)) => (String::new(), Outcome::TimedOut),
        };
        Ok(Ran { stderr, outcome })
    }
}

/// Where the generated config lives under the per-vdir state directory.
pub fn config_path(vstate: &Path) -> PathBuf {
    vstate.join("vdirsyncer/config")
}

fn q(s: &str) -> String {
    serde_json::to_string(s).expect("string serializes")
}

fn dir_q(p: &Path) -> String {
    q(&format!("{}/", p.to_string_lossy().trim_end_matches('/')))
}

/// Renders the vdirsyncer config for `sync`.
pub fn render(vdir: &Path, sync: &SyncConfig, vstate: &Path, test_remote: Option<&Path>) -> String {
    let mut out = String::new();
    out.push_str("# Generated by Twydo from config.toml; do not edit, it is overwritten.\n");
    out.push_str("[general]\n");
    out.push_str(&format!(
        "status_path = {}\n\n",
        dir_q(&vstate.join("vdirsyncer/status"))
    ));
    out.push_str(&format!("[pair {PAIR}]\n"));
    out.push_str(&format!("a = {}\n", q(REMOTE_STORAGE)));
    out.push_str(&format!("b = {}\n", q(LOCAL_STORAGE)));
    out.push_str("collections = [\"from a\", \"from b\"]\n");
    out.push_str("metadata = [\"displayname\", \"color\"]\n");
    out.push_str("conflict_resolution = [\"command\", \"tdo\", \"conflicts\", \"hook\"]\n\n");
    out.push_str(&format!("[storage {LOCAL_STORAGE}]\n"));
    out.push_str("type = \"filesystem\"\n");
    out.push_str(&format!("path = {}\n", dir_q(vdir)));
    out.push_str("fileext = \".ics\"\n\n");
    out.push_str(&format!("[storage {REMOTE_STORAGE}]\n"));
    if let Some(remote) = test_remote {
        out.push_str("type = \"filesystem\"\n");
        out.push_str(&format!("path = {}\n", dir_q(remote)));
        out.push_str("fileext = \".ics\"\n");
    } else {
        out.push_str("type = \"caldav\"\n");
        out.push_str(&format!("url = {}\n", q(&sync.url)));
        out.push_str(&format!("username = {}\n", q(&sync.username)));
        let words: Vec<String> = std::iter::once("command")
            .chain(sync.password_command.iter().map(String::as_str))
            .map(q)
            .collect();
        out.push_str(&format!("password.fetch = [{}]\n", words.join(", ")));
        if let Some(ca) = &sync.ca_file {
            out.push_str(&format!("verify = {}\n", q(&ca.to_string_lossy())));
        }
    }
    out
}

/// Writes the generated config if it is missing or differs, first leaving the rediscover
/// marker so the next sync discovers collections again. Returns the config path. Reads
/// [`TEST_REMOTE_ENV`] (a test-only seam): every process that calls this must agree on it.
pub fn ensure(cfg: &Config, sync: &SyncConfig, vstate: &Path) -> io::Result<PathBuf> {
    let test_remote = std::env::var_os(TEST_REMOTE_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let text = render(&cfg.vdir, sync, vstate, test_remote.as_deref());
    let path = config_path(vstate);
    if std::fs::read(&path).is_ok_and(|old| old == text.as_bytes()) {
        return Ok(path);
    }
    let marker = vstate.join("sync/rediscover");
    std::fs::create_dir_all(vstate.join("sync"))?;
    write_atomic(&marker, b"")?;
    std::fs::create_dir_all(vstate.join("vdirsyncer"))?;
    write_atomic(&path, text.as_bytes())?;
    Ok(path)
}
