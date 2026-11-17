//! `tdo gateway`: serve the phone app over HTTP and WebSocket, and manage paired devices.
//! The gateway is a daemon client like any other; it never reads the configuration or data.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use clap::{Subcommand, ValueEnum};
use twydo_daemon::client::Client;
use twydo_daemon::protocol::methods::HelloResult;
use twydo_gateway::http::{ServeOptions, normalize_origin};
use twydo_gateway::state::{Role, StateError, Store};
use twydo_gateway::{Connector, serve};

use super::EXIT_ERROR;
use crate::client::{Target, connect};

#[derive(Clone, Copy, ValueEnum)]
pub enum RoleArg {
    Phone,
    Display,
}

#[derive(Subcommand)]
pub enum GatewayCommand {
    /// Serve the phone app and the device WebSocket
    Serve {
        /// Address to listen on
        #[arg(long, env = "TWYDO_GATEWAY_LISTEN", default_value = "127.0.0.1:8787")]
        listen: SocketAddr,
        /// The public origin phones use (https://...); required unless --insecure
        #[arg(long, env = "TWYDO_GATEWAY_ORIGIN", value_name = "URL")]
        origin: Option<String>,
        /// Plain HTTP on a trusted network: no Secure cookie, no origin required
        #[arg(long)]
        insecure: bool,
        /// Directory of the built web app (default: the app built into tdo)
        #[arg(long, value_name = "DIR")]
        assets: Option<PathBuf>,
    },
    /// Issue a pairing code for a new device
    Pair {
        name: String,
        #[arg(long, value_enum, default_value = "phone")]
        role: RoleArg,
    },
    /// List paired devices
    Devices,
    /// Revoke a paired device
    Revoke { name: String },
}

struct DaemonConnector(Target);

impl Connector for DaemonConnector {
    fn connect(&self, session: Option<String>) -> Result<(Client, HelloResult), String> {
        connect(&self.0, "tdo-gateway", session).map_err(|e| e.to_string())
    }
}

fn error(msg: impl std::fmt::Display, code: i32) -> i32 {
    eprintln!("tdo gateway: {msg}");
    code
}

pub fn run(state_dir: Option<&Path>, target: &Target, command: GatewayCommand) -> i32 {
    let dir = state_dir.map_or_else(Store::default_dir, Path::to_path_buf);
    match command {
        GatewayCommand::Serve {
            listen,
            origin,
            insecure,
            assets,
        } => {
            if origin.is_none() && !insecure {
                return error(
                    "--origin is required (or --insecure for plain HTTP on a trusted network)",
                    EXIT_ERROR,
                );
            }
            let origin = match origin {
                None => None,
                Some(raw) => match normalize_origin(&raw) {
                    Ok(o) if o.starts_with("http://") && !insecure => {
                        return error(
                            format!("--origin {raw:?} is plain http; use https or pass --insecure"),
                            EXIT_ERROR,
                        );
                    }
                    Ok(o) => Some(o),
                    Err(e) => return error(format!("invalid --origin: {e}"), EXIT_ERROR),
                },
            };
            let opts = ServeOptions {
                listen,
                origin,
                insecure,
                assets_dir: assets,
                state_dir: dir,
            };
            match serve(opts, Arc::new(DaemonConnector(target.clone()))) {
                Ok(()) => 0,
                Err(e) => error(e, EXIT_ERROR),
            }
        }
        GatewayCommand::Pair { name, role } => {
            let store = match Store::open(&dir) {
                Ok(s) => s,
                Err(e) => return error(e, EXIT_ERROR),
            };
            let role = match role {
                RoleArg::Phone => Role::Phone,
                RoleArg::Display => Role::Display,
            };
            match store.new_code(&name, role, Utc::now()) {
                Ok(code) => {
                    println!("Pairing code for {name}: {code} (valid 10 minutes)");
                    0
                }
                Err(e) => error(e, 1),
            }
        }
        GatewayCommand::Devices => match Store::open(&dir).and_then(|s| s.devices()) {
            Ok(devices) => {
                for d in devices {
                    let role = match d.role {
                        Role::Phone => "phone",
                        Role::Display => "display",
                    };
                    let seen = d
                        .last_seen
                        .map_or_else(|| "never".to_string(), |t| t.to_rfc3339());
                    println!(
                        "{}\t{}\t{}\t{}",
                        d.name,
                        role,
                        d.paired_at.to_rfc3339(),
                        seen
                    );
                }
                0
            }
            Err(e) => error(e, EXIT_ERROR),
        },
        GatewayCommand::Revoke { name } => {
            let store = match Store::open(&dir) {
                Ok(s) => s,
                Err(e) => return error(e, EXIT_ERROR),
            };
            match store.revoke(&name) {
                Ok(()) => {
                    println!("Revoked {name}");
                    0
                }
                Err(StateError::NoSuchDevice) => error(format!("no device named {name}"), 1),
                Err(e) => error(e, EXIT_ERROR),
            }
        }
    }
}
