//! The Rust CalDAV calls against the fake server. Names and addresses are synthetic.

use std::path::Path;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use twydo_core::config::SyncConfig;
use twydo_core::lists::call::{Refused, wording};
use twydo_core::lists::dav::{Changes, Dav, Properties, canonical_color, connect, validate_target};
use twydo_testdav::{FakeDav, add_collection, add_resource, calls, collections, flag, flag_value};

struct Setup {
    root: TempDir,
    server: FakeDav,
}

fn setup() -> Setup {
    let root = tempfile::tempdir().unwrap();
    let server = FakeDav::start(root.path());
    Setup { root, server }
}

fn sync(url: String, password_command: &[&str]) -> SyncConfig {
    SyncConfig {
        url,
        username: "alice".into(),
        password_command: password_command.iter().map(|s| s.to_string()).collect(),
        ..SyncConfig::default()
    }
}

fn client(s: &Setup) -> Dav {
    let config = sync(s.server.url(), &["printf", "secret"]);
    connect(&config, Instant::now() + Duration::from_secs(60)).expect("connect")
}

fn url(s: &Setup, id: &str) -> String {
    format!("{}{id}/", s.server.url())
}

fn root(s: &Setup) -> &Path {
    s.root.path()
}

fn count(s: &Setup, method: &str) -> usize {
    calls(root(s)).iter().filter(|c| *c == method).count()
}

fn seeded(s: &Setup) {
    add_collection(root(s), "work", "Work", "#112233");
    add_resource(
        root(s),
        "work",
        "a.ics",
        "\"1\"",
        "BEGIN:VCALENDAR\nEND:VCALENDAR\n",
    );
    add_resource(
        root(s),
        "work",
        "b.ics",
        "\"2\"",
        "second synthetic resource",
    );
}

fn props(name: &str, color: &str) -> Properties {
    Properties {
        name: Some(name.into()),
        color: Some(color.into()),
    }
}

#[test]
fn reads_name_and_color() {
    let s = setup();
    add_collection(root(&s), "work", "Work", "#112233");
    let dav = client(&s);
    let list = dav.collection(&url(&s, "work"), "work").unwrap();
    assert_eq!(list.read_properties().unwrap(), props("Work", "#112233"));
}

#[test]
fn create_sends_if_none_match_and_reads_back() {
    let s = setup();
    let dav = client(&s);
    let list = dav.collection(&url(&s, "fresh"), "fresh").unwrap();
    assert!(list.absent().unwrap());
    let made = list.create("Fresh & <new>", "#abcdef").unwrap();
    assert_eq!(made, props("Fresh & <new>", "#abcdef"));
    assert_eq!(count(&s, "MKCALENDAR"), 1);
    let all = collections(root(&s));
    assert_eq!(all["fresh"].0.as_deref(), Some("Fresh & <new>"));
}

#[test]
fn create_refuses_an_existing_collection() {
    let s = setup();
    add_collection(root(&s), "work", "Work", "#112233");
    let dav = client(&s);
    let list = dav.collection(&url(&s, "work"), "work").unwrap();
    let refused = list.create("Other", "#000000").unwrap_err();
    assert_eq!(refused.code, "exists");
    assert!(!refused.sent);
    assert_eq!(count(&s, "MKCALENDAR"), 0);
}

#[test]
fn write_metadata_refuses_a_stale_expectation() {
    let s = setup();
    add_collection(root(&s), "work", "Work", "#112233");
    let dav = client(&s);
    let list = dav.collection(&url(&s, "work"), "work").unwrap();
    let changes = Changes {
        name: Some("Renamed".into()),
        color: None,
    };
    let refused = list
        .write_metadata(&changes, &props("Not the name", "#112233"))
        .unwrap_err();
    assert_eq!(refused.code, "stale");
    assert!(!refused.sent);
    assert_eq!(count(&s, "PROPPATCH"), 0);
    // A matching expectation writes, and a colour compares without case or alpha.
    let changes = Changes {
        name: Some("Renamed".into()),
        color: Some("#445566".into()),
    };
    let after = list
        .write_metadata(&changes, &props("Work", "#112233FF"))
        .unwrap();
    assert_eq!(after, props("Renamed", "#445566"));
    assert_eq!(count(&s, "PROPPATCH"), 1);
}

#[test]
fn ignored_write_is_a_dav_refusal() {
    let s = setup();
    add_collection(root(&s), "work", "Work", "#112233");
    flag(root(&s), "ignore_write");
    let dav = client(&s);
    let list = dav.collection(&url(&s, "work"), "work").unwrap();
    let changes = Changes {
        name: Some("Renamed".into()),
        color: None,
    };
    let refused = list
        .write_metadata(&changes, &props("Work", "#112233"))
        .unwrap_err();
    assert_eq!(refused.code, "dav");
    assert!(refused.sent);
}

#[test]
fn capture_matches_what_delete_revalidates() {
    let s = setup();
    seeded(&s);
    let dav = client(&s);
    let list = dav.collection(&url(&s, "work"), "work").unwrap();
    let backup = list.capture().unwrap();
    assert_eq!(backup.resources.len(), 2);
    let json = serde_json::to_value(&backup).unwrap();
    assert_eq!(json["format"], 1);
    assert_eq!(json["resources"][0]["etag"], "\"1\"");
    assert_eq!(
        json["resources"][0]["bytes"],
        "QkVHSU46VkNBTEVOREFSCkVORDpWQ0FMRU5EQVIK"
    );
    assert_eq!(json["resources"][0]["sha256"].as_str().unwrap().len(), 64);
    list.delete(&backup).unwrap();
    assert!(collections(root(&s)).is_empty());
    assert_eq!(count(&s, "DELETE"), 1);
}

#[test]
fn delete_refuses_a_changed_inventory() {
    let s = setup();
    seeded(&s);
    let dav = client(&s);
    let list = dav.collection(&url(&s, "work"), "work").unwrap();
    let backup = list.capture().unwrap();
    flag(root(&s), "mutate_before_delete");
    let refused = list.delete(&backup).unwrap_err();
    assert_eq!(refused.code, "changed");
    assert!(!refused.sent);
    assert_eq!(count(&s, "DELETE"), 0);
}

#[test]
fn inventory_faults_are_refused() {
    // A listing that cannot be verified is refused while capturing.
    for (fault, code) in [
        ("failed", "inventory"),
        ("foreign", "inventory"),
        ("nested", "inventory"),
        ("missing", "inventory"),
        ("duplicate", "inventory"),
        ("weak", "inventory"),
        ("properties", "inventory"),
        ("get-stale", "inventory"),
        // The second listing differs from the first.
        ("changed", "stale"),
    ] {
        let s = setup();
        seeded(&s);
        flag_value(root(&s), "inventory_fault", fault);
        let dav = client(&s);
        let list = dav.collection(&url(&s, "work"), "work").unwrap();
        let refused = list.capture().unwrap_err();
        assert_eq!(refused.code, code, "{fault}");
    }
    // A consistent difference is a capture that no longer matches the backup: the delete
    // is refused before it is sent.
    for fault in [
        "extra",
        "partial",
        "etag-changed",
        "bytes-changed",
        "props-changed",
    ] {
        let s = setup();
        seeded(&s);
        let dav = client(&s);
        let list = dav.collection(&url(&s, "work"), "work").unwrap();
        let backup = list.capture().unwrap();
        flag_value(root(&s), "inventory_fault", fault);
        let refused = list.delete(&backup).unwrap_err();
        assert_eq!((refused.code, refused.sent), ("changed", false), "{fault}");
        assert_eq!(count(&s, "DELETE"), 0, "{fault}");
    }
    // A collection without an etag of its own is allowed.
    let s = setup();
    seeded(&s);
    flag_value(root(&s), "inventory_fault", "collection-etag-absent");
    let dav = client(&s);
    let list = dav.collection(&url(&s, "work"), "work").unwrap();
    assert_eq!(list.capture().unwrap().resources.len(), 2);
}

#[test]
fn reconcile_delete_reports_present_partial_and_absent() {
    let s = setup();
    seeded(&s);
    let dav = client(&s);
    let list = dav.collection(&url(&s, "work"), "work").unwrap();
    let backup = list.capture().unwrap();
    assert_eq!(list.reconcile_delete(&backup).unwrap_err().code, "present");
    // A DELETE that dies halfway leaves some resources behind.
    flag(root(&s), "partial_delete_once");
    assert_eq!(list.delete(&backup).unwrap_err().code, "dav");
    assert_eq!(list.reconcile_delete(&backup).unwrap_err().code, "partial");
    // Anything else that differs is a change.
    add_resource(root(&s), "work", "c.ics", "\"3\"", "an unrelated resource");
    assert_eq!(list.reconcile_delete(&backup).unwrap_err().code, "changed");
    // And once the list is gone, reconciling succeeds.
    let gone = {
        let fresh = setup();
        seeded(&fresh);
        let dav = client(&fresh);
        let list = dav.collection(&url(&fresh, "work"), "work").unwrap();
        let backup = list.capture().unwrap();
        list.delete(&backup).unwrap();
        list.reconcile_delete(&backup)
    };
    assert!(gone.is_ok());
    assert_eq!(count(&s, "DELETE"), 1);
}

#[test]
fn list_home_finds_collections_through_the_principal() {
    let s = setup();
    add_collection(root(&s), "work", "Work", "#112233");
    add_collection(root(&s), "home", "Home", "#445566");
    let dav = client(&s);
    let mut found = dav.list_home().unwrap();
    found.sort_by(|a, b| a.id.cmp(&b.id));
    let ids: Vec<&str> = found.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(ids, ["home", "work"]);
    assert_eq!(found[1].url, url(&s, "work"));
    assert!(found[1].url.starts_with("http://127.0.0.1:"));
    assert!(found[1].url.ends_with("/dav/alice/work/"));
    assert!(dav.home().unwrap().ends_with("/dav/alice/"));
    // The listed URL passes the target check.
    dav.collection(&found[1].url, "work").unwrap();
}

#[test]
fn a_redirect_is_refused_and_not_followed() {
    let s = setup();
    add_collection(root(&s), "work", "Work", "#112233");
    flag(root(&s), "redirect");
    let dav = client(&s);
    let list = dav.collection(&url(&s, "work"), "work").unwrap();
    assert_eq!(list.read_properties().unwrap_err().code, "dav");
    assert_eq!(dav.list_home().unwrap_err().code, "dav");
    // Only the requests to the server itself were made.
    assert!(calls(root(&s)).iter().all(|c| c == "PROPFIND"));
}

#[test]
fn an_oversized_body_is_refused() {
    let s = setup();
    add_collection(root(&s), "work", "Work", "#112233");
    flag(root(&s), "oversize");
    let dav = client(&s);
    let list = dav.collection(&url(&s, "work"), "work").unwrap();
    assert_eq!(list.read_properties().unwrap_err().code, "dav");
}

#[test]
fn a_failing_password_command_is_a_credentials_refusal() {
    let s = setup();
    let deadline = Instant::now() + Duration::from_secs(60);
    for command in [
        &["false"][..],
        &["printf", ""][..],
        &["printf", "\n"][..],
        &[][..],
    ] {
        let config = sync(s.server.url(), command);
        let refused = connect(&config, deadline).err().expect("refused");
        assert_eq!(refused.code, "credentials", "{command:?}");
    }
    assert!(calls(root(&s)).is_empty());
}

#[test]
fn a_password_command_that_leaves_a_child_running_still_times_out() {
    let s = setup();
    let config = sync(s.server.url(), &["sh", "-c", "sleep 30 & sleep 30"]);
    let started = Instant::now();
    let refused = connect(&config, started + Duration::from_secs(1))
        .err()
        .expect("refused");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(
        ["credentials", "timeout"].contains(&refused.code),
        "{}",
        refused.code
    );
    // A command that exits but leaves a writer behind does not hang either.
    let config = sync(s.server.url(), &["sh", "-c", "sleep 30 & printf secret"]);
    let started = Instant::now();
    let dav = connect(&config, started + Duration::from_secs(10)).expect("connect");
    assert!(started.elapsed() < Duration::from_secs(5));
    drop(dav);
}

#[test]
fn a_trailing_newline_is_stripped_and_oversized_output_is_refused() {
    let s = setup();
    add_collection(root(&s), "work", "Work", "#112233");
    let config = sync(s.server.url(), &["printf", "secret\n"]);
    let dav = connect(&config, Instant::now() + Duration::from_secs(60)).unwrap();
    let list = dav.collection(&url(&s, "work"), "work").unwrap();
    assert!(list.read_properties().is_ok());
    let config = sync(
        s.server.url(),
        &["sh", "-c", "head -c 70000 /dev/zero | tr '\\0' x"],
    );
    let refused = connect(&config, Instant::now() + Duration::from_secs(10))
        .err()
        .expect("refused");
    assert_eq!(refused.code, "credentials");
}

#[test]
fn a_wrong_password_is_a_dav_refusal() {
    let s = setup();
    add_collection(root(&s), "work", "Work", "#112233");
    let config = sync(s.server.url(), &["printf", "wrong\r\n"]);
    let dav = connect(&config, Instant::now() + Duration::from_secs(60)).unwrap();
    let list = dav.collection(&url(&s, "work"), "work").unwrap();
    assert_eq!(list.read_properties().unwrap_err().code, "dav");
}

#[test]
fn colors_compare_case_insensitively_and_ignore_alpha() {
    assert_eq!(canonical_color("#abcdef").as_deref(), Some("#ABCDEF"));
    assert_eq!(canonical_color("#abcdef80").as_deref(), Some("#ABCDEF"));
    assert_eq!(canonical_color("#ABCDEF"), canonical_color("#abcdefFF"));
    assert_eq!(canonical_color("abcdef"), None);
    assert_eq!(canonical_color("#abcde"), None);
    assert_eq!(canonical_color("#abcdeg"), None);
    assert_eq!(canonical_color("#abcdef8"), None);
}

#[test]
fn targets_outside_the_origin_or_with_dot_segments_are_refused() {
    let base = "https://dav.example.org/dav/alice/";
    let ok = |target: &str| validate_target(base, target, "work");
    assert!(ok("https://dav.example.org/dav/alice/work/").is_ok());
    assert!(ok("https://dav.example.org/dav/alice/work").is_ok());
    assert!(ok("https://dav.example.org/dav/alice/work%20x/").is_err());
    for bad in [
        "http://dav.example.org/dav/alice/work/",
        "https://other.example.org/dav/alice/work/",
        "https://dav.example.org:8443/dav/alice/work/",
        "https://bob@dav.example.org/dav/alice/work/",
        "https://dav.example.org/dav/alice/work/?x=1",
        "https://dav.example.org/dav/alice/work/#x",
        "https://dav.example.org/dav/alice/../work/",
        "https://dav.example.org/dav/alice/./work/",
        "https://dav.example.org/dav/alice/%2e%2e/work/",
        "https://dav.example.org/dav/alice%2fx/work/",
        "https://dav.example.org/work/",
        "https://dav.example.org/dav/alice/other/",
        "https://dav.example.org/dav/alice/work/inner/",
    ] {
        assert_eq!(ok(bad).unwrap_err().code, "mapping", "{bad}");
    }
    for id in ["", ".hidden", "a/b", "a%b", "a:b", "a\\b"] {
        assert_eq!(
            validate_target(base, "https://dav.example.org/dav/alice/work/", id)
                .unwrap_err()
                .code,
            "mapping",
            "{id:?}"
        );
    }
    assert!(
        validate_target(
            "ftp://dav.example.org/a/",
            "ftp://dav.example.org/a/work/",
            "work"
        )
        .is_err()
    );
}

#[test]
fn error_text_never_contains_the_url() {
    for code in [
        "dav",
        "inventory",
        "stale",
        "exists",
        "changed",
        "present",
        "partial",
        "connect",
        "mapping",
        "credentials",
        "operation",
        "timeout",
        "upgrade",
        "discovery",
        "vdirsyncer",
        "protocol",
        "anything else",
    ] {
        assert!(!wording(code).contains("http"), "{code}");
        assert!(!wording(code).is_empty());
    }
    assert_eq!(
        wording("credentials"),
        "collection credential fetch failed; check [sync].password_command"
    );
    let shown = Refused::new("dav").to_string();
    assert!(!shown.contains("127.0.0.1"));
}

#[test]
fn a_failed_probe_before_create_is_not_sent() {
    let s = setup();
    flag(root(&s), "fail_propfind_once");
    let dav = client(&s);
    let list = dav.collection(&url(&s, "fresh"), "fresh").unwrap();
    let refused = list.create("Fresh", "#aabbcc").unwrap_err();
    assert!(!refused.sent);
    assert!(!calls(root(&s)).iter().any(|c| c == "MKCALENDAR"));
    assert!(!collections(root(&s)).contains_key("fresh"));
}

#[test]
fn a_refused_connection_is_a_connect_failure_that_sent_nothing() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let base = format!("http://127.0.0.1:{port}/dav/alice/");
    let config = sync(base.clone(), &["printf", "secret"]);
    let dav = connect(&config, Instant::now() + Duration::from_secs(20)).unwrap();
    let list = dav.collection(&format!("{base}work/"), "work").unwrap();
    let refused = list.read_properties().unwrap_err();
    assert_eq!(refused.code, "connect");
    assert!(!refused.sent);
    assert!(!refused.to_string().contains("127.0.0.1"));
}

/// The principal names a calendar home on another origin: it is refused with `mapping`, and
/// the other server, which would have seen the Authorization header, is never contacted.
#[test]
fn a_calendar_home_on_another_origin_is_refused_without_contacting_it() {
    let s = setup();
    let other_root = tempfile::tempdir().unwrap();
    let other = FakeDav::start(other_root.path());
    flag_value(root(&s), "foreign_home", &other.url());
    let dav = client(&s);
    let refused = dav.list_home().unwrap_err();
    assert_eq!(refused.code, "mapping");
    assert!(calls(other_root.path()).is_empty());
}

/// The server applies the write, then stops accepting connections: the read-back fails with a
/// connect-class error, which after a write must still count as sent.
#[test]
fn a_failed_read_back_after_create_is_sent() {
    let s = setup();
    flag(root(&s), "gone_after_write");
    let dav = client(&s);
    let list = dav.collection(&url(&s, "fresh"), "fresh").unwrap();
    let refused = list.create("Fresh", "#aabbcc").unwrap_err();
    assert_eq!(refused.code, "connect");
    assert!(refused.sent);
    assert!(calls(root(&s)).iter().any(|c| c == "MKCALENDAR"));
    assert!(collections(root(&s)).contains_key("fresh"));
}

#[test]
fn a_failed_read_back_after_a_metadata_write_is_sent() {
    let s = setup();
    add_collection(root(&s), "work", "Work", "#112233");
    flag(root(&s), "gone_after_write");
    let dav = client(&s);
    let list = dav.collection(&url(&s, "work"), "work").unwrap();
    let changes = Changes {
        name: Some("Renamed".into()),
        color: None,
    };
    let refused = list
        .write_metadata(&changes, &props("Work", "#112233"))
        .unwrap_err();
    assert_eq!(refused.code, "connect");
    assert!(refused.sent);
    assert!(calls(root(&s)).iter().any(|c| c == "PROPPATCH"));
    assert_eq!(collections(root(&s))["work"].0.as_deref(), Some("Renamed"));
}

/// DELETE has no read-back; its own failure after the request left is sent.
#[test]
fn a_failed_delete_request_is_sent() {
    let s = setup();
    seeded(&s);
    let dav = client(&s);
    let list = dav.collection(&url(&s, "work"), "work").unwrap();
    let backup = list.capture().unwrap();
    flag(root(&s), "fail_delete_once");
    let refused = list.delete(&backup).unwrap_err();
    assert!(refused.sent);
    assert!(calls(root(&s)).iter().any(|c| c == "DELETE"));
}
