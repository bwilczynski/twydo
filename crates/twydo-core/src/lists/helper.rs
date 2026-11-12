//! Versioned, bounded subprocess transport for the bundled collection helper.

use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Config;

use super::ListError;

const SOURCE: &str = include_str!("helper.py");
const MAX_REPLY: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelperRequest {
    pub version: u32,
    pub action: String,
    pub pair: String,
    pub local_storage: String,
    pub vdir: PathBuf,
    pub config: Option<PathBuf>,
    pub collection: Option<String>,
    #[serde(default)]
    pub operation: Option<PathBuf>,
    #[serde(default)]
    pub state: Option<PathBuf>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub expected: Option<Value>,
    /// A `delete` request names the preview backup its server state is revalidated against.
    #[serde(default)]
    pub backup: Option<String>,
    #[serde(default = "execute_mode")]
    pub mode: String,
    #[serde(default)]
    pub keep_server: bool,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_timeout() -> u64 {
    300
}

fn execute_mode() -> String {
    "execute".into()
}

impl HelperRequest {
    /// `state` is the vdir's state directory: the helper takes the shared vdir lock there.
    pub fn discover(cfg: &Config, state: &std::path::Path) -> Result<Self, ListError> {
        let mut request = Self::probe(cfg)?;
        request.action = "discover".into();
        request.state = Some(std::path::absolute(state)?);
        Ok(request)
    }

    pub fn probe(cfg: &Config) -> Result<Self, ListError> {
        let sync = cfg.sync.as_ref().ok_or_else(|| {
            ListError::Refused("list management needs a configured CalDAV sync pair".into())
        })?;
        Ok(Self {
            version: 1,
            action: "probe".into(),
            pair: sync.pair.clone(),
            local_storage: sync.local_storage.clone(),
            vdir: std::path::absolute(&cfg.vdir)?,
            config: sync
                .vdirsyncer_config
                .as_ref()
                .map(std::path::absolute)
                .transpose()?,
            collection: None,
            operation: None,
            state: None,
            name: None,
            color: None,
            expected: None,
            backup: None,
            mode: execute_mode(),
            keep_server: false,
            timeout_secs: sync.timeout_secs.max(1),
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct HelperReply {
    pub version: u32,
    pub ok: bool,
    #[serde(default)]
    pub result: Value,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub missing: Vec<String>,
}

pub fn invoke(cfg: &Config, request: &HelperRequest) -> Result<HelperReply, ListError> {
    let reply = exchange(cfg, request)?;
    if reply.ok {
        Ok(reply)
    } else {
        Err(ListError::Refused(refusal(&reply.code).into()))
    }
}

/// Observes a helper child so that its caller can record it and stop it on request.
pub(crate) trait Watch {
    /// The child, which leads a process group of its own, started.
    fn started(&self, _pgid: u32) {}
    /// The child is gone.
    fn finished(&self) {}
    /// Whether the child is to be killed now (also asked before it is started).
    fn stop_requested(&self) -> bool {
        false
    }
}

pub(crate) struct Unwatched;
impl Watch for Unwatched {}

/// Why a helper exchange gave no reply.
pub(crate) enum ExchangeError {
    /// The helper cannot run at all (its interpreter does not start).
    Unavailable(String),
    /// [`Watch::stop_requested`] killed it.
    Stopped,
    /// Killed at its deadline.
    TimedOut(ListError),
    /// Died without a reply (a signal), so it could not roll anything back.
    Killed(ListError),
    Failed(ListError),
}

impl From<ListError> for ExchangeError {
    fn from(e: ListError) -> Self {
        Self::Failed(e)
    }
}

impl From<ExchangeError> for ListError {
    fn from(e: ExchangeError) -> Self {
        match e {
            ExchangeError::Unavailable(m) => ListError::Refused(m),
            ExchangeError::Stopped => ListError::Refused("collection helper was stopped".into()),
            ExchangeError::TimedOut(e) | ExchangeError::Killed(e) => e,
            ExchangeError::Failed(e) => e,
        }
    }
}

/// Transport only: runs the helper and returns its reply, ok or not.
pub(crate) fn exchange(cfg: &Config, request: &HelperRequest) -> Result<HelperReply, ListError> {
    Ok(exchange_watched(cfg, request, &Unwatched, None)?)
}

/// [`exchange`], reporting the child to `watch` and killing it (its whole process group) once
/// `watch` asks for a stop.
///
/// With a `lock` (a held shared vdir lock, see `VdirLock::shared_for_child`) the child
/// inherits it as its stdin and holds it for as long as it lives, even when this process is
/// killed; the request then travels in the environment (`TWYDO_HELPER_REQUEST`) instead.
pub(crate) fn exchange_watched(
    cfg: &Config,
    request: &HelperRequest,
    watch: &dyn Watch,
    lock: Option<File>,
) -> Result<HelperReply, ExchangeError> {
    let sync = cfg.sync.as_ref().ok_or_else(|| {
        ListError::Refused("list management needs a configured CalDAV sync pair".into())
    })?;
    let payload = serde_json::to_vec(request)
        .map_err(std::io::Error::other)
        .map_err(ListError::from)?;
    if payload.len() > 16 * 1024 {
        return Err(ListError::Refused("collection helper request is too large".into()).into());
    }
    if watch.stop_requested() {
        return Err(ExchangeError::Stopped);
    }
    let mut command = Command::new(&sync.list_python);
    command.args(["-I", "-c", SOURCE]);
    let piped = lock.is_none();
    match lock {
        Some(lock) => {
            command.env(
                "TWYDO_HELPER_REQUEST",
                String::from_utf8_lossy(&payload).as_ref(),
            );
            command.stdin(Stdio::from(lock));
        }
        None => {
            // A stray value must not replace the request on stdin.
            command.env_remove("TWYDO_HELPER_REQUEST");
            command.stdin(Stdio::piped());
        }
    }
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn().map_err(|_| {
        ExchangeError::Unavailable(
            "cannot start collection helper; check [sync].list_python".into(),
        )
    })?;
    watch.started(child.id());
    let finish = |child: &mut std::process::Child| {
        #[cfg(unix)]
        crate::sync::running::kill_group(child.id());
        let _ = child.kill();
        let _ = child.wait();
        watch.finished();
    };
    if piped {
        let mut input = child.stdin.take().expect("piped stdin");
        std::thread::spawn(move || {
            let _ = input.write_all(&payload);
        });
    }
    let output = child.stdout.take().expect("piped stdout");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = output
            .take(MAX_REPLY + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = tx.send(result);
    });
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(sync.timeout_secs))
        .unwrap_or_else(|| Instant::now() + Duration::from_secs(365 * 24 * 3600));
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if watch.stop_requested() => {
                finish(&mut child);
                return Err(ExchangeError::Stopped);
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                finish(&mut child);
                return Err(ExchangeError::TimedOut(ListError::Refused(
                    if request.action == "discover" {
                        "list discovery timed out or could not be monitored; it will be retried at the next discovery".into()
                    } else {
                        "collection helper timed out or could not be monitored; reconcile before retrying".into()
                    },
                )));
            }
        }
    };
    watch.finished();
    let incomplete = || ListError::Refused("collection helper output was incomplete".into());
    let read = rx
        .recv_timeout(Duration::from_secs(1))
        .ok()
        .and_then(Result::ok);
    // Killed by a signal with no reply: decided whether or not the reader finished, as a
    // grandchild holding stdout open must not turn it into a plain failure.
    if status.code().is_none() && read.as_ref().is_none_or(Vec::is_empty) {
        return Err(ExchangeError::Killed(incomplete()));
    }
    let bytes = read.ok_or_else(incomplete)?;
    // An interpreter that starts and exits with an error but no reply is a setup problem
    // (a broken venv, a wrapper rejecting `-I`), not a verdict from the helper. A signal
    // (its own alarm) is not.
    if bytes.is_empty() && status.code().is_some_and(|c| c != 0) {
        return Err(ExchangeError::Unavailable(
            "collection helper exited without a reply; check [sync].list_python".into(),
        ));
    }
    if bytes.len() as u64 > MAX_REPLY {
        return Err(ListError::Refused("collection helper reply is too large".into()).into());
    }
    let reply: HelperReply = serde_json::from_slice(&bytes)
        .map_err(|_| ListError::Refused("collection helper returned an invalid response".into()))?;
    if reply.version != 1 {
        return Err(ListError::Refused("collection helper protocol mismatch".into()).into());
    }
    if !status.success() && reply.ok {
        return Err(ListError::Refused(refusal("").into()).into());
    }
    Ok(reply)
}

/// Only our protocol's known error codes are displayable. Never forward arbitrary
/// helper/credential output, even when it happens to be valid JSON.
pub(crate) fn refusal(code: &str) -> &'static str {
    match code {
        "protocol" => "collection helper protocol/request is invalid",
        "dependency" => "collection helper needs vdirsyncer 0.21.x; check [sync].list_python",
        "mapping" => "collection helper could not validate this pair and exact list identity",
        "dav" => "collection helper could not verify the CalDAV response",
        "inventory" => {
            "cannot back up the complete remote list: inventory, strong resource ETags or resource reads could not be verified"
        }
        "operation" => {
            "collection helper refused stale or mismatched operation state; reconcile it"
        }
        "stale" => "list changed on the server; sync and review your changes before saving",
        "config" => {
            "cannot load the collection pair; check vdirsyncer_config, pair and local_storage"
        }
        "credentials" => {
            "collection credential fetch failed; check the existing vdirsyncer credential helper"
        }
        "exists" => "list identity already exists on the server",
        "choice" => {
            "server values differ from the requested save; review them or explicitly keep server values"
        }
        "discovery" => {
            "collection discovery found missing lists; resolve them before retrying (no unrelated list was created)"
        }
        "timeout" => {
            "collection helper timed out; reconcile its recorded operation before retrying"
        }
        _ => "collection helper failed; inspect its operation before retrying",
    }
}
