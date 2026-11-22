//! A fake push service and a fake subscriber, for tests only (feature `test-support`).
//! A TLS listener on 127.0.0.1 with a generated CA; the endpoint host is `localhost`.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes128Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use hkdf::Hkdf;
use p256::elliptic_curve::sec1::ToSec1Point;
use p256::{PublicKey, SecretKey};
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair};
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use sha2::Sha256;

/// What the fake service does with a request after recording it.
#[derive(Clone, Copy, Debug)]
pub enum Reply {
    /// Answer with this status and an empty body.
    Status(u16),
    /// Never answer.
    Hang,
}

/// One request the fake service received.
#[derive(Clone, Debug)]
pub struct Request {
    pub path: String,
    /// Header names lowercased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

pub struct FakePush {
    pub port: u16,
    /// The CA to trust (`PushOptions::ca_file`).
    pub ca_file: PathBuf,
    requests: Arc<Mutex<Vec<Request>>>,
    _dir: tempfile::TempDir,
}

impl FakePush {
    pub fn start(reply: Reply) -> FakePush {
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "Test push CA");
        let ca_key = KeyPair::generate().unwrap();
        let ca_cert = ca_params.self_signed(&ca_key).unwrap();
        let mut leaf = CertificateParams::new(vec!["localhost".to_string()]).unwrap();
        leaf.distinguished_name.push(DnType::CommonName, "leaf");
        let leaf_key = KeyPair::generate().unwrap();
        let issuer = Issuer::from_params(&ca_params, &ca_key);
        let leaf_cert = leaf.signed_by(&leaf_key, &issuer).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let ca_file = dir.path().join("ca.pem");
        std::fs::write(&ca_file, ca_cert.pem()).unwrap();

        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = Arc::new(
            ServerConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(
                    vec![leaf_cert.der().clone()],
                    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
                )
                .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let (config, seen) = (config.clone(), seen.clone());
                std::thread::spawn(move || {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(10)))
                        .unwrap();
                    let tls = StreamOwned::new(ServerConnection::new(config).unwrap(), stream);
                    let mut reader = BufReader::new(tls);
                    let Some(request) = read_request(&mut reader) else {
                        return;
                    };
                    seen.lock().unwrap().push(request);
                    match reply {
                        Reply::Status(code) => {
                            let tls = reader.get_mut();
                            let _ = write!(
                                tls,
                                "HTTP/1.1 {code} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            );
                            let _ = tls.flush();
                        }
                        Reply::Hang => std::thread::sleep(Duration::from_secs(10)),
                    }
                });
            }
        });
        FakePush {
            port,
            ca_file,
            requests,
            _dir: dir,
        }
    }

    /// An endpoint on this service, e.g. `endpoint("send/abc")`.
    pub fn endpoint(&self, path: &str) -> String {
        format!("https://localhost:{}/{path}", self.port)
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

fn read_request<R: BufRead>(reader: &mut R) -> Option<Request> {
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let path = line.split_whitespace().nth(1)?.to_string();
    let mut headers = Vec::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':')?;
        headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
    }
    let length = headers
        .iter()
        .find(|(n, _)| n == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some(Request {
        path,
        headers,
        body,
    })
}

/// A subscriber: the P-256 key pair and 16-byte auth secret a browser would hold.
pub struct Subscriber {
    secret: SecretKey,
    pub auth: [u8; 16],
}

impl Default for Subscriber {
    fn default() -> Self {
        Self::new()
    }
}

impl Subscriber {
    pub fn new() -> Subscriber {
        let mut auth = [0u8; 16];
        getrandom::fill(&mut auth).unwrap();
        let secret = loop {
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes).unwrap();
            if let Ok(key) = SecretKey::from_slice(&bytes) {
                break key;
            }
        };
        Subscriber { secret, auth }
    }

    /// The uncompressed public key, base64url without padding (`p256dh`).
    pub fn p256dh_b64(&self) -> String {
        B64.encode(self.secret.public_key().to_sec1_point(false).as_bytes())
    }

    /// The auth secret, base64url without padding.
    pub fn auth_b64(&self) -> String {
        B64.encode(self.auth)
    }

    /// Decrypts an `aes128gcm` body (RFC 8291 receiver side); returns the plaintext without
    /// padding.
    pub fn decrypt(&self, body: &[u8]) -> Result<Vec<u8>, String> {
        if body.len() < 21 {
            return Err("short body".into());
        }
        let salt = &body[..16];
        let idlen = body[20] as usize;
        let as_public = body.get(21..21 + idlen).ok_or("short key id")?;
        let sealed = &body[21 + idlen..];
        let sender = PublicKey::from_sec1_bytes(as_public).map_err(|e| e.to_string())?;
        let shared =
            p256::ecdh::diffie_hellman(self.secret.to_nonzero_scalar(), sender.as_affine());
        let ua_public = self.secret.public_key().to_sec1_point(false);
        let mut info = b"WebPush: info\0".to_vec();
        info.extend_from_slice(ua_public.as_bytes());
        info.extend_from_slice(as_public);
        let mut ikm = [0u8; 32];
        Hkdf::<Sha256>::new(Some(&self.auth), shared.raw_secret_bytes())
            .expand(&info, &mut ikm)
            .map_err(|e| e.to_string())?;
        let hk = Hkdf::<Sha256>::new(Some(salt), &ikm);
        let mut cek = [0u8; 16];
        hk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
            .map_err(|e| e.to_string())?;
        let mut nonce = [0u8; 12];
        hk.expand(b"Content-Encoding: nonce\0", &mut nonce)
            .map_err(|e| e.to_string())?;
        let mut plain = Aes128Gcm::new_from_slice(&cek)
            .map_err(|e| e.to_string())?
            .decrypt(&Nonce::from(nonce), sealed)
            .map_err(|_| "decryption failed".to_string())?;
        while plain.last() == Some(&0) {
            plain.pop();
        }
        if plain.pop() != Some(2) {
            return Err("missing delimiter".into());
        }
        Ok(plain)
    }
}
