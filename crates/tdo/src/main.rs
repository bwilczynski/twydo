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
        command: cli::conflicts::ConflictsCommand,
    },
    /// Report whether every active repeat rule class is verified
    Check {
        /// Compatibility matrix file
        #[arg(long, value_name = "PATH", default_value = "compat/matrix.toml")]
        matrix: PathBuf,
        /// Include task titles and parse-error paths
        #[arg(long)]
        verbose: bool,
    },
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
        Command::ExportJson => cli::export::run(cli.vdir.as_deref()),
        Command::Check { matrix, verbose } => {
            cli::check::run(cli.vdir.as_deref(), &matrix, verbose)
        }
    };
    if let Err(e) = result {
        cli::fail(&e);
    }
}
