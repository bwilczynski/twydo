//! TOML configuration (`~/.config/twydo/config.toml`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{NaiveTime, Weekday};
use chrono_tz::Tz;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

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
    #[error("{}: {message}", path.display())]
    Invalid { path: PathBuf, message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SortField {
    Due,
    Start,
    Creation,
    Title,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub name: String,
    #[serde(default)]
    pub lists: Vec<String>,
}

/// `verified` runs only operations the compatibility matrix passes; `test` also runs
/// unverified ones and must only be used against the isolated test account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum WriteMode {
    #[default]
    Verified,
    Test,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Writes {
    pub mode: WriteMode,
}

/// `[sync]` table. Absent means sync is off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct SyncConfig {
    /// CalDAV server URL.
    pub url: String,
    /// CalDAV username.
    pub username: String,
    /// Command (argv) whose output (trailing newlines removed) is the CalDAV password; run by vdirsyncer.
    pub password_command: Vec<String>,
    /// PEM file with the CA that signed the server certificate.
    pub ca_file: Option<PathBuf>,
    /// vdirsyncer executable.
    pub command: String,
    /// Python interpreter containing the supported vdirsyncer collection adapter. Unset means
    /// vdirsyncer's own interpreter (from the shebang of `command`), else `python3`.
    // Absent rather than null on the wire: released clients read it as a string and fill in
    // their old default, `python3`, which may not be what the daemon runs. Clients never use it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub list_python: Option<String>,
    pub metasync: bool,
    /// Unsynced edits that make a sync due.
    pub batch_size: u32,
    /// Seconds after the oldest unsynced edit at which a sync is due.
    pub max_wait_secs: u64,
    pub timeout_secs: u64,
    pub max_backoff_secs: u64,
    pub retention_days: u32,
}

impl Default for SyncConfig {
    fn default() -> Self {
        SyncConfig {
            url: String::new(),
            username: String::new(),
            password_command: Vec::new(),
            ca_file: None,
            command: "vdirsyncer".into(),
            list_python: None,
            metasync: true,
            batch_size: 5,
            max_wait_secs: 60,
            timeout_secs: 300,
            max_backoff_secs: 1800,
            retention_days: 30,
        }
    }
}

/// `[daemon]` table: preferences about the daemon that clients act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct DaemonPrefs {
    /// Warn in the TUI while no service unit is installed.
    pub unit_warning: bool,
}

impl Default for DaemonPrefs {
    fn default() -> Self {
        DaemonPrefs { unit_warning: true }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub vdir: PathBuf,
    #[serde(default, deserialize_with = "de_timezone")]
    pub timezone: Option<String>,
    #[serde(default = "default_week_start", deserialize_with = "de_weekday")]
    #[schemars(with = "String")]
    pub week_start: Weekday,
    #[serde(default = "default_work_days", deserialize_with = "de_weekdays")]
    #[schemars(with = "Vec<String>")]
    pub work_days: Vec<Weekday>,
    #[serde(
        default = "default_morning",
        deserialize_with = "de_time",
        serialize_with = "ser_time"
    )]
    #[schemars(with = "String")]
    pub morning_time: NaiveTime,
    #[serde(default)]
    pub default_list: Option<String>,
    #[serde(default)]
    pub lists: BTreeMap<String, ListPrefs>,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub writes: Writes,
    #[serde(default)]
    pub state_dir: Option<PathBuf>,
    #[serde(default)]
    pub sync: Option<SyncConfig>,
    /// Never on the wire: a client one release older rejects keys it does not know. It
    /// travels as `WireSnapshot.unit_warning`.
    #[serde(default, skip_serializing)]
    pub daemon: DaemonPrefs,
}

fn default_week_start() -> Weekday {
    Weekday::Mon
}

fn default_work_days() -> Vec<Weekday> {
    vec![
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
    ]
}

fn default_morning() -> NaiveTime {
    NaiveTime::from_hms_opt(9, 0, 0).expect("valid time")
}

fn de_weekday<'de, D: Deserializer<'de>>(d: D) -> Result<Weekday, D::Error> {
    let s = String::deserialize(d)?;
    Weekday::from_str(s.trim())
        .map_err(|_| serde::de::Error::custom(format!("invalid week_start {s:?} (use mon..sun)")))
}

fn de_weekdays<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Weekday>, D::Error> {
    let names = Vec::<String>::deserialize(d)?;
    let days = names
        .iter()
        .map(|s| {
            Weekday::from_str(s.trim()).map_err(|_| {
                serde::de::Error::custom(format!("invalid work_days entry {s:?} (use mon..sun)"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if days.is_empty() {
        return Err(serde::de::Error::custom("work_days must not be empty"));
    }
    Ok(days)
}

fn de_time<'de, D: Deserializer<'de>>(d: D) -> Result<NaiveTime, D::Error> {
    let s = String::deserialize(d)?;
    NaiveTime::parse_from_str(s.trim(), "%H:%M")
        .map_err(|_| serde::de::Error::custom(format!("invalid time {s:?} (use HH:MM)")))
}

/// `HH:MM`, the form [`de_time`] reads.
fn ser_time<S: Serializer>(t: &NaiveTime, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&t.format("%H:%M").to_string())
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

    /// Configured `state_dir`, else `$XDG_STATE_HOME/twydo`, else `~/.local/state/twydo`.
    pub fn state_dir(&self) -> PathBuf {
        if let Some(d) = &self.state_dir {
            return d.clone();
        }
        let xdg = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from);
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        state_dir_from(xdg.as_deref(), &home)
    }

    /// Defaults with the given vdir (system timezone, no list prefs).
    pub fn with_vdir(vdir: PathBuf) -> Config {
        Config {
            vdir,
            timezone: None,
            week_start: default_week_start(),
            work_days: default_work_days(),
            morning_time: default_morning(),
            default_list: None,
            lists: BTreeMap::new(),
            groups: Vec::new(),
            writes: Writes::default(),
            state_dir: None,
            sync: None,
            daemon: DaemonPrefs::default(),
        }
    }

    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        check_removed_sync_keys(path, &text)?;
        let mut cfg: Config = toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
            cfg.vdir = expand_tilde(&cfg.vdir, Path::new(&home));
            cfg.state_dir = cfg.state_dir.map(|d| expand_tilde(&d, Path::new(&home)));
            if let Some(sync) = &mut cfg.sync {
                sync.ca_file = sync
                    .ca_file
                    .take()
                    .map(|p| expand_tilde(&p, Path::new(&home)));
            }
        }
        if let Some(sync) = &cfg.sync {
            let missing = |key: &str| ConfigError::Invalid {
                path: path.to_path_buf(),
                message: format!("[sync] {key} is required; run `tdo config init`"),
            };
            if sync.url.trim().is_empty() {
                return Err(missing("url"));
            }
            if sync.username.trim().is_empty() {
                return Err(missing("username"));
            }
            if sync
                .password_command
                .first()
                .is_none_or(|w| w.trim().is_empty())
            {
                return Err(missing("password_command"));
            }
            for (key, zero) in [
                ("batch_size", sync.batch_size == 0),
                ("max_wait_secs", sync.max_wait_secs == 0),
            ] {
                if zero {
                    return Err(ConfigError::Invalid {
                        path: path.to_path_buf(),
                        message: format!("[sync] {key} must be at least 1"),
                    });
                }
            }
        }
        Ok(cfg)
    }

    /// Non-fatal notes about the loaded configuration, for the user to see. None exist at
    /// present; the method and the wire field stay.
    pub fn warnings(&self) -> Vec<String> {
        Vec::new()
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

/// Keys Twydo no longer reads: it generates the vdirsyncer config itself.
const REMOVED_SYNC_KEYS: [&str; 4] = ["pair", "local_storage", "vdirsyncer_config", "after_edit"];

/// Checked on the raw table so this message wins over serde's "unknown field".
fn check_removed_sync_keys(path: &Path, text: &str) -> Result<(), ConfigError> {
    let Ok(table) = text.parse::<toml::Table>() else {
        return Ok(());
    };
    let Some(sync) = table.get("sync").and_then(|v| v.as_table()) else {
        return Ok(());
    };
    for key in REMOVED_SYNC_KEYS {
        if sync.contains_key(key) {
            return Err(ConfigError::Invalid {
                path: path.to_path_buf(),
                message: format!(
                    "[sync] {key} is no longer supported: Twydo generates the vdirsyncer config; run `tdo config init`"
                ),
            });
        }
    }
    Ok(())
}

/// Pure form of the default state dir. An empty `xdg_state` counts as unset.
pub fn state_dir_from(xdg_state: Option<&Path>, home: &Path) -> PathBuf {
    let base = match xdg_state {
        Some(x) if !x.as_os_str().is_empty() => x.to_path_buf(),
        _ => home.join(".local").join("state"),
    };
    base.join("twydo")
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
