pub mod bridge;
pub mod http;
pub mod state;
pub mod ws;

use twydo_daemon::client::Client;
use twydo_daemon::protocol::methods::HelloResult;

pub use http::{bind, serve, serve_on};
/// The embedded web app (the `twydo-web` package).
pub use twydo_web as assets;

/// How the gateway reaches the daemon: one connection per device WebSocket.
pub trait Connector: Send + Sync + 'static {
    /// Connects and says hello as `tdo-gateway`, resuming `session` when given.
    fn connect(&self, session: Option<String>) -> Result<(Client, HelloResult), String>;
}
