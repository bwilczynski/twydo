//! The sync summary `views.get` carries for thin clients. Synthetic names only.

use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use twydo_daemon::protocol::methods::{SyncState, SyncView, WireCollState};
use twydo_daemon::server::syncsummary::summary;

fn now() -> DateTime<FixedOffset> {
    FixedOffset::east_opt(3600)
        .unwrap()
        .with_ymd_and_hms(2026, 11, 18, 14, 0, 0)
        .unwrap()
}

fn well() -> SyncView {
    SyncView {
        state: SyncState::Idle,
        auto: true,
        last_success: Some(Utc.with_ymd_and_hms(2026, 11, 18, 12, 55, 0).unwrap()),
        last_attempt: None,
        next_attempt: None,
        failures: 0,
        last_error: None,
        job_id: None,
        collections: BTreeMap::new(),
        metasync_error: None,
        discovery_error: None,
    }
}

fn name(id: &str) -> String {
    match id {
        "c1" => "Inbox".into(),
        "c2" => "Work".into(),
        other => other.into(),
    }
}

fn run(s: &SyncView, conflicts: usize) -> twydo_daemon::protocol::views::SyncSummary {
    summary(s, conflicts, now(), &name)
}

#[test]
fn failing_sync() {
    let mut s = well();
    s.last_error = Some("Cannot reach the server".into());
    s.next_attempt = Some(Utc.with_ymd_and_hms(2026, 11, 18, 13, 10, 0).unwrap());
    let r = run(&s, 0);
    assert!(r.attention);
    assert_eq!(r.headline, "Sync failing");
    assert_eq!(
        r.detail.as_deref(),
        Some("Cannot reach the server. Next try 14:10.")
    );
}

#[test]
fn discovery_failure_in_last_error() {
    let mut s = well();
    s.last_error = Some("list discovery failed; boom".into());
    let r = run(&s, 0);
    assert!(r.attention);
    assert_eq!(r.headline, "List discovery failed");
    assert_eq!(r.detail.as_deref(), Some("list discovery failed; boom"));
}

#[test]
fn discovery_error_only() {
    let mut s = well();
    s.discovery_error = Some("x".into());
    let r = run(&s, 0);
    assert!(r.attention);
    assert_eq!(r.headline, "List discovery failed");
    assert_eq!(r.detail.as_deref(), Some("x"));
}

#[test]
fn emptied_not_applied() {
    let mut s = well();
    s.collections
        .insert("c1".into(), WireCollState::EmptiedRemote);
    let r = run(&s, 0);
    assert!(r.attention);
    assert_eq!(r.headline, "\"Inbox\" was emptied on the server");
    assert_eq!(
        r.detail.as_deref(),
        Some("Not applied automatically. Run: tdo sync --force-delete \"Inbox\"")
    );
    s.collections
        .insert("c1".into(), WireCollState::EmptiedLocal);
    assert_eq!(run(&s, 0).headline, "\"Inbox\" was emptied here");
    s.collections
        .insert("c2".into(), WireCollState::EmptiedRemote);
    let r = run(&s, 0);
    assert_eq!(r.headline, "2 lists were emptied");
    assert_eq!(
        r.detail.as_deref(),
        Some("Run: tdo sync --force-delete \"Inbox\"; Run: tdo sync --force-delete \"Work\"")
    );
}

#[test]
fn conflicts() {
    let r = run(&well(), 2);
    assert!(r.attention);
    assert_eq!(r.headline, "2 conflicts");
    assert_eq!(r.detail.as_deref(), Some("Resolve them in tdo."));
    assert_eq!(run(&well(), 1).headline, "1 conflict");
}

#[test]
fn sync_off() {
    let mut s = well();
    s.auto = false;
    let r = run(&s, 0);
    assert!(r.attention);
    assert_eq!(r.headline, "Sync is off");
    assert_eq!(r.detail.as_deref(), Some("Turn it on in tdo."));
}

#[test]
fn priority_order() {
    let mut s = well();
    s.auto = false;
    s.collections
        .insert("c1".into(), WireCollState::EmptiedRemote);
    assert_eq!(run(&s, 3).headline, "\"Inbox\" was emptied on the server");
    s.collections.clear();
    assert_eq!(run(&s, 3).headline, "3 conflicts");
    assert_eq!(run(&s, 0).headline, "Sync is off");
    s.discovery_error = Some("d".into());
    s.collections
        .insert("c1".into(), WireCollState::EmptiedRemote);
    assert_eq!(run(&s, 3).headline, "List discovery failed");
    s.last_error = Some("Cannot reach the server".into());
    assert_eq!(run(&s, 3).headline, "Sync failing");
}

#[test]
fn all_well() {
    let r = run(&well(), 0);
    assert!(!r.attention);
    assert_eq!(r.headline, "Synced 5m ago");
    assert_eq!(r.detail, None);
    assert_eq!(r.last_success, well().last_success);
    let mut s = well();
    s.last_success = None;
    assert_eq!(run(&s, 0).headline, "Not synced yet");
    s.last_success = Some(Utc.with_ymd_and_hms(2026, 11, 18, 13, 0, 0).unwrap());
    assert_eq!(run(&s, 0).headline, "Synced just now");
    s.last_success = Some(Utc.with_ymd_and_hms(2026, 11, 18, 10, 59, 0).unwrap());
    assert_eq!(run(&s, 0).headline, "Synced 2h ago");
    s.last_success = Some(Utc.with_ymd_and_hms(2026, 11, 15, 10, 0, 0).unwrap());
    assert_eq!(run(&s, 0).headline, "Synced 3d ago");
}

#[test]
fn detail_is_capped() {
    let mut s = well();
    s.last_error = Some("e".repeat(1000));
    let d = run(&s, 0).detail.unwrap();
    assert_eq!(d.chars().count(), 300);
    assert!(d.ends_with('…'));
}

#[test]
fn failure_detail_never_carries_credentials_or_paths() {
    let mut s = well();
    s.last_error = Some(
        "error: Unexpected status 401 for https://alice:s3cret@dav.example.net/private/dav/ (see \
         http://dav.example.net:8080/cal/alice/)."
            .into(),
    );
    let detail = run(&s, 0).detail.unwrap();
    for secret in ["alice", "s3cret", "/private", "/cal"] {
        assert!(!detail.contains(secret), "{secret} in {detail}");
    }
    assert!(detail.contains("dav.example.net"), "{detail}");
    assert!(detail.contains("dav.example.net:8080"), "{detail}");
}

#[test]
fn discovery_detail_never_carries_credentials_or_paths() {
    let mut s = well();
    s.discovery_error = Some("cannot list https://bob@dav.example.net/home/bob/".into());
    let detail = run(&s, 0).detail.unwrap();
    assert!(!detail.contains("bob"), "{detail}");
    assert!(detail.contains("dav.example.net"), "{detail}");
}

#[test]
fn detail_redaction_survives_parentheses_and_apostrophes() {
    for (err, secrets) in [
        (
            "failure https://alice:s3(cret@dav.example.net/private/dav/",
            &["alice", "s3(cret", "/private"][..],
        ),
        (
            "failure https://dav.example.net/cal/private(alice)/tasks/",
            &["alice", "/tasks", "/cal"][..],
        ),
        (
            "failure https://o'neil:pw@dav.example.net/o'neil/",
            &["neil", "pw@"][..],
        ),
        ("(see https://dav.example.net/cal/x)", &["/cal"][..]),
    ] {
        let mut s = well();
        s.last_error = Some(err.into());
        let detail = run(&s, 0).detail.unwrap();
        for secret in secrets {
            assert!(!detail.contains(secret), "{secret} in {detail}");
        }
        assert!(detail.contains("dav.example.net"), "{detail}");
    }
}

#[test]
fn detail_redaction_fails_closed_on_an_odd_host() {
    let mut s = well();
    s.last_error = Some("failure https://dav.example.net%2Fprivate%2Falice/x".into());
    let detail = run(&s, 0).detail.unwrap();
    assert!(!detail.contains("alice"), "{detail}");
    assert!(detail.contains("the server"), "{detail}");
}
