//! `check.report`. Synthetic fixture content.

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
fn check_report_matches_the_old_output() {
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
            .any(|l| l.contains("  1 tasks  preserve=") && l.contains(" edit=")),
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
fn check_report_counts_queued_edits() {
    let fx = fx();
    let td = TestDaemon::start(&fx);
    let c = td.client();
    let job = td.handle().hold_long(LongKind::Test);
    let r = common::submit(
        &c,
        twydo_core::edit::Op::Create(twydo_core::edit::NewTask::plain("c", "Queued")),
    )
    .unwrap();
    assert_eq!(
        r.outcome,
        twydo_daemon::protocol::methods::WriteOutcome::Queued
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let r: CheckReport = c.call("check.report", params(false, None)).unwrap();
        if r.lines.iter().any(|l| l == "tasks: 3") {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{:?}", r.lines);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
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
        &ics("own", "X-2DOAPP-TYPE:0\r\nX-2DOAPP-STAR:1\r\n"),
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
