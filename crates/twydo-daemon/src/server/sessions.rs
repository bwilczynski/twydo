//! Sessions (daemon spec §6.7): one store and one pair of undo/redo title stacks per session.
//! A session lives while a connection holds it and for `session_ttl` after the last one
//! closes; expiry is checked lazily, when sessions are opened or looked up.
//!
//! The title stacks and their messages are ported from the TUI's `Writer`
//! (`crates/tdo/src/write.rs`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use twydo_core::compat::Matrix;
use twydo_core::config::Config;
use twydo_core::store::queue::{Queue, Queued};
use twydo_core::store::{Store, Submitted, WriteError};

/// The message of a queued write, after its success text.
pub const QUEUED: &str = " · queued (a sync is running)";

/// A title on an undo or redo stack. `queued`: the id of the queue item that stands for the
/// write, so a refusal can drop the step the store skips. `op_id`: the journal entry or queue
/// item the step names, for `undid`.
#[derive(Debug, Clone)]
pub struct Titled {
    pub title: String,
    pub queued: Option<String>,
    pub op_id: Option<String>,
}

impl Titled {
    pub fn new(title: impl Into<String>, s: &Submitted) -> Titled {
        let (queued, op_id) = match s {
            Submitted::Applied(id) => (None, Some(id.clone())),
            Submitted::Queued(item) => (Some(item.id.clone()), Some(item.id.clone())),
            Submitted::Cancelled => (None, None),
        };
        Titled {
            title: title.into(),
            queued,
            op_id,
        }
    }
}

pub struct Session {
    pub store: Store,
    /// Titles of the writes `u` would undo and `Ctrl-r` would redo, in step with the store's
    /// stacks, so the messages can name the task.
    pub done: Vec<Titled>,
    pub undone: Vec<Titled>,
}

impl Session {
    /// Records a new write: it goes on the undo stack and clears the redo stack.
    pub fn pushed(&mut self, title: String, s: &Submitted) {
        self.done.push(Titled::new(title, s));
        self.undone.clear();
    }

    /// Drops the steps whose queued item a drain refused, as the store skips them.
    pub fn prune_refused(&mut self, queue: Option<&Queue>) {
        let Some(queue) = queue else { return };
        let refused = |t: &Titled| {
            t.queued
                .as_ref()
                .is_some_and(|id| matches!(queue.state(id), Ok(Some(Queued::Refused(_)))))
        };
        self.done.retain(|t| !refused(t));
        self.undone.retain(|t| !refused(t));
    }
}

/// The note a queued write adds to its message.
pub fn queued_note(s: &Submitted) -> &'static str {
    if matches!(s, Submitted::Queued(_)) {
        QUEUED
    } else {
        ""
    }
}

/// Moves the top title from `from` to `to` and names it in the message; returns the message
/// and the moved title. `s`: what the store did, for the step on the other stack.
pub fn step(
    verb: &str,
    from: &mut Vec<Titled>,
    to: &mut Vec<Titled>,
    s: &Submitted,
) -> (String, Option<Titled>) {
    match from.pop() {
        Some(t) => {
            let message = format!("{verb}: {}", t.title);
            to.push(Titled::new(t.title.clone(), s));
            (message, Some(t))
        }
        None => (verb.to_string(), None),
    }
}

struct Slot {
    session: Arc<Mutex<Session>>,
    /// Connections holding the session.
    conns: usize,
    /// When the last connection let go.
    detached: Option<Instant>,
}

impl Slot {
    fn expired(&self, now: Instant, ttl: Duration) -> bool {
        self.conns == 0 && self.detached.is_some_and(|d| now.duration_since(d) >= ttl)
    }
}

pub struct Sessions {
    cfg: Config,
    matrix: Matrix,
    ttl: Duration,
    slots: Mutex<HashMap<String, Slot>>,
}

impl Sessions {
    pub fn new(cfg: Config, matrix: Matrix, ttl: Duration) -> Sessions {
        Sessions {
            cfg,
            matrix,
            ttl,
            slots: Mutex::new(HashMap::new()),
        }
    }

    /// Resumes `resume` if it is alive, else creates a new session; returns its id, attached
    /// once for the caller's connection (atomically, so it cannot expire in between; the
    /// caller [detaches](Sessions::detach) it when the connection closes). Expired sessions
    /// are dropped.
    pub fn open(&self, resume: Option<&str>) -> Result<String, WriteError> {
        let mut slots = self.slots.lock().unwrap();
        let now = Instant::now();
        slots.retain(|_, s| !s.expired(now, self.ttl));
        if let Some(id) = resume
            && let Some(s) = slots.get_mut(id)
        {
            s.conns += 1;
            s.detached = None;
            return Ok(id.to_string());
        }
        let mut store = Store::without_recovery(&self.cfg, self.matrix.clone())?;
        // Writes hand the items they drained on their way to `op.finished`.
        store.set_collect_drained(true);
        let id = format!("s-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        slots.insert(
            id.clone(),
            Slot {
                session: Arc::new(Mutex::new(Session {
                    store,
                    done: Vec::new(),
                    undone: Vec::new(),
                })),
                conns: 1,
                detached: None,
            },
        );
        Ok(id)
    }

    /// The session `id`, unless it is unknown or expired.
    pub fn get(&self, id: &str) -> Option<Arc<Mutex<Session>>> {
        let mut slots = self.slots.lock().unwrap();
        if slots
            .get(id)
            .is_some_and(|s| s.expired(Instant::now(), self.ttl))
        {
            slots.remove(id);
            return None;
        }
        slots.get(id).map(|s| Arc::clone(&s.session))
    }

    /// A connection holds `id`.
    pub fn attach(&self, id: &str) {
        if let Some(s) = self.slots.lock().unwrap().get_mut(id) {
            s.conns += 1;
            s.detached = None;
        }
    }

    /// A connection let go of `id`; the ttl starts when the last one does.
    pub fn detach(&self, id: &str) {
        if let Some(s) = self.slots.lock().unwrap().get_mut(id) {
            s.conns = s.conns.saturating_sub(1);
            if s.conns == 0 {
                s.detached = Some(Instant::now());
            }
        }
    }
}
