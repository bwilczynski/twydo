use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::Command;

use twydo_testdav::{FakeDav, add_collection, calls, collections, fake_vdirsyncer};

const AUTH: &str = "Authorization: Basic YWxpY2U6c2VjcmV0\r\n";

/// One raw HTTP exchange; returns the status code and the body.
fn request(dav: &FakeDav, method: &str, path: &str, headers: &str, body: &str) -> (u16, String) {
    let port = dav
        .url()
        .split(':')
        .nth(2)
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .to_string();
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    let text = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{headers}Content-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(text.as_bytes()).unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let status = raw.split(' ').nth(1).unwrap().parse().unwrap();
    let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
}

#[test]
fn unauthenticated_request_is_401() {
    let root = tempfile::tempdir().unwrap();
    let dav = FakeDav::start(root.path());
    let (status, _) = request(&dav, "PROPFIND", "/dav/alice/", "Depth: 0\r\n", "");
    assert_eq!(status, 401);
    let (status, _) = request(
        &dav,
        "PROPFIND",
        "/dav/alice/",
        "Depth: 0\r\nAuthorization: Basic YWxpY2U6d3Jvbmc=\r\n",
        "",
    );
    assert_eq!(status, 401);
}

#[test]
fn home_lists_added_collections() {
    let root = tempfile::tempdir().unwrap();
    let dav = FakeDav::start(root.path());
    assert!(dav.url().starts_with("http://127.0.0.1:"));
    assert!(dav.url().ends_with("/dav/alice/"));
    add_collection(root.path(), "groceries", "Groceries", "#336699");
    let headers = format!("{AUTH}Depth: 0\r\n");
    let (status, body) = request(&dav, "PROPFIND", "/dav/alice/", &headers, "");
    assert_eq!(status, 207);
    assert!(body.contains("current-user-principal"));
    assert!(body.contains("/dav/principals/alice/"));
    let (_, body) = request(&dav, "PROPFIND", "/dav/principals/alice/", &headers, "");
    assert!(body.contains("calendar-home-set"));
    let headers = format!("{AUTH}Depth: 1\r\n");
    let (status, body) = request(&dav, "PROPFIND", "/dav/alice/", &headers, "");
    assert_eq!(status, 207);
    assert!(body.contains("/dav/alice/groceries/"));
    assert!(body.contains("<c:calendar/>"));
}

#[test]
fn mkcalendar_then_propfind_reads_name_and_color() {
    let root = tempfile::tempdir().unwrap();
    let dav = FakeDav::start(root.path());
    let make = "<c:mkcalendar xmlns:c=\"urn:ietf:params:xml:ns:caldav\" xmlns:d=\"DAV:\" \
        xmlns:a=\"http://apple.com/ns/ical/\"><d:set><d:prop><d:displayname>Errands &amp; more\
        </d:displayname><a:calendar-color>#112233</a:calendar-color></d:prop></d:set></c:mkcalendar>";
    let (status, _) = request(&dav, "MKCALENDAR", "/dav/alice/errands/", AUTH, make);
    assert_eq!(status, 201);
    let (status, _) = request(&dav, "MKCALENDAR", "/dav/alice/errands/", AUTH, make);
    assert_eq!(status, 405);
    let headers = format!("{AUTH}Depth: 0\r\n");
    let (status, body) = request(
        &dav,
        "PROPFIND",
        "/dav/alice/errands/",
        &headers,
        "<d:propfind xmlns:d=\"DAV:\"><d:prop><d:displayname/></d:prop></d:propfind>",
    );
    assert_eq!(status, 207);
    assert!(body.contains("Errands &amp; more"));
    assert!(body.contains("#112233"));
    assert!(body.contains("VTODO"));
    let patch = "<d:propertyupdate xmlns:d=\"DAV:\"><d:set><d:prop><d:displayname>Renamed\
        </d:displayname></d:prop></d:set></d:propertyupdate>";
    let (status, _) = request(&dav, "PROPPATCH", "/dav/alice/errands/", AUTH, patch);
    assert_eq!(status, 207);
    let found = collections(root.path());
    assert_eq!(
        found["errands"],
        (Some("Renamed".to_string()), Some("#112233".to_string()))
    );
}

#[test]
fn delete_removes_the_collection() {
    let root = tempfile::tempdir().unwrap();
    let dav = FakeDav::start(root.path());
    add_collection(root.path(), "gone", "Gone", "#000000");
    let (status, _) = request(&dav, "DELETE", "/dav/alice/gone/", AUTH, "");
    assert_eq!(status, 204);
    assert!(collections(root.path()).is_empty());
    let headers = format!("{AUTH}Depth: 0\r\n");
    let (status, _) = request(&dav, "PROPFIND", "/dav/alice/gone/", &headers, "");
    assert_eq!(status, 404);
    let (status, _) = request(&dav, "DELETE", "/dav/alice/gone/", AUTH, "");
    assert_eq!(status, 404);
}

#[test]
fn calls_are_logged_in_order() {
    let root = tempfile::tempdir().unwrap();
    let dav = FakeDav::start(root.path());
    let headers = format!("{AUTH}Depth: 0\r\n");
    request(&dav, "PROPFIND", "/dav/alice/", &headers, "");
    request(&dav, "MKCALENDAR", "/dav/alice/x/", AUTH, "");
    request(&dav, "DELETE", "/dav/alice/x/", AUTH, "");
    let script = fake_vdirsyncer(root.path());
    let vdir = tempfile::tempdir().unwrap();
    for sub in ["sync", "metasync"] {
        let out = Command::new(&script)
            .arg(sub)
            .env("TWYDO_SYNC_VDIR", vdir.path())
            .env("VDIRSYNCER_CONFIG", root.path().join("config"))
            .output()
            .unwrap();
        assert!(out.status.success());
    }
    assert_eq!(
        calls(root.path()),
        ["PROPFIND", "MKCALENDAR", "DELETE", "SYNC", "METASYNC"]
    );
}

fn discover(root: &Path, vdir: &Path) -> std::process::Output {
    Command::new(fake_vdirsyncer(root))
        .args(["discover", "twydo"])
        .env("TWYDO_SYNC_VDIR", vdir)
        .env("VDIRSYNCER_CONFIG", root.join("cfg/config"))
        .output()
        .unwrap()
}

#[test]
fn fake_vdirsyncer_discover_fails_on_a_one_sided_list() {
    let root = tempfile::tempdir().unwrap();
    let vdir = tempfile::tempdir().unwrap();
    add_collection(root.path(), "both", "Both", "#111111");
    std::fs::create_dir(vdir.path().join("both")).unwrap();
    std::fs::create_dir(vdir.path().join(".hidden")).unwrap();
    let out = discover(root.path(), vdir.path());
    assert!(out.status.success(), "{out:?}");
    assert!(root.path().join("cfg/status/twydo.collections").exists());

    // Local only.
    std::fs::create_dir(vdir.path().join("local-only")).unwrap();
    let out = discover(root.path(), vdir.path());
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("error: Unable to find or create collection")
    );
    std::fs::remove_dir(vdir.path().join("local-only")).unwrap();

    // Server only.
    add_collection(root.path(), "remote-only", "Remote", "#222222");
    let out = discover(root.path(), vdir.path());
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Unable to find or create collection"));
    assert_eq!(
        calls(root.path())
            .iter()
            .filter(|c| *c == "DISCOVER")
            .count(),
        3
    );
}
