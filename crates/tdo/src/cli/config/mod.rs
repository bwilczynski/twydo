//! `tdo config`: guided setup and checks. Never contacts or starts the daemon.

pub mod check;
pub mod init;
pub mod prompt;
pub mod write;

use std::path::Path;

use clap::Subcommand;
use twydo_core::config::Config;

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Check, step by step, that this machine is ready to sync (no network)
    Check,
    /// Guided setup: write config.toml (vdir and, optionally, the CalDAV connection)
    Init(init::InitArgs),
}

/// Run a `tdo config` subcommand and return the process exit code. `vdir` and `socket` are the
/// global flags.
pub fn run(command: ConfigCommand, vdir: Option<&Path>, socket: Option<&Path>) -> i32 {
    let path = Config::path();
    match command {
        ConfigCommand::Check => report(&path),
        ConfigCommand::Init(args) => {
            let home = std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_default();
            let mut prompter = prompt::TerminalPrompter;
            match init::run(&path, &home, vdir, &args, &mut prompter) {
                Ok(written) => {
                    eprintln!("wrote {}", written.display());
                    let code = report(&path);
                    let target = crate::client::Target::from_flags(vdir, socket);
                    if matches!(
                        crate::client::probe(&target),
                        crate::client::Probe::Running(_)
                    ) {
                        eprintln!("restart the daemon to apply: tdo daemon stop");
                    }
                    code
                }
                Err(e) => {
                    eprintln!("tdo: {e}");
                    crate::cli::EXIT_ERROR
                }
            }
        }
    }
}

fn report(path: &Path) -> i32 {
    let report = check::run(path);
    print!("{}", report.render());
    i32::from(report.failed())
}
