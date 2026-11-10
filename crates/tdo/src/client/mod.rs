//! Connecting to the daemon from `tdo`: finds it, starts it on demand and applies the
//! upgrade rule (daemon spec §6.1, §10.3). Nothing here reads the configuration file; only
//! its path matters, so a missing configuration is reported without starting anything.

pub mod start;

use std::io::ErrorKind;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use twydo_core::config::Config;
use twydo_daemon::client::{Client, ClientError, HELLO_TIMEOUT};
use twydo_daemon::paths;
use twydo_daemon::protocol::methods::{DaemonInfo, HelloParams, HelloResult, ShutdownParams};
use twydo_daemon::protocol::rpc::ErrorKind as RpcKind;

/// How long a CLI call may take before the client gives up on the daemon.
pub const CLI_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a freshly started daemon has to answer.
const START_WAIT: Duration = Duration::from_secs(2);
/// How long an old daemon has to finish its jobs and go after being asked to.
pub const STOP_WAIT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(100);

/// Which daemon to talk to: the socket, and the `--vdir` flag the instance check uses.
#[derive(Debug, Clone)]
pub struct Target {
    pub socket: PathBuf,
    pub vdir: Option<PathBuf>,
}

impl Target {
    /// From the global flags: `--socket` (else `TWYDO_SOCKET`, else the per-user default)
    /// and `--vdir`.
    pub fn from_flags(vdir: Option<&std::path::Path>, socket: Option<&std::path::Path>) -> Target {
        Target {
            socket: paths::socket_path(socket),
            vdir: vdir.map(std::path::Path::to_path_buf),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("no config at {}; pass --vdir, or create it with: vdir = \"<path to your vdir>\" (see docs/config.md)", .0.display())]
    NoConfig(PathBuf),
    #[error("{0}")]
    WrongInstance(String),
    #[error("tdo was upgraded; restart it")]
    TooOld { client: String, daemon: String },
    #[error("daemon not running: {0}")]
    Unavailable(String),
}

/// The version of this client.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The hello this client sends: its version, protocol and instance (no file is read).
pub fn hello_params(
    t: &Target,
    client: &str,
    session: Option<String>,
) -> Result<HelloParams, ConnectError> {
    let instance = paths::instance_for(&Config::path(), t.vdir.as_deref())
        .map_err(|e| ConnectError::Unavailable(e.to_string()))?;
    Ok(HelloParams {
        client: client.to_string(),
        version: VERSION.to_string(),
        protocol: twydo_daemon::PROTOCOL,
        session,
        instance,
    })
}

/// What one attempt to connect found.
enum Attempt {
    Connected(Client, HelloResult),
    /// No socket, or nothing accepting on it.
    Absent,
    /// Something accepted but did not answer hello in time: never started over, retried.
    Silent,
    Failed(ConnectError),
    /// The daemon speaks another protocol; its version, when it said, and the connection that
    /// received the refusal (lifecycle calls go on it).
    Protocol(Client, Option<String>),
}

/// Connects and says hello, starting the daemon when none answers and replacing one that is
/// older than this client (once it is idle). Never starts or stops anything for a daemon of
/// another instance, or for one newer than this client.
pub fn connect(
    t: &Target,
    client: &str,
    session: Option<String>,
) -> Result<(Client, HelloResult), ConnectError> {
    let config = Config::path();
    if !config.exists() && t.vdir.is_none() {
        return Err(ConnectError::NoConfig(config));
    }
    let hello = hello_params(t, client, session)?;
    let log = start::LogMark::take(&t.socket);

    let first = attempt(t, &hello);
    let (c, r) = match first {
        Attempt::Connected(c, r) => (c, r),
        Attempt::Failed(e) => return Err(e),
        Attempt::Absent => {
            start::detached(t)
                .map_err(|e| ConnectError::Unavailable(format!("cannot start it: {e}")))?;
            wait_for_daemon(t, &hello, &log, false)?
        }
        // Something is there: start nothing, ask again within the window.
        Attempt::Silent => wait_for_daemon(t, &hello, &log, false)?,
        // A `protocol` refusal means the daemon accepted our instance: it checks the
        // instance before the protocol in hello, so restarting it below is safe.
        Attempt::Protocol(c, daemon) => {
            return upgrade_or_refuse(t, &hello, &log, c, daemon);
        }
    };
    if newer(VERSION, &r.daemon) {
        // On the connection whose hello succeeded, not on a new one.
        return restart_on(Some(c), t, &hello, &log, Some(&r.epoch));
    }
    Ok((c, r))
}

/// A protocol mismatch: replace the daemon when this client is newer than it; refuse when
/// it is not, or when the daemon did not say its version (never stop what is not known to
/// be older).
fn upgrade_or_refuse(
    t: &Target,
    hello: &HelloParams,
    log: &start::LogMark,
    conn: Client,
    daemon: Option<String>,
) -> Result<(Client, HelloResult), ConnectError> {
    match daemon {
        Some(d) if newer(VERSION, &d) => restart_on(Some(conn), t, hello, log, None),
        other => Err(ConnectError::TooOld {
            client: VERSION.to_string(),
            daemon: other.unwrap_or_else(|| "unknown".into()),
        }),
    }
}

/// Replaces the daemon this client saw as older (`seen_epoch` is the epoch it saw, if it got
/// as far as hello): asks it to stop once idle, waits for its socket to go and starts a new
/// one. The daemon answering now is asked first: when it is no longer older than this client
/// or is not the one that was seen (another client already upgraded it), nothing is stopped
/// and the client just reconnects (spec §10.3: only a newer client restarts an older daemon).
pub fn restart(
    t: &Target,
    hello: &HelloParams,
    log: &start::LogMark,
    seen_epoch: Option<&str>,
) -> Result<(Client, HelloResult), ConnectError> {
    restart_on(
        Client::connect_raw(&t.socket).ok(),
        t,
        hello,
        log,
        seen_epoch,
    )
}

/// [`restart`] with the connection that was just checked by hello (`conn`), on which
/// `daemon.info` and `daemon.shutdown` are sent: a socket swapped in since then is never
/// asked anything. `None` (nothing was checked) connects once without hello.
fn restart_on(
    conn: Option<Client>,
    t: &Target,
    hello: &HelloParams,
    log: &start::LogMark,
    seen_epoch: Option<&str>,
) -> Result<(Client, HelloResult), ConnectError> {
    if let Some(raw) = conn {
        let info: Result<DaemonInfo, _> =
            raw.caller()
                .call_timeout("daemon.info", serde_json::json!({}), HELLO_TIMEOUT);
        let Ok(info) = info else {
            return Err(ConnectError::Unavailable(
                "the daemon did not report its version".into(),
            ));
        };
        if !newer(VERSION, &info.version) || seen_epoch.is_some_and(|e| e != info.epoch) {
            drop(raw);
            return wait_for_daemon(t, hello, log, false);
        }
        let _: Result<serde_json::Value, _> = raw.caller().call_timeout(
            "daemon.shutdown",
            ShutdownParams { when_idle: true },
            HELLO_TIMEOUT,
        );
        drop(raw);
        if !wait_gone(&t.socket, STOP_WAIT) {
            return Err(ConnectError::Unavailable(
                "the older daemon is still busy; try again once its job is done".into(),
            ));
        }
    }
    start::detached(t).map_err(|e| ConnectError::Unavailable(format!("cannot start it: {e}")))?;
    wait_for_daemon(t, hello, log, true)
}

/// Retries every 100 ms for up to 2 s. After an upgrade the old daemon may still hold its
/// lock for a moment after its socket went, so the new one is started once more halfway.
fn wait_for_daemon(
    t: &Target,
    hello: &HelloParams,
    log: &start::LogMark,
    restarted: bool,
) -> Result<(Client, HelloResult), ConnectError> {
    let begun = Instant::now();
    let mut silent: bool;
    let mut retried = !restarted;
    loop {
        match attempt(t, hello) {
            Attempt::Connected(c, r) => return Ok((c, r)),
            Attempt::Failed(e) => return Err(e),
            Attempt::Protocol(_, daemon) => {
                return Err(ConnectError::TooOld {
                    client: VERSION.to_string(),
                    daemon: daemon.unwrap_or_else(|| "unknown".into()),
                });
            }
            Attempt::Absent => silent = false,
            Attempt::Silent => silent = true,
        }
        if !retried && begun.elapsed() > START_WAIT / 2 {
            retried = true;
            let _ = start::detached(t);
        }
        if begun.elapsed() > START_WAIT {
            if silent {
                return Err(ConnectError::Unavailable(
                    "the daemon did not answer".into(),
                ));
            }
            let mut msg = format!("no daemon answered on {} within 2 s", t.socket.display());
            if let Some(line) = log.last_line() {
                msg.push_str(&format!(": {line}"));
            }
            return Err(ConnectError::Unavailable(msg));
        }
        std::thread::sleep(POLL);
    }
}

fn attempt(t: &Target, hello: &HelloParams) -> Attempt {
    let raw = match Client::connect_raw(&t.socket) {
        Ok(c) => c,
        Err(ClientError::Io(e))
            if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) =>
        {
            return Attempt::Absent;
        }
        Err(e) => return Attempt::Failed(ConnectError::Unavailable(e.to_string())),
    };
    match raw.hello_on(hello.clone()) {
        Ok(r) => Attempt::Connected(raw, r),
        Err(ClientError::TimedOut(_)) => Attempt::Silent,
        Err(ClientError::Rpc(e)) => match e.data.as_ref().map(|d| d.kind) {
            Some(RpcKind::WrongInstance) => Attempt::Failed(ConnectError::WrongInstance(e.message)),
            Some(RpcKind::Protocol) => Attempt::Protocol(raw, e.data.and_then(|d| d.daemon)),
            _ => Attempt::Failed(ConnectError::Unavailable(e.message)),
        },
        Err(e) => Attempt::Failed(ConnectError::Unavailable(e.to_string())),
    }
}

/// Waits until nothing answers on `socket` and its file is gone; false on timeout. A file
/// that nothing answers on but that stays (a crashed daemon's) counts as gone after 5 s.
pub fn wait_gone(socket: &std::path::Path, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    let mut stale_since: Option<Instant> = None;
    loop {
        match std::os::unix::net::UnixStream::connect(socket) {
            Err(e) if e.kind() == ErrorKind::NotFound => return true,
            Err(e) if e.kind() == ErrorKind::ConnectionRefused => {
                if !socket.exists() {
                    return true;
                }
                let since = *stale_since.get_or_insert_with(Instant::now);
                if since.elapsed() > Duration::from_secs(5) {
                    return true;
                }
            }
            _ => stale_since = None,
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// `(major, minor, patch)` of a version; a pre-release or build suffix is ignored. An
/// unparsable version compares as 0.0.0.
fn parse_version(v: &str) -> (u64, u64, u64) {
    let core = v.split(['-', '+']).next().unwrap_or("");
    let mut n = core.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    (
        n.next().unwrap_or(0),
        n.next().unwrap_or(0),
        n.next().unwrap_or(0),
    )
}

/// Is `a` a higher version than `b`?
fn newer(a: &str, b: &str) -> bool {
    parse_version(a) > parse_version(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(newer("0.10.0", "0.9.9"));
        assert!(newer("1.0.0", "0.99.99"));
        assert!(!newer("0.1.8", "0.1.8"));
        assert!(!newer("0.1.8-rc1", "0.1.8"));
        assert!(!newer("0.1.8", "99.0.0"));
    }
}
