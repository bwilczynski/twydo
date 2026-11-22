//! Web Push message encryption (RFC 8291, `aes128gcm` of RFC 8188) and VAPID (RFC 8292).
//! Pure functions: no I/O. The primitives come from RustCrypto; this is only the RFC glue.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes128Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use hkdf::Hkdf;
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use p256::elliptic_curve::sec1::ToSec1Point;
use p256::{PublicKey, SecretKey};
use sha2::Sha256;

/// Every plaintext is padded to exactly this many bytes (delimiter included).
pub const PLAINTEXT_LEN: usize = 1024;
/// RFC 8188 record size announced in the header.
const RECORD_SIZE: u32 = 4096;
/// VAPID token lifetime in seconds (RFC 8292 allows at most 24 hours).
const VAPID_TTL: i64 = 43200;

/// The gateway's VAPID signing key.
pub struct VapidKey(SecretKey);

impl VapidKey {
    /// A fresh key from the operating system's random source.
    pub fn generate() -> VapidKey {
        loop {
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes).expect("operating system randomness");
            if let Ok(key) = SecretKey::from_slice(&bytes) {
                return VapidKey(key);
            }
        }
    }

    /// The key with this private scalar, or `None` when it is not a valid P-256 scalar.
    pub fn from_bytes(bytes: &[u8; 32]) -> Option<VapidKey> {
        SecretKey::from_slice(bytes).ok().map(VapidKey)
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes().into()
    }

    /// The uncompressed public point, base64url without padding (the `applicationServerKey`).
    pub fn public_b64(&self) -> String {
        B64.encode(self.0.public_key().to_sec1_point(false).as_bytes())
    }
}

/// `scheme://host[:port]` of an endpoint, without the scheme's default port.
fn origin(endpoint: &str) -> Result<String, String> {
    let (scheme, rest) = endpoint.split_once("://").ok_or("endpoint has no scheme")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or("");
    if authority.is_empty() {
        return Err("endpoint has no host".into());
    }
    let default = match scheme {
        "https" => ":443",
        "http" => ":80",
        _ => "",
    };
    let authority = authority.strip_suffix(default).unwrap_or(authority);
    Ok(format!("{scheme}://{authority}"))
}

/// The `Authorization` header value: `vapid t=<jwt>, k=<public key>`.
pub fn vapid_header(
    key: &VapidKey,
    endpoint: &str,
    subject: &str,
    now: i64,
) -> Result<String, String> {
    let header = r#"{"typ":"JWT","alg":"ES256"}"#;
    let claims = serde_json::json!({
        "aud": origin(endpoint)?,
        "exp": now + VAPID_TTL,
        "sub": subject,
    });
    let signing_input = format!(
        "{}.{}",
        B64.encode(header),
        B64.encode(serde_json::to_vec(&claims).map_err(|e| e.to_string())?)
    );
    let signature: Signature = SigningKey::from(&key.0).sign(signing_input.as_bytes());
    Ok(format!(
        "vapid t={signing_input}.{}, k={}",
        B64.encode(signature.to_bytes()),
        key.public_b64()
    ))
}

/// What a reminder tells the phone.
pub struct Payload {
    pub title: String,
    pub body: String,
    pub key: String,
    pub uid: Option<String>,
}

fn json(title: &str, body: &str, p: &Payload) -> Vec<u8> {
    let mut map = serde_json::Map::new();
    map.insert("title".into(), title.into());
    map.insert("body".into(), body.into());
    map.insert("key".into(), p.key.as_str().into());
    if let Some(uid) = &p.uid {
        map.insert("uid".into(), uid.as_str().into());
    }
    serde_json::to_vec(&map).expect("a JSON object serialises")
}

/// The first `n` chars of `s` and `…`.
fn cut(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    out.push('…');
    out
}

/// Shortens `text` until `render` produces at most `max` bytes (or only `…` is left).
fn shorten(text: &str, max: usize, render: impl Fn(&str) -> Vec<u8>) -> String {
    let mut shortened = text.to_string();
    let mut n = text.chars().count();
    let mut len = render(&shortened).len();
    while len > max && n > 0 {
        // Every char takes at least one byte, so this never overshoots by much.
        n = n.saturating_sub((len - max).div_ceil(4).max(1));
        shortened = cut(text, n);
        len = render(&shortened).len();
    }
    shortened
}

/// The JSON payload, shortened until it fits, then `0x02` and zeros up to [`PLAINTEXT_LEN`].
pub fn padded(p: &Payload) -> Vec<u8> {
    let max = PLAINTEXT_LEN - 1;
    let body = if json(&p.title, &p.body, p).len() > max {
        shorten(&p.body, max, |b| json(&p.title, b, p))
    } else {
        p.body.clone()
    };
    let title = if json(&p.title, &body, p).len() > max {
        shorten(&p.title, max, |t| json(t, &body, p))
    } else {
        p.title.clone()
    };
    let mut out = json(&title, &body, p);
    out.truncate(max);
    out.push(0x02);
    out.resize(PLAINTEXT_LEN, 0);
    out
}

/// Encrypts `plain` (already ending in the `0x02` delimiter) for one subscription.
pub fn encrypt(plain: &[u8], p256dh: &[u8], auth: &[u8]) -> Result<Vec<u8>, String> {
    let ephemeral = loop {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
        if let Ok(key) = SecretKey::from_slice(&bytes) {
            break key;
        }
    };
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|e| e.to_string())?;
    encrypt_with(plain, p256dh, auth, ephemeral, salt)
}

fn encrypt_with(
    plain: &[u8],
    p256dh: &[u8],
    auth: &[u8],
    ephemeral: SecretKey,
    salt: [u8; 16],
) -> Result<Vec<u8>, String> {
    let ua_public =
        PublicKey::from_sec1_bytes(p256dh).map_err(|_| "invalid subscription key".to_string())?;
    let as_public = ephemeral.public_key().to_sec1_point(false);
    let ua_point = ua_public.to_sec1_point(false);

    let shared = p256::ecdh::diffie_hellman(ephemeral.to_nonzero_scalar(), ua_public.as_affine());

    // RFC 8291 section 3.3: combine the ECDH secret with the auth secret.
    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(ua_point.as_bytes());
    key_info.extend_from_slice(as_public.as_bytes());
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(auth), shared.raw_secret_bytes())
        .expand(&key_info, &mut ikm)
        .map_err(|e| e.to_string())?;

    // RFC 8188 section 2.2 and 2.3: content encryption key and nonce.
    let hk = Hkdf::<Sha256>::new(Some(&salt), &ikm);
    let mut cek = [0u8; 16];
    hk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .map_err(|e| e.to_string())?;
    let mut nonce = [0u8; 12];
    hk.expand(b"Content-Encoding: nonce\0", &mut nonce)
        .map_err(|e| e.to_string())?;

    let cipher = Aes128Gcm::new_from_slice(&cek).map_err(|e| e.to_string())?;
    let sealed = cipher
        .encrypt(&Nonce::from(nonce), plain)
        .map_err(|_| "encryption failed".to_string())?;

    let mut out = Vec::with_capacity(86 + sealed.len());
    out.extend_from_slice(&salt);
    out.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    out.push(as_public.len() as u8);
    out.extend_from_slice(as_public.as_bytes());
    out.extend_from_slice(&sealed);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::VerifyingKey;
    use p256::ecdsa::signature::Verifier;

    fn b64(s: &str) -> Vec<u8> {
        B64.decode(s.split_whitespace().collect::<String>())
            .unwrap()
    }

    #[test]
    fn rfc8291_appendix_a_vector() {
        let mut plain = b"When I grow up, I want to be a watermelon".to_vec();
        plain.push(0x02);
        let ua_public = b64(
            "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
        );
        let auth = b64("BTBZMqHH6r4Tts7J_aSIgg");
        let as_private =
            SecretKey::from_slice(&b64("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw")).unwrap();
        let salt: [u8; 16] = b64("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        let out = encrypt_with(&plain, &ua_public, &auth, as_private, salt).unwrap();
        // Header and ciphertext are separate base64 runs (86 bytes is not a multiple of 3).
        let mut expected = b64(
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIg\
             Dll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8",
        );
        expected.extend(b64(
            "8pfeW0KbunFT06SuDKoJH9Ql87S1QUrdirN6GcG7sFz1y1sqLgVi1VhjVkHsUoEsbI_0LpXMuGvnzQ",
        ));
        assert_eq!(out, expected);
    }

    #[test]
    fn encrypt_uses_fresh_randomness() {
        let ua = SecretKey::from_slice(&[7u8; 32]).unwrap();
        let point = ua.public_key().to_sec1_point(false);
        let a = encrypt(&padded(&sample(None)), point.as_bytes(), &[1; 16]).unwrap();
        let b = encrypt(&padded(&sample(None)), point.as_bytes(), &[1; 16]).unwrap();
        assert_ne!(a, b);
        assert_eq!(a.len(), 86 + PLAINTEXT_LEN + 16);
    }

    #[test]
    fn vapid_jwt_verifies() {
        let key = VapidKey::from_bytes(&[9u8; 32]).unwrap();
        let header = vapid_header(
            &key,
            "https://push.example.net:443/x",
            "https://tasks.example.test",
            1_000,
        )
        .unwrap();
        let rest = header.strip_prefix("vapid t=").unwrap();
        let (jwt, k) = rest.split_once(", k=").unwrap();
        assert_eq!(k, key.public_b64());
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(b64(parts[0]), br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims: serde_json::Value = serde_json::from_slice(&b64(parts[1])).unwrap();
        assert_eq!(claims["aud"], "https://push.example.net");
        assert_eq!(claims["exp"], 1_000 + 43200);
        assert_eq!(claims["sub"], "https://tasks.example.test");
        let verifying = VerifyingKey::from(key.0.public_key());
        let sig = Signature::from_slice(&b64(parts[2])).unwrap();
        verifying
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig)
            .unwrap();
    }

    #[test]
    fn vapid_origin_keeps_non_default_port() {
        assert_eq!(
            origin("https://h.example:8443/a?b").unwrap(),
            "https://h.example:8443"
        );
        assert!(origin("nonsense").is_err());
    }

    #[test]
    fn vapid_key_roundtrips() {
        let key = VapidKey::generate();
        let again = VapidKey::from_bytes(&key.to_bytes()).unwrap();
        assert_eq!(key.public_b64(), again.public_b64());
        assert_eq!(b64(&key.public_b64()).len(), 65);
    }

    fn sample(uid: Option<&str>) -> Payload {
        Payload {
            title: "Buy milk".into(),
            body: "Due now".into(),
            key: "k1".into(),
            uid: uid.map(str::to_string),
        }
    }

    fn unpad(v: &[u8]) -> &[u8] {
        let end = v.iter().rposition(|b| *b != 0).unwrap();
        assert_eq!(v[end], 0x02);
        &v[..end]
    }

    #[test]
    fn payload_is_always_1024_bytes() {
        let short = padded(&sample(None));
        assert_eq!(short.len(), 1024);
        let json = unpad(&short);
        assert_eq!(
            std::str::from_utf8(json).unwrap(),
            r#"{"title":"Buy milk","body":"Due now","key":"k1"}"#
        );
        let with_uid = padded(&sample(Some("u1")));
        assert!(unpad(&with_uid).ends_with(br#","uid":"u1"}"#));

        // Exactly 1023 bytes of JSON: no cutting.
        let mut p = sample(None);
        let base = json_len(&p);
        p.body = "x".repeat(1023 - base + p.body.len());
        let out = padded(&p);
        assert_eq!(out.len(), 1024);
        assert_eq!(unpad(&out).len(), 1023);
        assert!(!String::from_utf8_lossy(unpad(&out)).contains('…'));
    }

    fn json_len(p: &Payload) -> usize {
        json(&p.title, &p.body, p).len()
    }

    #[test]
    fn payload_cuts_long_text_on_char_boundaries() {
        let p = Payload {
            title: "ż".repeat(2000),
            body: "ż".repeat(2000),
            key: "k".into(),
            uid: Some("u".into()),
        };
        let out = padded(&p);
        assert_eq!(out.len(), 1024);
        let v: serde_json::Value = serde_json::from_slice(unpad(&out)).unwrap();
        assert!(v["body"].as_str().unwrap().ends_with('…'));
        assert_eq!(v["uid"], "u");
    }

    #[test]
    fn payload_cuts_title_only_after_body() {
        let p = Payload {
            title: "t".repeat(2000),
            body: "b".into(),
            key: "k".into(),
            uid: None,
        };
        let out = padded(&p);
        let v: serde_json::Value = serde_json::from_slice(unpad(&out)).unwrap();
        assert!(v["title"].as_str().unwrap().ends_with('…'));
        assert_eq!(out.len(), 1024);
    }
}
