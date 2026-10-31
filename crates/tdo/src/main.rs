mod cli;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

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
        Command::Tui => {
            println!("not implemented");
            Ok(())
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
