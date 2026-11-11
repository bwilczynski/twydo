//! The TUI end to end, in a pseudo-terminal against a daemon it starts on demand, and a scan
//! that keeps every test on its own socket. Content is synthetic.

mod common;

use std::fs;
use std::io::Read;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::time::{Duration, Instant};

use chrono::Local;
use common::TestEnv;

const BOUND: Duration = Duration::from_secs(30);

/// A pseudo-terminal pair: `(master, slave)`, 40 rows by 120 columns.
fn open_pty() -> (OwnedFd, OwnedFd) {
    let (mut m, mut s) = (0, 0);
    let size = libc::winsize {
        ws_row: 40,
        ws_col: 120,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: openpty fills two descriptors we then own.
    let rc = unsafe {
        libc::openpty(
            &mut m,
            &mut s,
            std::ptr::null_mut(),
            std::ptr::null(),
            &size,
        )
    };
    assert_eq!(rc, 0, "openpty");
    // SAFETY: both descriptors are fresh and owned by nobody else.
    unsafe { (OwnedFd::from_raw_fd(m), OwnedFd::from_raw_fd(s)) }
}

fn task_file(env: &TestEnv) -> std::path::PathBuf {
    env.vdir().join("inbox/uid-a.ics")
}

#[test]
fn tui_end_to_end_against_a_real_daemon() {
    let env = TestEnv::new("");
    let today = Local::now().format("%Y%m%d").to_string();
    fs::write(
        task_file(&env),
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\n\
             UID:uid-a\r\nSUMMARY:Synthetic\r\nDUE;VALUE=DATE:{today}\r\nEND:VTODO\r\n\
             END:VCALENDAR\r\n"
        ),
    )
    .unwrap();

    let (master, slave) = open_pty();
    let mut cmd = env.tdo_cmd();
    cmd.stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave))
        .env("TERM", "xterm-256color");
    // SAFETY: only async-signal-safe calls (setsid, ioctl) between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            libc::ioctl(0, libc::TIOCSCTTY as _, 0);
            Ok(())
        });
    }
    let mut child = common::Proc(cmd.spawn().unwrap());

    let out = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let reader_out = out.clone();
    let mut reader = fs::File::from(master.try_clone().unwrap());
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            reader_out.lock().unwrap().extend_from_slice(&buf[..n]);
        }
    });
    let screen = || String::from_utf8_lossy(&out.lock().unwrap()).into_owned();
    let send = |bytes: &[u8]| {
        // SAFETY: writes to the master descriptor we own.
        let n = unsafe {
            libc::write(
                std::os::fd::AsRawFd::as_raw_fd(&master),
                bytes.as_ptr().cast(),
                bytes.len(),
            )
        };
        assert_eq!(n as usize, bytes.len());
    };

    let end = Instant::now() + BOUND;
    while !screen().contains("Synthetic") {
        assert!(
            Instant::now() < end,
            "no task row on screen: {:?}",
            screen()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    send(b"x");
    let end = Instant::now() + BOUND;
    while !fs::read_to_string(task_file(&env))
        .unwrap()
        .contains("STATUS:COMPLETED")
    {
        assert!(Instant::now() < end, "never completed: {:?}", screen());
        std::thread::sleep(Duration::from_millis(50));
    }
    send(b":q\r");
    let end = Instant::now() + BOUND;
    let status = loop {
        if let Some(s) = child.0.try_wait().unwrap() {
            break s;
        }
        assert!(Instant::now() < end, "tdo did not quit: {:?}", screen());
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success(), "{status:?}");

    // The TUI started the daemon on demand; stop it, and check that none still names this
    // test's socket.
    let sock = env.socket().display().to_string();
    assert_eq!(daemons_naming(&sock), 1, "the TUI should have started one");
    let stop = env.quiet_cmd().args(["daemon", "stop"]).status().unwrap();
    assert_eq!(stop.code(), Some(0));
    let end = Instant::now() + Duration::from_secs(10);
    while daemons_naming(&sock) > 0 {
        assert!(Instant::now() < end, "a daemon still runs on {sock}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// How many running `tdo daemon` processes were started with this socket in their
/// environment (the daemon is given its socket by `TWYDO_SOCKET`, not on its command line).
fn daemons_naming(socket: &str) -> usize {
    let want = format!("TWYDO_SOCKET={socket}");
    let mut n = 0;
    for e in fs::read_dir("/proc").unwrap().flatten() {
        let (Ok(cmdline), Ok(environ)) = (
            fs::read(e.path().join("cmdline")),
            fs::read(e.path().join("environ")),
        ) else {
            continue;
        };
        let args: Vec<&[u8]> = cmdline.split(|b| *b == 0).collect();
        let is_daemon = args.iter().any(|a| *a == b"daemon");
        let has_sock = environ.split(|b| *b == 0).any(|v| v == want.as_bytes());
        if is_daemon && has_sock {
            n += 1;
        }
    }
    n
}

#[test]
fn no_test_spawns_tdo_without_its_own_socket() {
    fn rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                rs_files(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let needle = ["CARGO_BIN_EXE", "_tdo"].concat();
    let mut files = Vec::new();
    rs_files(&dir, &mut files);
    let mut offenders = Vec::new();
    for p in files {
        let rel = p.strip_prefix(&dir).unwrap().to_string_lossy().into_owned();
        for (i, line) in fs::read_to_string(&p).unwrap().lines().enumerate() {
            if !line.contains(&needle) {
                continue;
            }
            let allowed = rel == "common/mod.rs"
                // The conflict-hook path in sync_e2e.rs: it runs under that test's own socket
                // and never talks to a daemon.
                || (rel == "sync_e2e.rs" && line.trim() == format!("let hook = env!(\"{needle}\");"));
            if !allowed {
                offenders.push(format!("{rel}:{}", i + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "build tdo commands with tests/common (tdo_cmd or tdo_bin): {offenders:?}"
    );
}
