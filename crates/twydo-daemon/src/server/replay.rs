//! The in-memory request log (daemon spec §6.6, "replay comes first"): the results of this
//! epoch's requests by `request_id`, and the requests still running, so a resent request
//! returns the first result and a concurrent duplicate waits for it instead of running again.
//! Successes and deterministic refusals (`blocked`, `stale`, `invalid`, `not_found`) are kept,
//! so a resend gets the first answer even if the session changed since (a resent undo whose
//! "nothing to undo" was lost must not undo a later edit). Transient refusals (`unavailable`,
//! `stuck`, `internal`) are not kept: they wrote nothing, and a fresh run is allowed.

use std::collections::HashMap;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::protocol::rpc::{ErrorKind, RpcError};

/// How long a result is kept in memory; older ones are pruned when a new one is added. The
/// durable records (journal, queue) answer for longer.
const KEEP: Duration = Duration::from_secs(24 * 60 * 60);

enum Slot {
    Running,
    Done {
        result: Result<Value, RpcError>,
        at: Instant,
    },
}

#[derive(Default)]
pub struct RequestLog {
    slots: Mutex<HashMap<String, Slot>>,
    cv: Condvar,
}

/// What [`RequestLog::begin`] found.
pub enum Begin<'a> {
    /// First sight of the id: run the request and [finish](Ticket::finish) the ticket.
    New(Ticket<'a>),
    /// A result recorded for the id.
    Done(Result<Value, RpcError>),
}

/// The right and the duty to run a request. Dropped unfinished (a panic), it frees the id so
/// a waiter runs it instead.
pub struct Ticket<'a> {
    log: &'a RequestLog,
    id: String,
    finished: bool,
}

impl RequestLog {
    pub fn new() -> RequestLog {
        RequestLog::default()
    }

    /// Claims `request_id`, or returns its recorded result. While another request with the
    /// same id runs, waits for it to finish.
    pub fn begin(&self, request_id: &str) -> Begin<'_> {
        let mut slots = self.slots.lock().unwrap();
        loop {
            match slots.get(request_id) {
                None => {
                    slots.insert(request_id.to_string(), Slot::Running);
                    return Begin::New(Ticket {
                        log: self,
                        id: request_id.to_string(),
                        finished: false,
                    });
                }
                Some(Slot::Done { result, .. }) => return Begin::Done(result.clone()),
                Some(Slot::Running) => slots = self.cv.wait(slots).unwrap(),
            }
        }
    }

    /// The recorded result of `request_id`, without waiting: `Some(None)` while it runs.
    pub fn peek(&self, request_id: &str) -> Option<Option<Result<Value, RpcError>>> {
        match self.slots.lock().unwrap().get(request_id)? {
            Slot::Running => Some(None),
            Slot::Done { result, .. } => Some(Some(result.clone())),
        }
    }

    fn settle(&self, id: &str, result: Option<Result<Value, RpcError>>) {
        let mut slots = self.slots.lock().unwrap();
        match result {
            Some(result) => {
                let now = Instant::now();
                slots.retain(|_, s| match s {
                    Slot::Done { at, .. } => now.duration_since(*at) < KEEP,
                    Slot::Running => true,
                });
                slots.insert(id.to_string(), Slot::Done { result, at: now });
            }
            None => {
                slots.remove(id);
            }
        }
        self.cv.notify_all();
    }
}

impl Ticket<'_> {
    /// Records the result (unless it is a transient refusal, see the module docs) and wakes
    /// the waiters. Returns `result` for the caller to answer with.
    pub fn finish(mut self, result: Result<Value, RpcError>) -> Result<Value, RpcError> {
        self.finished = true;
        let keep = match &result {
            Ok(_) => true,
            Err(e) => matches!(
                e.data.as_ref().map(|d| d.kind),
                Some(
                    ErrorKind::Blocked
                        | ErrorKind::Stale
                        | ErrorKind::Invalid
                        | ErrorKind::NotFound
                )
            ),
        };
        self.log.settle(&self.id, keep.then(|| result.clone()));
        result
    }
}

impl Ticket<'_> {
    /// Answers with `result` without recording it (the id stays free for a fresh run).
    pub fn forget(mut self, result: Result<Value, RpcError>) -> Result<Value, RpcError> {
        self.finished = true;
        self.log.settle(&self.id, None);
        result
    }
}

impl Drop for Ticket<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.log.settle(&self.id, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    #[test]
    fn a_duplicate_waits_for_the_first_and_gets_its_result() {
        let log = Arc::new(RequestLog::new());
        let Begin::New(t) = log.begin("r1") else {
            panic!("new")
        };
        let l2 = Arc::clone(&log);
        let h = std::thread::spawn(move || match l2.begin("r1") {
            Begin::Done(r) => r.unwrap(),
            Begin::New(_) => panic!("ran twice"),
        });
        assert_eq!(log.peek("r1"), Some(None));
        t.finish(Ok(json!(1))).unwrap();
        assert_eq!(h.join().unwrap(), json!(1));
    }

    #[test]
    fn a_transient_failure_is_not_kept() {
        let log = RequestLog::new();
        for kind in [
            ErrorKind::Unavailable,
            ErrorKind::Stuck,
            ErrorKind::Internal,
        ] {
            let Begin::New(t) = log.begin("r1") else {
                panic!("new")
            };
            let _ = t.finish(Err(RpcError::refused(kind, "x")));
            assert!(matches!(log.begin("r1"), Begin::New(_)), "{kind:?}");
        }
    }

    #[test]
    fn a_deterministic_refusal_is_kept() {
        let log = RequestLog::new();
        for (i, kind) in [
            ErrorKind::Blocked,
            ErrorKind::Stale,
            ErrorKind::Invalid,
            ErrorKind::NotFound,
        ]
        .into_iter()
        .enumerate()
        {
            let id = format!("r{i}");
            let Begin::New(t) = log.begin(&id) else {
                panic!("new")
            };
            let _ = t.finish(Err(RpcError::refused(kind, "x")));
            match log.begin(&id) {
                Begin::Done(Err(e)) => assert_eq!(e.data.unwrap().kind, kind),
                _ => panic!("{kind:?} not kept"),
            }
        }
    }
}
