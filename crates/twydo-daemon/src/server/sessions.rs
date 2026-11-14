//! Sessions (daemon spec §6.7): one undo/redo [`History`] per session. A session lives while a
//! connection holds it and for `session_ttl` after the last one closes; expiry is checked
//! lazily, when sessions are opened or looked up.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use twydo_core::store::{History, Submitted};

/// Locks session `m`. A panic while it was held (in a job's body) poisons it; the history is
/// still sound.
pub fn lock(m: &Mutex<Session>) -> MutexGuard<'_, Session> {
    m.lock().unwrap_or_else(|poisoned| {
        m.clear_poison();
        poisoned.into_inner()
    })
}

/// The message of a queued write, after its success text.
pub const QUEUED: &str = " · queued (a sync is running)";

pub struct Session {
    /// What `op.undo` would undo and `op.redo` redo, with the titles the messages name.
    pub history: History,
}

/// The note a queued write adds to its message.
pub fn queued_note(s: &Submitted) -> &'static str {
    if matches!(s, Submitted::Queued(_)) {
        QUEUED
    } else {
        ""
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
    ttl: Duration,
    slots: Mutex<HashMap<String, Slot>>,
    test_point: Option<super::TestPoint>,
}

impl Sessions {
    pub fn new(ttl: Duration) -> Sessions {
        Sessions {
            ttl,
            slots: Mutex::new(HashMap::new()),
            test_point: None,
        }
    }

    /// Passes the daemon's test point to the table (`sessions_locked`, `without_recovery`: test
    /// seams only).
    pub fn with_test_point(mut self, hook: Option<super::TestPoint>) -> Sessions {
        self.test_point = hook;
        self
    }

    fn point(&self, name: &str) {
        if let Some(hook) = &self.test_point {
            hook(name);
        }
    }

    /// The table, even if a panic poisoned it: its entries stay sound whatever a holder was
    /// doing when it panicked.
    fn table(&self) -> MutexGuard<'_, HashMap<String, Slot>> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Resumes `resume` if it is alive, else creates a new session; returns its id, attached
    /// once for the caller's connection (atomically, so it cannot expire in between; the
    /// caller [detaches](Sessions::detach) it when the connection closes). Expired sessions
    /// are dropped.
    pub fn open(&self, resume: Option<&str>) -> String {
        {
            let mut slots = self.table();
            self.point("sessions_locked");
            let now = Instant::now();
            slots.retain(|_, s| !s.expired(now, self.ttl));
            if let Some(id) = resume
                && let Some(s) = slots.get_mut(id)
            {
                s.conns += 1;
                s.detached = None;
                return id.to_string();
            }
        }
        // A test seam only: opening a session does no I/O now. It stays outside the table lock
        // so a test that blocks here never holds up other connections.
        self.point("without_recovery");
        let id = format!("s-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        self.table().insert(
            id.clone(),
            Slot {
                session: Arc::new(Mutex::new(Session {
                    history: History::new(),
                })),
                conns: 1,
                detached: None,
            },
        );
        id
    }

    /// The session `id`, unless it is unknown or expired.
    pub fn get(&self, id: &str) -> Option<Arc<Mutex<Session>>> {
        let mut slots = self.table();
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
        if let Some(s) = self.table().get_mut(id) {
            s.conns += 1;
            s.detached = None;
        }
    }

    /// A connection let go of `id`; the ttl starts when the last one does.
    pub fn detach(&self, id: &str) {
        if let Some(s) = self.table().get_mut(id) {
            s.conns = s.conns.saturating_sub(1);
            if s.conns == 0 {
                s.detached = Some(Instant::now());
            }
        }
    }
}
