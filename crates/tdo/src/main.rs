use std::path::PathBuf;

use chrono::Utc;
use clap::{Parser, Subcommand};
use tdo::app::App;
use tdo::{cli, term};

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
        /// Compatibility matrix file (test hook)
        #[arg(long, value_name = "PATH", hide = true)]
        matrix: Option<PathBuf>,
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
        /// For the timer: no waiting for a running sync, honour backoff, quiet on success
        #[arg(long)]
        background: bool,
        /// With --background: wait until unsynced edits are due, then sync (the batch waiter)
        #[arg(long, hide = true, requires = "background")]
        when_due: bool,
        /// Apply an emptied list to both sides (collection id or display name)
        #[arg(long, value_name = "LIST")]
        force_delete: Vec<String>,
    },
    /// Show sync status: last success, pending changes, conflicts, failures
    Status,
    /// Run the daemon (it owns the data; clients talk to it over a socket), or stop it
    #[command(hide = true)]
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
        Command::Tui => cli::load_snapshot(cli.vdir.as_deref()).map(|snapshot| {
            let tz = snapshot.tz();
            let root = snapshot.config.vdir.clone();
            let mut app = App::new(snapshot, Utc::now().with_timezone(&tz));
            let path = twydo_core::config::Config::path();
            if path.is_file() {
                app.config_path = Some(path);
            }
            if let Err(e) = term::run(app, root, tz) {
                eprintln!("tdo: {e}");
                std::process::exit(cli::EXIT_ERROR);
            }
        }),
        Command::Op { matrix, command } => {
            std::process::exit(cli::op::run(cli.vdir.as_deref(), matrix.as_ref(), command))
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
        Command::Conflicts { command } => {
            std::process::exit(cli::conflicts::run(cli.vdir.as_deref(), command))
        }
        Command::Sync {
            switch: Some(switch),
            ..
        } => std::process::exit(cli::sync::switch_command(
            cli.vdir.as_deref(),
            matches!(switch, Switch::On),
        )),
        Command::Sync {
            switch: None,
            background,
            when_due,
            force_delete,
        } => std::process::exit(cli::sync::sync(
            cli.vdir.as_deref(),
            background,
            when_due,
            &force_delete,
        )),
        Command::Status => std::process::exit(cli::sync::status(cli.vdir.as_deref())),
        Command::ExportJson => cli::export::run(cli.vdir.as_deref()),
        Command::Check { matrix, verbose } => {
            cli::check::run(cli.vdir.as_deref(), matrix.as_deref(), verbose)
        }
    };
    if let Err(e) = result {
        cli::fail(&e);
    }
}
