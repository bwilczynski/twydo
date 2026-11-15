//! TLS behaviour of the CalDAV client with `[sync].ca_file`, against a local TLS listener.
//! Certificates are generated here; only 127.0.0.1 and `localhost` are used.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use twydo_core::config::SyncConfig;
use twydo_core::lists::call::Refused;
use twydo_core::lists::dav::{Properties, connect};

struct Pair {
    pem: String,
    der: CertificateDer<'static>,
    key: KeyPair,
}

fn ca(name: &str) -> (Pair, CertificateParams) {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.distinguished_name.push(DnType::CommonName, name);
    let key = KeyPair::generate().unwrap();
    let cert = params.self_signed(&key).unwrap();
    let pair = Pair {
        pem: cert.pem(),
        der: cert.der().clone(),
        key,
    };
    (pair, params)
}

/// A leaf for `localhost` signed by the given authority.
fn leaf_signed_by(authority: &Pair, params: &CertificateParams) -> Pair {
    let mut leaf = CertificateParams::new(vec!["localhost".to_string()]).unwrap();
    leaf.distinguished_name.push(DnType::CommonName, "leaf");
    let key = KeyPair::generate().unwrap();
    let issuer = Issuer::from_params(params, &authority.key);
    let cert = leaf.signed_by(&key, &issuer).unwrap();
    Pair {
        pem: cert.pem(),
        der: cert.der().clone(),
        key,
    }
}

/// A self-signed certificate for `localhost`, optionally flagged as a CA like `openssl req -x509`.
fn self_signed(as_ca: bool) -> Pair {
    let mut params = CertificateParams::new(vec!["localhost".to_string()]).unwrap();
    params
        .distinguished_name
        .push(DnType::CommonName, "localhost");
    if as_ca {
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    }
    let key = KeyPair::generate().unwrap();
    let cert = params.self_signed(&key).unwrap();
    Pair {
        pem: cert.pem(),
        der: cert.der().clone(),
        key,
    }
}

const BODY: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?><d:multistatus xmlns:d=\"DAV:\" \
    xmlns:c=\"urn:ietf:params:xml:ns:caldav\" xmlns:a=\"http://apple.com/ns/ical/\">\
    <d:response><d:href>/dav/alice/work/</d:href><d:propstat><d:prop><d:displayname>Work</d:displayname>\
    <a:calendar-color>#112233</a:calendar-color>\
    <d:resourcetype><d:collection/><c:calendar/></d:resourcetype>\
    <c:supported-calendar-component-set><c:comp name=\"VTODO\"/>\
    </c:supported-calendar-component-set></d:prop>\
    <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>";

struct Listener {
    port: u16,
    /// Application bytes the server read: zero when no handshake completed.
    received: Arc<AtomicUsize>,
}

/// Answers every request with the multistatus above, one request per connection.
fn serve(identity: &Pair) -> Listener {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![identity.der.clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.key.serialize_der())),
        )
        .unwrap();
    let config = Arc::new(config);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let received = Arc::new(AtomicUsize::new(0));
    let counter = received.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let tls = StreamOwned::new(ServerConnection::new(config.clone()).unwrap(), stream);
            let mut reader = BufReader::new(tls);
            let mut length = 0;
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => counter.fetch_add(n, Ordering::SeqCst),
                };
                if line == "\r\n" {
                    let mut body = vec![0; length];
                    if reader.read_exact(&mut body).is_ok() {
                        counter.fetch_add(length, Ordering::SeqCst);
                        let reply = format!(
                            "HTTP/1.1 207 Multi-Status\r\nContent-Type: application/xml\r\n\
                             Content-Length: {}\r\nConnection: close\r\n\r\n{BODY}",
                            BODY.len()
                        );
                        let tls = reader.get_mut();
                        let _ = tls.write_all(reply.as_bytes());
                        let _ = tls.flush();
                    }
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
        }
    });
    Listener { port, received }
}

/// One `read_properties` call against `https://localhost:<port>/dav/alice/work/` trusting `ca_file`.
fn call(listener: &Listener, ca_file: &Path) -> Result<Properties, Refused> {
    let url = format!("https://localhost:{}/dav/alice/", listener.port);
    let sync = SyncConfig {
        url: url.clone(),
        username: "alice".into(),
        password_command: vec!["printf".into(), "secret".into()],
        ca_file: Some(ca_file.to_path_buf()),
        ..SyncConfig::default()
    };
    let dav = connect(&sync, Instant::now() + Duration::from_secs(20))?;
    let list = dav.collection(&format!("{url}work/"), "work")?;
    list.read_properties()
}

fn trusting(pems: &[&str]) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ca.pem");
    std::fs::write(&path, pems.concat()).unwrap();
    (dir, path)
}

#[test]
fn a_server_signed_by_the_ca_file_is_trusted() {
    let (authority, params) = ca("Test CA one");
    let leaf = leaf_signed_by(&authority, &params);
    let listener = serve(&leaf);
    let (_dir, file) = trusting(&[&authority.pem]);
    let properties = call(&listener, &file).expect("trusted");
    assert_eq!(properties.name.as_deref(), Some("Work"));
}

#[test]
fn a_server_not_signed_by_the_ca_file_is_refused() {
    let (trusted, _) = ca("Test CA one");
    let (other, other_params) = ca("Test CA two");
    let leaf = leaf_signed_by(&other, &other_params);
    let listener = serve(&leaf);
    let (_dir, file) = trusting(&[&trusted.pem]);
    let refused = call(&listener, &file).unwrap_err();
    assert_eq!(refused.code, "connect");
    assert!(!refused.sent);
    assert_eq!(listener.received.load(Ordering::SeqCst), 0);
}

/// What `openssl req -x509` makes by default (basic constraints CA:TRUE) is refused: the
/// client's verifier does not accept a CA certificate as the end entity, even when it is
/// the trust anchor itself.
#[test]
fn a_self_signed_ca_flagged_leaf_in_the_ca_file_is_refused() {
    let leaf = self_signed(true);
    let listener = serve(&leaf);
    let (_dir, file) = trusting(&[&leaf.pem]);
    let refused = call(&listener, &file).unwrap_err();
    assert_eq!(refused.code, "connect");
    assert!(!refused.sent);
    assert_eq!(listener.received.load(Ordering::SeqCst), 0);
}

/// A self-signed end-entity certificate (no CA flag) with a subjectAltName that is the only
/// certificate in `ca_file` is trusted.
#[test]
fn a_self_signed_end_entity_leaf_in_the_ca_file_is_trusted() {
    let leaf = self_signed(false);
    let listener = serve(&leaf);
    let (_dir, file) = trusting(&[&leaf.pem]);
    let properties = call(&listener, &file).expect("trusted");
    assert_eq!(properties.name.as_deref(), Some("Work"));
}
