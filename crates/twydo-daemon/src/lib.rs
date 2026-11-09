//! The `tdo` daemon: the wire protocol, the server and the client.

pub mod protocol;
pub mod schema;

/// The wire protocol version this build speaks.
pub const PROTOCOL: u32 = 1;
pub mod client;
pub mod paths;
pub mod server;

/// Exit code of `tdo daemon` when another daemon already serves the vdir.
pub const EXIT_ALREADY_RUNNING: i32 = 10;
