//! Non-interactive subcommands and the helpers they share.

pub mod check;
pub mod conflicts;
pub mod export;
pub mod op;
pub mod sync;

use std::path::Path;

use twydo_core::config::{Config, ConfigError};
use twydo_core::store::existing_vdir_state;
use twydo_core::store::fsio::VdirLock;
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
    #[error("cannot lock state dir {}: {source}", path.display())]
    Lock {
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

/// Roll forward unfinished journal entries before reading, unless a sync holds the vdir.
/// Problems are reported on stderr and reading continues, read-only as far as the journal is
/// concerned.
pub fn recover_at_startup(config: &Config) {
    let state = match existing_vdir_state(&config.state_dir(), &config.vdir) {
        Ok(Some(state)) => state,
        // No state for this vdir: nothing has been written through Twydo here.
        Ok(None) => return,
        Err(e) => return eprintln!("tdo: journal recovery failed: {e}"),
    };
    // A running sync holds the vdir: it recovered before it started, so do not wait for it.
    match twydo_core::store::journal::try_recover(&config.vdir, &state) {
        Ok(None) => {}
        Ok(Some(report)) if report.stuck.is_empty() => {}
        Ok(Some(_)) => eprintln!("tdo: journal has unfinished operations; run 'tdo op journal'"),
        Err(e) => eprintln!("tdo: journal recovery failed: {e}"),
    }
}

/// Load the vdir named by the resolved configuration into a snapshot (its `errors` hold the
/// files that failed to load).
pub fn load_snapshot(vdir: Option<&Path>) -> Result<Snapshot, CliError> {
    let config = resolve_config(vdir)?;
    recover_at_startup(&config);
    let report = load_vdir(&config)?;
    Ok(Snapshot::new(report, config))
}

/// Read the vdir under the shared vdir lock (spec §6: readers never see a partly applied
/// operation). The lock is taken only when the vdir's state dir exists; it is never created
/// here.
pub fn load_vdir(config: &Config) -> Result<twydo_core::vdir::LoadReport, CliError> {
    let _lock = reader_lock(config)?;
    twydo_core::vdir::load(&config.vdir).map_err(|source| CliError::Vdir {
        path: config.vdir.clone(),
        source,
    })
}

/// The shared lock of `config`'s vdir (see [`VdirLock::reader`]), from its per-vdir state
/// dir; `None` when that dir does not exist.
pub fn reader_lock(config: &Config) -> Result<Option<VdirLock>, CliError> {
    let lock_err = |source| CliError::Lock {
        path: config.state_dir(),
        source,
    };
    match existing_vdir_state(&config.state_dir(), &config.vdir).map_err(lock_err)? {
        Some(state) => VdirLock::reader(&state).map_err(|source| CliError::Lock {
            path: state.clone(),
            source,
        }),
        None => Ok(None),
    }
}

/// Print an error and exit with [`EXIT_ERROR`].
pub fn fail(e: &CliError) -> ! {
    eprintln!("tdo: {e}");
    std::process::exit(EXIT_ERROR);
}
