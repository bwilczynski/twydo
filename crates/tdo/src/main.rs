use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tdo::{cli, term};

/// How long the TUI waits for the daemon's first snapshot before it opens empty.
const FIRST_SNAPSHOT: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Parser)]
#[command(name = "tdo", version, about = "Twydo: a terminal task app")]
struct Cli {
    /// Path to the vdir (collection directory root)
    #[arg(long, global = true, value_name = "PATH")]
    vdir: Option<PathBuf>,

    /// The daemon's socket (default: TWYDO_SOCKET, else the per-user path)
    #[arg(long, global = true, value_name = "PATH", hide = true)]
    socket: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Open the terminal UI (default)
    Tui,
    /// Export all tasks as JSON (schema v1)
    ExportJson,
    /// Write operations on tasks (scriptable)
    Op {
        #[command(subcommand)]
        command: cli::op::OpCommand,
    },
    /// Sync conflicts
    Conflicts {
        #[command(subcommand)]
        command: Option<cli::conflicts::ConflictsCommand>,
    },
    /// Sync the vdir with vdirsyncer (`[sync]` in the config); `on`/`off` switch automatic sync
    Sync {
        /// Switch automatic sync (timer, startup, after edits) on or off instead of syncing
        #[arg(value_enum, conflicts_with_all = ["background", "force_delete"])]
        switch: Option<Switch>,
        // Only keeps the daemon alive, silently (what the installed timer runs).
        #[arg(long, hide = true)]
        background: bool,
        // With --background: the same (kept for the installed timer).
        #[arg(long, hide = true, requires = "background")]
        when_due: bool,
        /// Apply an emptied list to both sides (collection id or display name)
        #[arg(long, value_name = "LIST")]
        force_delete: Vec<String>,
        /// Find lists created on another device (local folders only), then sync
        #[arg(long, conflicts_with_all = ["switch", "background"])]
        discover: bool,
    },
    /// Show sync status: last success, pending changes, conflicts, failures
    Status,
    /// Set up and check the configuration (never contacts the daemon)
    Config {
        #[command(subcommand)]
        command: cli::config::ConfigCommand,
    },
    /// Run the daemon (it owns the data; clients talk to it over a socket), or manage it
    Daemon {
        /// Compatibility matrix file (test hook)
        #[arg(long, value_name = "PATH", hide = true)]
        matrix: Option<PathBuf>,
        /// Abort after committing a write, before replying (crash tests)
        #[arg(long, hide = true)]
        test_crash_after_commit: bool,
        /// Run in a session of its own, logging to a file, and return at once
        #[arg(long)]
        detach: bool,
        #[command(subcommand)]
        command: Option<cli::daemon::DaemonCommand>,
    },
    /// Report data health, recurrence evidence and operation permissions
    Check {
        /// Compatibility matrix file (default: the matrix built into tdo)
        #[arg(long, value_name = "PATH")]
        matrix: Option<PathBuf>,
        /// Include task titles and parse-error paths
        #[arg(long)]
        verbose: bool,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum Switch {
    On,
    Off,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command.unwrap_or(Command::Tui) {
        Command::Tui => {
            let config = twydo_core::config::Config::path();
            if !config.exists() && cli.vdir.is_none() {
                cli::fail(&cli::CliError::NoConfig(config));
            }
            let target =
                tdo::client::Target::from_flags(cli.vdir.as_deref(), cli.socket.as_deref());
            let unit_installed = tdo::service::unit_for_current(&target.socket).file.exists();
            let link = term::link::Link::start(target);
            let mut app = term::first_app(&link, cli.vdir.as_deref(), FIRST_SNAPSHOT);
            term::unit_notice(&mut app, unit_installed);
            app.lookup = Box::new(tdo::app::DaemonLookup(link.caller_source()));
            let tz = app.snapshot.tz();
            if let Err(e) = term::run(app, link, tz) {
                eprintln!("tdo: {e}");
                std::process::exit(cli::EXIT_ERROR);
            }
            Ok(())
        }
        Command::Op { command } => {
            let target =
                tdo::client::Target::from_flags(cli.vdir.as_deref(), cli.socket.as_deref());
            std::process::exit(cli::op::run(&target, command))
        }
        Command::Daemon {
            matrix,
            test_crash_after_commit,
            detach,
            command,
        } => std::process::exit(match command {
            Some(c) => cli::daemon::client_command(cli.vdir.as_deref(), cli.socket.as_deref(), c),
            None => cli::daemon::run(
                cli.vdir.as_deref(),
                cli.socket.as_deref(),
                matrix.as_ref(),
                test_crash_after_commit,
                detach,
            ),
        }),
        Command::Config { command } => std::process::exit(cli::config::run(
            command,
            cli.vdir.as_deref(),
            cli.socket.as_deref(),
        )),
        Command::Conflicts { command } => {
            let target =
                tdo::client::Target::from_flags(cli.vdir.as_deref(), cli.socket.as_deref());
            std::process::exit(cli::conflicts::run(&target, command))
        }
        Command::Sync {
            switch: Some(switch),
            ..
        } => {
            let target =
                tdo::client::Target::from_flags(cli.vdir.as_deref(), cli.socket.as_deref());
            std::process::exit(cli::sync::switch_command(
                &target,
                matches!(switch, Switch::On),
            ))
        }
        Command::Sync {
            switch: None,
            background,
            when_due,
            force_delete,
            discover,
        } => {
            let target =
                tdo::client::Target::from_flags(cli.vdir.as_deref(), cli.socket.as_deref());
            std::process::exit(cli::sync::sync(
                &target,
                background,
                when_due,
                &force_delete,
                discover,
            ))
        }
        Command::Status => {
            let target =
                tdo::client::Target::from_flags(cli.vdir.as_deref(), cli.socket.as_deref());
            std::process::exit(cli::sync::status(&target))
        }
        Command::ExportJson => {
            let target =
                tdo::client::Target::from_flags(cli.vdir.as_deref(), cli.socket.as_deref());
            std::process::exit(cli::export::run(&target))
        }
        Command::Check { matrix, verbose } => {
            let target =
                tdo::client::Target::from_flags(cli.vdir.as_deref(), cli.socket.as_deref());
            std::process::exit(cli::check::run(&target, matrix.as_deref(), verbose))
        }
    };
    if let Err(e) = result {
        cli::fail(&e);
    }
}
