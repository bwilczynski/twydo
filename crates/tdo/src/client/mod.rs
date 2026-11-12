//! Connecting to the daemon from `tdo`: finds it, starts it on demand and applies the
//! upgrade rule (daemon spec §6.1, §10.3). Nothing here reads the configuration file; only
//! its path matters, so a missing configuration is reported without starting anything.

pub mod start;

use std::io::ErrorKind;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use twydo_core::config::Config;
use twydo_daemon::client::{Client, ClientError, HELLO_TIMEOUT};
use twydo_daemon::paths;
use twydo_daemon::protocol::methods::{DaemonInfo, HelloParams, HelloResult, ShutdownParams};
use twydo_daemon::protocol::rpc::ErrorKind as RpcKind;
use twydo_daemon::protocol::rpc::STARTING;

/// How long a CLI call may take before the client gives up on the daemon.
pub const CLI_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a freshly started daemon has to answer.
pub const START_WAIT: Duration = Duration::from_secs(5);
/// How long an old daemon has to finish its jobs and go after being asked to.
pub const STOP_WAIT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(100);

/// Calls `method`, retrying (about every 100 ms, for up to `limit`) while the daemon answers
/// that it is still starting: it says hello before it is ready for requests.
pub fn call_ready<P: serde::Serialize, R: serde::de::DeserializeOwned>(
    caller: &twydo_daemon::client::Caller,
    method: &str,
    params: &P,
    call_limit: Duration,
    limit: Duration,
) -> Result<R, ClientError> {
    let begun = Instant::now();
    loop {
        match caller.call_timeout(method, params, call_limit) {
            Err(ClientError::Rpc(e))
                if e.data.as_ref().map(|d| d.kind) == Some(RpcKind::Unavailable)
                    && e.message == STARTING
                    && begun.elapsed() < limit =>
            {
                std::thread::sleep(POLL);
            }
            other => return other,
        }
    }
}

/// How long a changing call keeps asking (reconnecting as needed) before its outcome is
/// reported as unknown. `TDO_RECONNECT_FOR_MS` shortens it for tests.
pub fn reconnect_for() -> Duration {
    std::env::var("TDO_RECONNECT_FOR_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map_or(Duration::from_secs(60), Duration::from_millis)
}

/// Why a changing call ended without a reply it can show.
#[derive(Debug)]
pub enum ChangeError {
    /// The daemon answered with an error, or the reply could not be read: nothing more to try.
    Failed(ClientError),
    /// No answer within the window: the change may or may not have been applied.
    NoAnswer,
}

/// What a changing call left unknown when it got no answer; each kind names the check that
/// tells, and says whether running the command again is safe.
#[derive(Debug, Clone, Copy)]
pub enum Unknown<'a> {
    /// A task write or a revert: `tdo op status` shows what happened to the request.
    Change,
    /// A sync run: `tdo op status` shows the job.
    SyncStarted,
    /// `tdo sync on|off`: setting the same value again changes nothing.
    AutoSwitch { on: bool },
    /// `tdo conflicts resolve`: choosing the same side again is harmless.
    Choice,
    /// `tdo op abandon`: abandoning an abandoned entry answers `abandoned: false`.
    Abandon { op_id: &'a str },
}

/// The message for [`ChangeError::NoAnswer`].
pub fn no_answer_message(unknown: Unknown, request_id: &str) -> String {
    let status = |what: &str| {
        format!(
            "the daemon did not answer; the {what} (request {request_id}) \u{2014} check with tdo op status {request_id} before retrying"
        )
    };
    match unknown {
        Unknown::Change => status("change may have been applied"),
        Unknown::SyncStarted => status("sync may have started"),
        Unknown::AutoSwitch { on } => format!(
            "the daemon did not answer; automatic sync may already be {} (request {request_id}) \u{2014} check with tdo status; running it again is safe",
            if on { "on" } else { "off" }
        ),
        Unknown::Choice => format!(
            "the daemon did not answer; the choice may have been recorded (request {request_id}) \u{2014} check with tdo conflicts; choosing again is safe"
        ),
        Unknown::Abandon { op_id } => format!(
            "the daemon did not answer; {op_id} may already be abandoned (request {request_id}) \u{2014} check with tdo op journal; running it again is safe"
        ),
    }
}

/// Sends a changing `method` and, when the answer is lost (timeout, closed or broken
/// connection), reconnects (resuming the session and starting the daemon if needed) and sends
/// the same params again, so the same `request_id` makes the daemon apply it once. `params`
/// builds them from the session of the current connection (a restarted daemon opens a new
/// one; it replays a known request id before checking the session, and a write that never
/// became durable must not meet the old session). Gives up
/// when `window` has passed since the first send: the outcome is then unknown. A daemon that
/// is starting or still applying the request (`unavailable`) is asked again shortly; any other
/// error reply is the answer. `client` is replaced by the new connection after a reconnect.
pub fn call_changing<P: serde::Serialize, R: serde::de::DeserializeOwned>(
    target: &Target,
    client_name: &str,
    client: &mut Client,
    method: &str,
    params: impl Fn(&str) -> P,
    window: Duration,
) -> Result<R, ChangeError> {
    let begun = Instant::now();
    let mut session = client.session();
    loop {
        let left = window.saturating_sub(begun.elapsed());
        if left.is_zero() {
            return Err(ChangeError::NoAnswer);
        }
        let current = params(session.as_deref().unwrap_or_default());
        match client
            .caller()
            .call_timeout(method, &current, left.min(CLI_TIMEOUT))
        {
            Ok(r) => return Ok(r),
            Err(ClientError::Rpc(e))
                if e.data.as_ref().map(|d| d.kind) == Some(RpcKind::Unavailable) =>
            {
                std::thread::sleep(POLL);
            }
            Err(e @ (ClientError::Rpc(_) | ClientError::Decode(_))) => {
                return Err(ChangeError::Failed(e));
            }
            Err(ClientError::TimedOut(_) | ClientError::Closed | ClientError::Io(_)) => loop {
                if begun.elapsed() >= window {
                    return Err(ChangeError::NoAnswer);
                }
                match connect(target, client_name, session.clone()) {
                    Ok((c, hello)) => {
                        session = Some(hello.session);
                        *client = c;
                        break;
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(500)),
                }
            },
        }
    }
}

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
    #[error("no config at {}; run `tdo config init`, or pass --vdir", .0.display())]
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
            start::start(t)
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
        // Under a service unit the manager may already have started the next daemon.
        if !wait_replaced(&t.socket, Some(&info.epoch), STOP_WAIT) {
            return Err(ConnectError::Unavailable(
                "the older daemon is still busy; try again once its job is done".into(),
            ));
        }
        if UnixStream::connect(&t.socket).is_ok() {
            return wait_for_daemon(t, hello, log, false);
        }
    }
    start::start(t).map_err(|e| ConnectError::Unavailable(format!("cannot start it: {e}")))?;
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
            let _ = start::start(t);
        }
        if begun.elapsed() > START_WAIT {
            if silent {
                return Err(ConnectError::Unavailable(
                    "the daemon did not answer".into(),
                ));
            }
            let mut msg = format!(
                "no daemon answered on {} within {} s",
                t.socket.display(),
                START_WAIT.as_secs()
            );
            if let Some(line) = log.last_line() {
                msg.push_str(&format!(": {line}"));
            } else if let Some(j) = start::journal_hint(t) {
                msg.push_str(&format!("; see {j}"));
            }
            if let Some(e) = start::manager_error() {
                msg.push_str(&format!(" (the service manager failed to start it: {e})"));
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

/// What [`probe`] found on a socket.
#[derive(Debug)]
pub enum Probe {
    Running(DaemonInfo),
    /// A daemon of another instance answers; its message.
    WrongInstance(String),
    NotRunning,
}

/// Says hello on `t`'s socket and asks `daemon.info`, never starting or stopping anything.
/// A daemon that refuses only on protocol is ours and is reported as running.
pub fn probe(t: &Target) -> Probe {
    let Ok(hello) = hello_params(t, "tdo", None) else {
        return Probe::NotRunning;
    };
    let Ok(raw) = Client::connect_raw(&t.socket) else {
        return Probe::NotRunning;
    };
    match raw.hello_on(hello) {
        Ok(_) => {}
        Err(ClientError::Rpc(e)) => match e.data.as_ref().map(|d| d.kind) {
            Some(RpcKind::Protocol) => {}
            Some(RpcKind::WrongInstance) => return Probe::WrongInstance(e.message),
            _ => return Probe::NotRunning,
        },
        Err(_) => return Probe::NotRunning,
    }
    match raw
        .caller()
        .call_timeout("daemon.info", serde_json::json!({}), HELLO_TIMEOUT)
    {
        Ok(info) => Probe::Running(info),
        Err(_) => Probe::NotRunning,
    }
}

/// Waits until nothing answers on `socket` and its file is gone; false on timeout. A file
/// that nothing answers on but that stays (a crashed daemon's) counts as gone after 5 s.
pub fn wait_gone(socket: &std::path::Path, limit: Duration) -> bool {
    wait_replaced(socket, None, limit)
}

/// Like [`wait_gone`], and also true once a daemon answers `daemon.info` with an epoch other
/// than `old_epoch`: under a service unit the manager can restart the daemon before the
/// socket is seen gone.
pub fn wait_replaced(socket: &std::path::Path, old_epoch: Option<&str>, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    let mut stale_since: Option<Instant> = None;
    loop {
        match UnixStream::connect(socket) {
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
            Ok(_) => {
                stale_since = None;
                if let Some(old) = old_epoch
                    && answering_epoch(socket).is_some_and(|e| e != old)
                {
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

/// The epoch of the daemon answering on `socket` (`daemon.info` needs no hello).
fn answering_epoch(socket: &std::path::Path) -> Option<String> {
    let raw = Client::connect_raw(socket).ok()?;
    let info: DaemonInfo = raw
        .caller()
        .call_timeout("daemon.info", serde_json::json!({}), HELLO_TIMEOUT)
        .ok()?;
    Some(info.epoch)
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
