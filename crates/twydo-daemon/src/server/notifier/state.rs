//! The sent-state file: which reminder deliveries have already gone out.

use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use twydo_core::remind::Sent;
use twydo_core::store::fsio::write_atomic;

const FILE: &str = "notifier.json";

#[derive(Serialize, Deserialize)]
struct Disk {
    version: u32,
    sent: Sent,
}

pub struct SentState {
    path: PathBuf,
    sent: Sent,
}

impl SentState {
    /// Missing or unreadable file: empty state; the second value says why (the caller logs
    /// it once). `None` for a missing file or a good one.
    pub fn load(dir: &Path) -> (SentState, Option<String>) {
        let path = dir.join(FILE);
        let (sent, reason) = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Disk>(&bytes) {
                Ok(d) if d.version == 1 => (d.sent, None),
                Ok(d) => (
                    Sent::new(),
                    Some(format!("unsupported version {} in {}", d.version, FILE)),
                ),
                Err(e) => (Sent::new(), Some(format!("cannot parse {FILE}: {e}"))),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => (Sent::new(), None),
            Err(e) => (Sent::new(), Some(format!("cannot read {FILE}: {e}"))),
        };
        (SentState { path, sent }, reason)
    }

    pub fn sent(&self) -> &Sent {
        &self.sent
    }

    /// Records in memory first, drops `t:` entries sent before `now - 2 * catch_up` and `d:`
    /// entries dated more than 2 days before `now`'s UTC date, then saves
    /// atomically. A save error leaves the in-memory record in place.
    pub fn record(
        &mut self,
        key: &str,
        now: DateTime<Utc>,
        catch_up: chrono::Duration,
    ) -> io::Result<()> {
        self.sent.insert(key.to_string(), now);
        // An overflow (an absurd `catch_up_hours`) means nothing is old enough to drop.
        let cutoff = catch_up
            .checked_mul(2)
            .and_then(|d| now.checked_sub_signed(d));
        // A digest key (`d:<date>`) lives by its date, not its send time: the digest is decided
        // once per day, and a short window must not let it be decided again that afternoon.
        // Two days of margin keep it timezone-safe.
        let oldest_digest = now.date_naive().checked_sub_days(chrono::Days::new(2));
        self.sent.retain(|key, at| {
            let digest_date = key
                .strip_prefix("d:")
                .and_then(|d| d.parse::<chrono::NaiveDate>().ok());
            match (digest_date, oldest_digest) {
                (Some(date), Some(oldest)) => date >= oldest,
                _ => cutoff.is_none_or(|c| *at >= c),
            }
        });
        let bytes = serde_json::to_vec_pretty(&Disk {
            version: 1,
            sent: self.sent.clone(),
        })
        .map_err(io::Error::other)?;
        write_atomic(&self.path, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2030, 1, 10, 12, 0, 0).unwrap()
    }

    #[test]
    fn record_then_reload_keeps_keys() {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, why) = SentState::load(dir.path());
        assert!(why.is_none());
        s.record("k1", now(), Duration::hours(24)).unwrap();
        let (again, why) = SentState::load(dir.path());
        assert!(why.is_none());
        assert_eq!(again.sent().get("k1"), Some(&now()));
    }

    #[test]
    fn missing_file_is_empty_without_error() {
        let dir = tempfile::tempdir().unwrap();
        let (s, why) = SentState::load(dir.path());
        assert!(s.sent().is_empty());
        assert!(why.is_none());
    }

    #[test]
    fn corrupt_file_is_empty_with_reason() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), b"{not json").unwrap();
        let (s, why) = SentState::load(dir.path());
        assert!(s.sent().is_empty());
        assert!(why.unwrap().contains(FILE));
    }

    #[test]
    fn record_prunes_entries_older_than_twice_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, _) = SentState::load(dir.path());
        let c = Duration::hours(24);
        s.record("old", now() - Duration::hours(49), c).unwrap();
        s.record("recent", now() - Duration::hours(47), c).unwrap();
        // Recording "old" at its own time kept it; pruning happens relative to the new now.
        s.record("new", now(), c).unwrap();
        assert!(!s.sent().contains_key("old"));
        assert!(s.sent().contains_key("recent"));
        assert!(s.sent().contains_key("new"));
    }

    #[test]
    fn digest_keys_are_kept_through_their_day() {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, _) = SentState::load(dir.path());
        let c = Duration::hours(1);
        let morning = Utc.with_ymd_and_hms(2030, 1, 10, 8, 0, 0).unwrap();
        s.record("d:2030-01-10", morning, c).unwrap();
        s.record("d:2030-01-07", morning, c).unwrap();
        s.record("t:u1@2030-01-10T08:00:00Z", morning, c).unwrap();
        let evening = Utc.with_ymd_and_hms(2030, 1, 10, 20, 0, 0).unwrap();
        s.record("t:u2@2030-01-10T20:00:00Z", evening, c).unwrap();
        assert!(s.sent().contains_key("d:2030-01-10"), "today's digest key");
        assert!(!s.sent().contains_key("d:2030-01-07"), "3 days old");
        assert!(!s.sent().contains_key("t:u1@2030-01-10T08:00:00Z"));
        assert!(s.sent().contains_key("t:u2@2030-01-10T20:00:00Z"));
    }

    #[test]
    fn huge_window_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, _) = SentState::load(dir.path());
        s.record("k", now(), Duration::hours(i64::from(u32::MAX)))
            .unwrap();
        assert!(s.sent().contains_key("k"));
    }

    #[test]
    fn failed_save_keeps_the_key_in_memory() {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, _) = SentState::load(&dir.path().join("missing"));
        let r = s.record("k", now(), Duration::hours(24));
        assert!(r.is_err());
        assert!(s.sent().contains_key("k"));
    }
}
