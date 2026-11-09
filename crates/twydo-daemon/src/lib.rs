//! The `tdo` daemon: the wire protocol, the server and the client.

pub mod protocol;
pub mod schema;

/// The wire protocol version this build speaks.
pub const PROTOCOL: u32 = 1;
pub mod client;
pub mod paths;
pub mod server;
