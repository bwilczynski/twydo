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
        let cfg = tempfile::tempdir().unwrap();
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
        let o = tdo(&["--vdir", sample(), "export-json"]);
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
        assert!(out.contains("rule classes unverified)"), "{out}");
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
}
