//! `check.report` and `export` semantics over synthetic fixtures.

mod common;

use chrono::Utc;
use common::{Fixture, TestDaemon, kind};
use twydo_core::compat::Matrix;
use twydo_core::config::WriteMode;
use twydo_daemon::protocol::reports::{CheckReport, CheckReportParams, ExportResult};
use twydo_daemon::protocol::rpc::ErrorKind;
use twydo_daemon::server::lane::LongKind;
use twydo_daemon::server::permissions::{probes, task_lines};

fn ics(uid: &str, extra: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:{uid}\r\nSUMMARY:Synthetic {uid}\r\n{extra}END:VTODO\r\nEND:VCALENDAR\r\n"
    )
}

fn fx() -> Fixture {
    let fx = Fixture::new(&[]);
    fx.write(
        "c/weekly.ics",
        &ics("weekly", "DUE;VALUE=DATE:20261107\r\nRRULE:FREQ=WEEKLY\r\n"),
    );
    // A completed hourly task: the model keeps it read-only.
    fx.write(
        "c/blocked.ics",
        &ics("blocked", "STATUS:COMPLETED\r\nRRULE:FREQ=HOURLY\r\n"),
    );
    fx
}

fn params(verbose: bool, uid: Option<&str>) -> CheckReportParams {
    CheckReportParams {
        verbose,
        matrix: None,
        uid: uid.map(String::from),
    }
}

#[test]
fn check_report_summarizes_health_evidence_and_permissions() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let r: CheckReport = c.call("check.report", params(false, None)).unwrap();
    let out = r.lines.join("\n");
    assert!(
        out.starts_with("Data health\ntasks: 2\nparse errors: 0\n"),
        "{out}"
    );
    assert!(out.contains("model read-only resources: 1\n"), "{out}");
    assert!(
        r.lines
            .iter()
            .any(|l| l.contains("  1 tasks  complete=") && l.contains(" set-repeat=")),
        "{out}"
    );
    assert!(out.contains("\n\nRecurrence evidence"), "{out}");
    assert!(
        out.contains("\n\nOperation availability (mode=test)\n"),
        "{out}"
    );
    assert!(!out.contains("Synthetic"), "{out}");
    assert!(r.warnings.is_empty());
    let v: CheckReport = c.call("check.report", params(true, None)).unwrap();
    assert!(v.lines.iter().any(|l| l == "        Synthetic blocked"));
}

#[test]
fn check_report_for_one_uid() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let r: CheckReport = c
        .call("check.report", params(false, Some("weekly")))
        .unwrap();
    let snap = fx.snapshot();
    let idx = snap.find("weekly").unwrap();
    let want = task_lines(&probes(
        &snap,
        Matrix::embedded(),
        WriteMode::Test,
        idx,
        Utc::now(),
    ));
    assert_eq!(r.lines, want);
    assert!(
        r.lines
            .last()
            .unwrap()
            .starts_with("Permission snapshot only; "),
        "{:?}",
        r.lines
    );
    assert_eq!(
        r.warnings,
        vec!["test write mode: unverified operations are enabled".to_string()]
    );
}

#[test]
fn check_report_unknown_uid_is_not_found() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let e = c
        .call::<_, CheckReport>("check.report", params(false, Some("nope")))
        .unwrap_err();
    assert_eq!(kind(&e), ErrorKind::NotFound);
    assert!(e.to_string().contains("no task with UID nope"), "{e}");
}

#[test]
fn check_report_with_a_missing_matrix_is_invalid() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let mut p = params(false, None);
    p.matrix = Some("/nonexistent.toml".into());
    let e = c.call::<_, CheckReport>("check.report", p).unwrap_err();
    assert_eq!(kind(&e), ErrorKind::Invalid);
    assert!(e.to_string().contains("/nonexistent.toml"), "{e}");
}

#[test]
fn both_reports_see_a_queued_creation_without_draining_or_syncing() {
    use twydo_core::{
        edit::{NewTask, Op},
        store::queue,
    };
    use twydo_daemon::protocol::methods::WriteOutcome;
    let fx = fx();
    let marker_dir = tempfile::tempdir().unwrap();
    let marker = marker_dir.path().join("sync-ran");
    let mut cfg = fx.with_sync(&format!("touch '{}'", marker.display()));
    // Keep enqueueing separate from the report's responsibility for leaving sync alone.
    cfg.sync.as_mut().unwrap().after_edit = Some(false);
    let td = TestDaemon::start_cfg(&fx, cfg.clone(), |_| {});
    let c = td.client();
    let job = td.handle().hold_long(LongKind::Test);
    let count = fx.snapshot().tasks.len();
    let title = "Synthetic queued report target";
    let result = common::submit(&c, Op::Create(NewTask::plain("c", title))).unwrap();
    assert_eq!(result.outcome, WriteOutcome::Queued);
    let queued = queue::view(twydo_core::vdir::load(&fx.vdir()).unwrap(), cfg);
    let uid = &queued
        .tasks
        .iter()
        .find(|t| t.summary == title)
        .unwrap()
        .uid;
    let queue_path = std::fs::read_dir(fx.state().join("queue"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "json"))
        .unwrap();
    let queue_bytes = std::fs::read(&queue_path).unwrap();
    let files = resource_bytes(&fx);
    // Publication follows write.submit asynchronously; wait for its planning snapshot.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let aggregate = report(&td, None, true);
        if aggregate.contains(&format!("tasks: {}", count + 1)) {
            assert!(aggregate.contains(title), "{aggregate}");
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{aggregate}");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let single = task_report(&td, uid);
    assert!(single.contains("edit-title: allowed"), "{single}");
    assert_eq!(std::fs::read(queue_path).unwrap(), queue_bytes);
    assert_eq!(fx.snapshot().tasks.len(), count);
    assert_eq!(resource_bytes(&fx), files);
    assert!(!marker.exists(), "reporting started sync");
    drop(job);
}

fn export(c: &twydo_daemon::client::Client) -> ExportResult {
    c.call("export", serde_json::json!({})).unwrap()
}

#[test]
fn export_matches_schema_v1() {
    let fx = fx();
    fx.write(
        "a/own.ics",
        &ics("own", "X-2DOAPP-TYPE:0\r\nX-2DOAPP-STAR:1\r\nDUE;VALUE=DATE:20261107\r\nRRULE:FREQ=MONTHLY;BYMONTHDAY=7\r\n"),
    );
    fx.write(
        "c/weekly.ics",
        &ics("weekly", "DUE;TZID=America/New_York:20261106T090000\r\nRRULE:FREQ=WEEKLY\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\n"),
    );
    fx.write("a/displayname", "A");
    fx.write("a/color", "#102030");
    let td = TestDaemon::start(&fx);
    let r = export(&td.client());
    assert_eq!(r.skipped, 0);
    let d = &r.document;
    assert_eq!(d["schema"], 1);
    let cols: Vec<&str> = d["collections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    let mut sorted = cols.clone();
    sorted.sort();
    assert_eq!(cols, sorted);
    assert!(cols.contains(&"a") && cols.contains(&"c"), "{cols:?}");
    let a = d["collections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "a")
        .unwrap();
    assert_eq!(a["color"], "#102030");
    assert_eq!(a["display_name"], "A");
    let keys: Vec<(String, String)> = d["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["collection"].as_str().unwrap().into(),
                t["uid"].as_str().unwrap().into(),
            )
        })
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
    assert_eq!(keys.len(), 3);
    let own = d["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["uid"] == "own")
        .unwrap();
    assert_eq!(
        own["x2do_props"],
        serde_json::json!(["X-2DOAPP-STAR", "X-2DOAPP-TYPE"])
    );
    assert_eq!(own["rule_class"], "months-day-1-28");
    assert_eq!(own["due"], "2026-11-07");
    assert_eq!(own["due_is_date"], true);
    let timed = d["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["uid"] == "weekly")
        .unwrap();
    assert_eq!(timed["due"], "2026-11-06T09:00:00-05:00");
    assert_eq!(timed["alarms"][0]["relative_seconds"], -900);
    assert_eq!(timed["alarms"][0]["related_end"], false);
}

#[test]
fn export_ignores_queued_edits() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let job = td.handle().hold_long(LongKind::Test);
    let r = common::submit(&c, common::edit_title("weekly", "Edited")).unwrap();
    assert_eq!(
        r.outcome,
        twydo_daemon::protocol::methods::WriteOutcome::Queued
    );
    // The published (planning) view shows the edit; the export does not.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let snap: twydo_daemon::protocol::wire::WireSnapshot =
            c.call("snapshot.get", serde_json::json!({})).unwrap();
        if snap.tasks.iter().any(|t| t.task.summary == "Edited") {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "edit never shown");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let doc = export(&c).document;
    let titles: Vec<&str> = doc["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["summary"].as_str().unwrap())
        .collect();
    assert!(titles.contains(&"Synthetic weekly"), "{titles:?}");
    assert!(!titles.contains(&"Edited"), "{titles:?}");
    drop(job);
}

#[test]
fn export_counts_unparseable_files() {
    let fx = fx();
    fx.write("c/broken.ics", "this is not a calendar\r\n");
    let td = TestDaemon::start(&fx);
    let r = export(&td.client());
    assert_eq!(r.skipped, 1);
    assert_eq!(r.document["tasks"].as_array().unwrap().len(), 2);
}

/// Report semantics run against the daemon's verified-mode policy, independently of the CLI.
fn verified_daemon(fx: &Fixture) -> TestDaemon {
    let mut cfg = fx.config();
    cfg.writes.mode = WriteMode::Verified;
    TestDaemon::start_cfg(fx, cfg, |_| {})
}

fn matrix_file(dir: &std::path::Path, rows: &[(&str, &str, &str)]) -> std::path::PathBuf {
    use twydo_core::compat::{ContractId, OperationId};
    let mut text = String::from("matrix_version = 2\n");
    for (class, execute, edit) in rows {
        for (op, value) in [
            (OperationId::Complete, execute),
            (OperationId::SetRepeat, edit),
        ] {
            if let Some(id) = ContractId::schedule(op, class.trim_start_matches("mod:")) {
                text += &format!(
                    "[[contract]]\nid = {:?}\niphone_baseline = {value:?}\ntwydo_to_iphone = {value:?}\niphone_to_twydo = {value:?}\n",
                    id.as_str()
                );
            }
        }
    }
    let path = dir.join("matrix.toml");
    std::fs::write(&path, text).unwrap();
    path
}

fn report(td: &TestDaemon, matrix: Option<&std::path::Path>, verbose: bool) -> String {
    let mut p = params(verbose, None);
    p.matrix = matrix.map(std::path::Path::to_path_buf);
    let r: CheckReport = td.client().call("check.report", p).unwrap();
    // Preserve the trailing newline used by report rendering.
    format!("{}\n", r.lines.join("\n"))
}

#[test]
fn healthy_data_and_partial_parse_still_report_refused_operations() {
    for broken in [false, true] {
        let fx = Fixture::new(&[]);
        fx.write(
            "c/a.ics",
            &ics("t1", "DUE;VALUE=DATE:20261107\r\nRRULE:FREQ=DAILY\r\n"),
        );
        if broken {
            fx.write("c/b.ics", "this is not ical");
        }
        let dir = tempfile::tempdir().unwrap();
        let matrix = matrix_file(dir.path(), &[("days", "pass", "pass")]);
        let td = verified_daemon(&fx);
        let out = report(&td, Some(&matrix), false);
        assert!(!out.contains("GATE:"), "{out}");
        assert!(out.contains("delete: allowed=0 refused=1"), "{out}");
        if broken {
            assert!(out.contains("parse errors: 1"), "{out}");
            assert!(out.contains("edit-title: allowed=0 refused=1"), "{out}");

            // Rule-edit evidence and operation permission are separate, even with parse errors.
            matrix_file(dir.path(), &[("days", "pass", "unverified")]);
            let out = report(&td, Some(&matrix), false);
            assert!(out.contains("complete=pass set-repeat=unverified"), "{out}");
            assert!(out.contains("edit-title: allowed=0 refused=1"), "{out}");
            assert!(
                out.contains("edit-title.resource is not verified yet"),
                "{out}"
            );
            assert!(!out.contains("b.ics"), "{out}");
            let verbose = report(&td, Some(&matrix), true);
            assert!(verbose.contains("b.ics"), "{verbose}");
        }
    }
}

#[test]
fn unverified_modifier_blocks_class() {
    let fx = Fixture::new(&[]);
    fx.write(
        "c/a.ics",
        &ics(
            "t1",
            "DUE;VALUE=DATE:20261107\r\nRRULE:FREQ=DAILY;COUNT=3\r\n",
        ),
    );
    let td = verified_daemon(&fx);
    let dir = tempfile::tempdir().unwrap();
    for verdict in ["unverified", "pass"] {
        let matrix = matrix_file(
            dir.path(),
            &[("days", "pass", "pass"), ("mod:count", verdict, verdict)],
        );
        let out = report(&td, Some(&matrix), false);
        assert!(
            out.contains(&format!("days+count  1 tasks  complete={verdict}")),
            "{out}"
        );
        assert!(out.contains("delete: allowed=0 refused=1"), "{out}");
        assert!(!out.contains("GATE:"), "{out}");
    }
}

#[test]
fn model_read_only_count_includes_completed_tasks() {
    let fx = Fixture::new(&[]);
    fx.write("c/a.ics", "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t1\r\nSUMMARY:Synthetic override\r\nDUE;VALUE=DATE:20261107\r\nRRULE:FREQ=DAILY\r\nEND:VTODO\r\nBEGIN:VTODO\r\nUID:t1\r\nSUMMARY:Synthetic override\r\nRECURRENCE-ID;VALUE=DATE:20261108\r\nDUE;VALUE=DATE:20261109\r\nEND:VTODO\r\nEND:VCALENDAR\r\n");
    fx.write(
        "c/b.ics",
        &ics("t2", "STATUS:COMPLETED\r\nRRULE:FREQ=HOURLY\r\n"),
    );
    let td = verified_daemon(&fx);
    let dir = tempfile::tempdir().unwrap();
    let matrix = matrix_file(dir.path(), &[("days", "pass", "pass")]);
    let out = report(&td, Some(&matrix), false);
    assert!(out.contains("model read-only resources: 2\n"), "{out}");
    assert!(
        out.contains("    recurrence overrides are not supported: 1\n"),
        "{out}"
    );
    assert!(!out.contains("Synthetic override"), "{out}");
    assert!(out.contains("tasks: 2"), "{out}");
    assert!(
        out.contains("reopen: allowed=0 refused=1 inapplicable=1"),
        "{out}"
    );
    let verbose = report(&td, Some(&matrix), true);
    assert!(
        verbose.contains("        Synthetic override\n"),
        "{verbose}"
    );
}

#[test]
fn empty_input_does_not_claim_all_operations_pass() {
    let fx = Fixture::new(&[]);
    let td = verified_daemon(&fx);
    let out = report(&td, None, false);
    assert!(out.contains("tasks: 0"), "{out}");
    assert!(out.contains("No tasks to assess"), "{out}");
    assert!(!out.contains("GATE:"), "{out}");
}

#[test]
fn duplicate_and_cancelled_resources_are_not_blanket_permissions() {
    let fx = Fixture::new(&[]);
    for (file, uid, status) in [
        ("a", "duplicate", "NEEDS-ACTION"),
        ("b", "duplicate", "COMPLETED"),
        ("c", "cancelled", "CANCELLED"),
    ] {
        fx.write(&format!("c/{file}.ics"), &format!("BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:{uid}\r\nSUMMARY:Synthetic private title\r\nDESCRIPTION:Synthetic private note\r\nSTATUS:{status}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"));
    }
    let td = verified_daemon(&fx);
    let out = report(&td, None, false);
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
    let verbose = report(&td, None, true);
    assert!(verbose.contains("Synthetic private title"), "{verbose}");
    assert!(!verbose.contains("Synthetic private note"), "{verbose}");
}

#[test]
fn check_finishes_permissions_for_a_large_collection() {
    let fx = Fixture::new(&[]);
    let source =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../fixtures/complete/standalone-lifecycle/edit/list1/0a04f22aeb294b719633c978fe1975e5.ics",
        ))
        .unwrap();
    for n in 0..1000 {
        let uid = format!("{n:032x}");
        fx.write(
            &format!("synthetic/{uid}.ics"),
            &source.replace("0a04f22aeb294b719633c978fe1975e5", &uid),
        );
    }
    let td = verified_daemon(&fx);
    let r: CheckReport = td
        .client()
        .caller()
        .call_timeout(
            "check.report",
            params(false, None),
            std::time::Duration::from_secs(10),
        )
        .expect("permission reporting finishes within 10 seconds");
    let out = r.lines.join("\n");
    assert!(out.contains("tasks: 1000"), "{out}");
    assert!(out.contains("delete: allowed=1000 refused=0"), "{out}");
    assert!(
        out.contains("reparent: allowed=0 refused=0 inapplicable=0 needs-input=1000"),
        "{out}"
    );
}

fn task_report(td: &TestDaemon, uid: &str) -> String {
    let r: CheckReport = td
        .client()
        .call("check.report", params(false, Some(uid)))
        .unwrap();
    r.lines.join("\n")
}

/// Resource content, excluding the daemon's separate state directory.
fn resource_bytes(fx: &Fixture) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut result = std::collections::BTreeMap::new();
    for collection in std::fs::read_dir(fx.vdir()).unwrap() {
        for entry in std::fs::read_dir(collection.unwrap().path()).unwrap() {
            let path = entry.unwrap().path();
            result.insert(path.clone(), std::fs::read(path).unwrap());
        }
    }
    result
}

fn capture(path: &str) -> Fixture {
    let fx = Fixture::new(&[]);
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(path);
    for file in std::fs::read_dir(source).unwrap() {
        let path = file.unwrap().path();
        if path.extension().is_some_and(|e| e == "ics") {
            fx.write(
                &format!("c/{}", path.file_name().unwrap().to_str().unwrap()),
                &std::fs::read_to_string(&path).unwrap(),
            );
        }
    }
    fx
}

#[test]
fn both_reports_use_the_configured_mode_and_real_gate() {
    use twydo_core::{
        edit::{Change, Op},
        gate,
        views::Snapshot,
    };
    let fx = Fixture::new(&[]);
    fx.write(
        "c/monthly.ics",
        &ics(
            "monthly",
            "DUE;VALUE=DATE:20261107\r\nRRULE:FREQ=MONTHLY;BYMONTHDAY=7\r\n",
        ),
    );
    let dir = tempfile::tempdir().unwrap();
    let matrix_path = matrix_file(dir.path(), &[("months-day-1-28", "pass", "pass")]);
    // The isolated iPhone baseline passes; cross-device evidence stays unverified.
    // Test mode admits the text edits, while verified mode must refuse them.
    let mut matrix_text = std::fs::read_to_string(&matrix_path).unwrap();
    for id in ["edit-title.resource", "edit-notes.resource"] {
        matrix_text.push_str(&format!("[[contract]]\nid = {id:?}\niphone_baseline = \"pass\"\ntwydo_to_iphone = \"unverified\"\niphone_to_twydo = \"unverified\"\n"));
    }
    std::fs::write(&matrix_path, matrix_text).unwrap();
    let matrix = Matrix::load(&matrix_path).unwrap();
    for mode in [WriteMode::Verified, WriteMode::Test] {
        let mut cfg = fx.config();
        cfg.writes.mode = mode;
        let snap = Snapshot::new(twydo_core::vdir::load(&fx.vdir()).unwrap(), cfg.clone());
        let task = &snap.tasks[snap.find("monthly").unwrap()];
        let td = TestDaemon::start_cfg(&fx, cfg.clone(), |o| o.matrix = matrix.clone());
        let single = task_report(&td, "monthly");
        let aggregate = report(&td, None, false);
        assert!(
            single.contains(if mode == WriteMode::Test {
                "edit-title: allowed"
            } else {
                "edit-title: refused"
            }),
            "{single}"
        );
        for (action, op) in [
            (
                "edit-title",
                Op::Edit {
                    uid: task.uid.clone(),
                    changes: vec![Change::Summary(task.summary.clone())],
                },
            ),
            (
                "edit-notes",
                Op::Edit {
                    uid: task.uid.clone(),
                    changes: vec![Change::Description(task.description.clone())],
                },
            ),
            (
                "delete",
                Op::Delete {
                    uid: task.uid.clone(),
                },
            ),
        ] {
            match gate::check(
                &snap,
                &matrix,
                mode,
                &op,
                Utc::now().with_timezone(&cfg.tz()).date_naive(),
            ) {
                Ok(()) => assert!(single.contains(&format!("{action}: allowed")), "{single}"),
                Err(b) => {
                    assert!(
                        single.contains(&format!("{action}: refused: {b}")),
                        "{single}"
                    );
                    assert!(aggregate.contains(&b.0), "{aggregate}");
                }
            }
        }
        let name = if mode == WriteMode::Verified {
            "verified"
        } else {
            "test"
        };
        assert!(aggregate.contains(&format!("mode={name}")), "{aggregate}");
    }
}

#[test]
fn history_reopen_reports_match_real_gate_without_task_content() {
    use twydo_core::{edit::Op, gate, model::Status, views::Snapshot};
    let fx = capture("complete/family-lifecycle/complete-parent/list1");
    let mut cfg = fx.config();
    cfg.timezone = Some("America/New_York".into());
    cfg.writes.mode = WriteMode::Verified;
    let snap = Snapshot::new(twydo_core::vdir::load(&fx.vdir()).unwrap(), cfg.clone());
    let td = TestDaemon::start_cfg(&fx, cfg, |_| {});
    let today = Utc::now().with_timezone(&snap.tz()).date_naive();
    let mut allowed = 0;
    let mut refused = 0;
    for task in snap.tasks.iter().filter(|t| t.status == Status::Completed) {
        let permits = gate::check(
            &snap,
            Matrix::embedded(),
            WriteMode::Verified,
            &Op::Reopen {
                uid: task.uid.clone(),
            },
            today,
        )
        .is_ok();
        if permits {
            allowed += 1;
        } else {
            refused += 1;
        }
        let single = task_report(&td, &task.uid);
        assert!(
            single.contains(if permits {
                "reopen: allowed"
            } else {
                "reopen: refused"
            }),
            "{single}"
        );
        assert!(
            !single.contains(&format!("SUMMARY:{}", task.summary)),
            "{single}"
        );
    }
    assert!(allowed + refused > 0, "capture contains completed tasks");
    let aggregate = report(&td, None, false);
    assert!(
        aggregate.contains(&format!("reopen: allowed={allowed} refused={refused}")),
        "{aggregate}"
    );
    assert!(!aggregate.contains("r1"), "{aggregate}");
}

#[test]
fn both_reports_reflect_exact_resource_deletion_without_titles() {
    let fx = Fixture::new(&[]);
    fx.write(
        "c/weekly.ics",
        &ics("weekly", "DUE;VALUE=DATE:20261107\r\nRRULE:FREQ=WEEKLY\r\n"),
    );
    fx.write(
        "c/parent.ics",
        &ics(
            "parent",
            "STATUS:NEEDS-ACTION\r\nX-2DOAPP-METADATA:@tasktype:checklist\r\n",
        ),
    );
    fx.write(
        "c/child.ics",
        &ics("child", "RELATED-TO:parent\r\nSTATUS:NEEDS-ACTION\r\n"),
    );
    let td = verified_daemon(&fx);
    for uid in ["weekly", "parent", "child"] {
        let single = task_report(&td, uid);
        assert!(single.contains("delete: allowed"), "{single}");
    }
    let aggregate = report(&td, None, false);
    assert!(aggregate.contains("delete: allowed="), "{aggregate}");
    for title in ["Synthetic parent", "Synthetic child"] {
        assert!(!aggregate.contains(title), "{aggregate}");
    }
    let verbose = report(&td, None, true);
    for title in ["Synthetic parent", "Synthetic child"] {
        assert!(verbose.contains(title), "{verbose}");
    }
}

#[test]
fn both_reports_admit_text_edits_on_closed_recurring_checklists() {
    let fx = capture("complete/checklist-reset/phone-items-and-text/list1");
    let uid = "twydo-p8-c29-b";
    let rel = format!("c/{uid}.ics");
    fx.write(
        &rel,
        &fx.read(&rel).replace(
            "STATUS:NEEDS-ACTION",
            "STATUS:COMPLETED\nCOMPLETED:20261210T120000Z",
        ),
    );
    let td = verified_daemon(&fx);
    for text in [report(&td, None, false), task_report(&td, uid)] {
        assert!(
            !text.contains("checklist creation and text editing"),
            "{text}"
        );
        assert!(text.contains("edit-title: allowed"), "{text}");
        assert!(text.contains("edit-notes: allowed"), "{text}");
        assert!(!text.contains(uid), "{text}");
    }
}
