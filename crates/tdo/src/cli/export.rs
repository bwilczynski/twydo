//! `tdo export-json`: schema v1 dump consumed by `tools/reconcile.py`.

use std::path::Path;

use chrono::SecondsFormat;
use serde_json::{Map, Value, json};
use twydo_core::ical::value::{IcalDateTime, Resolved};
use twydo_core::model::{Status, Task, TaskKind, Trigger};
use twydo_core::views::Snapshot;

use super::{CliError, load_snapshot, task_rule_class};

pub fn run(vdir: Option<&Path>) -> Result<(), CliError> {
    let (snap, _errors) = load_snapshot(vdir)?;
    let out = export(&snap);
    println!("{}", serde_json::to_string_pretty(&out).expect("json"));
    Ok(())
}

fn date_string(v: &IcalDateTime, snap: &Snapshot) -> String {
    match v {
        IcalDateTime::Date(d) => d.format("%Y-%m-%d").to_string(),
        IcalDateTime::Floating(n) => n.format("%Y-%m-%dT%H:%M:%S").to_string(),
        IcalDateTime::Utc(t) => t.to_rfc3339_opts(SecondsFormat::Secs, true),
        IcalDateTime::Zoned { .. } => match v.resolve(snap.tz()).0 {
            Resolved::Instant(t) => t.to_rfc3339_opts(SecondsFormat::Secs, false),
            Resolved::Date(d) => d.format("%Y-%m-%d").to_string(),
        },
    }
}

fn opt_date(v: &Option<IcalDateTime>, snap: &Snapshot) -> Value {
    v.as_ref()
        .map_or(Value::Null, |d| Value::String(date_string(d, snap)))
}

fn is_date(v: &Option<IcalDateTime>) -> bool {
    v.as_ref().is_some_and(|d| d.is_date_only())
}

/// Names of the `X-2DO*` properties on the resource's VTODO, sorted. Re-reads the file
/// (read-only); an unreadable file yields none.
fn x2do_props(path: &Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let Ok(cal) = twydo_core::ical::parse(&bytes) else {
        return Vec::new();
    };
    let mut names: Vec<String> = cal
        .root
        .children_named("VTODO")
        .flat_map(|c| c.props.iter())
        .filter(|p| p.name.starts_with("X-2DO"))
        .map(|p| p.name.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

fn task_json(t: &Task, snap: &Snapshot) -> Value {
    let kind = match t.kind {
        TaskKind::Task => "task",
        TaskKind::Project => "project",
        TaskKind::Checklist => "checklist",
    };
    let status = match t.status {
        Status::Open => "open",
        Status::Completed => "completed",
        Status::Cancelled => "cancelled",
    };
    let alarms: Vec<Value> = t
        .alarms
        .iter()
        .map(|a| match &a.trigger {
            Trigger::Relative {
                offset,
                related_end,
            } => json!({"relative_seconds": offset.num_seconds(), "related_end": related_end}),
            Trigger::Absolute(at) => json!({
                "absolute": at.to_rfc3339_opts(SecondsFormat::Secs, true),
                "related_end": false,
            }),
        })
        .collect();
    let tokens: Map<String, Value> = t
        .meta
        .tokens
        .iter()
        .map(|(k, v)| (k.clone(), v.clone().map_or(Value::Null, Value::String)))
        .collect();
    json!({
        "uid": t.uid,
        "collection": t.collection,
        "summary": t.summary,
        "kind": kind,
        "status": status,
        "due": opt_date(&t.due, snap),
        "due_is_date": is_date(&t.due),
        "start": opt_date(&t.start, snap),
        "start_is_date": is_date(&t.start),
        "completed": opt_date(&t.completed, snap),
        "rrule": t.rrule,
        "rule_class": task_rule_class(t, snap),
        "from_completion": t.from_completion,
        "parent": t.parent,
        "alarms": alarms,
        "x2do_props": x2do_props(&t.path),
        "meta_tokens": tokens,
        "warnings": t.warnings,
        "blocked": t.blocked,
    })
}

pub fn export(snap: &Snapshot) -> Value {
    let mut cols: Vec<_> = snap.collections.iter().collect();
    cols.sort_by(|a, b| a.id.cmp(&b.id));
    let collections: Vec<Value> = cols
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "display_name": c.display_name,
                "color": c.color.map(|(r, g, b)| format!("#{r:02X}{g:02X}{b:02X}")),
            })
        })
        .collect();
    let mut tasks: Vec<&Task> = snap.tasks.iter().collect();
    tasks.sort_by(|a, b| (&a.collection, &a.uid).cmp(&(&b.collection, &b.uid)));
    json!({
        "schema": 1,
        "collections": collections,
        "tasks": tasks.iter().map(|t| task_json(t, snap)).collect::<Vec<_>>(),
    })
}
