//! What the daemon logs about syncs: only changes, so an unattended container log stays short.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use twydo_core::sync::run::RunReport;
use twydo_core::sync::status::SyncStatus;

/// Remembers what was logged, and turns each finished sync into the lines that are news.
/// Lines carry collection ids, never list names or task titles.
#[derive(Default)]
pub struct SyncLog {
    ok_logged: bool,
    logged_error: Option<String>,
    failures: u32,
    discovery_error: Option<String>,
    held: BTreeSet<String>,
}

impl SyncLog {
    /// The lines for a sync that ended. `report` is `None` when the run returned an error,
    /// whose text is `run_error`. A run that failed (`run_error`, or `report.error`) is logged
    /// as a failure whatever the saved status says: it may be stale, written before the failure.
    /// A skipped run logs nothing and changes nothing.
    pub fn lines(
        &mut self,
        report: Option<&RunReport>,
        run_error: Option<&str>,
        status: &SyncStatus,
        now: DateTime<Utc>,
    ) -> Vec<String> {
        if report.is_some_and(|r| r.skipped.is_some()) {
            return Vec::new();
        }
        let mut out = Vec::new();
        let run_failure = run_error.or(report.and_then(|r| r.error.as_deref()));
        let error = run_failure.or(status.last_error.as_deref());
        if let Some(e) = error {
            // The status describes this failure only when it carries the same text.
            let from_status = status.last_error.as_deref() == Some(e);
            let failures = if from_status {
                status.failures
            } else {
                self.failures + 1
            };
            if self.logged_error.as_deref() != Some(e) {
                let next = status
                    .next_attempt
                    .filter(|_| from_status)
                    .map(|t| format!(", next in {}s", (t - now).num_seconds().max(0)))
                    .unwrap_or_default();
                out.push(format!("sync failed (attempt {failures}{next}): {e}"));
                self.logged_error = Some(e.to_string());
            }
            self.failures = failures;
            self.ok_logged = false;
        } else if self.logged_error.take().is_some() {
            out.push(format!("sync ok again after {} failures", self.failures));
            self.ok_logged = true;
        } else if !self.ok_logged && status.last_success.is_some() {
            out.push(format!("sync ok ({} lists)", status.collections.len()));
            self.ok_logged = true;
        }
        if status.discovery_error != self.discovery_error {
            if let Some(e) = &status.discovery_error {
                out.push(format!("list discovery failed: {e}"));
            }
            self.discovery_error = status.discovery_error.clone();
        }
        if let Some(r) = report {
            for id in &r.auto_forced {
                if r.auto_forced_remote.contains(id) {
                    out.push(format!("applied list {id} emptied on the server"));
                } else {
                    out.push(format!("applied list {id} emptied here on the server"));
                }
            }
            for (id, h) in &r.held_emptied {
                if self.held.contains(id) {
                    continue;
                }
                let side = if h.remote { "on the server" } else { "here" };
                out.push(format!(
                    "list {id} emptied {side}; not applied ({})",
                    h.reason
                ));
            }
            self.held = r.held_emptied.keys().cloned().collect();
        }
        out
    }
}
