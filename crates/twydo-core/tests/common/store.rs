//! A per-session store for tests: the call shapes the TUI once had (apply, submit, undo, redo,
//! drain) on top of the library's [`Writer`], [`History`] and planner. Nothing here is the
//! daemon's code; the daemon's writes are covered by its own suites.
#![allow(dead_code)]

use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};
use twydo_core::compat::Matrix;
use twydo_core::config::Config;
use twydo_core::edit::Op;
use twydo_core::store::journal::{RequestRef, try_recover};
use twydo_core::store::queue::Kind;
use twydo_core::store::{
    Access, Ctx, Drained, Fence, History, RecoveryReport, Submitted, WriteError, Writer,
    bind_vdir_state, recover_list,
};
use twydo_core::views::Snapshot;

/// How long `submit`, `drain` and the history try the vdir lock before they queue or give up.
const TRY_LOCK: Duration = Duration::from_millis(250);

#[derive(Debug)]
pub struct Store {
    pub writer: Writer,
    history: History,
    cfg: Config,
    request: Option<RequestRef>,
    busy: bool,
}

impl Store {
    /// [`Store::open_with`] the embedded compatibility matrix.
    pub fn open(cfg: &Config) -> Result<(Store, RecoveryReport), WriteError> {
        Store::open_with(cfg, Matrix::embedded().clone())
    }

    /// Recovers under a lock it does not wait for (a holder recovers first), then opens the
    /// writer. While any entry is stuck, writes return [`WriteError::Stuck`].
    pub fn open_with(cfg: &Config, matrix: Matrix) -> Result<(Store, RecoveryReport), WriteError> {
        // Recovery runs before the writer opens the journal: opening creates `journal/`, and a
        // state directory without one is skipped by recovery.
        let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir)?;
        let report = try_recover(&cfg.vdir, &state)?.unwrap_or_default();
        Ok((Store::without_recovery(cfg, matrix)?, report))
    }

    /// A store that skips recovery (the caller holds the vdir lock, or recovers itself).
    pub fn without_recovery(cfg: &Config, matrix: Matrix) -> Result<Store, WriteError> {
        Ok(Store {
            writer: Writer::open(cfg, matrix)?,
            history: History::new(),
            cfg: cfg.clone(),
            request: None,
            busy: false,
        })
    }

    /// Every entry or queue item this store creates until the next call carries `r`.
    pub fn set_request(&mut self, r: Option<RequestRef>) {
        self.request = r;
    }

    /// While `busy`, `submit`, `undo_last` and `redo_last` never try the vdir lock.
    pub fn set_vdir_busy(&mut self, busy: bool) {
        self.busy = busy;
    }

    pub fn state_dir(&self) -> &Path {
        self.writer.state_dir()
    }

    fn ctx(&self, now: DateTime<Utc>) -> Ctx<'_> {
        Ctx {
            now,
            request: self.request.clone(),
            config: &self.cfg,
        }
    }

    /// Apply `op` to the tasks loaded in `snap`, waiting for the vdir lock. Returns the journal
    /// entry id.
    pub fn apply(
        &mut self,
        snap: &Snapshot,
        op: &Op,
        now: DateTime<Utc>,
    ) -> Result<String, WriteError> {
        self.writer.check_fence(Fence::Now)?;
        let prepared = self.writer.plan(snap, op, now)?;
        let lock = self.writer.lock_waiting()?;
        let id = lock.commit(prepared, &self.ctx(now))?;
        self.history
            .record(String::new(), &Submitted::Applied(id.clone()));
        Ok(id)
    }

    /// Applies `op` when the vdir lock is free within 250 ms and queues it otherwise.
    pub fn submit(
        &mut self,
        view: &Snapshot,
        op: &Op,
        label: &str,
        tool: &str,
        now: DateTime<Utc>,
    ) -> Result<Submitted, WriteError> {
        self.writer.check_fence(Fence::Now)?;
        let prepared = self.writer.plan(view, op, now)?;
        let ctx = self.ctx(now);
        let lock = if self.busy {
            None
        } else {
            self.writer.lock(TRY_LOCK)?
        };
        let submitted = match lock {
            Some(lock) => {
                lock.drain(&ctx)?;
                Submitted::Applied(lock.commit(prepared, &ctx)?)
            }
            None => {
                let kind = Kind::Op {
                    op: op.clone(),
                    prepared,
                };
                Submitted::Queued(self.writer.enqueue(kind, label, tool, Fence::Now, &ctx)?)
            }
        };
        self.history.record(String::new(), &submitted);
        Ok(submitted)
    }

    /// Commits the queued items, oldest first. [`WriteError::Busy`] if a holder keeps the lock.
    pub fn drain(&mut self, now: DateTime<Utc>) -> Result<Vec<Drained>, WriteError> {
        let lock = self.writer.lock(TRY_LOCK)?.ok_or(WriteError::Busy)?;
        lock.drain(&self.ctx(now))
    }

    /// Writes one file through the journal (kind "resolve") without the gate.
    pub fn restore(
        &mut self,
        path: &str,
        expect: Option<&str>,
        content: Option<&str>,
        uids: Vec<String>,
        now: DateTime<Utc>,
    ) -> Result<String, WriteError> {
        let lock = self.writer.lock_waiting()?;
        lock.restore(path, expect, content, uids, &self.ctx(now))
    }

    /// [`recover_list`] under the vdir lock, recorded on the history when it wrote.
    pub fn recover_list(
        &mut self,
        active: &Snapshot,
        backup_id: &str,
        to: &str,
        now: DateTime<Utc>,
    ) -> Result<(String, bool), WriteError> {
        let lock = self.writer.lock_waiting()?;
        let (id, written) = recover_list(&lock, active, backup_id, to, &self.ctx(now))?;
        drop(lock);
        if written {
            self.history
                .record(String::new(), &Submitted::Applied(id.clone()));
        }
        Ok((id, written))
    }

    /// Revert entry `id` with a new entry: applied now when the lock is free, queued otherwise.
    pub fn undo(&mut self, id: &str, now: DateTime<Utc>) -> Result<Submitted, WriteError> {
        let ctx = self.ctx(now);
        let lock = if self.busy {
            None
        } else {
            self.writer.lock(TRY_LOCK)?
        };
        let done = match lock {
            Some(lock) => {
                lock.drain(&ctx)?;
                Submitted::Applied(lock.revert(id, &ctx)?)
            }
            None => {
                let kind = self.writer.revert_item(id)?;
                Submitted::Queued(self.writer.enqueue(kind, "undo", "op", Fence::Now, &ctx)?)
            }
        };
        self.history.forget_applied(id);
        Ok(done)
    }

    /// Undo the last step applied, queued or redone this session.
    pub fn undo_last(&mut self, now: DateTime<Utc>) -> Result<Submitted, WriteError> {
        self.step_history(true, now)
    }

    /// Redo the last undo of this session by reverting it.
    pub fn redo_last(&mut self, now: DateTime<Utc>) -> Result<Submitted, WriteError> {
        self.step_history(false, now)
    }

    fn step_history(&mut self, undo: bool, now: DateTime<Utc>) -> Result<Submitted, WriteError> {
        let ctx = Ctx {
            now,
            request: self.request.clone(),
            config: &self.cfg,
        };
        let access = if self.busy {
            Access::Queue(&self.writer, Fence::Now)
        } else {
            Access::Try {
                writer: &self.writer,
                wait: TRY_LOCK,
                fence: Fence::Now,
            }
        };
        let mut sink = Vec::new();
        let moved = if undo {
            self.history.undo(access, "tui", &ctx, &mut sink)
        } else {
            self.history.redo(access, "tui", &ctx, &mut sink)
        };
        moved.map(|(done, _)| done)
    }

    /// Mark the unfinished entry `id` Abandoned so writes can continue.
    pub fn abandon(&mut self, id: &str) -> Result<bool, WriteError> {
        self.writer.lock_waiting()?.abandon(id)
    }
}
