//! Non-interactive subcommands and the helpers they share.

pub mod check;
pub mod conflicts;
pub mod daemon;
pub mod export;
pub mod op;
pub mod service;
pub mod sync;

use std::path::Path;

use twydo_core::config::{Config, ConfigError};
pub use twydo_core::views::task_rule_class;

/// Exit code for usage, configuration and read errors.
pub const EXIT_ERROR: i32 = 2;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("no config at {}; run `tdo config init`, or pass --vdir", .0.display())]
    NoConfig(std::path::PathBuf),
    #[error(transparent)]
    Config(#[from] ConfigError),
}

/// Resolve the configuration: the config file when it exists (`--vdir` overrides its vdir),
/// else defaults with the `--vdir` given. An invalid config file is an error.
pub fn resolve_config(vdir: Option<&Path>) -> Result<Config, CliError> {
    let path = Config::path();
    if path.exists() {
        let mut cfg = Config::load(&path)?;
        if let Some(v) = vdir {
            cfg.vdir = v.to_path_buf();
        }
        Ok(cfg)
    } else if let Some(v) = vdir {
        Ok(Config::with_vdir(v.to_path_buf()))
    } else {
        Err(CliError::NoConfig(path))
    }
}

/// Print an error and exit with [`EXIT_ERROR`].
pub fn fail(e: &CliError) -> ! {
    eprintln!("tdo: {e}");
    std::process::exit(EXIT_ERROR);
}
