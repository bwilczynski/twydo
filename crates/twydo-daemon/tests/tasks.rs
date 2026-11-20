//! `tasks.get`: one task and its children for a thin client. Synthetic fixture content.

mod common;

use chrono::{TimeZone, Utc};
use common::{FakeClock, Fixture, TestDaemon, empty, kind, submit_at};
use serde_json::json;
use twydo_core::conflicts::{ConflictKind, ConflictStore, NewConflict, Side};
use twydo_core::details;
use twydo_core::edit::{Change, Op};
use twydo_core::model::TaskKind;
use twydo_core::store::bind_vdir_state;
use twydo_core::store::fsio::VdirLock;
use twydo_daemon::protocol::methods::{BasedOn, DaemonInfo, WriteOutcome};
use twydo_daemon::protocol::rpc::ErrorKind;
use twydo_daemon::protocol::tasks::{TasksFormResult, TasksGetResult};
use twydo_daemon::protocol::wire::WireSnapshot;

fn ics(uid: &str, summary: &str, extra: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//synthetic//EN\r\nBEGIN:VTODO\r\nUID:{uid}\r\n\
         SUMMARY:{summary}\r\n{extra}END:VTODO\r\nEND:VCALENDAR\r\n"
    )
}

/// 2026-11-17 09:00 UTC.
fn morning() -> std::sync::Arc<FakeClock> {
    FakeClock::at(Utc.with_ymd_and_hms(2026, 11, 17, 9, 0, 0).unwrap())
}

fn fx() -> Fixture {
    let fx = Fixture::new(&[]);
    fx.write(
        "inbox/chk.ics",
        &ics(
            "chk",
            "Pack",
            "X-2DOAPP-METADATA:@tasktype:checklist\r\nDUE:20261117T170000\r\n\
             DESCRIPTION:Bring the form\r\n",
        ),
    );
    for (uid, extra) in [
        ("i1", "RELATED-TO;RELTYPE=PARENT:chk\r\n"),
        (
            "i2",
            "RELATED-TO;RELTYPE=PARENT:chk\r\nDESCRIPTION:Mind the size\r\n",
        ),
        (
            "i3",
            "RELATED-TO;RELTYPE=PARENT:chk\r\nSTATUS:COMPLETED\r\nCOMPLETED:20261116T100000Z\r\n",
        ),
    ] {
        fx.write(
            &format!("inbox/{uid}.ics"),
            &ics(uid, &format!("Item {uid}"), extra),
        );
    }
    fx.write(
        "inbox/rep.ics",
        &ics(
            "rep",
            "Water plants",
            "DUE:20261117T180000\r\nRRULE:FREQ=DAILY\r\n\
             BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\n",
        ),
    );
    fx
}

fn start(fx: &Fixture) -> TestDaemon {
    let mut cfg = fx.config();
    cfg.timezone = Some("UTC".into());
    TestDaemon::start_cfg(fx, cfg, |o| o.clock = morning())
}

fn get(td: &TestDaemon, uid: &str) -> TasksGetResult {
    td.client()
        .call("tasks.get", json!({ "uid": uid }))
        .unwrap()
}

#[test]
fn checklist_with_children() {
    let fx = fx();
    let td = start(&fx);
    let r = get(&td, "chk");
    assert_eq!(r.task.kind, TaskKind::Checklist);
    assert_eq!(r.task.due.as_deref(), Some("Today 17:00"));
    assert!(r.task.due_today);
    assert!(!r.task.overdue);
    assert_eq!(r.task.note.as_deref(), Some("Bring the form"));
    assert!(r.task.parent.is_none());
    let uids: Vec<_> = r.children.iter().map(|c| c.uid.as_str()).collect();
    assert_eq!(uids, ["i1", "i2", "i3"]);
    assert!(r.children[2].completed);
    assert!(!r.children[0].completed);
    assert!(r.children[1].has_note);
    assert!(!r.children[0].has_note);
    let c = td.client();
    let snap: WireSnapshot = c.call("snapshot.get", empty()).unwrap();
    let info: DaemonInfo = c.call("daemon.info", empty()).unwrap();
    assert_eq!(r.revision, snap.revision);
    assert_eq!(r.epoch, info.epoch);
}

#[test]
fn item_names_its_parent() {
    let td = start(&fx());
    let r = get(&td, "i1");
    let p = r.task.parent.expect("parent");
    assert_eq!(p.uid, "chk");
    assert_eq!(p.title, "Pack");
    assert_eq!(p.kind, TaskKind::Checklist);
    assert!(r.children.is_empty());
}

#[test]
fn repeat_and_alerts_worded_like_the_tui() {
    let fx = fx();
    let td = start(&fx);
    let r = get(&td, "rep");
    let snap = fx.snapshot();
    let idx = snap.tasks.iter().position(|t| t.uid == "rep").unwrap();
    let t = &snap.tasks[idx];
    assert!(details::repeat_text(t, snap.tz()).is_some());
    assert_eq!(r.task.repeat, details::repeat_text(t, snap.tz()));
    assert_eq!(r.task.alerts, details::alert_lines(t, snap.tz()));
    assert_eq!(r.task.alerts.len(), 1);
}

#[test]
fn conflict_and_pending_flags() {
    let fx = fx();
    let text = ics("rep", "Water plants", "DUE:20261117T180000\r\n");
    ConflictStore::open(&fx.state())
        .unwrap()
        .record(
            NewConflict {
                uid: "rep".into(),
                collection: "inbox".into(),
                path: "inbox/rep.ics".into(),
                kind: ConflictKind::BothChanged,
                held: false,
                vdir_holds: Side::Remote,
                local: Some(text.clone()),
                remote: Some(text),
            },
            Utc::now(),
        )
        .unwrap();
    let cfg = fx.config();
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let lock = VdirLock::exclusive(&state).unwrap();
    let op = Op::Edit {
        uid: "chk".into(),
        changes: vec![Change::Summary("Pack, queued".into())],
    };
    let at = Utc.with_ymd_and_hms(2026, 11, 9, 12, 0, 0).unwrap();
    common::queue_op(&cfg, &fx.snapshot(), &op, "edit", None, at);
    drop(lock);
    let _sync = VdirLock::shared(&state).unwrap();
    let td = start(&fx);
    assert!(get(&td, "rep").task.conflict);
    assert!(!get(&td, "rep").task.pending);
    let chk = get(&td, "chk");
    assert!(chk.task.pending);
    assert!(!chk.task.conflict);
    assert_eq!(chk.task.title, "Pack, queued");
}

#[test]
fn unknown_uid_is_invalid() {
    let td = start(&fx());
    let err = td
        .client()
        .call::<_, TasksGetResult>("tasks.get", json!({ "uid": "nope" }))
        .unwrap_err();
    assert_eq!(kind(&err), ErrorKind::Invalid);
}

fn field_names(r: &TasksGetResult) -> Vec<String> {
    r.form
        .fields
        .iter()
        .map(|f| {
            serde_json::to_value(f)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}

fn preview_of(r: &TasksGetResult, field: &str) -> Option<String> {
    r.form
        .previews
        .iter()
        .find(|p| serde_json::to_value(p.field).unwrap() == field)
        .map(|p| p.text.clone())
}

#[test]
fn form_block_for_a_task() {
    let fx = Fixture::new(&[]);
    fx.write(
        "inbox/wk.ics",
        &ics("wk", "Weekly", "DUE:20261118\r\nRRULE:FREQ=WEEKLY\r\n"),
    );
    let td = start(&fx);
    let r = get(&td, "wk");
    assert_eq!(
        field_names(&r),
        [
            "title", "list", "type", "due", "start", "repeat", "alert", "notes"
        ]
    );
    assert_eq!(r.form.values.due, "2026-11-18");
    assert!(!r.form.values.repeat.is_empty());
    assert!(preview_of(&r, "repeat").unwrap().starts_with('→'));
    assert!(r.form.alert_editable);
}

#[test]
fn form_block_for_a_checklist_item() {
    let fx = fx();
    let td = start(&fx);
    let r = get(&td, "i1");
    assert_eq!(field_names(&r), ["title", "notes"]);
}

#[test]
fn form_block_keeps_unwritable_rule() {
    let rule = "FREQ=MONTHLY;BYSETPOS=2;BYDAY=MO,TU";
    assert_eq!(twydo_core::repeatinput::to_text(rule, false), None);
    let fx = Fixture::new(&[]);
    fx.write(
        "inbox/kr.ics",
        &ics(
            "kr",
            "Odd rule",
            &format!("DUE:20261118\r\nRRULE:{rule}\r\n"),
        ),
    );
    let td = start(&fx);
    let r = get(&td, "kr");
    assert_eq!(r.form.values.repeat, "");
    assert!(preview_of(&r, "repeat").unwrap().ends_with("(kept)"));
}

#[test]
fn form_block_alert_read_only() {
    let fx = Fixture::new(&[]);
    let alarm =
        |t: &str| format!("BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:{t}\r\nEND:VALARM\r\n");
    fx.write(
        "inbox/two.ics",
        &ics(
            "two",
            "Two alarms",
            &format!(
                "DUE:20261118T100000\r\n{}{}",
                alarm("-PT15M"),
                alarm("-PT1H")
            ),
        ),
    );
    let td = start(&fx);
    let r = get(&td, "two");
    assert!(!r.form.alert_editable);
}

fn form_call(td: &TestDaemon, params: serde_json::Value) -> TasksFormResult {
    td.client().call("tasks.form", params).unwrap()
}

fn values_of(r: &TasksGetResult) -> serde_json::Value {
    serde_json::to_value(&r.form.values).unwrap()
}

fn typed(r: &TasksGetResult, edits: serde_json::Value) -> serde_json::Value {
    let mut v = values_of(r);
    for (k, x) in edits.as_object().unwrap() {
        v[k] = x.clone();
    }
    v
}

fn fields_of(r: &TasksFormResult) -> Vec<String> {
    r.form
        .fields
        .iter()
        .map(|f| {
            serde_json::to_value(f)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}

#[test]
fn form_previews_and_op_for_an_edit() {
    let td = start(&fx());
    let g = get(&td, "rep");
    let r = form_call(
        &td,
        json!({ "uid": "rep", "initial": values_of(&g), "values": typed(&g, json!({"due": "tomorrow"})) }),
    );
    let due = r
        .form
        .previews
        .iter()
        .find(|p| serde_json::to_value(p.field).unwrap() == "due")
        .unwrap();
    assert_eq!(due.text, "→ Wed 18 Nov");
    let op = serde_json::to_value(r.op.as_ref().unwrap()).unwrap();
    assert_eq!(op["Save"]["uid"], "rep");
    assert_eq!(op["Save"]["to"], serde_json::Value::Null);
    assert_eq!(op["Save"]["changes"].as_array().unwrap().len(), 1);
    assert!(op["Save"]["changes"][0].get("Due").is_some());
    assert!(!r.changed_since);
    assert!(r.rebased.is_none());
    assert_eq!(
        (r.revision, r.epoch.as_str()),
        (g.revision, g.epoch.as_str())
    );
}

#[test]
fn form_errors() {
    let td = start(&fx());
    let g = get(&td, "rep");
    let r = form_call(&td, json!({ "uid": "rep", "values": values_of(&g) }));
    assert!(r.op.is_none());
    assert_eq!(r.error.as_deref(), Some("no changes"));
    let r = form_call(
        &td,
        json!({ "uid": "rep", "values": typed(&g, json!({"title": ""})) }),
    );
    assert_eq!(r.error.as_deref(), Some("title must not be empty"));
    let r = form_call(
        &td,
        json!({ "uid": "rep", "values": typed(&g, json!({"due": "zzz"})) }),
    );
    assert!(r.op.is_none());
    assert!(r.error.unwrap().starts_with("due:"));
    let p = r
        .form
        .previews
        .iter()
        .find(|p| serde_json::to_value(p.field).unwrap() == "due")
        .unwrap();
    assert!(!p.text.starts_with('→'));
}

#[test]
fn form_type_change_changes_fields() {
    let td = start(&fx());
    let g = get(&td, "rep");
    let r = form_call(
        &td,
        json!({ "uid": "rep", "values": typed(&g, json!({"kind": "Checklist"})) }),
    );
    assert_eq!(fields_of(&r), field_names(&g));
    let r = form_call(
        &td,
        json!({ "new": {"collection": "inbox", "parent": "chk"} }),
    );
    assert_eq!(fields_of(&r), ["title", "notes"]);
}

#[test]
fn form_for_a_new_task() {
    let td = start(&fx());
    let r = form_call(
        &td,
        json!({ "new": {"collection": "inbox", "due": "2026-11-17"} }),
    );
    assert_eq!(r.form.values.due, "today");
    assert!(!fields_of(&r).contains(&"alert".to_string()));
    assert!(r.op.is_none());
    let mut v = serde_json::to_value(&r.form.values).unwrap();
    v["title"] = json!("Synthetic new");
    let r = form_call(
        &td,
        json!({ "new": {"collection": "inbox", "due": "2026-11-17"}, "values": v }),
    );
    let op = serde_json::to_value(r.op.unwrap()).unwrap();
    assert_eq!(op["Create"]["due"], json!({"Date": "2026-11-17"}));
    // Another date is ignored.
    let r = form_call(
        &td,
        json!({ "new": {"collection": "inbox", "due": "2026-11-20"} }),
    );
    assert_eq!(r.form.values.due, "");
}

#[test]
fn form_for_a_new_item() {
    let td = start(&fx());
    let r = form_call(
        &td,
        json!({ "new": {"collection": "inbox", "parent": "chk"} }),
    );
    let mut v = serde_json::to_value(&r.form.values).unwrap();
    v["title"] = json!("Synthetic item");
    let r = form_call(
        &td,
        json!({ "new": {"collection": "inbox", "parent": "chk"}, "values": v }),
    );
    let op = serde_json::to_value(r.op.unwrap()).unwrap();
    assert_eq!(op["Create"]["parent"], "chk");
}

#[test]
fn changed_elsewhere_rebases() {
    let fx = fx();
    let td = start(&fx);
    let g = get(&td, "rep");
    let rev = td.touch_outside(
        "inbox/rep.ics",
        &ics(
            "rep",
            "Water elsewhere",
            "DUE:20261117T180000\r\nRRULE:FREQ=DAILY\r\nDESCRIPTION:Elsewhere\r\n",
        ),
    );
    assert!(rev > g.revision);
    let r = form_call(
        &td,
        json!({
            "uid": "rep",
            "initial": values_of(&g),
            "values": typed(&g, json!({"title": "Typed title", "notes": "Typed notes"})),
        }),
    );
    assert!(r.changed_since);
    let rb = r.rebased.unwrap();
    let over: Vec<_> = rb
        .overwritten
        .iter()
        .map(|f| serde_json::to_value(f).unwrap())
        .collect();
    assert_eq!(over, [json!("title"), json!("notes")]);
    assert_eq!(rb.initial.title, "Water elsewhere");
    assert_eq!(r.form.values.title, "Typed title");
    let op = serde_json::to_value(r.op.unwrap()).unwrap();
    let changes = op["Save"]["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 2);
    assert!(changes.iter().all(|c| {
        let k = c.as_object().unwrap().keys().next().unwrap().as_str();
        k == "Summary" || k == "Description"
    }));
    // Reviewed: the rebased starting point is current, so the next check is clean.
    let r = follow_up(&td, json!({ "uid": "rep" }), &r.form.values, &rb);
    assert!(!r.changed_since);
    assert!(r.rebased.is_none());
    assert_eq!(change_keys(&r), ["Summary", "Description"]);
}

/// The next check after Review: the rebased `initial` and `basis` with the typed values.
fn follow_up(
    td: &TestDaemon,
    target: serde_json::Value,
    values: &twydo_core::form::FormValues,
    rb: &twydo_daemon::protocol::tasks::RebasedForm,
) -> TasksFormResult {
    let mut p = target;
    p["initial"] = serde_json::to_value(&rb.initial).unwrap();
    p["opened"] = serde_json::to_value(&rb.basis).unwrap();
    p["values"] = serde_json::to_value(values).unwrap();
    form_call(td, p)
}

fn names(fields: &[twydo_core::form::Field]) -> Vec<String> {
    fields.iter().map(|f| f.name().to_string()).collect()
}

/// The change kinds of a `Save` op, in order.
fn change_keys(r: &TasksFormResult) -> Vec<String> {
    let op = serde_json::to_value(r.op.as_ref().expect("an op")).unwrap();
    op["Save"]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_object().unwrap().keys().next().unwrap().clone())
        .collect()
}

#[test]
fn repeat_changed_elsewhere_is_reviewed() {
    let fx = fx();
    // JSON metadata, which a repeat write needs.
    const META: &str = "X-2DOAPP-METADATA:<2Do Meta>%7B%22DisplayOrder%22%3A0%7D</2Do Meta>\r\n";
    fx.write(
        "inbox/plain.ics",
        &ics(
            "plain",
            "Sort mail",
            &format!("DUE:20261117T180000\r\n{META}"),
        ),
    );
    let td = start(&fx);
    let g = get(&td, "plain");
    td.touch_outside(
        "inbox/plain.ics",
        &ics(
            "plain",
            "Sort mail",
            &format!("DUE:20261117T180000\r\nRRULE:FREQ=WEEKLY;UNTIL=20261231T170000Z\r\n{META}"),
        ),
    );
    // The field cannot show a timed end: the starting text is unchanged, the rule is not.
    assert_eq!(get(&td, "plain").form.values, g.form.values);
    let r = form_call(
        &td,
        json!({
            "uid": "plain",
            "initial": values_of(&g),
            "opened": g.form.basis,
            "values": typed(&g, json!({"repeat": "weekly"})),
        }),
    );
    assert!(r.changed_since);
    let rb = r.rebased.clone().unwrap();
    assert_eq!(names(&rb.overwritten), ["repeat"]);
    assert!(rb.dropped.is_empty());
    assert_eq!(rb.basis, r.form.basis);
    assert_ne!(rb.basis, g.form.basis);
    let r = follow_up(&td, json!({ "uid": "plain" }), &r.form.values, &rb);
    assert!(!r.changed_since);
    assert_eq!(change_keys(&r), ["Repeat"]);
    let c = td.client();
    let based = BasedOn {
        epoch: r.epoch.clone(),
        revision: r.revision,
    };
    let w = submit_at(&c, based, &c.next_request_id(), r.op.unwrap()).unwrap();
    assert_eq!(w.outcome, WriteOutcome::Applied);
    let text = fx.read("inbox/plain.ics");
    assert!(text.contains("RRULE:FREQ=WEEKLY"), "{text}");
    assert!(!text.contains("UNTIL"), "{text}");
}

/// A project `proj` with one item `pi`, for parent-kind changes.
fn with_project(fx: &Fixture) {
    fx.write(
        "inbox/proj.ics",
        &ics(
            "proj",
            "Move house",
            "X-2DOAPP-METADATA:@tasktype:project\r\n",
        ),
    );
    fx.write(
        "inbox/pi.ics",
        &ics("pi", "Book van", "RELATED-TO;RELTYPE=PARENT:proj\r\n"),
    );
}

fn project_becomes_checklist(td: &TestDaemon) {
    td.touch_outside(
        "inbox/proj.ics",
        &ics(
            "proj",
            "Move house",
            "X-2DOAPP-METADATA:@tasktype:checklist\r\n",
        ),
    );
}

#[test]
fn new_item_parent_kind_changed_drops_typing() {
    let fx = fx();
    with_project(&fx);
    let td = start(&fx);
    let target = json!({ "new": {"collection": "inbox", "parent": "proj"} });
    let r0 = form_call(&td, target.clone());
    assert!(fields_of(&r0).contains(&"due".to_string()));
    let mut v = r0.form.values.clone();
    v.title = "Synthetic item".into();
    v.due = "tomorrow".into();
    project_becomes_checklist(&td);
    let mut p = target.clone();
    p["initial"] = serde_json::to_value(&r0.form.values).unwrap();
    p["opened"] = serde_json::to_value(&r0.form.basis).unwrap();
    p["values"] = serde_json::to_value(&v).unwrap();
    let r = form_call(&td, p);
    assert!(r.changed_since);
    let rb = r.rebased.clone().unwrap();
    assert_eq!(names(&rb.dropped), ["due"]);
    assert!(rb.overwritten.is_empty());
    assert_eq!(fields_of(&r), ["title", "notes"]);
    assert_eq!(r.form.values.title, "Synthetic item");
    let r = follow_up(&td, target, &r.form.values, &rb);
    assert!(!r.changed_since);
    let op = serde_json::to_value(r.op.unwrap()).unwrap();
    assert_eq!(op["Create"]["summary"], "Synthetic item");
    assert_eq!(op["Create"]["parent"], "proj");
    assert_eq!(op["Create"]["due"], serde_json::Value::Null);
}

#[test]
fn edit_parent_kind_changed_drops_typing() {
    let fx = fx();
    with_project(&fx);
    let td = start(&fx);
    let g = get(&td, "pi");
    project_becomes_checklist(&td);
    // The item's own text is unchanged; only its parent's kind (its fields) differs.
    assert_eq!(get(&td, "pi").form.values, g.form.values);
    let r = form_call(
        &td,
        json!({
            "uid": "pi",
            "initial": values_of(&g),
            "opened": g.form.basis,
            "values": typed(&g, json!({"title": "Book a van", "due": "tomorrow"})),
        }),
    );
    assert!(r.changed_since);
    let rb = r.rebased.clone().unwrap();
    assert_eq!(names(&rb.dropped), ["due"]);
    assert!(rb.overwritten.is_empty());
    assert_eq!(fields_of(&r), ["title", "notes"]);
    let r = follow_up(&td, json!({ "uid": "pi" }), &r.form.values, &rb);
    assert!(!r.changed_since);
    assert_eq!(change_keys(&r), ["Summary"]);
}

#[test]
fn queued_edit_shows_in_the_form() {
    let fx = fx();
    let cfg = fx.config();
    let state = bind_vdir_state(&cfg.state_dir(), &cfg.vdir).unwrap();
    let lock = VdirLock::exclusive(&state).unwrap();
    let op = Op::Edit {
        uid: "chk".into(),
        changes: vec![Change::Summary("Pack, queued".into())],
    };
    let at = Utc.with_ymd_and_hms(2026, 11, 9, 12, 0, 0).unwrap();
    common::queue_op(&cfg, &fx.snapshot(), &op, "edit", None, at);
    drop(lock);
    // A sync holds the vdir: the queued edit is not written yet.
    let _sync = VdirLock::shared(&state).unwrap();
    let td = start(&fx);
    let g = get(&td, "chk");
    assert!(g.task.pending);
    assert_eq!(g.form.values.title, "Pack, queued");
    let first = form_call(&td, json!({ "uid": "chk" }));
    assert_eq!(first.form.values, g.form.values);
    assert_eq!(first.form.basis, g.form.basis);
    let r = form_call(
        &td,
        json!({
            "uid": "chk",
            "initial": values_of(&g),
            "opened": g.form.basis,
            "values": typed(&g, json!({"notes": "Typed notes"})),
        }),
    );
    assert!(!r.changed_since);
    assert_eq!(r.form.values.title, "Pack, queued");
    assert_eq!(change_keys(&r), ["Description"]);
}

#[test]
fn form_unknown_uid_and_bad_params() {
    let td = start(&fx());
    for params in [
        json!({ "uid": "nope" }),
        json!({}),
        json!({ "uid": "rep", "new": {"collection": "inbox"} }),
        json!({ "new": {"collection": "inbox", "parent": "nope"} }),
    ] {
        let err = td
            .client()
            .call::<_, TasksFormResult>("tasks.form", params)
            .unwrap_err();
        assert_eq!(kind(&err), ErrorKind::Invalid);
    }
}
