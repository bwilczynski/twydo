use std::process::Command;

#[test]
fn version_flag_prints_name() {
    let out = Command::new(env!("CARGO_BIN_EXE_tdo"))
        .arg("--version")
        .output()
        .expect("run tdo");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.starts_with("tdo "), "stdout was {stdout:?}");
}

mod gate {
    use std::path::PathBuf;
    use std::process::{Command, Output};

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn sample() -> &'static str {
        "crates/twydo-core/tests/data/sample-vdir"
    }

    fn tdo(args: &[&str]) -> Output {
        tdo_cfg(args, None)
    }

    /// Run tdo with an isolated XDG dir, optionally holding a `twydo/config.toml`.
    fn tdo_cfg(args: &[&str], config: Option<&str>) -> Output {
        let cfg = tempfile::tempdir().unwrap();
        if let Some(text) = config {
            std::fs::create_dir_all(cfg.path().join("twydo")).unwrap();
            std::fs::write(cfg.path().join("twydo/config.toml"), text).unwrap();
        }
        Command::new(env!("CARGO_BIN_EXE_tdo"))
            .args(args)
            .current_dir(root())
            .env("XDG_CONFIG_HOME", cfg.path())
            .env("XDG_STATE_HOME", cfg.path().join("state"))
            .output()
            .expect("run tdo")
    }

    fn stdout(o: &Output) -> String {
        String::from_utf8(o.stdout.clone()).unwrap()
    }

    #[test]
    fn export_json_sample() {
        let o = tdo_cfg(
            &["--vdir", sample(), "export-json"],
            Some("vdir = \"/nonexistent/overridden\"\ntimezone = \"America/New_York\"\n"),
        );
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
        assert_eq!(v["schema"], 1);
        assert_eq!(v["collections"].as_array().unwrap().len(), 3);
        let tasks = v["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 16);
        let rent = tasks.iter().find(|t| t["summary"] == "Pay rent").unwrap();
        assert_eq!(rent["rule_class"], "months-day-1-28");
        assert_eq!(rent["due"], "2026-11-07");
        assert_eq!(rent["due_is_date"], true);
        let water = tasks
            .iter()
            .find(|t| t["summary"] == "Pay water bill")
            .unwrap();
        assert_eq!(water["due"], "2026-11-06T09:00:00-05:00");
        assert_eq!(water["alarms"][0]["relative_seconds"], -900);
        assert_eq!(water["alarms"][0]["related_end"], false);
        let uids: Vec<_> = tasks
            .iter()
            .map(|t| {
                (
                    t["collection"].as_str().unwrap(),
                    t["uid"].as_str().unwrap(),
                )
            })
            .collect();
        let mut sorted = uids.clone();
        sorted.sort();
        assert_eq!(uids, sorted);
    }

    #[test]
    fn check_reports_rules_without_titles() {
        let o = tdo(&[
            "--vdir",
            sample(),
            "check",
            "--matrix",
            "compat/matrix.toml",
        ]);
        assert_eq!(o.status.code(), Some(0));
        let out = stdout(&o);
        // Verdicts come from the live compat/matrix.toml, so check the row's shape, not its values.
        assert!(
            out.lines()
                .any(|l| l.starts_with("months-day-1-28  1 tasks  preserve=")
                    && l.contains(" execute=")
                    && l.contains(" edit=")),
            "{out}"
        );
        assert!(out.contains("parse errors: 0"), "{out}");
        assert!(out.contains("Data health"), "{out}");
        assert!(out.contains("Recurrence evidence"), "{out}");
        assert!(
            out.contains("Operation availability (mode=verified)"),
            "{out}"
        );
        assert!(!out.contains("GATE:"), "{out}");
        assert!(!out.contains("Pay rent"), "{out}");
        let v = tdo(&["--vdir", sample(), "check", "--verbose"]);
        assert!(stdout(&v).contains("Pay rent"));
    }

    #[test]
    fn check_missing_matrix_is_a_configuration_error() {
        let o = tdo(&["--vdir", sample(), "check", "--matrix", "nope.toml"]);
        assert_eq!(o.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&o.stderr).contains("nope.toml"));
    }

    /// An installed `tdo` runs from anywhere: without `--matrix`, `check` uses the matrix built
    /// into the binary, as `tdo op` and the TUI do, not a path relative to the working directory.
    #[test]
    fn check_without_matrix_flag_uses_embedded_matrix() {
        let elsewhere = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        let o = Command::new(env!("CARGO_BIN_EXE_tdo"))
            .args(["--vdir", root().join(sample()).to_str().unwrap(), "check"])
            .current_dir(elsewhere.path())
            .env("XDG_CONFIG_HOME", cfg.path())
            .env("XDG_STATE_HOME", cfg.path().join("state"))
            .output()
            .expect("run tdo");
        let out = stdout(&o);
        assert!(!out.contains("matrix not found"), "{out}");
        assert!(
            out.lines()
                .any(|l| l.starts_with("months-day-1-28  1 tasks  preserve=pass")),
            "{out}"
        );
    }

    #[test]
    fn check_exit_code() {
        let o = tdo(&["--vdir", "/nonexistent/twydo-vdir", "check"]);
        assert_eq!(o.status.code(), Some(2));
        let o = tdo(&["export-json"]);
        assert_eq!(o.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&o.stderr).contains("pass --vdir"));
    }

    #[test]
    fn config_supplies_vdir_and_timezone() {
        let cfg = format!("vdir = \"{}\"\ntimezone = \"Asia/Tokyo\"\n", sample());
        let o = tdo_cfg(&["export-json"], Some(&cfg));
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
        let t = v["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["summary"] == "Pay water bill")
            .unwrap();
        assert_eq!(t["due"], "2026-11-06T09:00:00-05:00");
    }

    #[test]
    fn invalid_config_exits_2() {
        let o = tdo_cfg(&["--vdir", sample(), "check"], Some("bogus = 1\n"));
        assert_eq!(o.status.code(), Some(2));
    }

    fn matrix_file(dir: &std::path::Path, rows: &[(&str, &str)]) -> String {
        let mut s = String::from("matrix_version = 1\n");
        for (class, v) in rows {
            s += &format!(
                "[[rule]]\nclass = \"{class}\"\npreserve = \"{v}\"\nexecute = \"{v}\"\nedit = \"{v}\"\n"
            );
        }
        let p = dir.join("m.toml");
        std::fs::write(&p, s).unwrap();
        p.to_str().unwrap().to_string()
    }

    /// Copy of the sample vdir keeping only the repeating `bills` tasks, plus optional extras.
    fn mini_vdir(dir: &std::path::Path, broken: bool) -> String {
        let v = dir.join("vdir/c");
        std::fs::create_dir_all(&v).unwrap();
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t1\r\nSUMMARY:x\r\nDUE;VALUE=DATE:20261107\r\nRRULE:FREQ=DAILY\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        std::fs::write(v.join("a.ics"), ics).unwrap();
        if broken {
            std::fs::write(v.join("b.ics"), "this is not ical").unwrap();
        }
        dir.join("vdir").to_str().unwrap().to_string()
    }

    #[test]
    fn healthy_data_still_reports_refused_operations() {
        let d = tempfile::tempdir().unwrap();
        let m = matrix_file(d.path(), &[("days", "pass")]);
        let good = mini_vdir(d.path(), false);
        let out = stdout(&tdo(&["--vdir", &good, "check", "--matrix", &m]));
        assert!(!out.contains("GATE:"), "{out}");
        assert!(out.contains("delete: allowed=0 refused=1"), "{out}");

        let d2 = tempfile::tempdir().unwrap();
        let m2 = matrix_file(d2.path(), &[("days", "pass")]);
        let bad = mini_vdir(d2.path(), true);
        let out = stdout(&tdo(&["--vdir", &bad, "check", "--matrix", &m2]));
        assert!(out.contains("parse errors: 1"), "{out}");
        assert!(
            out.contains("parse errors: 1") && out.contains("edit title: allowed=0 refused=1"),
            "{out}"
        );
    }

    #[test]
    fn unverified_modifier_blocks_class() {
        let d = tempfile::tempdir().unwrap();
        let v = d.path().join("vdir/c");
        std::fs::create_dir_all(&v).unwrap();
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t1\r\nSUMMARY:x\r\nDUE;VALUE=DATE:20261107\r\nRRULE:FREQ=DAILY;COUNT=3\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        std::fs::write(v.join("a.ics"), ics).unwrap();
        let vd = d.path().join("vdir");
        let vd = vd.to_str().unwrap();
        let m = matrix_file(d.path(), &[("days", "pass"), ("mod:count", "unverified")]);
        let out = stdout(&tdo(&["--vdir", vd, "check", "--matrix", &m]));
        assert!(
            out.contains("days+count  1 tasks  preserve=unverified"),
            "{out}"
        );
        assert!(out.contains("delete: allowed=0 refused=1"), "{out}");
        let m = matrix_file(d.path(), &[("days", "pass"), ("mod:count", "pass")]);
        let out = stdout(&tdo(&["--vdir", vd, "check", "--matrix", &m]));
        assert!(!out.contains("GATE:"), "{out}");
        assert!(out.contains("delete: allowed=0 refused=1"), "{out}");
    }

    #[test]
    fn model_read_only_count_includes_completed_tasks() {
        let d = tempfile::tempdir().unwrap();
        let v = d.path().join("vdir/c");
        std::fs::create_dir_all(&v).unwrap();
        // A daily task with a recurrence override (blocked), and a completed blocked task.
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t1\r\nSUMMARY:Synthetic override\r\nDUE;VALUE=DATE:20261107\r\nRRULE:FREQ=DAILY\r\nEND:VTODO\r\nBEGIN:VTODO\r\nUID:t1\r\nSUMMARY:Synthetic override\r\nRECURRENCE-ID;VALUE=DATE:20261108\r\nDUE;VALUE=DATE:20261109\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        std::fs::write(v.join("a.ics"), ics).unwrap();
        let done = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t2\r\nSUMMARY:Synthetic done\r\nSTATUS:COMPLETED\r\nRRULE:FREQ=HOURLY\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        std::fs::write(v.join("b.ics"), done).unwrap();
        let vd = d.path().join("vdir");
        let vd = vd.to_str().unwrap();
        let m = matrix_file(d.path(), &[("days", "pass")]);
        let out = stdout(&tdo(&["--vdir", vd, "check", "--matrix", &m]));
        assert!(out.contains("model read-only resources: 2\n"), "{out}");
        assert!(
            out.contains("    recurrence overrides are not supported: 1\n"),
            "{out}"
        );
        assert!(!out.contains("Synthetic override"), "{out}");
        assert!(
            out.contains("tasks: 2") && out.contains("reopen: allowed=0 refused=1 inapplicable=1"),
            "{out}"
        );
        let out = stdout(&tdo(&["--vdir", vd, "check", "--matrix", &m, "--verbose"]));
        assert!(out.contains("        Synthetic override\n"), "{out}");
    }
    #[test]
    fn report_separates_rule_edit_evidence_from_permissions_and_partial_parse() {
        let d = tempfile::tempdir().unwrap();
        let vd = mini_vdir(d.path(), true);
        let m = matrix_file(d.path(), &[("days", "pass")]);
        let text = std::fs::read_to_string(&m)
            .unwrap()
            .replace("edit = \"pass\"", "edit = \"unverified\"");
        std::fs::write(&m, text).unwrap();
        let result = tdo(&["--vdir", &vd, "check", "--matrix", &m]);
        assert_eq!(result.status.code(), Some(0));
        let out = stdout(&result);
        assert!(out.contains("execute=pass edit=unverified"), "{out}");
        assert!(out.contains("edit title: allowed=0 refused=1"), "{out}");
        assert!(
            out.contains("edit for daily tasks is not verified yet"),
            "{out}"
        );
        assert!(!out.contains("b.ics"), "{out}");
        let verbose = stdout(&tdo(&["--vdir", &vd, "check", "--matrix", &m, "--verbose"]));
        assert!(verbose.contains("b.ics"), "{verbose}");
    }

    #[test]
    fn empty_input_does_not_claim_all_operations_pass() {
        let d = tempfile::tempdir().unwrap();
        let result = tdo(&["--vdir", d.path().to_str().unwrap(), "check"]);
        assert_eq!(result.status.code(), Some(0));
        let out = stdout(&result);
        assert!(out.contains("tasks: 0"), "{out}");
        assert!(out.contains("No tasks to assess"), "{out}");
        assert!(!out.contains("GATE:"), "{out}");
    }

    #[test]
    fn duplicate_and_cancelled_resources_are_not_blanket_permissions() {
        let d = tempfile::tempdir().unwrap();
        let c = d.path().join("c");
        std::fs::create_dir(&c).unwrap();
        for (file, uid, status) in [
            ("a", "duplicate", "NEEDS-ACTION"),
            ("b", "duplicate", "COMPLETED"),
            ("c", "cancelled", "CANCELLED"),
        ] {
            std::fs::write(c.join(format!("{file}.ics")), format!("BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:{uid}\r\nSUMMARY:Synthetic private title\r\nDESCRIPTION:Synthetic private note\r\nSTATUS:{status}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n")).unwrap();
        }
        let result = tdo(&["--vdir", d.path().to_str().unwrap(), "check"]);
        assert_eq!(result.status.code(), Some(0));
        let out = stdout(&result);
        assert!(out.contains("tasks: 3"), "{out}");
        assert!(out.contains("delete: allowed=1 refused=2"), "{out}");
        assert!(
            out.contains("complete: allowed=0 refused=1 inapplicable=2"),
            "{out}"
        );
        assert!(
            out.contains("reopen: allowed=0 refused=1 inapplicable=2"),
            "{out}"
        );
        assert!(
            out.contains("move: allowed=0 refused=0 inapplicable=0 needs-input=3"),
            "{out}"
        );
        assert!(!out.contains("Synthetic private"), "{out}");
        let verbose = stdout(&tdo(&[
            "--vdir",
            d.path().to_str().unwrap(),
            "check",
            "--verbose",
        ]));
        assert!(verbose.contains("Synthetic private title"), "{verbose}");
        assert!(!verbose.contains("Synthetic private note"), "{verbose}");
    }
}

#[cfg(unix)]
mod sync {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};
    use std::time::{Duration, Instant};

    use twydo_core::config::Config;

    /// A temp dir holding `xdg/twydo/config.toml`, `vdir/c1/` and `state/`.
    struct Fx {
        dir: tempfile::TempDir,
    }

    impl Fx {
        /// `sync` is the `[sync]` table body, or `None` for no `[sync]`.
        fn new(sync: Option<&str>) -> Fx {
            let fx = Fx {
                dir: tempfile::tempdir().unwrap(),
            };
            fs::create_dir_all(fx.path("vdir/c1")).unwrap();
            fs::create_dir_all(fx.path("xdg/twydo")).unwrap();
            fx.configure(sync);
            fx
        }

        /// `[sync]` whose command is a fake `sh` script (`metasync` off); `body` gets the
        /// fixture so the script can name files in it.
        fn with_fake(body: impl FnOnce(&Fx) -> String) -> Fx {
            let fx = Fx::new(None);
            let fake = fx.path("fake.sh");
            write_script(&fake, &body(&fx));
            fx.configure(Some(&format!(
                "command = \"{}\"\nmetasync = false",
                fake.display()
            )));
            fx
        }

        fn configure(&self, sync: Option<&str>) {
            let mut text = format!(
                "vdir = \"{}\"\nstate_dir = \"{}\"\n",
                self.path("vdir").display(),
                self.path("state").display()
            );
            if let Some(body) = sync {
                text.push_str(&format!("\n[sync]\n{body}\n"));
            }
            fs::write(self.config_path(), text).unwrap();
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.dir.path().join(rel)
        }

        fn config_path(&self) -> PathBuf {
            self.path("xdg/twydo/config.toml")
        }

        fn tdo(&self, args: &[&str]) -> Output {
            Command::new(env!("CARGO_BIN_EXE_tdo"))
                .args(args)
                .env("XDG_CONFIG_HOME", self.path("xdg"))
                .output()
                .expect("run tdo")
        }
    }

    fn stderr(o: &Output) -> String {
        String::from_utf8_lossy(&o.stderr).into_owned()
    }

    fn stdout(o: &Output) -> String {
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    /// Writes an executable `sh` script and waits until it can be executed (a script
    /// written while another test thread forks can briefly fail with ETXTBSY). Every
    /// script answers `__probe` by exiting at once.
    fn write_script(path: &Path, body: &str) {
        fs::write(
            path,
            format!("#!/bin/sh\n[ \"$1\" = __probe ] && exit 0\n{body}\n"),
        )
        .unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match Command::new(path).arg("__probe").status() {
                Ok(st) => {
                    assert!(st.success(), "probe of {} failed", path.display());
                    return;
                }
                Err(e) if e.raw_os_error() == Some(26) && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("cannot run {}: {e}", path.display()),
            }
        }
    }

    #[test]
    fn sync_without_config_exits_2() {
        let fx = Fx::new(None);
        let o = fx.tdo(&["sync"]);
        assert_eq!(o.status.code(), Some(2));
        assert!(
            stderr(&o).contains("sync is not configured: add a [sync] table (docs/sync.md)"),
            "{}",
            stderr(&o)
        );
        assert!(!fx.path("state").exists());
    }

    #[test]
    fn status_prints_never_synced() {
        let fx = Fx::new(Some("command = \"/nonexistent/vdirsyncer\""));
        let o = fx.tdo(&["status"]);
        assert!(o.status.success(), "{}", stderr(&o));
        let out = stdout(&o);
        assert!(out.contains("last sync: never"), "{out}");
        assert!(out.contains("pending: 0"), "{out}");
        assert!(out.contains("conflicts: 0"), "{out}");
        assert!(!fx.path("state").exists(), "status creates nothing");
    }

    #[test]
    fn clean_sync_exits_0_and_status_shows_it() {
        let fx = Fx::with_fake(|_| "echo 'Syncing twydo/c1' >&2".into());
        let o = fx.tdo(&["sync"]);
        assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
        let o = fx.tdo(&["sync", "--background"]);
        assert_eq!(o.status.code(), Some(0));
        assert!(o.stdout.is_empty() && o.stderr.is_empty(), "{o:?}");
        let out = stdout(&fx.tdo(&["status"]));
        assert!(!out.contains("last sync: never"), "{out}");
        assert!(out.contains("c1: ok"), "{out}");
    }

    #[test]
    fn failed_sync_exits_1() {
        let fx = Fx::with_fake(|_| "echo 'error: Connection refused' >&2\nexit 1".into());
        let o = fx.tdo(&["sync"]);
        assert_eq!(o.status.code(), Some(1));
        assert!(stderr(&o).contains("Connection refused"), "{}", stderr(&o));
        let out = stdout(&fx.tdo(&["status"]));
        assert!(out.contains("failures: 1"), "{out}");
    }

    #[test]
    fn force_delete_resolves_display_name() {
        let fx = Fx::with_fake(|fx| format!("echo \"$@\" >> '{}'", fx.path("args.txt").display()));
        fs::write(fx.path("vdir/c1/displayname"), "Errands\n").unwrap();
        let o = fx.tdo(&["sync", "--force-delete", "Errands"]);
        assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
        assert_eq!(
            fs::read_to_string(fx.path("args.txt")).unwrap(),
            "sync --force-delete twydo/c1\n"
        );
        let o = fx.tdo(&["sync", "--force-delete", "nope"]);
        assert_eq!(o.status.code(), Some(2));
        assert!(stderr(&o).contains("nope"), "{}", stderr(&o));
    }

    #[test]
    fn sync_off_and_on_switch_automatic_sync() {
        let fx = Fx::with_fake(|fx| format!("touch '{}'", fx.path("marker").display()));
        let o = fx.tdo(&["sync", "off"]);
        assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
        assert!(stdout(&o).contains("automatic sync off"), "{}", stdout(&o));
        let out = stdout(&fx.tdo(&["status"]));
        assert!(out.contains("automatic sync: off"), "{out}");
        let o = fx.tdo(&["sync", "--background"]);
        assert_eq!(o.status.code(), Some(0));
        assert!(
            !fx.path("marker").exists(),
            "a background sync ran while off"
        );

        let o = fx.tdo(&["sync", "on"]);
        assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
        assert!(stdout(&o).contains("automatic sync on"), "{}", stdout(&o));
        let out = stdout(&fx.tdo(&["status"]));
        assert!(out.contains("automatic sync: on"), "{out}");
    }

    /// Final review: once the switch is on, an error reading the unsynced edits starts a
    /// sync (which checks again) rather than reporting the switch as failed.
    #[test]
    fn sync_on_succeeds_when_the_edits_cannot_be_read() {
        let fx = Fx::with_fake(|_| "exit 0".into());
        let o = fx.tdo(&["sync", "off"]);
        assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
        let state = twydo_core::store::vdir_state_dir(&fx.path("state"), &fx.path("vdir")).unwrap();
        fs::create_dir_all(state.join("journal")).unwrap();
        fs::write(state.join("sync/status.json"), "not json").unwrap();
        let o = fx.tdo(&["sync", "on"]);
        assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
        assert!(stdout(&o).contains("automatic sync on"), "{}", stdout(&o));
    }

    /// A refusal for a queued edit titled `label`, not yet shown.
    fn record_refusal(fx: &Fx, label: &str) {
        use chrono::{TimeZone, Utc};
        use twydo_core::edit::Op;
        use twydo_core::store::journal::{Entry, Stage};
        use twydo_core::store::queue::{FORMAT, Item, Kind, Prepared, Queue, new_id, origin};
        let at = Utc.with_ymd_and_hms(2026, 11, 6, 12, 0, 0).unwrap();
        let item = Item {
            format: FORMAT,
            id: new_id(at),
            created: at,
            origin: origin("op"),
            label: label.to_string(),
            kind: Kind::Op {
                op: Op::Complete {
                    uid: "uid-a".into(),
                },
                prepared: Prepared {
                    entry: Entry {
                        format: 1,
                        id: "x".into(),
                        created: at,
                        kind: "edit".into(),
                        uids: vec!["uid-a".into()],
                        stage: Stage::Prepared,
                        inverse_of: None,
                        from_queue: None,
                        synced: None,
                        holds: Vec::new(),
                        family: None,
                        writes: Vec::new(),
                    },
                    holds: Vec::new(),
                    family: None,
                },
            },
        };
        let state = twydo_core::store::vdir_state_dir(&fx.path("state"), &fx.path("vdir")).unwrap();
        let q = Queue::open(&state).unwrap();
        q.push(&item).unwrap();
        assert!(q.claim(&item.id).unwrap());
        q.refuse(&item, "changed on disk", true, at).unwrap();
    }

    #[test]
    fn sync_prints_unshown_refusals_once() {
        let fx = Fx::with_fake(|_| "exit 0".into());
        record_refusal(&fx, "Synthetic task");
        let o = fx.tdo(&["sync"]);
        assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
        assert!(
            stderr(&o).contains("Not saved: \"Synthetic task\" changed on another device"),
            "{}",
            stderr(&o)
        );
        let o = fx.tdo(&["sync"]);
        assert!(!stderr(&o).contains("Not saved"), "{}", stderr(&o));
    }

    #[test]
    fn sync_off_without_config_exits_2() {
        let fx = Fx::new(None);
        for arg in ["off", "on"] {
            let o = fx.tdo(&["sync", arg]);
            assert_eq!(o.status.code(), Some(2));
            assert!(
                stderr(&o).contains("sync is not configured: add a [sync] table (docs/sync.md)"),
                "{}",
                stderr(&o)
            );
        }
    }

    #[test]
    fn when_due_waiter_exits_when_nothing_is_unsynced() {
        let fx = Fx::with_fake(|fx| format!("touch '{}'", fx.path("marker").display()));
        let o = fx.tdo(&["sync", "--background", "--when-due"]);
        assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
        assert!(
            !fx.path("marker").exists(),
            "nothing was due, yet it synced"
        );
        let o = fx.tdo(&["sync", "--when-due"]);
        assert_eq!(o.status.code(), Some(2), "--when-due requires --background");

        let cfg = Config::load(&fx.config_path()).unwrap();
        let cmd = tdo::cli::sync::background_command(Path::new("tdo"), &cfg, None, true).unwrap();
        let args: Vec<_> = cmd.get_args().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(args, ["sync", "--background", "--when-due"]);
    }

    #[test]
    fn spawn_background_starts_runner() {
        let fx = Fx::with_fake(|fx| format!("touch '{}'", fx.path("marker").display()));
        let marker = fx.path("marker");
        let cfg = Config::load(&fx.config_path()).unwrap();
        let mut cmd = tdo::cli::sync::background_command(
            Path::new(env!("CARGO_BIN_EXE_tdo")),
            &cfg,
            None,
            false,
        )
        .expect("sync is configured");
        cmd.env("XDG_CONFIG_HOME", fx.path("xdg"));
        tdo::cli::sync::detach(cmd);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            marker.exists(),
            "the background runner never ran the command"
        );

        let mut no_sync = cfg.clone();
        no_sync.sync = None;
        assert!(
            tdo::cli::sync::background_command(Path::new("tdo"), &no_sync, None, false).is_none()
        );
    }
}
