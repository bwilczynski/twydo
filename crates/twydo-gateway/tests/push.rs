use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use chrono::{DateTime, Utc};
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use twydo_gateway::push::crypto::Payload;
use twydo_gateway::push::send::{PushOptions, push};
use twydo_gateway::state::{Role, Store, Subscription};
use twydo_gateway::testing::{FakePush, Reply, Subscriber};

fn now() -> DateTime<Utc> {
    "2026-11-22T10:00:00Z".parse().unwrap()
}

fn payload() -> Payload {
    Payload {
        title: "Secret title".into(),
        body: "Secret body".into(),
        key: "inbox:abc".into(),
        uid: Some("uid-1".into()),
    }
}

struct Setup {
    _dir: tempfile::TempDir,
    store: Store,
    fake: FakePush,
    phone: Subscriber,
}

fn setup(reply: Reply) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("gw")).unwrap();
    store.ensure_vapid("https://tasks.example.test").unwrap();
    let fake = FakePush::start(reply);
    let phone = Subscriber::new();
    let code = store.new_code("Pixel", Role::Phone, now()).unwrap();
    let (device, _) = store.redeem(&code, now()).unwrap();
    store
        .set_subscription(
            &device.id,
            Subscription {
                endpoint: fake.endpoint("send/secret-token"),
                p256dh: phone.p256dh_b64(),
                auth: phone.auth_b64(),
                saved_at: now(),
            },
        )
        .unwrap();
    Setup {
        _dir: dir,
        store,
        fake,
        phone,
    }
}

fn opts(s: &Setup, deadline: Duration) -> PushOptions {
    PushOptions {
        ca_file: Some(s.fake.ca_file.clone()),
        deadline,
    }
}

fn run(s: &Setup, deadline: Duration) -> (Result<(), String>, String) {
    let mut log = Vec::new();
    let r = push(&s.store, &payload(), &opts(s, deadline), now(), &mut log);
    (r, String::from_utf8(log).unwrap())
}

#[test]
fn delivers_a_decryptable_payload() {
    let s = setup(Reply::Status(201));
    let (r, _) = run(&s, Duration::from_secs(8));
    assert_eq!(r, Ok(()));
    let requests = s.fake.requests();
    assert_eq!(requests.len(), 1);
    let req = &requests[0];
    assert_eq!(req.path, "/send/secret-token");
    assert_eq!(req.header("ttl"), Some("86400"));
    assert_eq!(req.header("urgency"), Some("high"));
    assert_eq!(req.header("content-encoding"), Some("aes128gcm"));
    assert_eq!(req.header("content-type"), Some("application/octet-stream"));
    let plain = s.phone.decrypt(&req.body).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&plain).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"title": "Secret title", "body": "Secret body",
            "key": "inbox:abc", "uid": "uid-1"})
    );
    let header = req.header("authorization").unwrap();
    let (jwt, k) = header
        .strip_prefix("vapid t=")
        .unwrap()
        .split_once(", k=")
        .unwrap();
    let vapid = s.store.vapid().unwrap().unwrap();
    assert_eq!(k, vapid.key.public_b64());
    let parts: Vec<&str> = jwt.split('.').collect();
    let public = B64.decode(k).unwrap();
    let verifying = VerifyingKey::from_sec1_bytes(&public).unwrap();
    let sig = Signature::from_slice(&B64.decode(parts[2]).unwrap()).unwrap();
    verifying
        .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig)
        .unwrap();
}

#[test]
fn gone_subscription_is_removed() {
    let s = setup(Reply::Status(410));
    let (r, _) = run(&s, Duration::from_secs(8));
    assert!(r.unwrap_err().starts_with("no phone"));
    assert!(s.store.subscriptions().unwrap().is_empty());
}

#[test]
fn server_error_keeps_the_subscription_and_fails() {
    let s = setup(Reply::Status(500));
    let (r, _) = run(&s, Duration::from_secs(8));
    assert_eq!(
        r,
        Err("push failed for 1 of 1 phones: Pixel: status 500".to_string())
    );
    assert_eq!(s.store.subscriptions().unwrap().len(), 1);
}

#[test]
fn hung_service_is_cut_at_the_deadline() {
    let s = setup(Reply::Hang);
    let start = Instant::now();
    let (r, _) = run(&s, Duration::from_secs(1));
    assert!(r.is_err());
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(s.store.subscriptions().unwrap().len(), 1);
}

#[test]
fn logs_hold_no_title_or_endpoint_path() {
    for reply in [Reply::Status(201), Reply::Status(500)] {
        let s = setup(reply);
        let (_, log) = run(&s, Duration::from_secs(8));
        assert!(log.contains("push Pixel localhost:"), "{log}");
        for secret in ["Secret title", "Secret body", "secret-token", "/send"] {
            assert!(!log.contains(secret), "{log}");
        }
    }
}

#[test]
fn a_subscription_that_cannot_be_encrypted_for_is_dropped() {
    let s = setup(Reply::Status(201));
    let code = s.store.new_code("Broken", Role::Phone, now()).unwrap();
    let (device, _) = s.store.redeem(&code, now()).unwrap();
    s.store
        .set_subscription(
            &device.id,
            Subscription {
                endpoint: s.fake.endpoint("send/other"),
                p256dh: "!!not base64!!".into(),
                auth: s.phone.auth_b64(),
                saved_at: now(),
            },
        )
        .unwrap();
    let (r, log) = run(&s, Duration::from_secs(8));
    assert_eq!(r, Ok(()));
    assert!(log.contains("push Broken localhost:"), "{log}");
    assert!(log.contains("invalid subscription"), "{log}");
    let left = s.store.subscriptions().unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].0.name, "Pixel");
}
