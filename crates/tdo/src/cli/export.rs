//! `tdo export-json`: schema v1 dump consumed by `tools/reconcile.py`. The daemon builds the
//! document (`export`); this prints it.

use std::time::Duration;

use twydo_daemon::protocol::reports::ExportResult;

use super::sync::{client_error, connect, warn_stuck_journal};
use crate::client::{self, CLI_TIMEOUT, Target};

/// How long the call waits for a daemon that is still starting.
const STARTUP_WAIT: Duration = Duration::from_secs(30);

/// `tdo export-json`. Exit 0, or 2 when the daemon refuses (an unreadable vdir) or is out of
/// reach.
pub fn run(target: &Target) -> i32 {
    let client = match connect(target) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let result: ExportResult = match client::call_ready(
        &client.caller(),
        "export",
        &serde_json::json!({}),
        CLI_TIMEOUT,
        STARTUP_WAIT,
    ) {
        Ok(r) => r,
        Err(e) => return client_error(&e),
    };
    warn_stuck_journal(&client);
    if result.skipped > 0 {
        eprintln!("skipped {} unparseable resources", result.skipped);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&result.document).expect("json")
    );
    0
}
