//! Paired devices and pending pairing codes, kept in one `devices.json`.
//!
//! Keys and codes are stored only as SHA-256 hashes. Every read-modify-write holds an exclusive
//! `flock` on `devices.lock` and replaces the file atomically (temp file, fsync, rename).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const CODE_TTL_MINUTES: i64 = 10;
const ATTEMPTS: u8 = 5;
const FILE: &str = "devices.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Phone,
    Display,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub role: Role,
    pub paired_at: DateTime<Utc>,
    pub last_seen: Option<DateTime<Utc>>,
    pub session: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("a device or pending code already has that name")]
    NameTaken,
    #[error("a name is 1-40 characters without control characters")]
    NameInvalid,
    #[error("no such device")]
    NoSuchDevice,
    #[error("invalid or expired pairing code")]
    CodeInvalid,
    #[error("state I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("corrupt device state: {0}")]
    Corrupt(#[from] serde_json::Error),
}

#[derive(Serialize, Deserialize)]
struct Record {
    #[serde(flatten)]
    device: Device,
    key_sha256: String,
}

#[derive(Serialize, Deserialize)]
struct Code {
    code_sha256: String,
    name: String,
    role: Role,
    expires_at: DateTime<Utc>,
    attempts_left: u8,
}

#[derive(Default, Serialize, Deserialize)]
struct Data {
    #[serde(default)]
    devices: Vec<Record>,
    #[serde(default)]
    codes: Vec<Code>,
}

pub struct Store {
    dir: PathBuf,
}

fn sha_hex(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn random<const N: usize>() -> Result<[u8; N], StateError> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(b)
}

/// Upper-case, drop dashes and spaces, and read the look-alikes I, L as 1 and O as 0.
fn normalize(code: &str) -> String {
    code.chars()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .map(|c| match c.to_ascii_uppercase() {
            'I' | 'L' => '1',
            'O' => '0',
            c => c,
        })
        .collect()
}

fn valid_name(name: &str) -> bool {
    let n = name.chars().count();
    (1..=40).contains(&n) && !name.chars().any(char::is_control)
}

impl Store {
    /// Open (creating, mode 0700) the state directory.
    pub fn open(dir: &Path) -> Result<Store, StateError> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        Ok(Store {
            dir: dir.to_owned(),
        })
    }

    /// `$TWYDO_GATEWAY_STATE`, else `$XDG_STATE_HOME/twydo/gateway`, else
    /// `~/.local/state/twydo/gateway`.
    pub fn default_dir() -> PathBuf {
        let non_empty = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
        if let Some(d) = non_empty("TWYDO_GATEWAY_STATE") {
            return PathBuf::from(d);
        }
        if let Some(d) = non_empty("XDG_STATE_HOME") {
            return PathBuf::from(d).join("twydo/gateway");
        }
        let home = non_empty("HOME").map(PathBuf::from).unwrap_or_default();
        home.join(".local/state/twydo/gateway")
    }

    fn path(&self) -> PathBuf {
        self.dir.join(FILE)
    }

    fn lock(&self) -> Result<File, StateError> {
        let f = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(self.dir.join("devices.lock"))?;
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(f)
    }

    fn read(&self) -> Result<Data, StateError> {
        match fs::read(self.path()) {
            Ok(b) => Ok(serde_json::from_slice(&b)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Data::default()),
            Err(e) => Err(e.into()),
        }
    }

    fn write(&self, data: &Data) -> Result<(), StateError> {
        let tmp = self.dir.join(format!("{FILE}.tmp"));
        // A leftover temp file would keep its old mode; start from nothing.
        match fs::remove_file(&tmp) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let result = (|| -> Result<(), StateError> {
            let mut f = OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&tmp)?;
            f.write_all(&serde_json::to_vec_pretty(data)?)?;
            f.sync_all()?;
            fs::rename(&tmp, self.path())?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
            return result;
        }
        File::open(&self.dir)?.sync_all()?;
        Ok(())
    }

    /// Run `f` on the data under the exclusive lock; write back when it returns `Ok` and
    /// `persist` says so (a failed redeem must still persist its attempt count).
    fn modify<T>(
        &self,
        f: impl FnOnce(&mut Data) -> (Result<T, StateError>, bool),
    ) -> Result<T, StateError> {
        let _lock = self.lock()?;
        let mut data = self.read()?;
        let (out, persist) = f(&mut data);
        if persist {
            self.write(&data)?;
        }
        out
    }

    /// Issue a pairing code (`XXXX-XXXX`) valid for 10 minutes.
    pub fn new_code(
        &self,
        name: &str,
        role: Role,
        now: DateTime<Utc>,
    ) -> Result<String, StateError> {
        if !valid_name(name) {
            return Err(StateError::NameInvalid);
        }
        let bytes = random::<8>()?;
        let raw: String = bytes
            .iter()
            .map(|b| ALPHABET[(b & 31) as usize] as char)
            .collect();
        self.modify(|d| {
            d.codes.retain(|c| c.expires_at >= now);
            let taken = d.devices.iter().any(|r| r.device.name == name)
                || d.codes.iter().any(|c| c.name == name);
            if taken {
                return (Err(StateError::NameTaken), false);
            }
            d.codes.push(Code {
                code_sha256: sha_hex(&raw),
                name: name.to_owned(),
                role,
                expires_at: now + Duration::minutes(CODE_TTL_MINUTES),
                attempts_left: ATTEMPTS,
            });
            (Ok(()), true)
        })?;
        Ok(format!("{}-{}", &raw[..4], &raw[4..]))
    }

    /// Exchange a pairing code for a device and its key (shown once, stored hashed).
    pub fn redeem(&self, code: &str, now: DateTime<Utc>) -> Result<(Device, String), StateError> {
        let hash = sha_hex(&normalize(code));
        let key = URL_SAFE_NO_PAD.encode(random::<32>()?);
        let id: String = random::<4>()?.iter().map(|b| format!("{b:02x}")).collect();
        self.modify(|d| {
            let before = d.codes.len();
            d.codes.retain(|c| c.expires_at >= now);
            let mut changed = d.codes.len() != before;
            if let Some(i) = d.codes.iter().position(|c| c.code_sha256 == hash) {
                let c = d.codes.remove(i);
                let device = Device {
                    id,
                    name: c.name,
                    role: c.role,
                    paired_at: now,
                    last_seen: None,
                    session: None,
                };
                d.devices.push(Record {
                    device: device.clone(),
                    key_sha256: sha_hex(&key),
                });
                (Ok((device, key)), true)
            } else {
                // Attempts cannot be attributed to a code: charge every pending one.
                for c in &mut d.codes {
                    c.attempts_left = c.attempts_left.saturating_sub(1);
                    changed = true;
                }
                d.codes.retain(|c| c.attempts_left > 0);
                (Err(StateError::CodeInvalid), changed)
            }
        })
    }

    /// The device owning `key`, if any. Read-only; I/O or parse failures count as no match.
    pub fn authenticate(&self, key: &str) -> Option<Device> {
        if key.is_empty() {
            return None;
        }
        let hash = sha_hex(key);
        let data = self.read().ok()?;
        data.devices
            .into_iter()
            .find(|r| r.key_sha256 == hash)
            .map(|r| r.device)
    }

    pub fn devices(&self) -> Result<Vec<Device>, StateError> {
        Ok(self.read()?.devices.into_iter().map(|r| r.device).collect())
    }

    pub fn revoke(&self, name: &str) -> Result<(), StateError> {
        self.modify(|d| {
            let before = d.devices.len();
            d.devices.retain(|r| r.device.name != name);
            if d.devices.len() == before {
                (Err(StateError::NoSuchDevice), false)
            } else {
                (Ok(()), true)
            }
        })
    }

    pub fn touch(
        &self,
        id: &str,
        now: DateTime<Utc>,
        session: Option<&str>,
    ) -> Result<(), StateError> {
        self.modify(|d| match d.devices.iter_mut().find(|r| r.device.id == id) {
            Some(r) => {
                r.device.last_seen = Some(now);
                if let Some(s) = session {
                    r.device.session = Some(s.to_owned());
                }
                (Ok(()), true)
            }
            None => (Err(StateError::NoSuchDevice), false),
        })
    }
}
