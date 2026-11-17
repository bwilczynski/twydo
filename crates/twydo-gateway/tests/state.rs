use chrono::{DateTime, Duration, Utc};
use std::os::unix::fs::PermissionsExt;
use twydo_gateway::state::{Role, StateError, Store};

fn now() -> DateTime<Utc> {
    "2026-11-17T10:00:00Z".parse().unwrap()
}

fn store() -> (tempfile::TempDir, Store) {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(&d.path().join("gw")).unwrap();
    (d, s)
}

#[test]
fn code_redeems_once() {
    let (_d, s) = store();
    let code = s.new_code("Pixel", Role::Phone, now()).unwrap();
    assert_eq!(code.len(), 9);
    assert_eq!(code.as_bytes()[4], b'-');
    let (dev, key) = s.redeem(&code, now()).unwrap();
    assert_eq!(dev.name, "Pixel");
    assert_eq!(dev.role, Role::Phone);
    assert_eq!(dev.id.len(), 8);
    assert!(!key.is_empty());
    assert!(matches!(
        s.redeem(&code, now()),
        Err(StateError::CodeInvalid)
    ));
}

#[test]
fn code_expires_after_10_minutes() {
    let (_d, s) = store();
    let code = s.new_code("A", Role::Phone, now()).unwrap();
    let late = now() + Duration::minutes(10) + Duration::seconds(1);
    assert!(matches!(
        s.redeem(&code, late),
        Err(StateError::CodeInvalid)
    ));
    let code = s.new_code("B", Role::Phone, now()).unwrap();
    assert!(s.redeem(&code, now() + Duration::minutes(10)).is_ok());
}

#[test]
fn five_wrong_attempts_void_the_code() {
    let (_d, s) = store();
    let code = s.new_code("A", Role::Phone, now()).unwrap();
    for _ in 0..5 {
        assert!(matches!(
            s.redeem("0000-0000", now()),
            Err(StateError::CodeInvalid)
        ));
    }
    assert!(matches!(
        s.redeem(&code, now()),
        Err(StateError::CodeInvalid)
    ));
}

#[test]
fn four_wrong_attempts_leave_the_code_usable() {
    let (_d, s) = store();
    let code = s.new_code("A", Role::Phone, now()).unwrap();
    for _ in 0..4 {
        let _ = s.redeem("0000-0000", now());
    }
    assert!(s.redeem(&code, now()).is_ok());
}

#[test]
fn code_matching_ignores_case_and_dash() {
    let (_d, s) = store();
    let code = s.new_code("A", Role::Display, now()).unwrap();
    let sloppy = code.replace('-', "").to_lowercase();
    let (dev, _) = s.redeem(&sloppy, now()).unwrap();
    assert_eq!(dev.role, Role::Display);
}

#[test]
fn authenticate_accepts_key_and_rejects_others() {
    let (_d, s) = store();
    let code = s.new_code("A", Role::Phone, now()).unwrap();
    let (dev, key) = s.redeem(&code, now()).unwrap();
    assert_eq!(s.authenticate(&key).unwrap().id, dev.id);
    assert!(s.authenticate("nope").is_none());
    assert!(s.authenticate("").is_none());
}

#[test]
fn revoked_device_no_longer_authenticates() {
    let (_d, s) = store();
    let code = s.new_code("A", Role::Phone, now()).unwrap();
    let (_, key) = s.redeem(&code, now()).unwrap();
    s.revoke("A").unwrap();
    assert!(s.authenticate(&key).is_none());
    assert!(s.devices().unwrap().is_empty());
    assert!(matches!(s.revoke("A"), Err(StateError::NoSuchDevice)));
}

#[test]
fn files_hold_hashes_only() {
    let (d, s) = store();
    let code = s.new_code("A", Role::Phone, now()).unwrap();
    let pending = std::fs::read_to_string(d.path().join("gw/devices.json")).unwrap();
    let bare = code.replace('-', "");
    assert!(!pending.contains(&code) && !pending.contains(&bare));
    let (_, key) = s.redeem(&code, now()).unwrap();
    let text = std::fs::read_to_string(d.path().join("gw/devices.json")).unwrap();
    assert!(!text.contains(&key) && !text.contains(&code) && !text.contains(&bare));
    assert!(text.contains("key_sha256"));
}

#[test]
fn state_files_are_private() {
    let (d, s) = store();
    s.new_code("A", Role::Phone, now()).unwrap();
    let mode = |p: &str| {
        std::fs::metadata(d.path().join(p))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    };
    assert_eq!(mode("gw"), 0o700);
    assert_eq!(mode("gw/devices.json"), 0o600);
    assert_eq!(mode("gw/devices.lock"), 0o600);
}

#[test]
fn duplicate_name_refused() {
    let (_d, s) = store();
    let code = s.new_code("A", Role::Phone, now()).unwrap();
    s.redeem(&code, now()).unwrap();
    assert!(matches!(
        s.new_code("A", Role::Phone, now()),
        Err(StateError::NameTaken)
    ));
    s.new_code("B", Role::Phone, now()).unwrap();
    assert!(matches!(
        s.new_code("B", Role::Phone, now()),
        Err(StateError::NameTaken)
    ));
    assert!(matches!(
        s.new_code("", Role::Phone, now()),
        Err(StateError::NameInvalid)
    ));
    assert!(matches!(
        s.new_code("a\nb", Role::Phone, now()),
        Err(StateError::NameInvalid)
    ));
    let long = "x".repeat(41);
    assert!(matches!(
        s.new_code(&long, Role::Phone, now()),
        Err(StateError::NameInvalid)
    ));
}

#[test]
fn touch_records_last_seen_and_session() {
    let (_d, s) = store();
    let code = s.new_code("A", Role::Phone, now()).unwrap();
    let (dev, _) = s.redeem(&code, now()).unwrap();
    let t = now() + Duration::hours(1);
    s.touch(&dev.id, t, Some("sess")).unwrap();
    let d = &s.devices().unwrap()[0];
    assert_eq!(d.last_seen, Some(t));
    assert_eq!(d.session.as_deref(), Some("sess"));
    assert!(matches!(
        s.touch("zzzzzzzz", t, None),
        Err(StateError::NoSuchDevice)
    ));
}

/// Pin the mtime so the test does not depend on the filesystem's timestamp granularity.
fn set_mtime(d: &tempfile::TempDir, secs: u64) {
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(d.path().join("gw/devices.json"))
        .unwrap();
    f.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs))
        .unwrap();
}

#[test]
fn changed_since_sees_admin_change() {
    let (d, s) = store();
    let mut mark = None;
    assert!(!s.changed_since(&mut mark));
    let code = s.new_code("A", Role::Phone, now()).unwrap();
    set_mtime(&d, 1_000_000);
    assert!(s.changed_since(&mut mark));
    assert!(!s.changed_since(&mut mark));
    s.redeem(&code, now()).unwrap();
    set_mtime(&d, 2_000_000);
    assert!(s.changed_since(&mut mark));
}

#[test]
fn wrong_attempt_with_nothing_pending_does_not_write() {
    let (d, s) = store();
    assert!(s.redeem("0000-0000", now()).is_err());
    assert!(!d.path().join("gw/devices.json").exists());
}

#[test]
fn wrong_attempt_charges_every_pending_code() {
    let (_d, s) = store();
    let a = s.new_code("A", Role::Phone, now()).unwrap();
    let b = s.new_code("B", Role::Phone, now()).unwrap();
    for _ in 0..4 {
        assert!(s.redeem("0000-0000", now()).is_err());
    }
    assert!(s.redeem(&a, now()).is_ok());
    // B was charged four times too: one more wrong attempt voids it.
    assert!(s.redeem("0000-0000", now()).is_err());
    assert!(matches!(s.redeem(&b, now()), Err(StateError::CodeInvalid)));
}
