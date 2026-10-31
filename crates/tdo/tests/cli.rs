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
        assert!(
            out.contains(
                "months-day-1-28  1 tasks  preserve=unverified execute=unverified edit=unverified"
            ),
            "{out}"
        );
        assert!(out.contains("parse errors: 0"), "{out}");
        assert!(out.contains("GATE: NOT PASSED ("), "{out}");
        assert!(
            out.contains("rule classes unverified, 0 parse errors)"),
            "{out}"
        );
        assert!(!out.contains("Pay rent"), "{out}");
        let v = tdo(&["--vdir", sample(), "check", "--verbose"]);
        assert!(stdout(&v).contains("Pay rent"));
    }

    #[test]
    fn check_missing_matrix_reports_unverified() {
        let o = tdo(&["--vdir", sample(), "check", "--matrix", "nope.toml"]);
        assert_eq!(o.status.code(), Some(0));
        let out = stdout(&o);
        assert!(out.contains("matrix not found: nope.toml"), "{out}");
        assert!(out.contains("GATE: NOT PASSED"), "{out}");
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
    fn gate_passes_only_when_all_verified_and_no_parse_errors() {
        let d = tempfile::tempdir().unwrap();
        let m = matrix_file(d.path(), &[("days", "pass")]);
        let good = mini_vdir(d.path(), false);
        let out = stdout(&tdo(&["--vdir", &good, "check", "--matrix", &m]));
        assert!(out.contains("GATE: PASSED"), "{out}");

        let d2 = tempfile::tempdir().unwrap();
        let m2 = matrix_file(d2.path(), &[("days", "pass")]);
        let bad = mini_vdir(d2.path(), true);
        let out = stdout(&tdo(&["--vdir", &bad, "check", "--matrix", &m2]));
        assert!(out.contains("parse errors: 1"), "{out}");
        assert!(
            out.contains("GATE: NOT PASSED (0 rule classes unverified, 1 parse error)"),
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
        assert!(
            out.contains("GATE: NOT PASSED (1 rule classes unverified, 0 parse errors)"),
            "{out}"
        );
        let m = matrix_file(d.path(), &[("days", "pass"), ("mod:count", "pass")]);
        let out = stdout(&tdo(&["--vdir", vd, "check", "--matrix", &m]));
        assert!(out.contains("GATE: PASSED"), "{out}");
    }
}
