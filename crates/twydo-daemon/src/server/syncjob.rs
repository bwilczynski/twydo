//! Syncs inside the daemon: [`run_sync`] runs one under the vdir lane, [`SyncDone::finish`]
//! publishes its end once the lane is let go, and `sync.run` starts one as a job.

use std::sync::Arc;

use chrono::Utc;
use serde_json::Value;
use twydo_core::sync::report::{NOT_CONFIGURED, collection_id, messages};
use twydo_core::sync::run::{RunOptions, run};

use super::Shared;
use super::dispatch::ConnState;
use super::jobs::JobBody;
use super::lane::LongKind;
use super::writes;
use crate::protocol::events::{Cause, Event};
use crate::protocol::jobs::{JobKind, JobResult};
use crate::protocol::rpc::{ErrorKind, RpcError};
use crate::protocol::sync::{SyncOutcome, SyncReason, SyncResult, SyncRunParams};

/// A sync that has run: what is left to do once the lane is free.
pub struct SyncDone {
    job_id: Option<String>,
    reason: SyncReason,
    pub result: SyncResult,
}

impl SyncDone {
    /// The end of a sync, after the caller let the lane go (so the drain hook has run):
    /// publishes the vdir as the sync left it, reports the sync idle and sends
    /// `sync.finished` with the status after it.
    pub fn finish(self, shared: &Arc<Shared>) {
        if let Err(e) = shared.publisher.reload(Cause::Sync, Vec::new()) {
            eprintln!("tdo daemon: reload after a sync failed: {e}");
        }
        shared.publisher.set_sync_state(false, None);
        let status_rev = shared.publisher.refresh_status();
        shared.bus.emit(Event::SyncFinished {
            job_id: self.job_id,
            reason: self.reason,
            result: self.result,
            status_rev,
        });
    }
}

/// Runs one sync. The caller holds the lane long, and calls [`SyncDone::finish`] after
/// letting it go. The queued edits the runner drained get their `op.finished` here.
pub fn run_sync(
    shared: &Arc<Shared>,
    job_id: Option<&str>,
    reason: SyncReason,
    opts: RunOptions,
) -> SyncDone {
    let job_id = job_id.map(str::to_string);
    shared.publisher.set_sync_state(true, job_id.clone());
    shared.publisher.refresh_status();
    shared.bus.emit(Event::SyncStarted {
        job_id: job_id.clone(),
        reason,
    });
    let result = match run(&shared.cfg, &opts, Utc::now()) {
        Ok(mut report) => {
            let m = messages(&shared.cfg, &report);
            let drained = std::mem::take(&mut report.drained);
            let count = drained.len();
            writes::finish_drain(shared, drained);
            let outcome = if report.stopped {
                SyncOutcome::Stopped
            } else if report.skipped.is_some() {
                SyncOutcome::Skipped
            } else if !m.problems.is_empty() {
                SyncOutcome::Failed
            } else {
                SyncOutcome::Ok
            };
            SyncResult {
                outcome,
                problems: m.problems,
                warnings: m.warnings,
                notes: m.notes,
                new_conflicts: report.new_conflicts.len(),
                drained: count,
            }
        }
        Err(e) => SyncResult {
            outcome: SyncOutcome::Failed,
            problems: vec![e.to_string()],
            warnings: Vec::new(),
            notes: Vec::new(),
            new_conflicts: 0,
            drained: 0,
        },
    };
    SyncDone {
        job_id,
        reason,
        result,
    }
}

/// `sync.run`: a manual sync as a job. It runs regardless of the automatic-sync switch and
/// waits behind a running sync (daemon spec §8). A resend answers with its job before the
/// configuration or the `force_delete` names are looked at.
pub fn sync_run(
    shared: &Arc<Shared>,
    conn: &ConnState,
    p: SyncRunParams,
) -> Result<Value, RpcError> {
    let session = conn.session.lock().unwrap().clone();
    let prepare = || -> Result<JobBody, RpcError> {
        let invalid = |m: String| RpcError::refused(ErrorKind::Invalid, m);
        if shared.cfg.sync.is_none() {
            return Err(invalid(NOT_CONFIGURED.to_string()));
        }
        let force_delete = p
            .force_delete
            .iter()
            .map(|name| collection_id(&shared.cfg, name).map_err(invalid))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Box::new(move |shared| {
            let job_id = shared.runner.current_job();
            let done = run_sync(
                shared,
                job_id.as_deref(),
                SyncReason::Manual,
                RunOptions {
                    background: false,
                    force_delete,
                },
            );
            let result = done.result.clone();
            shared.runner.defer_sync_end(done);
            Ok(JobResult::Sync(result))
        }))
    };
    shared.start_job_with(
        &p.request_id,
        session.as_deref(),
        JobKind::Sync,
        LongKind::Sync,
        prepare,
    )
}
