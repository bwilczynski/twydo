//! The daemon: a private Unix socket, a single-instance lock, lifecycle methods and the
//! dispatch table that later tasks extend.

pub mod bus;
pub mod conn;
pub mod dispatch;
pub mod lane;
pub mod publish;
pub mod replay;
pub mod sessions;
pub mod writes;

pub use dispatch::{ConnState, dispatch, is_shutdown};

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

use twydo_core::compat::Matrix;
use twydo_core::config::Config;
use twydo_core::store::{Store, bind_vdir_state};

use crate::paths::DaemonLock;
use crate::protocol::events::Cause;
use crate::protocol::methods::Instance;

pub struct DaemonOptions {
    pub socket: PathBuf,
    pub matrix: Matrix,
    pub heartbeat: Duration,
    pub event_buffer: usize,
    pub session_ttl: Duration,
    pub crash_after_commit: bool,
    /// Runs before recovery; tests block here to observe the starting state.
    pub before_recovery: Option<Box<dyn FnOnce() + Send>>,
    /// Runs in every queue drain after its last pass, before publishing; tests block here to
    /// queue a write while the drain hook still holds the lane.
    #[doc(hidden)]
    pub after_drain_passes: Option<TestHook>,
}

/// A test hook shared with the daemon's threads.
pub type TestHook = Arc<dyn Fn() + Send + Sync>;

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
            after_drain_passes: None,
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
    pub bus: Arc<bus::EventBus>,
    pub publisher: publish::Publisher,
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
    pub after_drain_passes: Option<TestHook>,
    /// The per-vdir state directory (journal, queue, locks).
    pub state_dir: PathBuf,
    /// The compatibility matrix the gate checks against.
    pub matrix: Matrix,
    /// The vdir lane token; its drain hook is [`writes::drain_queue`].
    pub lane: lane::Lane,
    pub sessions: sessions::Sessions,
    pub requests: replay::RequestLog,
    /// The background status refresher; see [`Shared::request_status_refresh`].
    status_refresh: Mutex<Refresher>,
    status_refresh_cv: Condvar,
    /// The background drainer; see [`Shared::request_drain`].
    drainer: Mutex<Drainer>,
    drainer_cv: Condvar,
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
    heartbeat_thread: Mutex<Option<JoinHandle<()>>>,
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
        let bus = bus::EventBus::new();
        let publisher = publish::Publisher::new(cfg.clone(), state.clone(), Arc::clone(&bus));
        let sessions = sessions::Sessions::new(cfg.clone(), opts.matrix.clone(), opts.session_ttl);
        let shared = Arc::new(Shared {
            cfg,
            bus,
            publisher,
            instance,
            vdir,
            epoch: format!("d-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]),
            started: Instant::now(),
            socket: opts.socket.clone(),
            heartbeat: opts.heartbeat,
            event_buffer: opts.event_buffer,
            session_ttl: opts.session_ttl,
            crash_after_commit: opts.crash_after_commit,
            after_drain_passes: opts.after_drain_passes.take(),
            state_dir: state.clone(),
            matrix: opts.matrix.clone(),
            lane: lane::Lane::new(),
            sessions,
            requests: replay::RequestLog::new(),
            status_refresh: Mutex::new(Refresher::default()),
            status_refresh_cv: Condvar::new(),
            drainer: Mutex::new(Drainer::default()),
            drainer_cv: Condvar::new(),
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
            heartbeat_thread: Mutex::new(None),
        });
        *shared.heartbeat_thread.lock().unwrap() =
            Some(shared.bus.start_heartbeat(shared.heartbeat));
        {
            // A weak reference: the hook lives inside `shared` (in its lane). A long guard
            // borrows the lane, so `shared` outlives every run of the hook.
            let weak = Arc::downgrade(&shared);
            shared.lane.set_drain_hook(Box::new(move || {
                if let Some(shared) = weak.upgrade()
                    && shared.is_ready()
                {
                    writes::drain_queue(&shared);
                }
            }));
            // A write that found the job still draining was queued after the drain's last
            // pass: drain again once the lane is free (on the drainer thread).
            let weak = Arc::downgrade(&shared);
            shared.lane.set_release_hook(Box::new(move || {
                if let Some(shared) = weak.upgrade()
                    && shared.is_ready()
                    && shared.queue_has_items()
                {
                    shared.request_drain();
                }
            }));
        }

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
/// add their steps here, in this order: publish the first snapshot, drain the queue, start
/// the watcher (Task 9). Ready is set last.
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
            // The recovery store is dropped; only the stuck ids are kept, for the status.
            let (_store, report) =
                Store::open_with(&shared.cfg, matrix).map_err(|e| e.to_string())?;
            shared
                .publisher
                .set_stuck(report.stuck.into_iter().map(|(id, _)| id).collect());
            Ok(())
        }),
        Box::new(|shared| {
            shared
                .publisher
                .reload(Cause::Recovery, Vec::new())
                .map_err(|e| e.to_string())?;
            shared.publisher.refresh_status();
            Ok(())
        }),
        Box::new(|shared| {
            // Items an earlier daemon left queued. No long job runs before ready (the drain
            // hook skips until then), and the lane is taken so no write could interleave.
            match shared.lane.write() {
                Ok(_lane) => writes::drain_queue(shared),
                // Not a startup failure: the job's end (or the drain after ready) drains.
                Err(b) => eprintln!(
                    "tdo daemon: a {:?} job holds the vdir lane at startup; draining later",
                    b.0
                ),
            }
            Ok(())
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
    // A long job that ended before ready skipped its drain (and the startup drain may have
    // found the lane held): catch up now.
    if shared.queue_has_items() {
        shared.request_drain();
    }
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
            conn::serve(&shared, id, stream);
            shared.unregister(id);
        });
    }
    drop(listener);
    teardown(&shared);
}

/// How long teardown waits for the status refresher. A refresher stuck behind a reload that
/// waits for an outside vdir-lock holder is not joined: the publisher is closed first, so when
/// it gets `reload_lock` it returns without publishing, and the thread then ends on its own.
const REFRESHER_WAIT: Duration = Duration::from_secs(1);

#[derive(Default)]
struct Refresher {
    running: bool,
    dirty: bool,
    /// The next run reloads the vdir (`Cause::Op`) before refreshing the status, publishing
    /// the queue overlay; `uids` are the hint for its `data.changed`.
    reload: bool,
    uids: Vec<String>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct Drainer {
    running: bool,
    dirty: bool,
    handle: Option<JoinHandle<()>>,
}

fn teardown(shared: &Shared) {
    shared.close_connections();
    let _ = fs::remove_file(&shared.socket);
    shared.publisher.close();
    if !shared.wait_drainer(REFRESHER_WAIT) {
        eprintln!("tdo daemon: queue drainer still waiting; not joined");
    }
    if !shared.wait_refresher(REFRESHER_WAIT) {
        eprintln!("tdo daemon: status refresher still waiting for the vdir lock; not joined");
    }
    // Startup may still be writing (recovery): the lock stays held until it has finished.
    if let Some(h) = shared.startup.lock().unwrap().take() {
        let _ = h.join();
    }
    shared.bus.stop();
    if let Some(h) = shared.heartbeat_thread.lock().unwrap().take() {
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

    /// Reloads the vdir as if the watcher had seen `cause` (until the watcher exists), then
    /// refreshes the status.
    #[doc(hidden)]
    pub fn reload_for_test(&self, cause: Cause) -> io::Result<u64> {
        self.reload_for_test_with(cause, Vec::new())
    }

    #[doc(hidden)]
    pub fn publisher_for_test(&self) -> &publish::Publisher {
        &self.shared.publisher
    }

    /// Waits up to `limit` until no status refresh is running; true if none is.
    #[doc(hidden)]
    pub fn refresher_idle_for_test(&self, limit: Duration) -> bool {
        let g = self.shared.status_refresh.lock().unwrap();
        let (g, _) = self
            .shared
            .status_refresh_cv
            .wait_timeout_while(g, limit, |r| r.running)
            .unwrap();
        !g.running
    }

    /// Holds the vdir lane for a long job of `kind` (a sync or a list job in plan 2; tests use
    /// [`lane::LongKind::Test`]). Writes meanwhile are queued; dropping the guard drains them.
    pub fn hold_long(&self, kind: lane::LongKind) -> lane::LongGuard<'_> {
        self.shared.lane.long(kind)
    }

    #[doc(hidden)]
    pub fn lane_for_test(&self) -> &lane::Lane {
        &self.shared.lane
    }

    #[doc(hidden)]
    pub fn reload_for_test_with(&self, cause: Cause, uids: Vec<String>) -> io::Result<u64> {
        let rev = self.shared.publisher.reload(cause, uids)?;
        self.shared.publisher.refresh_status();
        Ok(rev)
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

impl Shared {
    /// Refreshes the status on a background thread, without waiting: requests made while a
    /// refresh is running are coalesced into one more run. For replies that must not wait on
    /// the publisher's `reload_lock` (a reload may hold it while a sync holds the vdir). Does
    /// nothing once the daemon is stopping; teardown waits for the refresher (bounded).
    pub fn request_status_refresh(self: &Arc<Self>) {
        self.request_refresh(None);
    }

    /// Like [`Shared::request_status_refresh`], but the run first reloads the vdir with
    /// `Cause::Op`, so the queue overlay (a queued row pending, or a cancelled one gone) is
    /// published without the caller waiting on `reload_lock` or the vdir lock. `uids` name the
    /// rows for the `data.changed` hint.
    pub fn request_overlay_publish(self: &Arc<Self>, uids: Vec<String>) {
        self.request_refresh(Some(uids));
    }

    fn request_refresh(self: &Arc<Self>, reload: Option<Vec<String>>) {
        let mut g = self.status_refresh.lock().unwrap();
        if self.is_stopping() {
            return;
        }
        g.dirty = true;
        if let Some(uids) = reload {
            g.reload = true;
            for u in uids {
                if !g.uids.contains(&u) {
                    g.uids.push(u);
                }
            }
        }
        if g.running {
            return;
        }
        g.running = true;
        let shared = Arc::clone(self);
        // A previous refresher has cleared `running`, so it is finished or about to return:
        // its handle is dropped without a join.
        g.handle = Some(std::thread::spawn(move || shared.run_refresher()));
    }

    fn run_refresher(&self) {
        /// Clears `running` if `refresh_status` panics, so later requests start a new
        /// refresher instead of being dropped forever.
        struct Running<'a> {
            shared: &'a Shared,
            armed: bool,
        }
        impl Drop for Running<'_> {
            fn drop(&mut self) {
                if self.armed {
                    let mut g = self
                        .shared
                        .status_refresh
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    g.running = false;
                    self.shared.status_refresh_cv.notify_all();
                }
            }
        }
        let mut guard = Running {
            shared: self,
            armed: true,
        };
        loop {
            let reload = {
                let mut g = self.status_refresh.lock().unwrap();
                if !g.dirty || self.is_stopping() {
                    g.running = false;
                    guard.armed = false;
                    self.status_refresh_cv.notify_all();
                    return;
                }
                g.dirty = false;
                std::mem::take(&mut g.reload).then(|| std::mem::take(&mut g.uids))
            };
            if let Some(uids) = reload
                && let Err(e) = self.publisher.reload(Cause::Op, uids)
            {
                eprintln!("tdo daemon: publishing the queue overlay failed: {e}");
            }
            self.publisher.refresh_status();
        }
    }

    /// Waits up to `limit` for the refresher to finish; true if it has (or none ran).
    fn wait_refresher(&self, limit: Duration) -> bool {
        let g = self.status_refresh.lock().unwrap();
        let (mut g, _) = self
            .status_refresh_cv
            .wait_timeout_while(g, limit, |r| r.running)
            .unwrap();
        if g.running {
            return false;
        }
        let h = g.handle.take();
        drop(g);
        if let Some(h) = h {
            let _ = h.join();
        }
        true
    }

    /// Does the write queue hold items (pending or claimed)? `false` when unreadable.
    pub fn queue_has_items(&self) -> bool {
        twydo_core::store::queue::Queue::existing(&self.state_dir)
            .is_some_and(|q| matches!(q.is_empty(), Ok(false)))
    }

    /// Drains the write queue on a background thread under a write token: for items queued
    /// after a long job's drain hook made its last pass (the lane's release hook, or the
    /// queued reply that finds no long job any more). Requests made while it runs are
    /// coalesced into one more run; nothing here requests a drain again, so it cannot loop.
    /// Does nothing once the daemon is stopping; teardown waits for it (bounded).
    pub fn request_drain(self: &Arc<Self>) {
        let mut g = self.drainer.lock().unwrap();
        if self.is_stopping() {
            return;
        }
        g.dirty = true;
        if g.running {
            return;
        }
        g.running = true;
        let shared = Arc::clone(self);
        g.handle = Some(std::thread::spawn(move || shared.run_drainer()));
    }

    fn run_drainer(self: &Arc<Self>) {
        /// Clears `running` if a drain panics, so later requests start a new drainer.
        struct Running<'a> {
            shared: &'a Shared,
            armed: bool,
        }
        impl Drop for Running<'_> {
            fn drop(&mut self) {
                if self.armed {
                    let mut g = self
                        .shared
                        .drainer
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    g.running = false;
                    self.shared.drainer_cv.notify_all();
                }
            }
        }
        let mut guard = Running {
            shared: self,
            armed: true,
        };
        loop {
            {
                let mut g = self.drainer.lock().unwrap();
                if !g.dirty || self.is_stopping() {
                    g.running = false;
                    guard.armed = false;
                    self.drainer_cv.notify_all();
                    return;
                }
                g.dirty = false;
            }
            // A long job holding the lane drains when it ends (its hooks); nothing to do.
            if let Ok(_lane) = self.lane.write() {
                writes::drain_queue(self);
            }
        }
    }

    /// Waits up to `limit` for the drainer to finish; true if it has (or none ran).
    fn wait_drainer(&self, limit: Duration) -> bool {
        let g = self.drainer.lock().unwrap();
        let (mut g, _) = self
            .drainer_cv
            .wait_timeout_while(g, limit, |r| r.running)
            .unwrap();
        if g.running {
            return false;
        }
        let h = g.handle.take();
        drop(g);
        if let Some(h) = h {
            let _ = h.join();
        }
        true
    }

    /// Called by the connection layer after the `daemon.shutdown` reply is queued.
    pub fn stop_after_reply(&self) {
        self.request_stop();
    }
}
