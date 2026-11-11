//! `check.report`. Synthetic fixture content.

mod common;

use chrono::Utc;
use common::{Fixture, TestDaemon, kind};
use twydo_core::compat::Matrix;
use twydo_core::config::WriteMode;
use twydo_daemon::protocol::reports::{CheckReport, CheckReportParams};
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
