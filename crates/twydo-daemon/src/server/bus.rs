//! The event bus: assigns `seq`, fans notifications out to subscribed connections without ever
//! blocking, and tells a subscriber that fell behind to refetch (`missed`).

use std::collections::HashMap;
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde::Serialize;
use serde_json::json;

use crate::protocol::events::{Event, EventEnvelope, Heartbeat, Missed};

/// A connection's id, assigned by the accept loop.
pub type ConnId = u64;

struct Sub {
    out: SyncSender<String>,
    /// A notification was dropped because `out` was full; `missed` is owed.
    behind: bool,
}

#[derive(Default)]
struct Inner {
    seq: u64,
    revision: u64,
    status_rev: u64,
    subs: HashMap<ConnId, Sub>,
}

pub struct EventBus {
    inner: Mutex<Inner>,
    stop: Mutex<bool>,
    stop_cv: Condvar,
}

fn line<P: Serialize>(method: &str, params: &P) -> String {
    json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string()
}

impl Inner {
    fn missed(&self) -> String {
        line(
            "missed",
            &Missed {
                seq: self.seq,
                revision: self.revision,
                status_rev: self.status_rev,
            },
        )
    }
}

impl EventBus {
    pub fn new() -> Arc<EventBus> {
        Arc::new(EventBus {
            inner: Mutex::new(Inner::default()),
            stop: Mutex::new(false),
            stop_cv: Condvar::new(),
        })
    }

    /// Adds (or replaces) the subscriber of `conn`; returns the current `seq`.
    pub fn subscribe(&self, conn: ConnId, out: SyncSender<String>) -> u64 {
        let mut g = self.inner.lock().unwrap();
        g.subs.insert(conn, Sub { out, behind: false });
        g.seq
    }

    pub fn unsubscribe(&self, conn: ConnId) {
        self.inner.lock().unwrap().subs.remove(&conn);
    }

    /// The seq of the last event.
    pub fn seq(&self) -> u64 {
        self.inner.lock().unwrap().seq
    }

    /// Assigns the next `seq` and offers the event to every subscriber with `try_send`. A full
    /// queue marks the subscriber behind; the next emit (or heartbeat) to it sends `missed`
    /// instead. Returns the seq.
    pub fn emit(&self, event: Event) -> u64 {
        let mut g = self.inner.lock().unwrap();
        g.seq += 1;
        match &event {
            Event::DataChanged { revision, .. } => g.revision = *revision,
            Event::StatusChanged { status_rev } => g.status_rev = *status_rev,
            Event::OpFinished { .. }
            | Event::SyncStarted { .. }
            | Event::SyncFinished { .. }
            | Event::JobFinished { .. }
            | Event::Unknown => {}
        }
        let seq = g.seq;
        let ev = line("event", &EventEnvelope { seq, event });
        let missed = g.missed();
        g.subs.retain(|_, s| {
            let msg = if s.behind { &missed } else { &ev };
            match s.out.try_send(msg.clone()) {
                Ok(()) => s.behind = false,
                Err(TrySendError::Full(_)) => s.behind = true,
                Err(TrySendError::Disconnected(_)) => return false,
            }
            true
        });
        seq
    }

    fn heartbeat(&self) {
        let mut g = self.inner.lock().unwrap();
        let hb = line("heartbeat", &Heartbeat { seq: g.seq });
        let missed = g.missed();
        g.subs.retain(|_, s| {
            let msg = if s.behind { &missed } else { &hb };
            match s.out.try_send(msg.clone()) {
                Ok(()) => s.behind = false,
                Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => return false,
            }
            true
        });
    }

    /// Starts the heartbeat thread; [`EventBus::stop`] ends it.
    pub fn start_heartbeat(self: &Arc<Self>, every: Duration) -> JoinHandle<()> {
        let bus = Arc::clone(self);
        std::thread::spawn(move || {
            let mut stopped = bus.stop.lock().unwrap();
            loop {
                let (g, t) = bus.stop_cv.wait_timeout(stopped, every).unwrap();
                stopped = g;
                if *stopped {
                    return;
                }
                if t.timed_out() {
                    bus.heartbeat();
                }
            }
        })
    }

    pub fn stop(&self) {
        *self.stop.lock().unwrap() = true;
        self.stop_cv.notify_all();
        self.inner.lock().unwrap().subs.clear();
    }
}
