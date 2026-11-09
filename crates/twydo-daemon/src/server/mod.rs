//! The daemon: a private Unix socket, a single-instance lock, lifecycle methods and the
//! dispatch table that later tasks extend.

pub mod conn;

use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use twydo_core::compat::Matrix;
use twydo_core::config::Config;
use twydo_core::store::{Store, bind_vdir_state};

use crate::PROTOCOL;
use crate::paths::DaemonLock;
use crate::protocol::methods::{DaemonInfo, HelloParams, HelloResult, Instance, ShutdownParams};
use crate::protocol::rpc::{ErrorData, ErrorKind, INVALID_PARAMS, METHOD_NOT_FOUND, RpcError};
use crate::schema;

pub struct DaemonOptions {
    pub socket: PathBuf,
    pub matrix: Matrix,
    pub heartbeat: Duration,
    pub event_buffer: usize,
    pub session_ttl: Duration,
    pub crash_after_commit: bool,
    /// Runs before recovery; tests block here to observe the starting state.
    pub before_recovery: Option<Box<dyn FnOnce() + Send>>,
}

impl DaemonOptions {
    pub fn new(socket: PathBuf) -> DaemonOptions {
        DaemonOptions {
            socket,
            matrix: Matrix::embedded().clone(),
            heartbeat: Duration::from_secs(15),
            event_buffer: 1024,
            session_ttl: Duration::from_secs(600),
            crash_after_commit: false,
            before_recovery: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("another daemon already serves this vdir or socket")]
    AlreadyRunning,
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// State shared by every thread of one daemon. Later tasks add fields (publisher, bus, lane,
/// sessions, replay log).
pub struct Shared {
    pub cfg: Config,
    pub instance: Instance,
    /// The canonical vdir.
    pub vdir: PathBuf,
    pub epoch: String,
    pub started: Instant,
    pub socket: PathBuf,
    pub heartbeat: Duration,
    pub event_buffer: usize,
    pub session_ttl: Duration,
    pub crash_after_commit: bool,
    ready: Mutex<bool>,
    ready_cv: Condvar,
    startup_error: Mutex<Option<String>>,
    /// `true` once stopping has been requested.
    stop: Mutex<bool>,
    stop_cv: Condvar,
    /// Set when teardown (socket removed, lock released) is complete.
    stopped: Mutex<bool>,
    stopped_cv: Condvar,
    conns: Mutex<HashMap<u64, UnixStream>>,
    next_conn: AtomicU64,
    lock: Mutex<Option<DaemonLock>>,
    wake_w: OwnedFd,
    /// The startup thread; teardown joins it before the lock is released.
    startup: Mutex<Option<JoinHandle<()>>>,
}

impl Shared {
    fn is_ready(&self) -> bool {
        *self.ready.lock().unwrap()
    }

    /// Marks the daemon ready unless it is stopping (stop is set before the ready lock is
    /// taken here, so a stopped daemon never reports ready).
    fn set_ready(&self) {
        let mut r = self.ready.lock().unwrap();
        if !self.is_stopping() {
            *r = true;
            self.ready_cv.notify_all();
        }
    }

    pub fn is_stopping(&self) -> bool {
        *self.stop.lock().unwrap()
    }

    /// Asks the daemon to stop; the accept thread does the teardown.
    fn request_stop(&self) {
        *self.stop.lock().unwrap() = true;
        self.stop_cv.notify_all();
        // Wake the accept loop through the wake pipe, independent of the socket path.
        let b = 1u8;
        // SAFETY: writes one byte from a valid buffer to our own pipe; a full pipe or error
        // means a wake-up is already pending or the loop is gone.
        unsafe {
            libc::write(self.wake_w.as_raw_fd(), (&b as *const u8).cast(), 1);
        }
    }

    fn register(&self, stream: &UnixStream) -> io::Result<u64> {
        let id = self.next_conn.fetch_add(1, Ordering::Relaxed);
        self.conns.lock().unwrap().insert(id, stream.try_clone()?);
        Ok(id)
    }

    fn unregister(&self, id: u64) {
        self.conns.lock().unwrap().remove(&id);
    }

    /// Closes the read side of every connection: readers end, writers drain what is queued
    /// (including a final reply) and then close.
    fn close_connections(&self) {
        for s in self.conns.lock().unwrap().values() {
            let _ = s.shutdown(std::net::Shutdown::Read);
        }
    }
}

pub struct Daemon;

pub struct DaemonHandle {
    shared: Arc<Shared>,
    accept: Mutex<Option<JoinHandle<()>>>,
}

impl Daemon {
    /// Binds the socket and starts accepting; startup (recovery, first snapshot) continues on
    /// a background thread and flips ready. `cfg` is the configuration loaded for `instance`.
    pub fn start(
        cfg: Config,
        instance: Instance,
        mut opts: DaemonOptions,
    ) -> Result<DaemonHandle, StartError> {
        let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir)?;
        let vdir = fs::canonicalize(&cfg.vdir)?;
        let lock = DaemonLock::take(&state)?.ok_or(StartError::AlreadyRunning)?;

        let listener = bind_socket(&opts.socket)?;
        listener.set_nonblocking(true)?;
        let (wake_r, wake_w) = wake_pipe()?;
        let shared = Arc::new(Shared {
            cfg,
            instance,
            vdir,
            epoch: format!("d-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]),
            started: Instant::now(),
            socket: opts.socket.clone(),
            heartbeat: opts.heartbeat,
            event_buffer: opts.event_buffer,
            session_ttl: opts.session_ttl,
            crash_after_commit: opts.crash_after_commit,
            ready: Mutex::new(false),
            ready_cv: Condvar::new(),
            startup_error: Mutex::new(None),
            stop: Mutex::new(false),
            stop_cv: Condvar::new(),
            stopped: Mutex::new(false),
            stopped_cv: Condvar::new(),
            conns: Mutex::new(HashMap::new()),
            next_conn: AtomicU64::new(1),
            lock: Mutex::new(Some(lock)),
            wake_w,
            startup: Mutex::new(None),
        });

        {
            let for_thread = Arc::clone(&shared);
            let hook = opts.before_recovery.take();
            let matrix = opts.matrix;
            let h = std::thread::spawn(move || startup(&for_thread, matrix, hook));
            *shared.startup.lock().unwrap() = Some(h);
        }
        let accept = {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || accept_loop(listener, wake_r, shared))
        };
        Ok(DaemonHandle {
            shared,
            accept: Mutex::new(Some(accept)),
        })
    }
}

/// Takes the socket path: creates its directory (`0700`) if missing, removes a leftover socket
/// only when nothing answers on it, binds and restricts it to the owner.
fn bind_socket(path: &Path) -> Result<UnixListener, StartError> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        if dir.exists() {
            check_private_dir(dir)?;
        } else {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
        }
    }
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            return Err(StartError::AlreadyRunning);
        }
        fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// An existing socket directory must belong to us and be closed to group and others.
fn check_private_dir(dir: &Path) -> Result<(), StartError> {
    let md = fs::metadata(dir)?;
    // SAFETY: geteuid has no preconditions and cannot fail.
    let own = unsafe { libc::geteuid() };
    if md.uid() != own || md.mode() & 0o077 != 0 {
        return Err(StartError::Io(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "socket directory {} must be owned by you and mode 0700 (it is uid {}, mode {:o})",
                dir.display(),
                md.uid(),
                md.mode() & 0o777
            ),
        )));
    }
    Ok(())
}

fn wake_pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0i32; 2];
    // SAFETY: `fds` has room for the two descriptors pipe() writes.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both descriptors are fresh and owned by nobody else.
    let (r, w) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    for fd in [&r, &w] {
        // SAFETY: valid descriptors; flags only.
        unsafe {
            libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
            libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK);
        }
    }
    Ok((r, w))
}

/// The startup sequence behind the already-accepting socket (daemon spec §7.1). Later tasks
/// add their steps here, in this order: publish the first snapshot (Task 5), drain the queue
/// (Task 8), start the watcher (Task 9). Ready is set last.
fn startup(
    shared: &Arc<Shared>,
    matrix: Matrix,
    before_recovery: Option<Box<dyn FnOnce() + Send>>,
) {
    type Step<'a> = Box<dyn FnOnce(&Arc<Shared>) -> Result<(), String> + 'a>;
    // One entry per step; the stop flag is checked before each, so a stopping daemon never
    // starts the next one. Later tasks append theirs before the end.
    let steps: Vec<Step> = vec![
        Box::new(move |_| {
            if let Some(hook) = before_recovery {
                hook();
            }
            Ok(())
        }),
        Box::new(move |shared| {
            // The recovery store is dropped; Task 5 keeps what it needs.
            Store::open_with(&shared.cfg, matrix)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }),
    ];
    for step in steps {
        if shared.is_stopping() {
            return;
        }
        if let Err(e) = step(shared) {
            eprintln!("tdo daemon: startup failed: {e}");
            *shared.startup_error.lock().unwrap() = Some(e);
            return;
        }
    }
    shared.set_ready();
}

fn accept_loop(listener: UnixListener, wake_r: OwnedFd, shared: Arc<Shared>) {
    // SAFETY: geteuid has no preconditions and cannot fail.
    let own = unsafe { libc::geteuid() };
    loop {
        let mut fds = [
            libc::pollfd {
                fd: listener.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: wake_r.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: `fds` is a valid array of two pollfds.
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
        if shared.is_stopping() {
            break;
        }
        if rc < 0 || fds[0].revents == 0 {
            continue;
        }
        let Ok((stream, _)) = listener.accept() else {
            continue;
        };
        // Accepted sockets may inherit the listener's non-blocking mode on some systems.
        if stream.set_nonblocking(false).is_err() {
            continue;
        }
        match conn::peer_uid(&stream) {
            Ok(uid) if conn::peer_allowed(uid, own) => {}
            _ => continue, // dropped: closed without a response
        }
        let Ok(id) = shared.register(&stream) else {
            continue;
        };
        let shared = Arc::clone(&shared);
        std::thread::spawn(move || {
            conn::serve(&shared, stream);
            shared.unregister(id);
        });
    }
    drop(listener);
    teardown(&shared);
}

fn teardown(shared: &Shared) {
    shared.close_connections();
    let _ = fs::remove_file(&shared.socket);
    // Startup may still be writing (recovery): the lock stays held until it has finished.
    if let Some(h) = shared.startup.lock().unwrap().take() {
        let _ = h.join();
    }
    shared.lock.lock().unwrap().take();
    *shared.stopped.lock().unwrap() = true;
    shared.stopped_cv.notify_all();
}

impl DaemonHandle {
    pub fn epoch(&self) -> &str {
        &self.shared.epoch
    }

    pub fn socket(&self) -> &Path {
        &self.shared.socket
    }

    /// Waits until startup has finished; false on timeout.
    pub fn wait_ready(&self, timeout: Duration) -> bool {
        let g = self.shared.ready.lock().unwrap();
        let (g, _) = self
            .shared
            .ready_cv
            .wait_timeout_while(g, timeout, |r| !*r)
            .unwrap();
        *g
    }

    /// Stops accepting, closes connections, removes the socket and drops the lock. Idempotent.
    pub fn shutdown(&self) {
        self.shared.request_stop();
        self.wait_stopped();
    }

    /// Blocks until the daemon has stopped (here or by `daemon.shutdown`).
    pub fn wait_stopped(&self) {
        let g = self.shared.stopped.lock().unwrap();
        let _g = self.shared.stopped_cv.wait_while(g, |s| !*s).unwrap();
        if let Some(h) = self.accept.lock().unwrap().take() {
            let _ = h.join();
        }
    }

    /// Blocks until the daemon is asked to stop, then finishes stopping.
    pub fn wait(self) {
        {
            let g = self.shared.stop.lock().unwrap();
            let _g = self.shared.stop_cv.wait_while(g, |s| !*s).unwrap();
        }
        self.wait_stopped();
    }
}

impl Drop for DaemonHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn invalid_params(e: impl std::fmt::Display) -> RpcError {
    RpcError {
        code: INVALID_PARAMS,
        message: format!("invalid params: {e}"),
        data: Some(ErrorData::new(ErrorKind::Invalid)),
    }
}

fn parse<P: DeserializeOwned>(params: Option<Value>) -> Result<P, RpcError> {
    serde_json::from_value(params.unwrap_or_else(|| json!({}))).map_err(invalid_params)
}

fn to_result<R: serde::Serialize>(r: R) -> Result<Value, RpcError> {
    serde_json::to_value(r).map_err(|e| RpcError::refused(ErrorKind::Internal, e.to_string()))
}

/// Per-connection state the dispatch needs.
#[derive(Default)]
pub struct ConnState {
    pub session: Mutex<Option<String>>,
}

/// Is `method` answered without a successful hello?
fn is_lifecycle(method: &str) -> Option<bool> {
    schema::METHODS
        .iter()
        .find(|m| m.name == method)
        .map(|m| m.lifecycle)
}

/// Runs one request. Methods the later tasks implement get an arm in the final `match`.
pub fn dispatch(
    shared: &Arc<Shared>,
    conn: &ConnState,
    method: &str,
    params: Option<Value>,
) -> Result<Value, RpcError> {
    let Some(lifecycle) = is_lifecycle(method) else {
        return Err(RpcError {
            code: METHOD_NOT_FOUND,
            message: format!("method not found: {method}"),
            data: None,
        });
    };
    if lifecycle {
        return match method {
            "hello" => hello(shared, conn, parse(params)?),
            "rpc.discover" => Ok(schema::document()),
            "daemon.info" => to_result(DaemonInfo {
                version: env!("CARGO_PKG_VERSION").into(),
                protocol: PROTOCOL,
                pid: std::process::id(),
                uptime_secs: shared.started.elapsed().as_secs(),
                vdir: shared.vdir.clone(),
                epoch: shared.epoch.clone(),
            }),
            "daemon.shutdown" => {
                // Nothing long runs yet, so `when_idle` changes nothing. The reply is queued
                // by the caller before stopping: see `conn::serve`.
                let _: ShutdownParams = parse(params)?;
                Ok(json!({}))
            }
            _ => unreachable!("lifecycle table and dispatch disagree"),
        };
    }
    if conn.session.lock().unwrap().is_none() {
        return Err(RpcError::refused(ErrorKind::Invalid, "say hello first"));
    }
    if shared.is_stopping() {
        return Err(RpcError::refused(
            ErrorKind::Unavailable,
            "the daemon is shutting down",
        ));
    }
    if let Some(e) = shared.startup_error.lock().unwrap().as_ref() {
        return Err(RpcError::refused(
            ErrorKind::Internal,
            format!("the daemon failed to start: {e}"),
        ));
    }
    if !shared.is_ready() {
        return Err(RpcError::refused(
            ErrorKind::Unavailable,
            "the daemon is starting",
        ));
    }
    // Later tasks add a `match method` here: events.*, snapshot.get, status.get, op.*,
    // refusals.ack.
    Err(RpcError {
        code: METHOD_NOT_FOUND,
        message: format!("method not implemented yet: {method}"),
        data: None,
    })
}

fn hello(shared: &Arc<Shared>, conn: &ConnState, p: HelloParams) -> Result<Value, RpcError> {
    if p.protocol != PROTOCOL {
        let mut data = ErrorData::new(ErrorKind::Protocol);
        data.daemon = Some(env!("CARGO_PKG_VERSION").into());
        data.protocol = Some(PROTOCOL);
        return Err(RpcError {
            code: crate::protocol::rpc::REFUSED,
            message: format!(
                "protocol {} is not supported; this daemon speaks protocol {PROTOCOL}",
                p.protocol
            ),
            data: Some(data),
        });
    }
    if p.instance != shared.instance {
        return Err(RpcError::refused(
            ErrorKind::WrongInstance,
            format!(
                "this daemon serves {}, not the instance you asked for",
                shared.vdir.display()
            ),
        ));
    }
    // Task 7 creates or resumes sessions; for now every hello gets a fresh id.
    let session = format!("s-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    *conn.session.lock().unwrap() = Some(session.clone());
    to_result(HelloResult {
        daemon: env!("CARGO_PKG_VERSION").into(),
        protocol: PROTOCOL,
        epoch: shared.epoch.clone(),
        session,
        vdir: shared.vdir.clone(),
    })
}

/// True when the request is the shutdown call (the connection layer stops the daemon after
/// the reply is queued).
pub fn is_shutdown(method: &str) -> bool {
    method == "daemon.shutdown"
}

impl Shared {
    /// Called by the connection layer after the `daemon.shutdown` reply is queued.
    pub fn stop_after_reply(&self) {
        self.request_stop();
    }
}
