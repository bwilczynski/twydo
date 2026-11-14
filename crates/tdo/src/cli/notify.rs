//! `tdo notify test`: asks the daemon to send a test notification.

use twydo_daemon::client::{Client, ClientError};
use twydo_daemon::protocol::rpc::RpcError;

use super::sync::{CLIENT, STARTUP_WAIT};
use crate::client::{self, CLI_TIMEOUT, Target};

/// Exit code when the notification could not be sent.
const EXIT_FAILED: i32 = 1;

/// Prints `notification sent` and returns 0, or prints `tdo: <error>` and returns 1.
pub fn test(target: &Target) -> i32 {
    let (client, _): (Client, _) = match client::connect(target, CLIENT, None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("tdo: {e}");
            return EXIT_FAILED;
        }
    };
    match client::call_ready::<_, serde_json::Value>(
        &client.caller(),
        "notify.test",
        &serde_json::json!({}),
        CLI_TIMEOUT,
        STARTUP_WAIT,
    ) {
        Ok(_) => {
            println!("notification sent");
            0
        }
        Err(ClientError::Rpc(RpcError { message, .. })) => {
            eprintln!("tdo: {message}");
            EXIT_FAILED
        }
        Err(e) => {
            eprintln!("tdo: {e}");
            EXIT_FAILED
        }
    }
}
