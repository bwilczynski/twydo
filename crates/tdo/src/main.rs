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
    /// Report whether every active repeat rule class is verified
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
            let app = App::new(snapshot, Utc::now().with_timezone(&tz));
            if let Err(e) = term::run(app, root, tz) {
                eprintln!("tdo: {e}");
                std::process::exit(cli::EXIT_ERROR);
            }
        }),
        Command::Op { matrix, command } => {
            std::process::exit(cli::op::run(cli.vdir.as_deref(), matrix.as_ref(), command))
        }
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
