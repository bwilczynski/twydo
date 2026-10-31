//! Non-interactive subcommands and the helpers they share.

pub mod check;
pub mod export;

use std::path::Path;

use twydo_core::config::{Config, ConfigError};
use twydo_core::ical::value::IcalDateTime;
use twydo_core::model::Task;
use twydo_core::model::repeat::{parse_rrule, rule_class};
use twydo_core::views::Snapshot;

/// Exit code for usage, configuration and read errors.
pub const EXIT_ERROR: i32 = 2;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("no config at {}; pass --vdir", .0.display())]
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

/// Load the vdir named by the resolved configuration into a snapshot, with the load's
/// parse errors alongside.
pub fn load_snapshot(
    vdir: Option<&Path>,
) -> Result<(Snapshot, Vec<(std::path::PathBuf, String)>), CliError> {
    let config = resolve_config(vdir)?;
    let report = twydo_core::vdir::load(&config.vdir).map_err(|source| CliError::Vdir {
        path: config.vdir.clone(),
        source,
    })?;
    let errors = report.errors.clone();
    Ok((Snapshot::new(report, config), errors))
}

/// The matrix class of a task's repeat rule; `None` without an RRULE. A rule that does not
/// parse is `unrecognized`.
pub fn task_rule_class(t: &Task, snap: &Snapshot) -> Option<String> {
    let raw = t.rrule.as_deref()?;
    Some(match parse_rrule(raw) {
        Ok(r) => {
            let anchor = t
                .due
                .as_ref()
                .map(|d: &IcalDateTime| d.local_date(snap.tz()));
            rule_class(&r, anchor, t.from_completion)
        }
        Err(_) => "unrecognized".to_string(),
    })
}

/// Print an error and exit with [`EXIT_ERROR`].
pub fn fail(e: &CliError) -> ! {
    eprintln!("tdo: {e}");
    std::process::exit(EXIT_ERROR);
}
