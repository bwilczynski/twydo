//! Versioned, bounded subprocess transport for the bundled collection helper.

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
}

pub fn invoke(cfg: &Config, request: &HelperRequest) -> Result<HelperReply, ListError> {
    let sync = cfg.sync.as_ref().ok_or_else(|| {
        ListError::Refused("list management needs a configured CalDAV sync pair".into())
    })?;
    let payload = serde_json::to_vec(request).map_err(std::io::Error::other)?;
    if payload.len() > 16 * 1024 {
        return Err(ListError::Refused(
            "collection helper request is too large".into(),
        ));
    }
    let mut command = Command::new(&sync.list_python);
    command.args(["-I", "-c", SOURCE]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn().map_err(|_| {
        ListError::Refused("cannot start collection helper; check [sync].list_python".into())
    })?;
    let mut input = child.stdin.take().expect("piped stdin");
    std::thread::spawn(move || {
        let _ = input.write_all(&payload);
    });
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
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                #[cfg(unix)]
                crate::sync::running::kill_group(child.id());
                let _ = child.kill();
                let _ = child.wait();
                return Err(ListError::Refused("collection helper timed out or could not be monitored; reconcile before retrying".into()));
            }
        }
    };
    let bytes = rx
        .recv_timeout(Duration::from_secs(1))
        .ok()
        .and_then(Result::ok)
        .ok_or_else(|| ListError::Refused("collection helper output was incomplete".into()))?;
    if bytes.len() as u64 > MAX_REPLY {
        return Err(ListError::Refused(
            "collection helper reply is too large".into(),
        ));
    }
    let reply: HelperReply = serde_json::from_slice(&bytes)
        .map_err(|_| ListError::Refused("collection helper returned an invalid response".into()))?;
    if reply.version != 1 {
        return Err(ListError::Refused(
            "collection helper protocol mismatch".into(),
        ));
    }
    if !reply.ok || !status.success() {
        // Only our protocol's known error codes are displayable. Never forward arbitrary
        // helper/credential output, even when it happens to be valid JSON.
        let reason = match reply.code.as_str() {
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
        };
        return Err(ListError::Refused(reason.into()));
    }
    Ok(reply)
}
