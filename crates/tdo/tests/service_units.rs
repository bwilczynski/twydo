//! The contrib reference units are exactly what `tdo daemon install` renders.

use std::path::{Path, PathBuf};

use tdo::service::{Platform, Spec, Unit, render};

fn contrib(name: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../contrib")
        .join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn contrib_units_match_what_install_writes() {
    let spec = |socket: &str, log: &str, path: &str| Spec {
        exe: PathBuf::from("/home/you/.local/share/mise/shims/tdo"),
        socket: PathBuf::from(socket),
        default_socket: true,
        vdir: None,
        env: vec![("PATH".into(), path.into())],
        log: PathBuf::from(log),
    };
    let systemd = Unit {
        platform: Platform::Systemd,
        name: "twydo.service".into(),
        file: PathBuf::from("twydo.service"),
        socket: PathBuf::from("/run/user/1000/twydo/tdo.sock"),
        default_socket: true,
    };
    let s = spec(
        "/run/user/1000/twydo/tdo.sock",
        "/home/you/.local/state/twydo/daemon.log",
        "/home/you/.local/share/mise/shims:/usr/local/bin:/usr/bin:/bin",
    );
    assert_eq!(render(&systemd, &s), contrib("systemd/twydo.service"));

    let launchd = Unit {
        platform: Platform::Launchd,
        name: "app.twydo.daemon".into(),
        file: PathBuf::from("app.twydo.daemon.plist"),
        socket: PathBuf::from("/Users/you/Library/Caches/twydo/tdo.sock"),
        default_socket: true,
    };
    let l = spec(
        "/Users/you/Library/Caches/twydo/tdo.sock",
        "/Users/you/.local/state/twydo/daemon.log",
        "/home/you/.local/share/mise/shims:/usr/local/bin:/usr/bin:/bin",
    );
    assert_eq!(
        render(&launchd, &l),
        contrib("launchd/app.twydo.daemon.plist")
    );
}
