use std::fs;
use twydo_core::config::{Config, SyncConfig};
use twydo_core::lists::helper::{HelperRequest, invoke};

fn config(root: &std::path::Path) -> Config {
    let mut cfg = Config::with_vdir(root.to_path_buf());
    cfg.state_dir = Some(root.join("state"));
    cfg.sync = Some(SyncConfig::default());
    cfg
}

#[test]
fn bundled_helper_runs_outside_the_repository_and_validates_protocol() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let mut request = HelperRequest::probe(&cfg).unwrap();
    request.version = 77;
    let message = invoke(&cfg, &request).unwrap_err().to_string();
    assert!(message.contains("protocol"), "{message}");
}

#[cfg(unix)]
fn executable(root: &std::path::Path, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    use std::time::{Duration, Instant};
    let path = root.join("python-wrapper");
    fs::write(
        &path,
        format!("#!/bin/sh\n[ \"$1\" = __probe ] && exit 0\n{body}\n"),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    // A parallel test's fork can inherit the write descriptor until it execs.
    // Match the side-effect-free readiness probe used by the sync test fixtures.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match Command::new(&path).arg("__probe").status() {
            Ok(status) => {
                assert!(status.success(), "wrapper readiness probe failed");
                return path.to_str().unwrap().to_string();
            }
            Err(error) if error.raw_os_error() == Some(26) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("cannot execute helper test wrapper: {error}"),
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn wrapper_fixture_waits_until_new_scripts_are_executable() {
    use std::process::Command;
    use std::time::Duration;
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("python-wrapper");
    let wrapper = executable(d.path(), "echo wrapper-body");
    // Model the writable descriptor inherited by another thread's fork. Linux
    // refuses exec until every copy of that descriptor is closed.
    let writer = fs::OpenOptions::new().write(true).open(&path).unwrap();
    let error = Command::new(&wrapper).output().unwrap_err();
    assert_eq!(error.raw_os_error(), Some(26));
    let closer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        drop(writer);
    });
    let wrapper = executable(d.path(), "echo wrapper-body");
    let result = Command::new(wrapper).arg("__probe").output();
    closer.join().unwrap();
    let output = result.expect("fixture returned while its script was still busy");
    assert!(output.status.success());
    assert!(
        output.stdout.is_empty(),
        "probe must not run the wrapper body"
    );
}

#[cfg(unix)]
#[test]
fn invalid_helper_output_does_not_leak_raw_output_or_stderr() {
    let d = tempfile::tempdir().unwrap();
    let mut cfg = config(d.path());
    cfg.sync.as_mut().unwrap().list_python = executable(
        d.path(),
        "cat >/dev/null\necho private-account-secret\necho private-secret >&2",
    );
    let message = invoke(&cfg, &HelperRequest::probe(&cfg).unwrap())
        .unwrap_err()
        .to_string();
    assert!(!message.contains("private"), "{message}");
    assert!(message.contains("helper"), "{message}");
}

#[cfg(unix)]
#[test]
fn hung_helper_is_killed_with_a_bounded_timeout() {
    let d = tempfile::tempdir().unwrap();
    let mut cfg = config(d.path());
    let sync = cfg.sync.as_mut().unwrap();
    sync.list_python = executable(d.path(), "cat >/dev/null\nsleep 60");
    sync.timeout_secs = 1;
    let start = std::time::Instant::now();
    let result = invoke(&cfg, &HelperRequest::probe(&cfg).unwrap())
        .unwrap_err()
        .to_string();
    assert!(result.contains("timed out"), "{result}");
    assert!(start.elapsed().as_secs() < 5);
}

#[cfg(unix)]
#[test]
fn helper_preserves_caller_directory_for_relative_vdirsyncer_settings() {
    let d = tempfile::tempdir().unwrap();
    let mut cfg = config(d.path());
    cfg.sync.as_mut().unwrap().list_python = executable(
        d.path(),
        r#"cat >/dev/null
python3 -c 'import os,json; print(json.dumps({"version":1,"ok":True,"result":{"cwd":os.getcwd()}}))'"#,
    );
    let result = invoke(&cfg, &HelperRequest::probe(&cfg).unwrap()).unwrap();
    assert_eq!(
        result.result["cwd"].as_str(),
        std::env::current_dir().unwrap().to_str()
    );
}
