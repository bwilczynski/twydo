use chrono::{DateTime, Duration, Utc};
use std::os::unix::fs::PermissionsExt;
use twydo_gateway::state::{Role, StateError, Store, Subscription};

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

fn sub(endpoint: &str) -> Subscription {
    Subscription {
        endpoint: endpoint.into(),
        p256dh: "pk".into(),
        auth: "au".into(),
        saved_at: now(),
    }
}

fn paired(s: &Store, name: &str) -> String {
    let code = s.new_code(name, Role::Phone, now()).unwrap();
    s.redeem(&code, now()).unwrap().0.id
}

#[test]
fn vapid_is_created_once_and_never_rotated() {
    let (_d, s) = store();
    assert!(s.vapid().unwrap().is_none());
    let a = s.ensure_vapid("https://a.example").unwrap();
    let b = s.ensure_vapid("https://b.example").unwrap();
    assert_eq!(a.key.public_b64(), b.key.public_b64());
    assert_eq!(b.subject, "https://b.example");
    let v = s.vapid().unwrap().unwrap();
    assert_eq!(v.key.public_b64(), a.key.public_b64());
    assert_eq!(v.subject, "https://b.example");
}

#[test]
fn corrupt_vapid_key_is_an_error() {
    let (d, s) = store();
    std::fs::write(
        d.path().join("gw/devices.json"),
        r#"{"vapid":{"key":"AAAA","subject":"x"}}"#,
    )
    .unwrap();
    assert!(matches!(s.vapid(), Err(StateError::CorruptKey)));
    assert!(matches!(s.ensure_vapid("x"), Err(StateError::CorruptKey)));
}

#[test]
fn subscription_round_trip() {
    let (_d, s) = store();
    let id = paired(&s, "Pixel");
    paired(&s, "Other");
    s.set_subscription(&id, sub("https://push.example/1"))
        .unwrap();
    let all = s.subscriptions().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].0.id, id);
    assert_eq!(all[0].1, sub("https://push.example/1"));
    s.clear_subscription(&id).unwrap();
    assert!(s.subscriptions().unwrap().is_empty());
    assert!(matches!(
        s.set_subscription("nope", sub("https://x")),
        Err(StateError::NoSuchDevice)
    ));
}

#[test]
fn revoke_drops_the_subscription() {
    let (_d, s) = store();
    let id = paired(&s, "Pixel");
    s.set_subscription(&id, sub("https://push.example/1"))
        .unwrap();
    s.revoke("Pixel").unwrap();
    assert!(s.subscriptions().unwrap().is_empty());
}

#[test]
fn remove_dead_keeps_a_newer_subscription() {
    let (_d, s) = store();
    let id = paired(&s, "Pixel");
    s.set_subscription(&id, sub("https://push.example/A"))
        .unwrap();
    s.set_subscription(&id, sub("https://push.example/B"))
        .unwrap();
    s.remove_dead(&id, "https://push.example/A").unwrap();
    assert_eq!(
        s.subscriptions().unwrap()[0].1.endpoint,
        "https://push.example/B"
    );
    s.remove_dead(&id, "https://push.example/B").unwrap();
    assert!(s.subscriptions().unwrap().is_empty());
}

#[test]
fn old_devices_json_loads() {
    let (d, s) = store();
    std::fs::write(
        d.path().join("gw/devices.json"),
        r#"{"devices":[{"id":"abcd1234","name":"Old","role":"phone",
        "paired_at":"2026-11-01T00:00:00Z","last_seen":null,"session":null,
        "key_sha256":"00"}],"codes":[]}"#,
    )
    .unwrap();
    assert_eq!(s.devices().unwrap().len(), 1);
    assert!(s.vapid().unwrap().is_none());
    assert!(s.subscriptions().unwrap().is_empty());
}

#[test]
fn push_state_files_stay_0600() {
    let (d, s) = store();
    let id = paired(&s, "Pixel");
    s.ensure_vapid("https://a.example").unwrap();
    s.set_subscription(&id, sub("https://push.example/1"))
        .unwrap();
    for f in ["devices.json", "devices.lock"] {
        let m = std::fs::metadata(d.path().join("gw").join(f)).unwrap();
        assert_eq!(m.permissions().mode() & 0o777, 0o600, "{f}");
    }
}
