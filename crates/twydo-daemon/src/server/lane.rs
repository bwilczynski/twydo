//! The vdir lane token (daemon spec §5): one holder at a time among task writes and long jobs
//! (sync, list jobs). A write that finds a long job holding the token does not wait: it is
//! queued instead (the caller sees [`LongBusy`]). There is no worker thread; each holder runs
//! on its own request thread and the token is released when its guard drops.

use std::sync::{Condvar, Mutex};

/// What a long job is, for messages and for labelling reloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LongKind {
    Sync,
    List,
    Test,
}

/// A long job holds the lane: the write must be queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LongBusy(pub LongKind);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Holder {
    Free,
    Write,
    Long(LongKind),
}

/// Runs when a long job releases the lane, before the token is freed (Task 8 drains the queue
/// here). It must not take a session lock or the lane (see the lock order in `writes.rs`).
pub type DrainHook = Box<dyn Fn() + Send + Sync>;

pub struct Lane {
    holder: Mutex<Holder>,
    cv: Condvar,
    /// Empty in plan 1 task 7; Task 8 sets it with [`Lane::set_drain_hook`].
    drain_hook: Mutex<Option<DrainHook>>,
}

impl Default for Lane {
    fn default() -> Self {
        Lane::new()
    }
}

impl Lane {
    pub fn new() -> Lane {
        Lane {
            holder: Mutex::new(Holder::Free),
            cv: Condvar::new(),
            drain_hook: Mutex::new(None),
        }
    }

    /// Sets what runs when a long job ends (before the lane is free again).
    pub fn set_drain_hook(&self, hook: DrainHook) {
        *self.drain_hook.lock().unwrap() = Some(hook);
    }

    /// Takes the token for one write: waits while another write holds it, returns
    /// [`LongBusy`] at once while a long job does.
    pub fn write(&self) -> Result<WriteGuard<'_>, LongBusy> {
        let mut h = self.holder.lock().unwrap();
        loop {
            match *h {
                Holder::Free => {
                    *h = Holder::Write;
                    return Ok(WriteGuard { lane: self });
                }
                Holder::Long(kind) => return Err(LongBusy(kind)),
                Holder::Write => h = self.cv.wait(h).unwrap(),
            }
        }
    }

    /// Takes the token for a long job, waiting until it is free.
    pub fn long(&self, kind: LongKind) -> LongGuard<'_> {
        let mut h = self.holder.lock().unwrap();
        while *h != Holder::Free {
            h = self.cv.wait(h).unwrap();
        }
        *h = Holder::Long(kind);
        LongGuard { lane: self }
    }

    /// The long job holding the lane, if any.
    pub fn current_long(&self) -> Option<LongKind> {
        match *self.holder.lock().unwrap() {
            Holder::Long(k) => Some(k),
            _ => None,
        }
    }

    fn release(&self) {
        *self.holder.lock().unwrap() = Holder::Free;
        self.cv.notify_all();
    }
}

/// The lane held for one write (plan, commit, publish).
pub struct WriteGuard<'a> {
    lane: &'a Lane,
}

impl Drop for WriteGuard<'_> {
    fn drop(&mut self) {
        self.lane.release();
    }
}

/// The lane held by a long job. Dropping it runs the drain hook, then frees the lane.
pub struct LongGuard<'a> {
    lane: &'a Lane,
}

impl Drop for LongGuard<'_> {
    fn drop(&mut self) {
        // The hook runs while the lane is still held, so no write slips in between the job and
        // the drain of what was queued during it. Only the hook slot's own lock is taken.
        if let Some(hook) = self.lane.drain_hook.lock().unwrap().as_ref() {
            hook();
        }
        self.lane.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    #[test]
    fn a_write_during_a_long_job_is_busy_at_once() {
        let lane = Lane::new();
        let g = lane.long(LongKind::Sync);
        assert_eq!(lane.current_long(), Some(LongKind::Sync));
        assert_eq!(lane.write().err(), Some(LongBusy(LongKind::Sync)));
        drop(g);
        assert_eq!(lane.current_long(), None);
        assert!(lane.write().is_ok());
    }

    #[test]
    fn a_long_job_waits_for_a_write_and_runs_the_hook_on_release() {
        let lane = Arc::new(Lane::new());
        let ran = Arc::new(AtomicBool::new(false));
        {
            let ran = Arc::clone(&ran);
            lane.set_drain_hook(Box::new(move || ran.store(true, Ordering::SeqCst)));
        }
        let w = lane.write().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let l2 = Arc::clone(&lane);
        let h = std::thread::spawn(move || {
            let _g = l2.long(LongKind::Test);
            tx.send(()).unwrap();
        });
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        drop(w);
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        h.join().unwrap();
        assert!(ran.load(Ordering::SeqCst));
        assert_eq!(lane.current_long(), None);
    }
}
