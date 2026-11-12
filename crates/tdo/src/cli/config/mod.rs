//! `tdo config`: guided setup and checks. Never contacts or starts the daemon.

pub mod check;

use clap::Subcommand;
use twydo_core::config::Config;

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Check, step by step, that this machine is ready to sync (no network)
    Check,
}

/// Run a `tdo config` subcommand and return the process exit code.
pub fn run(command: ConfigCommand) -> i32 {
    match command {
        ConfigCommand::Check => {
            let report = check::run(&Config::path());
            print!("{}", report.render());
            i32::from(report.failed())
        }
    }
}
