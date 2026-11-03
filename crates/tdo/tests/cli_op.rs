//! `tdo op` and startup recovery, driven through the built binary. Content is synthetic
//! (phase-0 fixtures from the isolated test account).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const TASK_E: &str = "44123b09407c4ea1adae8f2ecdb64d49";
const TASK_A: &str = "0a04f22aeb294b719633c978fe1975e5";

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new(mode: &str) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let vdir = root.join("vdir");
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/phase0/v1/plan4-list1/02-edit");
        fs::create_dir_all(vdir.join("list1")).unwrap();
        fs::create_dir_all(vdir.join("list2")).unwrap();
        for e in fs::read_dir(&src).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "ics") {
                fs::copy(&p, vdir.join("list1").join(p.file_name().unwrap())).unwrap();
            }
        }
        fs::create_dir_all(root.join("config/twydo")).unwrap();
        fs::write(
            root.join("config/twydo/config.toml"),
            format!(
                "vdir = \"{}\"\ntimezone = \"America/New_York\"\n[writes]\nmode = \"{mode}\"\n",
                vdir.display()
            ),
        )
        .unwrap();
        let mut matrix = String::from("matrix_version = 1\n");
        // every case the tests touch passes the iPhone baseline; the other columns stay
        // unverified, which test mode accepts
        for (case, op) in [
            ("checklist", "complete"),
            ("one-time", "complete"),
            ("monthly", "complete"),
            ("one-time", "create"),
            ("one-time", "edit"),
            ("monthly", "create"),
            ("monthly", "edit"),
            ("date-only", "create"),
            ("date-only", "edit"),
            ("checklist", "create"),
            ("checklist", "edit"),
            ("project", "edit"),
            ("recurring-project", "edit"),
            ("daily", "create"),
            ("task-to-project", "convert"),
        ] {
            matrix.push_str(&format!(
                "[[operation]]\ncase = \"{case}\"\nop = \"{op}\"\niphone_baseline = \"pass\"\n\
                 mac_to_iphone = \"unverified\"\niphone_to_mac = \"unverified\"\n\
                 twydo_to_iphone = \"unverified\"\niphone_to_twydo = \"unverified\"\n"
            ));
        }
        matrix.push_str(
            "[[rule]]\nclass = \"months-day-1-28\"\npreserve = \"pass\"\nexecute = \"pass\"\nedit = \"pass\"\n",
        );
        fs::write(root.join("matrix.toml"), matrix).unwrap();
        Env { dir }
    }

    fn vdir(&self) -> PathBuf {
        self.dir.path().join("vdir")
    }

    fn state(&self) -> PathBuf {
        self.dir.path().join("state")
    }

    /// The per-vdir state dir `tdo` uses for this env's vdir (journal and lock).
    fn vdir_state(&self) -> PathBuf {
        twydo_core::store::vdir_state_dir(&self.state().join("twydo"), &self.vdir()).unwrap()
    }

    fn tdo(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_tdo"))
            .args(args)
            .env("XDG_CONFIG_HOME", self.dir.path().join("config"))
            .env("XDG_STATE_HOME", self.state())
            .output()
            .expect("run tdo")
    }

    fn op(&self, args: &[&str]) -> Output {
        let m = self.dir.path().join("matrix.toml");
        let mut a = vec!["op", "--matrix", m.to_str().unwrap()];
        a.extend_from_slice(args);
        self.tdo(&a)
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn op_id_ok(s: &str) -> bool {
    let b = s.trim().as_bytes();
    b.len() == 25
        && b[..8].iter().all(u8::is_ascii_digit)
        && b[8] == b'T'
        && b[9..15].iter().all(u8::is_ascii_digit)
        && b[15] == b'Z'
        && b[16] == b'-'
        && b[17..]
            .iter()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

#[test]
fn op_blocked_in_verified_mode_exits_1() {
    let env = Env::new("verified");
    let file = env.vdir().join(format!("list1/{TASK_E}.ics"));
    let before = fs::read(&file).unwrap();
    let o = env.op(&["complete", TASK_E]);
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    assert!(err(&o).contains("not verified yet"), "{}", err(&o));
    assert!(err(&o).starts_with("tdo: "));
    assert_eq!(fs::read(&file).unwrap(), before);
}

#[test]
fn op_test_mode_warns() {
    let env = Env::new("test");
    let o = env.op(&["journal"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        err(&o).contains("test write mode: unverified operations are enabled"),
        "{}",
        err(&o)
    );
    let o = Env::new("verified").op(&["journal"]);
    assert!(!err(&o).contains("test write mode"));
}

#[test]
fn op_complete_in_test_mode_prints_op_id() {
    let env = Env::new("test");
    let before = fs::read_dir(env.vdir().join("list1")).unwrap().count();
    let o = env.op(&["complete", TASK_E]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(op_id_ok(&out(&o)), "stdout was {:?}", out(&o));
    let after = fs::read_dir(env.vdir().join("list1")).unwrap().count();
    assert_eq!(after, before + 1, "history copy missing");

    let j = env.op(&["journal"]);
    assert!(j.status.success(), "{}", err(&j));
    let line = out(&j);
    assert!(line.contains(out(&o).trim()), "{line}");
    assert!(line.contains("complete  applied"), "{line}");
    assert!(line.contains(TASK_E), "{line}");
}

#[test]
fn op_check_lists_every_kind() {
    let env = Env::new("test");
    let o = env.op(&["check", TASK_E]);
    assert!(o.status.success(), "{}", err(&o));
    let text = out(&o);
    let kinds: Vec<&str> = text.lines().map(|l| l.split(':').next().unwrap()).collect();
    assert_eq!(
        kinds,
        ["create", "edit", "complete", "reopen", "move", "delete"],
        "{text}"
    );
    assert!(text.contains("complete: ok"), "{text}");
}

#[test]
fn op_journal_and_recover_run_on_empty_state() {
    let env = Env::new("verified");
    let j = env.op(&["journal"]);
    assert!(j.status.success(), "{}", err(&j));
    assert_eq!(out(&j), "");
    let r = env.op(&["recover"]);
    assert!(r.status.success(), "{}", err(&r));
    assert!(out(&r).contains("resumed: 0"), "{}", out(&r));
    assert!(out(&r).contains("stuck: 0"), "{}", out(&r));
}

#[test]
fn op_edit_without_changes_is_usage_error() {
    let env = Env::new("test");
    let o = env.op(&["edit", TASK_A]);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
}

#[test]
fn recover_runs_before_export() {
    let env = Env::new("verified");
    let rel = format!("list1/{TASK_A}.ics");
    let before = fs::read_to_string(env.vdir().join(&rel)).unwrap();
    let title = before
        .lines()
        .find_map(|l| l.strip_prefix("SUMMARY:"))
        .unwrap()
        .to_string();
    let after = before.replacen(&format!("SUMMARY:{title}"), "SUMMARY:Recovered title", 1);
    assert_ne!(before, after);
    let id = "20261101T120000Z-abcdef12";
    let entry = serde_json::json!({
        "format": 1,
        "id": id,
        "created": "2026-11-01T12:00:00Z",
        "kind": "edit",
        "uids": [TASK_A],
        "stage": "prepared",
        "inverse_of": null,
        "writes": [{"path": rel, "before": before, "after": after}],
    });
    let jdir = env.vdir_state().join("journal");
    fs::create_dir_all(&jdir).unwrap();
    fs::write(jdir.join(format!("{id}.json")), entry.to_string()).unwrap();

    let o = env.tdo(&["export-json"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("Recovered title"), "{}", out(&o));
    assert_eq!(fs::read_to_string(env.vdir().join(&rel)).unwrap(), after);
}

/// Readers take the shared vdir lock (spec §6): `export-json` waits while a writer holds the
/// exclusive lock.
#[test]
fn export_waits_for_writer_lock() {
    use std::time::{Duration, Instant};
    let env = Env::new("verified");
    let sdir = env.vdir_state();
    fs::create_dir_all(&sdir).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(sdir.join("vdir.lock"))
        .unwrap();
    lock.lock().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_tdo"))
        .arg("export-json")
        .env("XDG_CONFIG_HOME", env.dir.path().join("config"))
        .env("XDG_STATE_HOME", env.state())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(700) {
        assert!(
            child.try_wait().unwrap().is_none(),
            "export-json finished while the exclusive lock was held"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    lock.unlock().unwrap();
    let status = child.wait().unwrap();
    assert!(status.success());
}

/// A read-only command does not create the state dir.
#[test]
fn export_creates_no_state_dir() {
    let env = Env::new("verified");
    let o = env.tdo(&["export-json"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(!env.state().exists(), "export-json created the state dir");
}

/// A Prepared entry whose `before` no longer matches the disk is stuck: readers warn and
/// still read, writes exit 1 until the entry is abandoned.
#[test]
fn stuck_entry_blocks_writes_until_abandoned() {
    let env = Env::new("test");
    let rel = format!("list1/{TASK_A}.ics");
    let disk = fs::read_to_string(env.vdir().join(&rel)).unwrap();
    let id = "20261101T120000Z-0badf00d";
    let entry = serde_json::json!({
        "format": 1,
        "id": id,
        "created": "2026-11-01T12:00:00Z",
        "kind": "edit",
        "uids": [TASK_A],
        "stage": "prepared",
        "inverse_of": null,
        "writes": [{"path": rel, "before": "neither before", "after": "nor after"}],
    });
    let jdir = env.vdir_state().join("journal");
    fs::create_dir_all(&jdir).unwrap();
    fs::write(jdir.join(format!("{id}.json")), entry.to_string()).unwrap();

    let o = env.tdo(&["export-json"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        err(&o).contains("tdo: journal has unfinished operations; run 'tdo op journal'"),
        "{}",
        err(&o)
    );
    assert!(out(&o).contains(TASK_A), "export missing tasks");
    assert_eq!(fs::read_to_string(env.vdir().join(&rel)).unwrap(), disk);

    let e_file = env.vdir().join(format!("list1/{TASK_E}.ics"));
    let e_before = fs::read(&e_file).unwrap();
    let c = env.op(&["complete", TASK_E]);
    assert_eq!(c.status.code(), Some(1), "{}", err(&c));
    assert!(
        err(&c).contains("journal has unfinished operations"),
        "{}",
        err(&c)
    );
    assert_eq!(fs::read_to_string(env.vdir().join(&rel)).unwrap(), disk);
    assert_eq!(fs::read(&e_file).unwrap(), e_before);

    let a = env.op(&["abandon", id]);
    assert!(a.status.success(), "{}", err(&a));
    assert_eq!(out(&a).trim(), format!("abandoned {id}"));
    let c = env.op(&["complete", TASK_E]);
    assert!(c.status.success(), "{}", err(&c));
    assert!(op_id_ok(&out(&c)), "stdout was {:?}", out(&c));
    assert_ne!(fs::read(&e_file).unwrap(), e_before);
    // The abandoned entry's file is left as it was.
    assert_eq!(fs::read_to_string(env.vdir().join(&rel)).unwrap(), disk);
}

/// `tdo op` loads its snapshot under the shared vdir lock: `op check` waits while a writer
/// holds the exclusive lock, so it never judges a partly applied operation.
#[test]
fn op_snapshot_waits_for_writer_lock() {
    use std::time::{Duration, Instant};
    use twydo_core::store::fsio::VdirLock;
    let env = Env::new("verified");
    let lock = VdirLock::exclusive(&env.vdir_state()).unwrap();
    let m = env.dir.path().join("matrix.toml");
    let mut child = Command::new(env!("CARGO_BIN_EXE_tdo"))
        .args(["op", "--matrix", m.to_str().unwrap(), "check", TASK_E])
        .env("XDG_CONFIG_HOME", env.dir.path().join("config"))
        .env("XDG_STATE_HOME", env.state())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(300) {
        assert!(
            child.try_wait().unwrap().is_none(),
            "op check finished while the exclusive lock was held"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(lock);
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(0));
}

/// A missing vdir is reported as unreadable, not as a journal failure.
#[test]
fn missing_vdir_reports_only_read_error() {
    let env = Env::new("verified");
    let missing = env.dir.path().join("no-such-vdir");
    let o = env.tdo(&["--vdir", missing.to_str().unwrap(), "check"]);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    assert!(err(&o).contains("cannot read vdir"), "{}", err(&o));
    assert!(!err(&o).contains("journal recovery failed"), "{}", err(&o));
}

#[test]
fn op_journal_shows_synced() {
    let env = Env::new("test");
    let id = out(&env.op(&["complete", TASK_E])).trim().to_string();
    let j = out(&env.op(&["journal"]));
    assert!(j.lines().next().unwrap().ends_with("pending"), "{j}");

    let journal = twydo_core::store::journal::Journal::open(&env.vdir_state()).unwrap();
    let at = chrono::DateTime::parse_from_rfc3339("2026-11-02T08:30:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    journal.mark_synced(&id, at).unwrap();
    let j = out(&env.op(&["journal"]));
    assert!(
        j.lines()
            .next()
            .unwrap()
            .ends_with("synced 2026-11-02T08:30:00Z"),
        "{j}"
    );
}

/// Adds a `[sync]` table whose command is a fake script touching `marker`; returns the marker.
fn with_fake_sync(env: &Env, after_edit: bool) -> PathBuf {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let marker = env.dir.path().join("marker");
    let fake = env.dir.path().join("fake.sh");
    fs::write(
        &fake,
        format!(
            "#!/bin/sh\n[ \"$1\" = __probe ] && exit 0\ntouch '{}'\n",
            marker.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    let mut cfg = fs::OpenOptions::new()
        .append(true)
        .open(env.dir.path().join("config/twydo/config.toml"))
        .unwrap();
    write!(
        cfg,
        "[sync]\ncommand = \"{}\"\nmetasync = false\nafter_edit = {after_edit}\n",
        fake.display()
    )
    .unwrap();
    marker
}

fn marker_appears(marker: &Path) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !marker.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    marker.exists()
}

/// True if `marker` stays absent for a grace period.
fn marker_stays_absent(marker: &Path) -> bool {
    std::thread::sleep(std::time::Duration::from_millis(1500));
    !marker.exists()
}

#[test]
fn op_triggers_background_sync() {
    let env = Env::new("test");
    let marker = with_fake_sync(&env, true);
    let o = env.op(&["complete", TASK_E]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(marker_appears(&marker), "no background sync after op edit");
}

#[test]
fn op_check_does_not_trigger_sync() {
    let env = Env::new("test");
    let marker = with_fake_sync(&env, true);
    for args in [&["check", TASK_E][..], &["journal"], &["recover"]] {
        let o = env.op(args);
        assert!(o.status.success(), "{}", err(&o));
    }
    assert!(marker_stays_absent(&marker), "read-only op started a sync");
}

#[test]
fn failed_op_does_not_trigger_sync() {
    let env = Env::new("verified");
    let marker = with_fake_sync(&env, true);
    let o = env.op(&["complete", TASK_E]);
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    assert!(marker_stays_absent(&marker), "blocked op started a sync");
}

#[test]
fn after_edit_false_does_not_trigger() {
    let env = Env::new("test");
    let marker = with_fake_sync(&env, false);
    let o = env.op(&["complete", TASK_E]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        marker_stays_absent(&marker),
        "after_edit = false still synced"
    );
}

#[test]
fn complete_parent_needs_all_flag() {
    let env = Env::new("test");
    let task = |uid: &str, title: &str, extra: &str| {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\n\
             UID:{uid}\r\nSUMMARY:{title}\r\n{extra}STATUS:NEEDS-ACTION\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
        )
    };
    let dir = env.vdir().join("list1");
    fs::write(
        dir.join("fam-p.ics"),
        task(
            "fam-p",
            "Synthetic parent",
            "X-2DOAPP-METADATA:@tasktype:checklist\r\n",
        ),
    )
    .unwrap();
    for n in ["fam-1", "fam-2"] {
        fs::write(
            dir.join(format!("{n}.ics")),
            task(n, "Synthetic item", "RELATED-TO:fam-p\r\n"),
        )
        .unwrap();
    }
    let o = env.op(&["complete", "fam-p"]);
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    assert!(
        err(&o).contains("\"Synthetic parent\" has 2 open items; pass --all to complete them too"),
        "{}",
        err(&o)
    );
    let o = env.op(&["complete", "fam-p", "--all"]);
    assert!(o.status.success(), "{}", err(&o));
}

#[test]
fn complete_parent_with_one_item_uses_the_singular() {
    let env = Env::new("test");
    let dir = env.vdir().join("list1");
    let task = |uid: &str, extra: &str| {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\n\
             UID:{uid}\r\nSUMMARY:Synthetic {uid}\r\n{extra}STATUS:NEEDS-ACTION\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
        )
    };
    fs::write(
        dir.join("one-p.ics"),
        task("one-p", "X-2DOAPP-METADATA:@tasktype:checklist\r\n"),
    )
    .unwrap();
    fs::write(dir.join("one-1.ics"), task("one-1", "RELATED-TO:one-p\r\n")).unwrap();
    let o = env.op(&["complete", "one-p"]);
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    assert!(
        err(&o).contains("has 1 open item; pass --all"),
        "{}",
        err(&o)
    );
}

/// The text of the one vdir file whose summary is `title`.
fn file_titled(env: &Env, title: &str) -> String {
    for e in fs::read_dir(env.vdir().join("list1")).unwrap() {
        let s = fs::read_to_string(e.unwrap().path()).unwrap();
        if s.contains(&format!("SUMMARY:{title}\r\n")) || s.contains(&format!("SUMMARY:{title}\n"))
        {
            return s;
        }
    }
    panic!("no file titled {title}");
}

fn write_checklist(env: &Env) {
    fs::write(
        env.vdir().join("list1/cl-p.ics"),
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\n\
         UID:cl-p\r\nSUMMARY:Synthetic list\r\nX-2DOAPP-METADATA:@tasktype:checklist\r\n\
         STATUS:NEEDS-ACTION\r\nEND:VTODO\r\nEND:VCALENDAR\r\n",
    )
    .unwrap();
}

#[test]
fn add_item_under_checklist() {
    let env = Env::new("test");
    write_checklist(&env);
    let o = env.op(&[
        "add",
        "--list",
        "list1",
        "--title",
        "Synthetic item",
        "--parent",
        "cl-p",
    ]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(file_titled(&env, "Synthetic item").contains("RELATED-TO:cl-p"));
}

#[test]
fn add_with_repeat() {
    let env = Env::new("test");
    let o = env.op(&[
        "add",
        "--list",
        "list1",
        "--title",
        "Synthetic rep",
        "--due",
        "2026-12-05",
        "--repeat",
        "monthly",
    ]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(file_titled(&env, "Synthetic rep").contains("RRULE:FREQ=MONTHLY"));
    // without a due date the task becomes due today, as 2Do does
    let o = env.op(&[
        "add",
        "--list",
        "list1",
        "--title",
        "Synthetic undated",
        "--repeat",
        "monthly",
    ]);
    assert!(o.status.success(), "{}", err(&o));
    let f = file_titled(&env, "Synthetic undated");
    assert!(f.contains("RRULE:FREQ=MONTHLY") && f.contains("DUE"), "{f}");
}

#[test]
fn edit_kind() {
    let env = Env::new("test");
    let o = env.op(&["edit", TASK_E, "--kind", "project"]);
    assert!(o.status.success(), "{}", err(&o));
    let f = fs::read_to_string(env.vdir().join(format!("list1/{TASK_E}.ics")))
        .unwrap()
        .replace("\r\n ", "")
        .replace("\n ", "");
    assert!(f.contains("TaskType%22%3A2"), "{f}");
}

#[test]
fn edit_repeat_bad_text_exits_2() {
    let env = Env::new("test");
    let o = env.op(&["edit", TASK_E, "--repeat", "every blue moon"]);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    assert!(err(&o).starts_with("tdo: "), "{}", err(&o));
    assert!(err(&o).len() > 10, "{}", err(&o));
}
