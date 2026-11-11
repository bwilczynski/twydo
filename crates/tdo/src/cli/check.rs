//! `tdo check`: data health, recurrence evidence and concrete operation admission. The daemon
//! builds the report (`check.report`); this prints it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use twydo_daemon::protocol::reports::{CheckReport, CheckReportParams};

use super::sync::{client_error, connect, warn_stuck_journal};
use crate::client::{self, CLI_TIMEOUT, Target};

/// How long the call waits for a daemon that is still starting.
const STARTUP_WAIT: Duration = Duration::from_secs(30);

/// Asks the daemon for `check.report`; `matrix` is made absolute, as the daemon's working
/// directory is not the caller's.
pub fn report(
    target: &Target,
    matrix: Option<&Path>,
    verbose: bool,
    uid: Option<&str>,
) -> Result<CheckReport, i32> {
    let client = connect(target)?;
    let matrix: Option<PathBuf> =
        matrix.map(|m| std::path::absolute(m).unwrap_or_else(|_| m.to_path_buf()));
    client::call_ready(
        &client.caller(),
        "check.report",
        &CheckReportParams {
            verbose,
            matrix,
            uid: uid.map(String::from),
        },
        CLI_TIMEOUT,
        STARTUP_WAIT,
    )
    .map_err(|e| client_error(&e))
}

/// Prints a report: warnings on stderr as `tdo: <w>`, lines on stdout.
pub fn print(r: &CheckReport) {
    for w in &r.warnings {
        eprintln!("tdo: {w}");
    }
    for l in &r.lines {
        println!("{l}");
    }
}

/// `tdo check`. Exit 0, or 2 when the daemon refuses (a missing matrix, an unreadable
/// vdir) or is out of reach.
pub fn run(target: &Target, matrix: Option<&Path>, verbose: bool) -> i32 {
    match report(target, matrix, verbose, None) {
        Ok(r) => {
            if let Ok(client) = connect(target) {
                warn_stuck_journal(&client);
            }
            print(&r);
            0
        }
        Err(code) => code,
    }
}
