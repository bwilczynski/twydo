//! The sync runner as the tests call it: one run with its own writer on the embedded matrix.
#![allow(dead_code)]

use std::io;

use chrono::{DateTime, Utc};
use twydo_core::compat::Matrix;
use twydo_core::config::Config;
use twydo_core::store::{WriteError, Writer};
use twydo_core::sync::SyncError;
use twydo_core::sync::run::{RunOptions, RunReport, Skip, run_with};
use twydo_core::sync::switch;

/// [`run_with`] a writer opened here. The writer opens (and creates the state directory) only
/// after the not-configured and switched-off checks, so a skipped run creates no state.
pub fn run(cfg: &Config, opts: &RunOptions, now: DateTime<Utc>) -> Result<RunReport, SyncError> {
    let skipped = |s| RunReport {
        skipped: Some(s),
        ..RunReport::default()
    };
    if cfg.sync.is_none() {
        return Ok(skipped(Skip::NotConfigured));
    }
    if opts.background && switch::is_off(cfg)? {
        return Ok(skipped(Skip::Off));
    }
    let writer = Writer::open(cfg, Matrix::embedded().clone()).map_err(|e| match e {
        WriteError::Io(e) => e,
        e => io::Error::other(e.to_string()),
    })?;
    run_with(cfg, &writer, opts, now)
}
