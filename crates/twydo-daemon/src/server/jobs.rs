//! Jobs (daemon spec §7.4): their durable records ([`JobStore`]), the one runner thread that
//! runs jobs and background syncs in order on the vdir lane ([`JobRunner`]), starting a job
//! from a request ([`Shared::start_job`]) and `job.get`.

use std::collections::VecDeque;
use std::fs;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use twydo_core::store::fsio::write_atomic;
use twydo_core::store::journal::RequestRef;
use twydo_core::sync::run::RunOptions;
use twydo_core::sync::status::SyncStatus;
use twydo_core::sync::switch;

use super::Shared;
use super::lane::LongKind;
use super::replay::Begin;
use super::syncjob::{self, SyncDone};
use crate::protocol::events::Event;
use crate::protocol::jobs::{JobGetParams, JobKind, JobResult, JobStarted, JobState, JobView};
use crate::protocol::rpc::{ErrorKind, RpcError};
use crate::protocol::sync::SyncReason;

fn internal(e: impl std::fmt::Display) -> RpcError {
    RpcError::refused(ErrorKind::Internal, e.to_string())
}

// ---------------------------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------------------------

/// A job's file: its view, and the session that asked for it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    #[serde(flatten)]
    view: JobView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session: Option<String>,
}

/// The job records in `<vdir-state>/daemon/jobs/<job_id>.json`, written when a job is
/// accepted and rewritten when it starts and ends.
pub struct JobStore {
    dir: PathBuf,
    /// Serialises read-modify-write of one record against another writer.
    lock: Mutex<()>,
}

/// A new job id: `job-<UTC yyyymmddThhmmss>-<8 hex>`.
fn new_id(now: DateTime<Utc>) -> String {
    format!(
        "job-{}-{}",
        now.format("%Y%m%dT%H%M%S"),
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    )
}

/// Can `id` name a record file (no path separators, no hidden or temporary files)?
fn is_record_name(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl JobStore {
    /// Opens (creating) the record directory under the per-vdir state directory `state`.
    pub fn open(state: &Path) -> io::Result<JobStore> {
        let dir = state.join("daemon").join("jobs");
        fs::create_dir_all(&dir)?;
        Ok(JobStore {
            dir,
            lock: Mutex::new(()),
        })
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn write(&self, r: &Record) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(r).map_err(io::Error::other)?;
        write_atomic(&self.path(&r.view.job_id), &bytes)
    }

    fn read(&self, id: &str) -> io::Result<Option<Record>> {
        if !is_record_name(id) {
            return Ok(None);
        }
        match fs::read(self.path(id)) {
            Ok(b) => serde_json::from_slice(&b)
                .map(Some)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Every readable record; files that do not parse are skipped (and logged).
    fn all(&self) -> io::Result<Vec<Record>> {
        let mut out = Vec::new();
        for e in fs::read_dir(&self.dir)? {
            let name = e?.file_name();
            let Some(id) = name.to_str().and_then(|n| n.strip_suffix(".json")) else {
                continue;
            };
            match self.read(id) {
                Ok(Some(r)) => out.push(r),
                Ok(None) => {}
                Err(e) => eprintln!("tdo daemon: unreadable job record {id}: {e}"),
            }
        }
        Ok(out)
    }

    /// Records a new job, `queued`.
    pub fn create(&self, kind: JobKind, request: Option<RequestRef>) -> io::Result<JobView> {
        let now = Utc::now();
        let (request_id, session) = match request {
            Some(r) => (
                Some(r.request_id),
                Some(r.session).filter(|s| !s.is_empty()),
            ),
            None => (None, None),
        };
        let r = Record {
            view: JobView {
                job_id: new_id(now),
                request_id,
                kind,
                state: JobState::Queued,
                created: now,
                ended: None,
            },
            session,
        };
        let _g = self.lock.lock().unwrap();
        self.write(&r)?;
        Ok(r.view)
    }

    /// Moves job `id` to `state`; a final state sets `ended`.
    pub fn set_state(&self, id: &str, state: JobState) -> io::Result<JobView> {
        let _g = self.lock.lock().unwrap();
        let mut r = self.read(id)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("no job record {id}"))
        })?;
        if state.is_final() {
            r.view.ended = Some(Utc::now());
        }
        r.view.state = state;
        self.write(&r)?;
        Ok(r.view)
    }

    pub fn get(&self, id: &str) -> io::Result<Option<JobView>> {
        Ok(self.read(id)?.map(|r| r.view))
    }

    /// The job request `request_id` started.
    pub fn by_request(&self, request_id: &str) -> io::Result<Option<JobView>> {
        Ok(self
            .all()?
            .into_iter()
            .find(|r| r.view.request_id.as_deref() == Some(request_id))
            .map(|r| r.view))
    }

    /// Marks every record still `queued` or `running` (a daemon stopped before it ended)
    /// `interrupted`. Returns how many.
    pub fn interrupt_unfinished(&self) -> io::Result<usize> {
        let mut n = 0;
        for r in self.all()? {
            if !r.view.state.is_final() {
                self.set_state(&r.view.job_id, JobState::Interrupted)?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// Removes the ended records that ended (or, without an end, were created) before
    /// `before`. Returns how many.
    pub fn prune(&self, before: DateTime<Utc>) -> io::Result<usize> {
        let _g = self.lock.lock().unwrap();
        let mut n = 0;
        for r in self.all()? {
            if r.view.state.is_final() && r.view.ended.unwrap_or(r.view.created) < before {
                fs::remove_file(self.path(&r.view.job_id))?;
                n += 1;
            }
        }
        Ok(n)
    }
}

// ---------------------------------------------------------------------------------------------
// The runner
// ---------------------------------------------------------------------------------------------

/// What a job does on the runner thread, holding the lane long.
pub type JobBody = Box<dyn FnOnce(&Arc<Shared>) -> Result<JobResult, RpcError> + Send>;

/// One piece of work for the runner.
pub enum Work {
    /// A recorded job; `run` runs holding the lane long as `lane`.
    Job {
        view: JobView,
        run: JobBody,
        lane: LongKind,
    },
    /// A sync that is not a job (no record, no `job_id`). A `background` one is skipped,
    /// silently, while automatic sync is off or the runner backs off.
    Sync {
        reason: SyncReason,
        background: bool,
    },
}

#[derive(Default)]
struct RunnerState {
    queue: VecDeque<Work>,
    running: bool,
    closed: bool,
    handle: Option<JoinHandle<()>>,
    /// The end of a sync that a job ran; done after the job lets the lane go
    /// ([`JobRunner::defer_sync_end`]).
    sync_end: Option<SyncDone>,
    /// The id of the job running now.
    current: Option<String>,
}

/// One thread runs jobs and syncs, first in first out, each holding the vdir lane long.
#[derive(Default)]
pub struct JobRunner {
    state: Mutex<RunnerState>,
    cv: Condvar,
}

impl JobRunner {
    pub fn new() -> JobRunner {
        JobRunner::default()
    }

    /// Starts the runner thread (once; at the end of startup).
    pub(super) fn start(&self, shared: &Arc<Shared>) {
        let mut g = self.state.lock().unwrap();
        if g.handle.is_some() || g.closed {
            return;
        }
        let shared = Arc::clone(shared);
        g.handle = Some(std::thread::spawn(move || run_loop(&shared)));
    }

    /// Adds `w` to the end of the queue. Dropped once the runner is closed.
    pub fn post(&self, w: Work) {
        let mut g = self.state.lock().unwrap();
        if g.closed {
            return;
        }
        g.queue.push_back(w);
        self.cv.notify_all();
    }

    /// Queues a background sync for `reason`, unless one is already queued, or a manual sync
    /// job is (it covers this one). False when deduplicated.
    pub fn background_sync(&self, reason: SyncReason) -> bool {
        let mut g = self.state.lock().unwrap();
        if g.closed {
            return false;
        }
        let covered = g.queue.iter().any(|w| match w {
            Work::Sync { background, .. } => *background,
            Work::Job { view, .. } => view.kind == JobKind::Sync,
        });
        if covered {
            return false;
        }
        g.queue.push_back(Work::Sync {
            reason,
            background: true,
        });
        self.cv.notify_all();
        true
    }

    /// Nothing queued and nothing running.
    pub fn is_idle(&self) -> bool {
        let g = self.state.lock().unwrap();
        g.queue.is_empty() && !g.running
    }

    /// Stops taking work: the thread ends after what it is running. Returns the views of the
    /// queued jobs it dropped (background syncs are dropped silently).
    pub fn close(&self) -> Vec<JobView> {
        let mut g = self.state.lock().unwrap();
        g.closed = true;
        self.cv.notify_all();
        g.queue
            .drain(..)
            .filter_map(|w| match w {
                Work::Job { view, .. } => Some(view),
                Work::Sync { .. } => None,
            })
            .collect()
    }

    /// Waits for the runner thread to end (after [`JobRunner::close`]).
    pub(super) fn join(&self) {
        let h = self.state.lock().unwrap().handle.take();
        if let Some(h) = h {
            let _ = h.join();
        }
    }

    /// Hands the runner the end of a sync a job ran ([`SyncDone::finish`]): it runs after the
    /// job has let the lane go (its drain done) and before `job.finished`. Called from a job's
    /// body only.
    pub fn defer_sync_end(&self, end: SyncDone) {
        self.state.lock().unwrap().sync_end = Some(end);
    }

    /// The id of the job running now (for a job's body, which is built before its id).
    pub fn current_job(&self) -> Option<String> {
        self.state.lock().unwrap().current.clone()
    }

    fn take_sync_end(&self) -> Option<SyncDone> {
        self.state.lock().unwrap().sync_end.take()
    }

    /// The next work, or `None` once closed.
    fn next(&self) -> Option<Work> {
        let mut g = self.state.lock().unwrap();
        g.running = false;
        g.current = None;
        loop {
            if g.closed {
                return None;
            }
            if let Some(w) = g.queue.pop_front() {
                g.running = true;
                if let Work::Job { view, .. } = &w {
                    g.current = Some(view.job_id.clone());
                }
                return Some(w);
            }
            g = self.cv.wait(g).unwrap();
        }
    }
}

fn run_loop(shared: &Arc<Shared>) {
    while let Some(w) = shared.runner.next() {
        match w {
            Work::Job { view, run, lane } => run_job(shared, view, run, lane),
            Work::Sync { reason, background } => run_background(shared, reason, background),
        }
    }
}

fn run_job(shared: &Arc<Shared>, view: JobView, run: JobBody, lane: LongKind) {
    let id = view.job_id;
    if let Err(e) = shared.jobs.set_state(&id, JobState::Running) {
        eprintln!("tdo daemon: cannot record job {id} running: {e}");
    }
    let guard = shared.lane.long(lane);
    let outcome = catch_unwind(AssertUnwindSafe(|| run(shared))).unwrap_or_else(|_| {
        Err(RpcError::refused(
            ErrorKind::Internal,
            "the job failed unexpectedly",
        ))
    });
    let state = match outcome {
        Ok(result) => JobState::Finished { result },
        Err(e) => JobState::Failed {
            kind: e.data.as_ref().map_or(ErrorKind::Internal, |d| d.kind),
            message: e.message,
        },
    };
    if let Err(e) = shared.jobs.set_state(&id, state.clone()) {
        eprintln!("tdo daemon: cannot record the end of job {id}: {e}");
    }
    // The drain hook runs here, before the sync's reload.
    drop(guard);
    if let Some(end) = shared.runner.take_sync_end() {
        end.finish(shared);
    }
    shared.bus.emit(Event::JobFinished {
        job_id: id,
        request_id: view.request_id,
        state,
    });
}

/// Is a background sync to be skipped before it starts (no `[sync]`, switched off, backing
/// off)? A skip found here sends nothing. An error reading the switch or the status does not
/// skip: the runner checks again itself.
fn skip_background(shared: &Shared) -> bool {
    if shared.cfg.sync.is_none() {
        return true;
    }
    if matches!(switch::is_off(&shared.cfg), Ok(true)) {
        return true;
    }
    matches!(
        SyncStatus::load(&shared.state_dir),
        Ok(SyncStatus { next_attempt: Some(t), .. }) if t > Utc::now()
    )
}

fn run_background(shared: &Arc<Shared>, reason: SyncReason, background: bool) {
    if background && skip_background(shared) {
        return;
    }
    let guard = shared.lane.long(LongKind::Sync);
    let done = syncjob::run_sync(
        shared,
        None,
        reason,
        RunOptions {
            background,
            force_delete: Vec::new(),
        },
    );
    drop(guard);
    done.finish(shared);
}

// ---------------------------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------------------------

impl Shared {
    /// Starts a job for request `request_id`: a resent request (this epoch's log, then the
    /// records) answers with the job it started; otherwise the job is recorded `queued`,
    /// posted to the runner and its id answered at once.
    pub fn start_job(
        self: &Arc<Self>,
        request_id: &str,
        session: Option<&str>,
        kind: JobKind,
        lane: LongKind,
        run: JobBody,
    ) -> Result<Value, RpcError> {
        self.start_job_with(request_id, session, kind, lane, || Ok(run))
    }

    /// [`Shared::start_job`] with a body built by `prepare`, which runs only when the request
    /// is not a resend (a replay answers without it: it may check params that changed since,
    /// or read the vdir). Its refusal is the answer, recorded like any other.
    pub fn start_job_with(
        self: &Arc<Self>,
        request_id: &str,
        session: Option<&str>,
        kind: JobKind,
        lane: LongKind,
        prepare: impl FnOnce() -> Result<JobBody, RpcError>,
    ) -> Result<Value, RpcError> {
        super::writes::require_request_id(request_id)?;
        let ticket = match self.requests.begin(request_id) {
            Begin::Done(r) => return r,
            Begin::New(t) => t,
        };
        let started = |view: JobView| {
            serde_json::to_value(JobStarted {
                job_id: view.job_id,
            })
            .map_err(internal)
        };
        match self.jobs.by_request(request_id) {
            Ok(Some(view)) => return ticket.finish(started(view)),
            Ok(None) => {}
            Err(e) => return ticket.forget(Err(internal(e))),
        }
        let run = match prepare() {
            Ok(run) => run,
            Err(e) => return ticket.finish(Err(e)),
        };
        let request = RequestRef {
            session: session.unwrap_or_default().to_string(),
            request_id: request_id.to_string(),
        };
        let view = match self.jobs.create(kind, Some(request)) {
            Ok(v) => v,
            Err(e) => return ticket.forget(Err(internal(e))),
        };
        self.runner.post(Work::Job {
            view: view.clone(),
            run,
            lane,
        });
        ticket.finish(started(view))
    }
}

/// `job.get`: by `job_id` or by `request_id`, exactly one.
pub fn get(shared: &Arc<Shared>, p: JobGetParams) -> Result<Value, RpcError> {
    let found = match (p.job_id, p.request_id) {
        (Some(id), None) => shared
            .jobs
            .get(&id)
            .map_err(internal)?
            .ok_or_else(|| RpcError::refused(ErrorKind::NotFound, format!("no job {id}")))?,
        (None, Some(rid)) => shared
            .jobs
            .by_request(&rid)
            .map_err(internal)?
            .ok_or_else(|| RpcError::refused(ErrorKind::NotFound, format!("no request {rid}")))?,
        _ => {
            return Err(RpcError::refused(
                ErrorKind::Invalid,
                "give job_id or request_id, not both",
            ));
        }
    };
    serde_json::to_value(found).map_err(internal)
}
