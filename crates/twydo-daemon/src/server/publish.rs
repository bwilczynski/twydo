//! The publisher: loads the vdir, keeps the last published revisions and the status, and
//! emits `data.changed` and `status.changed`. One mutex serialises every publish, so
//! revisions and the `seq` of their events rise together.

use std::collections::{HashSet, VecDeque};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};

use twydo_core::config::Config;
use twydo_core::store::fsio::VdirLock;
use twydo_core::store::journal::Journal;
use twydo_core::store::queue::{Queue, refusal_message, view};
use twydo_core::sync;
use twydo_core::vdir::{self, LoadReport};
use twydo_core::views::Snapshot as CoreSnapshot;

use super::bus::{ConnId, EventBus};
use crate::protocol::events::{Cause, Event};
use crate::protocol::methods::{
    JournalView, RefusalView, StatusView, SubscribeResult, SyncState, SyncView,
};
use crate::protocol::wire::{WireSnapshot, to_wire};

/// How many published revisions are kept for `at`.
const KEPT: usize = 32;

/// One published revision: the planning view (queued edits laid over the files) and its wire
/// form.
pub struct Snapshot {
    pub revision: u64,
    pub snap: CoreSnapshot,
    pub wire: WireSnapshot,
}

struct State {
    ring: VecDeque<Arc<Snapshot>>,
    published_once: bool,
    /// What the last [`Publisher::reload`] read from the vdir; [`Publisher::overlay`] lays the
    /// queue over it without reading the vdir again.
    last_report: Option<LoadReport>,
    status: Option<StatusView>,
    stuck: Vec<String>,
    /// Why the last queue drain stopped with items left; see [`StatusView::drain_error`].
    drain_error: Option<String>,
    /// A sync is running, and the job it belongs to; see [`Publisher::set_sync_state`].
    sync_running: Option<Option<String>>,
}

pub struct Publisher {
    cfg: Config,
    state_dir: PathBuf,
    bus: Arc<EventBus>,
    /// Serialises reloads, overlays and status refreshes (slow I/O) so they publish in order,
    /// without holding `state`: readers never wait behind them. Never held while waiting for
    /// the vdir lock: [`Publisher::reload`] takes that first.
    reload_lock: Mutex<()>,
    state: Mutex<State>,
    /// Unreadable queue items already logged, so each is logged once.
    logged_unreadable: Mutex<HashSet<String>>,
    /// Set at teardown: reloads and status refreshes that get `reload_lock` afterwards do
    /// nothing (a caller blocked behind a vdir-lock wait may get it after the daemon stopped).
    closed: AtomicBool,
}

impl Publisher {
    pub fn new(cfg: Config, state_dir: PathBuf, bus: Arc<EventBus>) -> Publisher {
        let empty = CoreSnapshot::new(LoadReport::default(), cfg.clone());
        let wire = to_wire(&empty, 0);
        let first = Arc::new(Snapshot {
            revision: 0,
            snap: empty,
            wire,
        });
        Publisher {
            cfg,
            state_dir,
            bus,
            reload_lock: Mutex::new(()),
            closed: AtomicBool::new(false),
            logged_unreadable: Mutex::new(HashSet::new()),
            state: Mutex::new(State {
                ring: VecDeque::from([first]),
                published_once: false,
                last_report: None,
                status: None,
                stuck: Vec::new(),
                drain_error: None,
                sync_running: None,
            }),
        }
    }

    /// Stops publishing: later reloads and status refreshes return the current marks.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Ids of the journal entries recovery left stuck.
    pub fn set_stuck(&self, ids: Vec<String>) {
        self.state.lock().unwrap().stuck = ids;
    }

    /// Takes `id` off the stuck list (the entry was abandoned). Takes effect at the next
    /// [`Publisher::refresh_status`].
    pub fn unstick(&self, id: &str) {
        self.state.lock().unwrap().stuck.retain(|s| s != id);
    }

    /// Records the outcome of a queue drain for the status (`None`: nothing left behind).
    /// Takes effect at the next [`Publisher::refresh_status`].
    pub fn set_drain_error(&self, e: Option<String>) {
        self.state.lock().unwrap().drain_error = e;
    }

    /// What `status.sync.state` and `status.sync.job_id` report: running (with the job of a
    /// `sync.run`) or idle. Takes effect at the next [`Publisher::refresh_status`].
    pub fn set_sync_state(&self, running: bool, job_id: Option<String>) {
        self.state.lock().unwrap().sync_running = running.then_some(job_id);
    }

    /// Whether a sync runs in this daemon now (as [`Publisher::set_sync_state`] last said).
    pub fn sync_running(&self) -> bool {
        self.state.lock().unwrap().sync_running.is_some()
    }

    /// The newest published revision.
    pub fn current(&self) -> (u64, Arc<Snapshot>) {
        let s = self.state.lock().unwrap();
        let cur = Arc::clone(s.ring.back().expect("ring is never empty"));
        (cur.revision, cur)
    }

    /// A kept revision, or `None` once it has left the ring.
    pub fn at(&self, revision: u64) -> Option<Arc<Snapshot>> {
        let s = self.state.lock().unwrap();
        // Revision 0 is the placeholder before the first publish, never a published one.
        if revision == 0 {
            return None;
        }
        s.ring.iter().find(|p| p.revision == revision).cloned()
    }

    /// Loads the vdir; publishes a new revision (and `data.changed`) only when something
    /// differs from the current one. Returns the current revision either way.
    ///
    /// Lock order: the shared vdir lock first (a sync may hold it exclusively for minutes),
    /// `reload_lock` second. Nothing waits for the vdir lock while holding `reload_lock`, so
    /// a reload stuck behind a sync never blocks [`Publisher::overlay`] or a status refresh.
    pub fn reload(&self, cause: Cause, uids: Vec<String>) -> io::Result<u64> {
        let _lock = VdirLock::reader(&self.state_dir)?;
        let _order = self.reload_lock.lock().unwrap();
        // The vdir-lock wait above may have outlasted the daemon.
        if self.is_closed() {
            return Ok(self.current().0);
        }
        let report = vdir::load(&self.cfg.vdir)?;
        let snap = view(report.clone(), self.cfg.clone());
        self.publish(snap, Some(report), cause, uids)
    }

    /// Like [`Publisher::reload`] but without reading the vdir or taking its lock: lays the
    /// current queue over the report the last reload loaded, so a queued edit shows as pending
    /// while a long job holds the vdir. Before the first reload there is no report and this
    /// does nothing (returns the current revision).
    pub fn overlay(&self, cause: Cause, uids: Vec<String>) -> io::Result<u64> {
        let _order = self.reload_lock.lock().unwrap();
        if self.is_closed() {
            return Ok(self.current().0);
        }
        let Some(report) = self.state.lock().unwrap().last_report.clone() else {
            return Ok(self.current().0);
        };
        let snap = view(report, self.cfg.clone());
        self.publish(snap, None, cause, uids)
    }

    /// Publishes `snap` as a new revision when its wire form differs from the current one;
    /// `report`, when given, becomes the report `overlay` builds on. Callers hold `reload_lock`.
    fn publish(
        &self,
        snap: CoreSnapshot,
        report: Option<LoadReport>,
        cause: Cause,
        uids: Vec<String>,
    ) -> io::Result<u64> {
        let mut wire = to_wire(&snap, 0);
        let mut s = self.state.lock().unwrap();
        // Teardown may have started while the snapshot was built.
        if self.is_closed() {
            return Ok(s.ring.back().expect("ring is never empty").revision);
        }
        if report.is_some() {
            s.last_report = report;
        }
        let cur = s.ring.back().expect("ring is never empty");
        wire.revision = cur.revision;
        if s.published_once && wire == cur.wire {
            return Ok(cur.revision);
        }
        let revision = cur.revision + 1;
        wire.revision = revision;
        // Without a hint (the watcher, recovery), the uids are the tasks whose file changed.
        let uids = if uids.is_empty() {
            changed_uids(&cur.wire, &wire)
        } else {
            uids
        };
        s.published_once = true;
        s.ring.push_back(Arc::new(Snapshot {
            revision,
            snap,
            wire,
        }));
        while s.ring.len() > KEPT {
            s.ring.pop_front();
        }
        self.bus.emit(Event::DataChanged {
            revision,
            cause,
            uids,
        });
        Ok(revision)
    }

    /// Rebuilds the status; bumps `status_rev` and emits `status.changed` only when the
    /// content changed. Returns the current `status_rev`.
    pub fn refresh_status(&self) -> u64 {
        let _order = self.reload_lock.lock().unwrap();
        if self.is_closed() {
            return self
                .state
                .lock()
                .unwrap()
                .status
                .as_ref()
                .map_or(0, |v| v.status_rev);
        }
        let (stuck, drain_error, sync_running, old_rev) = {
            let s = self.state.lock().unwrap();
            (
                s.stuck.clone(),
                s.drain_error.clone(),
                s.sync_running.clone(),
                s.status.as_ref().map_or(0, |v| v.status_rev),
            )
        };
        let mut next = match self.build_status(&stuck, drain_error, sync_running, old_rev) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("tdo daemon: status unavailable: {e}");
                return old_rev;
            }
        };
        let mut s = self.state.lock().unwrap();
        if self.is_closed() {
            return old_rev;
        }
        if s.status.as_ref() == Some(&next) {
            return old_rev;
        }
        next.status_rev = old_rev + 1;
        let rev = next.status_rev;
        s.status = Some(next);
        self.bus.emit(Event::StatusChanged { status_rev: rev });
        rev
    }

    fn build_status(
        &self,
        stuck: &[String],
        drain_error: Option<String>,
        sync_running: Option<Option<String>>,
        status_rev: u64,
    ) -> io::Result<StatusView> {
        let summary = sync::summary(&self.cfg)?;
        let queue = Queue::existing(&self.state_dir);
        let refusals = match &queue {
            Some(q) => q
                .unshown()?
                .into_iter()
                .map(|r| RefusalView {
                    id: r.item.id.clone(),
                    message: refusal_message(&r),
                    session: r.item.request.as_ref().map(|q| q.session.clone()),
                    request_id: r.item.request.as_ref().map(|q| q.request_id.clone()),
                    at: r.at,
                })
                .collect(),
            None => Vec::new(),
        };
        let queue_unreadable: Vec<String> = match &queue {
            Some(q) => q
                .unreadable()?
                .into_iter()
                .map(|p| p.display().to_string())
                .collect(),
            None => Vec::new(),
        };
        {
            let mut logged = self.logged_unreadable.lock().unwrap();
            for p in &queue_unreadable {
                if logged.insert(p.clone()) {
                    eprintln!("tdo daemon: unreadable queue item {p}");
                }
            }
        }
        let (sync_view, pending_edits, conflicts, unfinished) = match &summary {
            Some(m) => (
                Some(SyncView {
                    state: if sync_running.is_some() {
                        SyncState::Running
                    } else {
                        SyncState::Idle
                    },
                    auto: !m.off,
                    last_success: m.status.last_success,
                    last_attempt: m.status.last_attempt,
                    next_attempt: m.status.next_attempt,
                    failures: m.status.failures,
                    last_error: m.status.last_error.clone(),
                    job_id: sync_running.flatten(),
                }),
                m.queued,
                m.conflicts.len(),
                m.pending,
            ),
            None => {
                let queued = match &queue {
                    Some(q) => q.len()?,
                    None => 0,
                };
                let unfinished = if self.state_dir.join("journal").is_dir() {
                    Journal::open(&self.state_dir)?.pending()?.len()
                } else {
                    0
                };
                (None, queued, 0, unfinished)
            }
        };
        Ok(StatusView {
            status_rev,
            sync: sync_view,
            pending_edits,
            conflicts,
            journal: JournalView {
                unfinished,
                stuck: stuck.to_vec(),
            },
            refusals,
            drain_error,
            queue_unreadable,
        })
    }

    pub fn status(&self) -> Option<StatusView> {
        self.state.lock().unwrap().status.clone()
    }

    /// Subscribes `conn`, atomically with respect to publishing: the marks returned are the
    /// state before the first event the subscriber receives.
    pub fn subscribe(&self, conn: ConnId, out: SyncSender<String>) -> SubscribeResult {
        let s = self.state.lock().unwrap();
        let seq = self.bus.subscribe(conn, out);
        SubscribeResult {
            seq,
            revision: s.ring.back().map_or(0, |p| p.revision),
            status_rev: s.status.as_ref().map_or(0, |v| v.status_rev),
        }
    }
}

/// Uids whose task was added, removed or changed (by `rev` or `pending`) between two
/// snapshots, sorted.
fn changed_uids(old: &WireSnapshot, new: &WireSnapshot) -> Vec<String> {
    use std::collections::BTreeMap;
    let key = |s: &WireSnapshot| -> BTreeMap<String, (String, bool)> {
        s.tasks
            .iter()
            .map(|t| (t.task.uid.clone(), (t.rev.clone(), t.pending)))
            .collect()
    };
    let (a, b) = (key(old), key(new));
    let mut out: Vec<String> = a
        .iter()
        .filter(|(u, v)| b.get(*u) != Some(v))
        .map(|(u, _)| u.clone())
        .collect();
    out.extend(b.keys().filter(|u| !a.contains_key(*u)).cloned());
    out.sort();
    out.dedup();
    out
}
