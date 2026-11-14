//! The daemon: a private Unix socket, a single-instance lock, lifecycle methods and the
//! dispatch table that later tasks extend.

pub mod admin;
pub mod bus;
pub mod config;
pub mod conn;
pub mod dispatch;
pub mod jobs;
pub mod lane;
pub mod lists;
pub mod permissions;
pub mod publish;
pub mod replay;
pub mod reports;
pub mod schedule;
pub mod sessions;
pub mod syncjob;
pub mod watch;
pub mod writes;

pub use dispatch::{ConnState, dispatch, is_shutdown};

use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock, RwLockReadGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use twydo_core::compat::Matrix;
use twydo_core::config::Config;
use twydo_core::store::{WriteError, Writer, bind_vdir_state};
use twydo_core::sync::running::{self, Reaped};

use crate::paths::DaemonLock;
use crate::protocol::events::Cause;
use crate::protocol::methods::Instance;
use crate::protocol::sync::SyncReason;

pub struct DaemonOptions {
    pub socket: PathBuf,
    pub matrix: Matrix,
    pub heartbeat: Duration,
    pub event_buffer: usize,
    pub session_ttl: Duration,
    pub crash_after_commit: bool,
    /// Runs before recovery; tests block here to observe the starting state.
    pub before_recovery: Option<Box<dyn FnOnce() + Send>>,
    /// Runs in every queue drain before its first pass, holding the lane; tests block here to
    /// change the files under a queued edit while a write waits for the lane.
    #[doc(hidden)]
    pub before_drain_passes: Option<TestHook>,
    /// Runs in every queue drain after its last pass, before publishing; tests block here to
    /// queue a write while the drain hook still holds the lane.
    #[doc(hidden)]
    pub after_drain_passes: Option<TestHook>,
    /// Runs inside the commit gate of every `op.submit`, before the writer is called; tests
    /// block here to hold a write in flight while the daemon is asked to stop.
    #[doc(hidden)]
    pub in_commit: Option<TestHook>,
    /// Runs in a queued write's reply path, after its item is on disk and before the reply is
    /// built, holding the session mutex; tests block here to end a long job at exactly that
    /// point.
    #[doc(hidden)]
    pub before_queued_reply: Option<TestHook>,
    /// Makes the first status refresher run panic, to check later refreshes still happen.
    #[doc(hidden)]
    pub status_panic_once: bool,
    /// Called with the name of a test point as the daemon passes it (`sync`: a sync has been
    /// reported started and is about to run; `recover`: `lists.recover` holds its session);
    /// tests panic here to check the daemon survives it.
    #[doc(hidden)]
    pub test_point: Option<TestPoint>,
    /// Runs the scheduler (startup, periodic and batched syncs). Tests turn it off.
    pub schedule: bool,
    /// The interval of the periodic sync.
    pub periodic: Duration,
    /// The scheduler's clock.
    pub clock: Arc<dyn schedule::Clock>,
    /// How long a sync running at shutdown may go on before it is stopped. Shutdown waits
    /// for the running job however long it takes (see `stop_jobs`): a job that never ends is
    /// ended by the service manager's stop timeout.
    pub shutdown_grace: Duration,
    /// How long past the grace teardown waits for the running job before it logs that it is
    /// still waiting; tests shorten it.
    #[doc(hidden)]
    pub stop_wait: Duration,
    /// Replaces the package version in `hello` and `daemon.info`; tests present an older or
    /// newer daemon with it.
    #[doc(hidden)]
    pub version_for_test: Option<String>,
}

/// A test hook shared with the daemon's threads.
pub type TestHook = Arc<dyn Fn() + Send + Sync>;

/// A test hook told which point the daemon passes; see [`DaemonOptions::test_point`].
pub type TestPoint = Arc<dyn Fn(&str) + Send + Sync>;

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
            before_drain_passes: None,
            after_drain_passes: None,
            in_commit: None,
            before_queued_reply: None,
            status_panic_once: false,
            test_point: None,
            schedule: true,
            periodic: Duration::from_secs(60),
            clock: Arc::new(schedule::SystemClock),
            shutdown_grace: Duration::from_secs(10),
            stop_wait: STOP_WAIT,
            version_for_test: None,
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
    /// What `hello` and `daemon.info` report as the daemon's version.
    pub version: String,
    pub started: Instant,
    pub socket: PathBuf,
    pub heartbeat: Duration,
    pub event_buffer: usize,
    pub session_ttl: Duration,
    pub crash_after_commit: bool,
    pub before_drain_passes: Option<TestHook>,
    pub after_drain_passes: Option<TestHook>,
    pub in_commit: Option<TestHook>,
    pub before_queued_reply: Option<TestHook>,
    /// Armed by [`DaemonOptions::status_panic_once`]; the first refresher run takes it.
    status_panic_once: AtomicBool,
    test_point: Option<TestPoint>,
    /// The commit gate: `true` once closed. Every writer call that can commit (submit, undo,
    /// redo, drain) runs under its read side ([`Shared::commit_gate`]); teardown takes the
    /// write side and closes it before the daemon lock is released, so a write in flight
    /// finishes first and none commits after (daemon spec §7.5). See the lock order in
    /// `writes.rs` for where it sits.
    commits: RwLock<bool>,
    /// The per-vdir state directory (journal, queue, locks).
    pub state_dir: PathBuf,
    /// The compatibility matrix the gate checks against.
    pub matrix: Matrix,
    /// The one writer of the vdir, opened with `matrix`; it runs no recovery and holds no lock.
    pub writer: Writer,
    /// The vdir lane token; its drain hook is [`writes::drain_queue`].
    pub lane: lane::Lane,
    pub sessions: sessions::Sessions,
    pub requests: replay::RequestLog,
    /// The job records.
    pub jobs: jobs::JobStore,
    /// Runs jobs and syncs on the lane, one at a time; started once ready.
    pub runner: jobs::JobRunner,
    /// Set once stopping: a sync then leaves the queued edits on disk
    /// ([`twydo_core::sync::run::RunOptions::hold_queue`]).
    pub hold_queue: Arc<AtomicBool>,
    /// How long a sync running at shutdown may go on before it is stopped.
    shutdown_grace: Duration,
    /// See [`DaemonOptions::stop_wait`].
    stop_wait: Duration,
    /// The background status refresher; see [`Shared::request_status_refresh`].
    status_refresh: Mutex<Refresher>,
    status_refresh_cv: Condvar,
    /// The scheduler, once started (after ready); taken at teardown.
    scheduler: Mutex<Option<schedule::Scheduler>>,
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
    /// The vdir watcher; dropped at teardown.
    watcher: Mutex<Option<watch::Watcher>>,
    /// The configuration file's watcher; dropped at teardown.
    config_watcher: Mutex<Option<watch::Watcher>>,
    /// The bytes of `config.toml` last applied (`None`: never read); see [`config::changed`].
    config_seen: Mutex<Option<Vec<u8>>>,
}

impl Shared {
    /// Passes test point `name` ([`DaemonOptions::test_point`]).
    pub(crate) fn test_point(&self, name: &str) {
        if let Some(hook) = &self.test_point {
            hook(name);
        }
    }

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

    /// Enters the commit gate: hold the guard around a store call that can commit. `None`
    /// once teardown has closed it (the caller answers `unavailable` or skips its drain).
    pub fn commit_gate(&self) -> Option<RwLockReadGuard<'_, bool>> {
        let g = self.commits.read().unwrap_or_else(|e| e.into_inner());
        (!*g).then_some(g)
    }

    /// Closes the commit gate, waiting for the writer calls inside it to return.
    fn close_commits(&self) {
        *self.commits.write().unwrap_or_else(|e| e.into_inner()) = true;
    }

    /// Asks the daemon to stop; the accept thread does the teardown.
    fn request_stop(&self) {
        self.hold_queue.store(true, Ordering::SeqCst);
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
        let job_store = jobs::JobStore::open(&state)?;

        let listener = bind_socket(&opts.socket, &vdir)?;
        listener.set_nonblocking(true)?;
        let (wake_r, wake_w) = wake_pipe()?;
        let bus = bus::EventBus::new();
        let publisher = publish::Publisher::new(cfg.clone(), state.clone(), Arc::clone(&bus));
        let sessions =
            sessions::Sessions::new(opts.session_ttl).with_test_point(opts.test_point.clone());
        let writer = Writer::open(&cfg, opts.matrix.clone()).map_err(|e| match e {
            WriteError::Io(e) => StartError::from(e),
            e => StartError::from(io::Error::other(e.to_string())),
        })?;
        let shared = Arc::new(Shared {
            cfg,
            bus,
            publisher,
            instance,
            vdir,
            epoch: format!("d-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]),
            version: opts
                .version_for_test
                .take()
                .unwrap_or_else(|| env!("CARGO_PKG_VERSION").into()),
            started: Instant::now(),
            socket: opts.socket.clone(),
            heartbeat: opts.heartbeat,
            event_buffer: opts.event_buffer,
            session_ttl: opts.session_ttl,
            crash_after_commit: opts.crash_after_commit,
            before_drain_passes: opts.before_drain_passes.take(),
            after_drain_passes: opts.after_drain_passes.take(),
            in_commit: opts.in_commit.take(),
            before_queued_reply: opts.before_queued_reply.take(),
            status_panic_once: AtomicBool::new(opts.status_panic_once),
            test_point: opts.test_point.take(),
            commits: RwLock::new(false),
            state_dir: state.clone(),
            matrix: opts.matrix.clone(),
            writer,
            lane: lane::Lane::new(),
            sessions,
            requests: replay::RequestLog::new(),
            jobs: job_store,
            runner: jobs::JobRunner::new(),
            hold_queue: Arc::new(AtomicBool::new(false)),
            shutdown_grace: opts.shutdown_grace,
            stop_wait: opts.stop_wait,
            status_refresh: Mutex::new(Refresher::default()),
            status_refresh_cv: Condvar::new(),
            scheduler: Mutex::new(None),
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
            watcher: Mutex::new(None),
            config_watcher: Mutex::new(None),
            config_seen: Mutex::new(None),
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
            let sched = opts.schedule.then_some((opts.clock, opts.periodic));
            let h = std::thread::spawn(move || startup(&for_thread, hook, sched));
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
/// only when nothing answers on it (anything else at the path is an error, never deleted),
/// binds and restricts it to the owner.
///
/// A bare relative name (`tdo.sock`) lives in the working directory, which is checked like any
/// other: it must be private too.
fn bind_socket(path: &Path, vdir: &Path) -> Result<UnixListener, StartError> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    if dir.exists() {
        check_private_dir(dir)?;
    } else {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    if let Ok(md) = fs::symlink_metadata(path) {
        if UnixStream::connect(path).is_ok() {
            // Something answers. A daemon of another vdir is a configuration mistake (exit 2
            // with the reason), not a second instance of ours.
            if let Some(other) = served_vdir(path)
                && other != vdir
            {
                return Err(StartError::Io(io::Error::other(format!(
                    "{} is served by a daemon for {}; use another --socket",
                    path.display(),
                    other.display()
                ))));
            }
            return Err(StartError::AlreadyRunning);
        }
        if !md.file_type().is_socket() {
            return Err(StartError::Io(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} exists and is not a socket", path.display()),
            )));
        }
        fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// The vdir of the daemon answering on `path`, if it answers `daemon.info` promptly.
fn served_vdir(path: &Path) -> Option<PathBuf> {
    let c = crate::client::Client::connect_raw(path).ok()?;
    let info: crate::protocol::methods::DaemonInfo = c
        .caller()
        .call_timeout("daemon.info", serde_json::json!({}), Duration::from_secs(2))
        .ok()?;
    Some(info.vdir)
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
    before_recovery: Option<Box<dyn FnOnce() + Send>>,
    sched: Option<(Arc<dyn schedule::Clock>, Duration)>,
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
        Box::new(|shared| {
            // A vdirsyncer whose runner is gone (a daemon killed during a sync) still holds
            // the vdir: stop it before recovery. Not fatal: its locks only make writes wait.
            if shared.cfg.sync.is_some() {
                match running::reap_orphan(&shared.cfg) {
                    Ok(Reaped::Killed) => {
                        eprintln!("tdo daemon: stopped a vdirsyncer left by an earlier run")
                    }
                    Ok(Reaped::LiveRunner) => eprintln!(
                        "tdo daemon: another process is syncing this vdir; writes wait for it"
                    ),
                    Ok(Reaped::Nothing | Reaped::Stale) => {}
                    Err(e) => eprintln!("tdo daemon: cannot check for a left-over sync: {e}"),
                }
            }
            Ok(())
        }),
        Box::new(|shared| {
            // Jobs a stopped daemon left unfinished, and records past their retention.
            let jobs = &shared.jobs;
            jobs.interrupt_unfinished().map_err(|e| e.to_string())?;
            let days = shared.cfg.sync.as_ref().map_or(30, |s| s.retention_days);
            jobs.prune(chrono::Utc::now() - chrono::Duration::days(days.into()))
                .map_err(|e| e.to_string())?;
            Ok(())
        }),
        Box::new(move |shared| {
            // Recovery runs outside the commit gate: the daemon is not ready yet, so no write is
            // in flight, and the stop flag was checked before this step.
            // An unfinished list operation fences the vdir and refuses recovery. The daemon
            // starts anyway (so the operation can be reconciled through it); recovery runs
            // when the operation ends (`admin::retry_deferred_recovery`).
            if let Ok(Some(collection)) =
                twydo_core::lists::operation::fenced_collection(&shared.state_dir)
            {
                shared.publisher.set_recovery_deferred(Some(format!(
                    "journal recovery waits for the unfinished operation on list {collection}; reconcile it"
                )));
                return Ok(());
            }
            // Only the stuck ids are kept, for the status. A busy vdir defers recovery: the
            // first write recovers under the lock anyway.
            let report = match shared
                .writer
                .lock(Duration::ZERO)
                .map_err(|e| e.to_string())?
            {
                Some(locked) => locked.recover().map_err(|e| e.to_string())?,
                None => Default::default(),
            };
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
            match shared
                .lane
                .write()
                .map(|_lane| writes::drain_passes(shared))
            {
                Ok(drained) => writes::finish_drain(shared, drained),
                // Not a startup failure: the job's end (or the drain after ready) drains.
                Err(b) => eprintln!(
                    "tdo daemon: a {:?} job holds the vdir lane at startup; draining later",
                    b.0
                ),
            }
            Ok(())
        }),
        Box::new(|shared| {
            // A watcher that cannot start is not fatal: the daemon serves its own writes
            // and reloads on request (the TUI does the same today).
            let weak = Arc::downgrade(shared);
            match watch::start(&shared.vdir, move || {
                if let Some(shared) = weak.upgrade() {
                    outside_change(&shared);
                }
            }) {
                Ok(w) => {
                    *shared.watcher.lock().unwrap() = Some(w);
                    // A change between the first reload and the watch is seen by nobody:
                    // look once more (publishes nothing if the snapshot is unchanged).
                    outside_change(shared);
                }
                Err(e) => eprintln!("tdo daemon: not watching the vdir: {e}"),
            }
            Ok(())
        }),
        Box::new(|shared| {
            // Not fatal either; a missing config directory skips the watch.
            let path = &shared.instance.config;
            if !path.parent().is_some_and(Path::is_dir) {
                eprintln!(
                    "tdo daemon: not watching {}: its directory is missing",
                    path.display()
                );
                return Ok(());
            }
            let weak = Arc::downgrade(shared);
            match watch::start_file(path, move || {
                if let Some(shared) = weak.upgrade() {
                    config::changed(&shared);
                }
            }) {
                Ok(w) => {
                    *shared.config_watcher.lock().unwrap() = Some(w);
                    // A change since the configuration was loaded is seen by nobody else.
                    config::changed(shared);
                }
                Err(e) => eprintln!("tdo daemon: not watching the configuration: {e}"),
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
    // Lists created on the server since the last run: the first sync that gets to it,
    // whatever starts it, looks for them (the startup sync may be skipped).
    if shared.cfg.sync.is_some() {
        let marker = shared.state_dir.join("sync/discover");
        if let Err(e) = std::fs::create_dir_all(shared.state_dir.join("sync"))
            .and_then(|()| twydo_core::store::fsio::write_atomic(&marker, b""))
        {
            eprintln!("tdo daemon: cannot schedule the startup list discovery: {e}");
        }
    }
    shared.set_ready();
    if shared.is_ready() {
        shared.runner.start(shared);
        // After the runner: the scheduler posts the startup sync at once.
        if let Some((clock, periodic)) = sched {
            shared.start_scheduler(clock, periodic);
        }
    }
    // A long job that ended before ready skipped its drain (and the startup drain may have
    // found the lane held): catch up now.
    if shared.queue_has_items() {
        shared.request_drain();
    }
}

/// The watcher's debounced callback: reload, labelled by the long job holding the lane (a
/// sync's own file changes), else as an outside change. The publisher publishes only when the
/// snapshot differs, so the daemon's own writes (already published) add nothing.
fn outside_change(shared: &Shared) {
    let cause = match shared.lane.current_long() {
        Some(lane::LongKind::Sync) => Cause::Sync,
        Some(lane::LongKind::List) => Cause::List,
        _ => Cause::Outside,
    };
    if let Err(e) = shared.publisher.reload(cause, Vec::new()) {
        eprintln!("tdo daemon: reload after a change failed: {e}");
        return;
    }
    shared.publisher.refresh_status();
}

fn accept_loop(listener: UnixListener, wake_r: OwnedFd, shared: Arc<Shared>) {
    // SAFETY: geteuid has no preconditions and cannot fail.
    let own = unsafe { libc::geteuid() };
    // Error kinds already logged: a persistent failure (no descriptors left) logs once.
    let mut logged: Vec<io::ErrorKind> = Vec::new();
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
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(e) => {
                if !logged.contains(&e.kind()) {
                    logged.push(e.kind());
                    eprintln!("tdo daemon: accept failed: {e}; retrying");
                }
                // The listener stays readable while the error lasts: back off, not spin.
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
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

/// How long past the grace teardown waits for the running job before it logs that it is
/// still waiting (it goes on waiting).
const STOP_WAIT: Duration = Duration::from_secs(10);

/// How often teardown looks at the running work while it waits.
const STOP_POLL: Duration = Duration::from_millis(100);

/// Ends the job runner before the connections close, so clients get the ends: queued jobs
/// become `interrupted` (with `job.finished`); the running job is waited for until its work
/// item has finished, however long that takes. Once the grace is over a running sync is
/// asked to stop, again at every poll (one that has not begun yet, or is still waiting for
/// a lock, cannot take the request; whatever job runs it, `lists.forget`'s included). A list
/// job's own work cannot be stopped at all. Past the grace and the stop wait, the wait is
/// logged once. Teardown never lets the vdir go while a job may still write it: a job that
/// never ends is ended by the service manager's stop timeout, killing the process.
fn stop_jobs(shared: &Shared) {
    use crate::protocol::events::Event;
    use crate::protocol::jobs::JobState;
    // No more background syncs are posted.
    shared.stop_scheduler();
    for view in shared.runner.close() {
        let state = JobState::Interrupted;
        if let Err(e) = shared.jobs.set_state(&view.job_id, state.clone()) {
            eprintln!(
                "tdo daemon: cannot record job {} interrupted: {e}",
                view.job_id
            );
        }
        shared.bus.emit(Event::JobFinished {
            job_id: view.job_id,
            request_id: view.request_id,
            state,
        });
    }
    let grace_end = Instant::now() + shared.shutdown_grace;
    let log_at = grace_end + shared.stop_wait;
    let mut logged = false;
    let mut stop_failed = false;
    while !shared.runner.wait_idle(STOP_POLL) {
        // The runner thread is gone (it cannot be, short of a panic outside its catch): there
        // is nothing left to wait for.
        if shared.runner.is_finished() {
            eprintln!("tdo daemon: the job runner ended; not waiting for it");
            break;
        }
        let now = Instant::now();
        if now < grace_end {
            continue;
        }
        // A sync runs: a sync job's, a background one, or the one `lists.forget` runs under
        // its list job's lane.
        let kind = shared.runner.current_kind();
        if (kind == Some(lane::LongKind::Sync) || shared.publisher.sync_running())
            && let Err(e) = syncjob::stop_running_sync(shared)
            && !stop_failed
        {
            stop_failed = true;
            eprintln!("tdo daemon: cannot stop the running sync: {e}");
        }
        if !logged && now >= log_at {
            logged = true;
            let kind = match kind {
                Some(lane::LongKind::List) => "list",
                Some(lane::LongKind::Test) => "test",
                _ => "sync",
            };
            match shared.runner.current_job() {
                Some(id) => eprintln!("tdo daemon: still waiting for {kind} job {id} to finish"),
                None => eprintln!("tdo daemon: still waiting for the background sync to finish"),
            }
        }
    }
    shared.runner.join();
}

fn teardown(shared: &Shared) {
    stop_jobs(shared);
    shared.close_connections();
    let _ = fs::remove_file(&shared.socket);
    shared.publisher.close();
    // Stops watching; a callback already running returns at once (the publisher is closed).
    shared.watcher.lock().unwrap().take();
    shared.config_watcher.lock().unwrap().take();
    // A write in flight finishes its commit; none starts after this (its store call answers
    // `unavailable`, a drain skips). Before the lock is released, so a second daemon never
    // writes the vdir while a thread of this one still could.
    shared.close_commits();
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

    /// Posts a sync that is not a job, as the scheduler will.
    #[doc(hidden)]
    pub fn post_sync_for_test(&self, reason: SyncReason) {
        self.shared.runner.post(jobs::Work::Sync {
            reason,
            background: true,
        });
    }

    /// Wakes the scheduler to decide again (tests move its clock first).
    #[doc(hidden)]
    pub fn poke_scheduler(&self) {
        self.shared.poke_scheduler();
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

    /// Has teardown finished (here, by `daemon.shutdown` or by a signal)?
    pub fn is_stopped(&self) -> bool {
        *self.shared.stopped.lock().unwrap()
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
            if self.status_panic_once.swap(false, Ordering::SeqCst) {
                panic!("status refresher panic requested by a test");
            }
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
            if let Some(uids) = reload {
                // A long job may hold the vdir lock for minutes: lay the queue over the last
                // load instead of waiting to read the vdir.
                let r = if self.lane.current_long().is_some() {
                    self.publisher.overlay(Cause::Op, uids)
                } else {
                    self.publisher.reload(Cause::Op, uids)
                };
                if let Err(e) = r {
                    eprintln!("tdo daemon: publishing the queue overlay failed: {e}");
                }
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
            // The lane is let go before the drain is finished: see the lock order in `writes`.
            let drained = match self.lane.write() {
                Ok(_lane) => writes::drain_passes(self),
                Err(_) => continue,
            };
            writes::finish_drain(self, drained);
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

    /// Called by the connection layer after a `daemon.shutdown {when_idle: true}` reply is
    /// queued (the runner already takes no new work): stops once the queued work has run.
    pub fn stop_when_idle(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        std::thread::spawn(move || {
            loop {
                let Some(shared) = weak.upgrade() else {
                    return;
                };
                if shared.is_stopping() {
                    return;
                }
                if shared.runner.wait_idle(Duration::from_millis(200)) {
                    shared.request_stop();
                    return;
                }
            }
        });
    }
}
