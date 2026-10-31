//! TOML configuration (`~/.config/twydo/config.toml`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{NaiveTime, Weekday};
use chrono_tz::Tz;
use serde::{Deserialize, Deserializer};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{}: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortField {
    Due,
    Start,
    Creation,
    Title,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Sort {
    pub field: SortField,
    pub descending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Sort {
            field: SortField::Due,
            descending: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ListPrefs {
    pub archived: bool,
    pub hidden: bool,
    pub show_in_all: bool,
    pub show_in_today: bool,
    pub show_in_done: bool,
    pub sort: Sort,
}

impl Default for ListPrefs {
    fn default() -> Self {
        ListPrefs {
            archived: false,
            hidden: false,
            show_in_all: true,
            show_in_today: true,
            show_in_done: true,
            sort: Sort::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub name: String,
    #[serde(default)]
    pub lists: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub vdir: PathBuf,
    #[serde(default, deserialize_with = "de_timezone")]
    pub timezone: Option<String>,
    #[serde(default = "default_week_start", deserialize_with = "de_weekday")]
    pub week_start: Weekday,
    #[serde(default = "default_morning", deserialize_with = "de_time")]
    pub morning_time: NaiveTime,
    #[serde(default)]
    pub default_list: Option<String>,
    #[serde(default)]
    pub lists: BTreeMap<String, ListPrefs>,
    #[serde(default)]
    pub groups: Vec<Group>,
}

fn default_week_start() -> Weekday {
    Weekday::Mon
}

fn default_morning() -> NaiveTime {
    NaiveTime::from_hms_opt(9, 0, 0).expect("valid time")
}

fn de_weekday<'de, D: Deserializer<'de>>(d: D) -> Result<Weekday, D::Error> {
    let s = String::deserialize(d)?;
    Weekday::from_str(s.trim())
        .map_err(|_| serde::de::Error::custom(format!("invalid week_start {s:?} (use mon..sun)")))
}

fn de_time<'de, D: Deserializer<'de>>(d: D) -> Result<NaiveTime, D::Error> {
    let s = String::deserialize(d)?;
    NaiveTime::parse_from_str(s.trim(), "%H:%M")
        .map_err(|_| serde::de::Error::custom(format!("invalid time {s:?} (use HH:MM)")))
}

fn de_timezone<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let Some(s) = Option::<String>::deserialize(d)? else {
        return Ok(None);
    };
    Tz::from_str(&s).map_err(|_| serde::de::Error::custom(format!("unknown timezone {s:?}")))?;
    Ok(Some(s))
}

impl Config {
    /// `$XDG_CONFIG_HOME/twydo/config.toml`, else `$HOME/.config/twydo/config.toml`.
    pub fn path() -> PathBuf {
        let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        Self::path_from(xdg.as_deref(), &home)
    }

    /// Pure form of [`Config::path`]. An empty `xdg` counts as unset.
    pub fn path_from(xdg: Option<&Path>, home: &Path) -> PathBuf {
        let base = match xdg {
            Some(x) if !x.as_os_str().is_empty() => x.to_path_buf(),
            _ => home.join(".config"),
        };
        base.join("twydo").join("config.toml")
    }

    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let mut cfg: Config = toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
            cfg.vdir = expand_tilde(&cfg.vdir, Path::new(&home));
        }
        Ok(cfg)
    }

    /// Configured zone, else the system zone, else UTC.
    pub fn tz(&self) -> Tz {
        self.timezone
            .as_deref()
            .and_then(|s| Tz::from_str(s).ok())
            .or_else(|| {
                iana_time_zone::get_timezone()
                    .ok()
                    .and_then(|s| Tz::from_str(&s).ok())
            })
            .unwrap_or(Tz::UTC)
    }

    /// Prefs for a list; lists without an entry get the defaults.
    pub fn list_prefs(&self, id: &str) -> ListPrefs {
        self.lists.get(id).cloned().unwrap_or_default()
    }
}

/// Expand a leading `~` or `~/` against `home`. Other paths (including `~user`) are unchanged.
pub fn expand_tilde(path: &Path, home: &Path) -> PathBuf {
    if path == Path::new("~") {
        home.to_path_buf()
    } else if let Ok(rest) = path.strip_prefix("~") {
        home.join(rest)
    } else {
        path.to_path_buf()
    }
}
