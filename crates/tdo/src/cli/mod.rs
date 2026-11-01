//! Non-interactive subcommands and the helpers they share.

pub mod check;
pub mod export;

use std::path::Path;

use twydo_core::config::{Config, ConfigError};
use twydo_core::views::Snapshot;
pub use twydo_core::views::task_rule_class;

/// Exit code for usage, configuration and read errors.
pub const EXIT_ERROR: i32 = 2;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("no config at {}; pass --vdir, or create it with: vdir = \"<path to your vdir>\" (see docs/config.md)", .0.display())]
    NoConfig(std::path::PathBuf),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("cannot read vdir {}: {source}", path.display())]
    Vdir {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
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

/// Load the vdir named by the resolved configuration into a snapshot (its `errors` hold the
/// files that failed to load).
pub fn load_snapshot(vdir: Option<&Path>) -> Result<Snapshot, CliError> {
    let config = resolve_config(vdir)?;
    let report = twydo_core::vdir::load(&config.vdir).map_err(|source| CliError::Vdir {
        path: config.vdir.clone(),
        source,
    })?;
    Ok(Snapshot::new(report, config))
}

/// Print an error and exit with [`EXIT_ERROR`].
pub fn fail(e: &CliError) -> ! {
    eprintln!("tdo: {e}");
    std::process::exit(EXIT_ERROR);
}
