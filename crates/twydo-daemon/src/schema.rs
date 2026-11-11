//! The OpenRPC document generated from the protocol types, and the breaking-change check
//! against the last released document (daemon spec §6.2, §6.3).

use std::collections::BTreeSet;

use schemars::generate::SchemaSettings;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::PROTOCOL;
use crate::protocol::admin::{
    AbandonResult, ConflictList, ConflictsResolveParams, InputParseParams, InputParsed,
    JournalAbandonParams, JournalEntryView, JournalRevertParams, RecoverResult, ResolveResult,
};
use crate::protocol::events::{EventEnvelope, Heartbeat, Missed};
use crate::protocol::jobs::{JobGetParams, JobStarted, JobView};
use crate::protocol::lists::{
    BackupPreviewParams, Backups, ForgetPreview, ListArchiveParams, ListCreateParams,
    ListDeleteParams, ListExportParams, ListForgetParams, ListPreviewDeleteParams,
    ListPreviewForgetParams, ListReconcileParams, ListRecoverParams, ListUpdateParams,
    PendingListOp,
};
use crate::protocol::methods::*;
use crate::protocol::reports::{CheckReport, CheckReportParams, ExportResult};
use crate::protocol::rpc::ErrorKind;
use crate::protocol::sync::{SyncRunParams, SyncSetAutoParams, SyncSetAutoResult, SyncStopResult};
use crate::protocol::wire::WireSnapshot;

/// An empty params or result object.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Empty {}

/// One method of the protocol. Plans 2 and 3 add rows.
pub struct MethodSpec {
    pub name: &'static str,
    /// Lifecycle methods are frozen across protocol versions (spec §6.3).
    pub lifecycle: bool,
    pub params: fn(&mut SchemaGenerator) -> Schema,
    pub result: fn(&mut SchemaGenerator) -> Schema,
}

fn of<T: JsonSchema>(g: &mut SchemaGenerator) -> Schema {
    g.subschema_for::<T>()
}

/// Any JSON object (the OpenRPC document itself).
fn any_object(_: &mut SchemaGenerator) -> Schema {
    Schema::try_from(json!({"type": "object"})).expect("object schema")
}

const fn m(
    name: &'static str,
    lifecycle: bool,
    params: fn(&mut SchemaGenerator) -> Schema,
    result: fn(&mut SchemaGenerator) -> Schema,
) -> MethodSpec {
    MethodSpec {
        name,
        lifecycle,
        params,
        result,
    }
}

pub const METHODS: &[MethodSpec] = &[
    m("hello", true, of::<HelloParams>, of::<HelloResult>),
    m("rpc.discover", true, of::<Empty>, any_object),
    m("daemon.info", true, of::<Empty>, of::<DaemonInfo>),
    m("daemon.shutdown", true, of::<ShutdownParams>, of::<Empty>),
    m(
        "events.subscribe",
        false,
        of::<Empty>,
        of::<SubscribeResult>,
    ),
    m("events.unsubscribe", false, of::<Empty>, of::<Empty>),
    m("snapshot.get", false, of::<Empty>, of::<WireSnapshot>),
    m("status.get", false, of::<Empty>, of::<StatusView>),
    m("op.submit", false, of::<SubmitParams>, of::<WriteResult>),
    m("op.undo", false, of::<UndoParams>, of::<WriteResult>),
    m("op.redo", false, of::<UndoParams>, of::<WriteResult>),
    m("op.get", false, of::<OpGetParams>, of::<OpState>),
    m(
        "check.report",
        false,
        of::<CheckReportParams>,
        of::<CheckReport>,
    ),
    m("export", false, of::<Empty>, of::<ExportResult>),
    m("op.check", false, of::<CheckParams>, of::<CheckResult>),
    m("refusals.ack", false, of::<AckParams>, of::<AckResult>),
    m("sync.run", false, of::<SyncRunParams>, of::<JobStarted>),
    m("sync.stop", false, of::<Empty>, of::<SyncStopResult>),
    m(
        "sync.set_auto",
        false,
        of::<SyncSetAutoParams>,
        of::<SyncSetAutoResult>,
    ),
    m("job.get", false, of::<JobGetParams>, of::<JobView>),
    m(
        "lists.create",
        false,
        of::<ListCreateParams>,
        of::<JobStarted>,
    ),
    m(
        "lists.update",
        false,
        of::<ListUpdateParams>,
        of::<JobStarted>,
    ),
    m(
        "lists.archive",
        false,
        of::<ListArchiveParams>,
        of::<JobStarted>,
    ),
    m(
        "lists.unarchive",
        false,
        of::<ListArchiveParams>,
        of::<JobStarted>,
    ),
    m(
        "lists.preview_delete",
        false,
        of::<ListPreviewDeleteParams>,
        of::<JobStarted>,
    ),
    m(
        "lists.delete",
        false,
        of::<ListDeleteParams>,
        of::<JobStarted>,
    ),
    m(
        "lists.reconcile",
        false,
        of::<ListReconcileParams>,
        of::<JobStarted>,
    ),
    m(
        "lists.export",
        false,
        of::<ListExportParams>,
        of::<JobStarted>,
    ),
    m(
        "lists.preview_forget",
        false,
        of::<ListPreviewForgetParams>,
        of::<ForgetPreview>,
    ),
    m(
        "lists.forget",
        false,
        of::<ListForgetParams>,
        of::<JobStarted>,
    ),
    m(
        "lists.recover",
        false,
        of::<ListRecoverParams>,
        of::<JobStarted>,
    ),
    m(
        "lists.pending",
        false,
        of::<Empty>,
        of::<Option<PendingListOp>>,
    ),
    m("lists.backups", false, of::<Empty>, of::<Backups>),
    m(
        "lists.backup_preview",
        false,
        of::<BackupPreviewParams>,
        of::<WireSnapshot>,
    ),
    m("conflicts.list", false, of::<Empty>, of::<ConflictList>),
    m(
        "conflicts.resolve",
        false,
        of::<ConflictsResolveParams>,
        of::<ResolveResult>,
    ),
    m(
        "journal.list",
        false,
        of::<Empty>,
        of::<Vec<JournalEntryView>>,
    ),
    m("journal.recover", false, of::<Empty>, of::<RecoverResult>),
    m(
        "journal.revert",
        false,
        of::<JournalRevertParams>,
        of::<WriteResult>,
    ),
    m(
        "journal.abandon",
        false,
        of::<JournalAbandonParams>,
        of::<AbandonResult>,
    ),
    m(
        "input.parse",
        false,
        of::<InputParseParams>,
        of::<InputParsed>,
    ),
];

fn to_value(s: Schema) -> Value {
    serde_json::to_value(s).expect("schema serializes")
}

/// Recursively rebuild every object with sorted keys so the output is stable.
fn sorted(v: Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<String> = map.keys().cloned().collect();
            keys.sort();
            let mut out = Map::new();
            let mut map = map;
            for k in keys {
                let val = map.remove(&k).expect("key present");
                out.insert(k, sorted(val));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.into_iter().map(sorted).collect()),
        other => other,
    }
}

/// The OpenRPC 1.3 document for the current protocol.
pub fn document() -> Value {
    let mut settings = SchemaSettings::draft07();
    settings.definitions_path = "/components/schemas".into();
    let mut g = SchemaGenerator::new(settings);

    let methods: Vec<Value> = METHODS
        .iter()
        .map(|spec| {
            json!({
                "name": spec.name,
                "params": [{"name": "params", "required": true, "schema": to_value((spec.params)(&mut g))}],
                "result": {"name": "result", "schema": to_value((spec.result)(&mut g))},
                "x-lifecycle": spec.lifecycle,
            })
        })
        .collect();

    let events = json!({
        "event": {"params": to_value(g.subschema_for::<EventEnvelope>())},
        "missed": {"params": to_value(g.subschema_for::<Missed>())},
        "heartbeat": {"params": to_value(g.subschema_for::<Heartbeat>())},
    });

    // Register ErrorKind and list its values under x-errors.
    g.subschema_for::<ErrorKind>();
    let defs = g.take_definitions(true);
    let errors = defs
        .get("ErrorKind")
        .and_then(enum_values)
        .map(Value::Array)
        .unwrap_or_else(|| json!([]));

    sorted(json!({
        "openrpc": "1.3.2",
        "info": {"title": "tdo", "version": PROTOCOL.to_string()},
        "x-protocol": PROTOCOL,
        "methods": methods,
        "x-events": events,
        "x-errors": errors,
        "components": {"schemas": Value::Object(defs)},
    }))
}

// ---------------------------------------------------------------------------------------------
// Breaking changes
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Role {
    Params,
    Other,
}

struct Walk<'a> {
    released: &'a Map<String, Value>,
    current: &'a Map<String, Value>,
    out: BTreeSet<String>,
    stack: Vec<String>,
}

fn defs(doc: &Value) -> &Map<String, Value> {
    static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
    doc.pointer("/components/schemas")
        .and_then(Value::as_object)
        .unwrap_or_else(|| EMPTY.get_or_init(Map::new))
}

/// Follow `$ref`s; return the definition name (if any) and the resolved schema.
fn deref<'a>(defs: &'a Map<String, Value>, mut s: &'a Value) -> (Option<String>, &'a Value) {
    let mut name = None;
    for _ in 0..16 {
        let Some(r) = s.get("$ref").and_then(Value::as_str) else {
            break;
        };
        let n = r.rsplit('/').next().unwrap_or(r).to_string();
        match defs.get(&n) {
            Some(d) => {
                name = Some(n);
                s = d;
            }
            None => break,
        }
    }
    (name, s)
}

fn strings(v: Option<&Value>) -> Vec<Value> {
    v.and_then(Value::as_array).cloned().unwrap_or_default()
}

/// Key a `oneOf`/`anyOf` variant: its tag value, ref name, or shape.
fn variant_key(defs: &Map<String, Value>, v: &Value, index: usize) -> String {
    let (name, r) = deref(defs, v);
    if let Some(c) = r.get("const") {
        return format!("const={c}");
    }
    if let Some(props) = r.get("properties").and_then(Value::as_object) {
        for (k, p) in props {
            if let Some(c) = p.get("const") {
                return format!("{k}={c}");
            }
            if let Some([only]) = p.get("enum").and_then(Value::as_array).map(Vec::as_slice) {
                return format!("{k}={only}");
            }
        }
    }
    if let Some(n) = name {
        return n;
    }
    // Externally tagged variant: an object with exactly one property, named after the variant.
    if let Some(props) = r.get("properties").and_then(Value::as_object)
        && props.len() == 1
    {
        return format!("tag={}", props.keys().next().expect("one key"));
    }
    if let Some(e) = r.get("enum") {
        return format!("enum:{e}");
    }
    if let Some(t) = r.get("type") {
        return format!("type:{t}");
    }
    format!("#{index}")
}

/// The values of a string enum, whether schemars emitted `enum: [..]` or, for documented
/// variants, `oneOf: [{const: ..}, ..]`.
fn enum_values(schema: &Value) -> Option<Vec<Value>> {
    if let Some(e) = schema.get("enum").and_then(Value::as_array) {
        return Some(e.clone());
    }
    let variants = schema.get("oneOf").and_then(Value::as_array)?;
    variants
        .iter()
        .map(|v| match v.get("const") {
            Some(c) => Some(c.clone()),
            None => match v.get("enum").and_then(Value::as_array).map(Vec::as_slice) {
                Some([only]) => Some(only.clone()),
                _ => None,
            },
        })
        .collect()
}

/// The set of JSON types a schema accepts; a nullable `anyOf`/`oneOf` counts as `"null"`.
fn type_set(defs: &Map<String, Value>, schema: &Value) -> BTreeSet<String> {
    let (_, s) = deref(defs, schema);
    let mut out = BTreeSet::new();
    match s.get("type") {
        Some(Value::String(t)) => {
            out.insert(t.clone());
        }
        Some(Value::Array(a)) => out.extend(a.iter().filter_map(Value::as_str).map(String::from)),
        _ => {}
    }
    for kw in ["anyOf", "oneOf"] {
        if let Some(vs) = s.get(kw).and_then(Value::as_array) {
            for v in vs {
                out.extend(type_set(defs, v));
            }
        }
    }
    out
}

/// Both `oneOf` and `anyOf` variants of a schema, in order.
fn variants(s: &Map<String, Value>) -> Vec<&Value> {
    ["oneOf", "anyOf"]
        .iter()
        .filter_map(|kw| s.get(*kw).and_then(Value::as_array))
        .flatten()
        .collect()
}

impl Walk<'_> {
    fn walk(&mut self, label: &str, rel: &Value, cur: &Value, role: Role) {
        let (rname, rel) = deref(self.released, rel);
        let (cname, cur) = deref(self.current, cur);
        let mut label = label.to_string();
        if let Some(n) = &rname {
            label = n.clone();
            if let Some(c) = &cname
                && c != n
            {
                self.out
                    .insert(format!("{label}: definition replaced by {c}"));
                return;
            }
            if self.stack.contains(n) {
                return;
            }
            self.stack.push(n.clone());
            self.walk_resolved(&label, rel, cur, role);
            self.stack.pop();
        } else {
            self.walk_resolved(&label, rel, cur, role);
        }
    }

    fn walk_resolved(&mut self, label: &str, rel: &Value, cur: &Value, role: Role) {
        let (Some(r), Some(c)) = (rel.as_object(), cur.as_object()) else {
            return;
        };

        if let Some(rp) = r.get("properties").and_then(Value::as_object) {
            let cp = c.get("properties").and_then(Value::as_object);
            for (k, rv) in rp {
                match cp.and_then(|p| p.get(k)) {
                    None => {
                        self.out.insert(format!("{label}.{k}: property removed"));
                    }
                    Some(cv) => self.walk(&format!("{label}.{k}"), rv, cv, role),
                }
            }
        }

        if role == Role::Params {
            let rr = strings(r.get("required"));
            for k in strings(c.get("required")) {
                if !rr.contains(&k) {
                    let k = k.as_str().unwrap_or("?");
                    self.out.insert(format!("{label}.{k}: now required"));
                }
            }
        }

        let (rt, ct) = (type_set(self.released, rel), type_set(self.current, cur));
        if !rt.is_empty() && !ct.is_empty() {
            let (what, bad): (&str, Vec<&String>) = match role {
                // `number` accepts every `integer`, so widening in params loses nothing.
                Role::Params => (
                    "no longer accepts",
                    rt.difference(&ct)
                        .filter(|t| !(t.as_str() == "integer" && ct.contains("number")))
                        .collect(),
                ),
                Role::Other => ("now may also be", ct.difference(&rt).collect()),
            };
            for t in bad {
                self.out.insert(format!("{label}: type {what} {t}"));
            }
        }

        if role == Role::Params
            && c.get("additionalProperties") == Some(&json!(false))
            && r.get("additionalProperties") != Some(&json!(false))
        {
            self.out
                .insert(format!("{label}: additional properties no longer allowed"));
        }

        // What clients receive (results, events) may gain values only where released clients
        // read an unknown value as `Unknown` (`x-open` in the released schema: opening an enum
        // now does not help the clients already out); params may always gain them.
        let closed_output = role == Role::Other && r.get("x-open") != Some(&json!(true));
        if let (Some(re), Some(ce)) = (enum_values(rel), enum_values(cur)) {
            for v in re.iter().filter(|v| !ce.contains(v)) {
                self.out.insert(format!("{label}: enum value {v} removed"));
            }
            if closed_output {
                for v in ce.iter().filter(|v| !re.contains(v)) {
                    self.out.insert(format!(
                        "{label}: enum value {v} added (clients cannot read it)"
                    ));
                }
            }
        }

        if let (Some(ri), Some(ci)) = (r.get("items"), c.get("items")) {
            self.walk(&format!("{label}[]"), ri, ci, role);
        }
        if let (Some(ra), Some(ca)) = (r.get("additionalProperties"), c.get("additionalProperties"))
            && ra.is_object()
            && ca.is_object()
        {
            self.walk(&format!("{label}{{}}"), ra, ca, role);
        }

        let (rv, cv) = (variants(r), variants(c));
        let ckeys: Vec<String> = cv
            .iter()
            .enumerate()
            .map(|(i, v)| variant_key(self.current, v, i))
            .collect();
        let mut rkeys = Vec::new();
        for (i, v) in rv.iter().enumerate() {
            let key = variant_key(self.released, v, i);
            match ckeys.iter().position(|k| *k == key) {
                None => {
                    self.out.insert(format!("{label}: variant {key} removed"));
                }
                Some(j) => self.walk(&format!("{label}({key})"), v, cv[j], role),
            }
            rkeys.push(key);
        }
        // Gained `null` is reported as a type change above.
        if closed_output && !rv.is_empty() {
            for key in ckeys.iter().filter(|k| !rkeys.contains(k)) {
                if key != "type:\"null\"" {
                    self.out.insert(format!(
                        "{label}: variant {key} added (clients cannot read it)"
                    ));
                }
            }
        }

        if let (Some(ra), Some(ca)) = (
            r.get("allOf").and_then(Value::as_array),
            c.get("allOf").and_then(Value::as_array),
        ) {
            // Match parts by content, not position: a part identical in both documents needs
            // no walk; the rest pair up in order.
            let norm = |defs: &Map<String, Value>, v: &Value| {
                strip_descriptions(expand(defs, v, &mut Vec::new()))
            };
            let cn: Vec<Value> = ca.iter().map(|v| norm(self.current, v)).collect();
            let mut taken = vec![false; ca.len()];
            let mut unmatched = Vec::new();
            for (i, v) in ra.iter().enumerate() {
                let rn = norm(self.released, v);
                match (0..ca.len()).find(|&j| !taken[j] && cn[j] == rn) {
                    Some(j) => taken[j] = true,
                    None => unmatched.push((i, v)),
                }
            }
            let mut free = (0..ca.len()).filter(|&j| !taken[j]);
            for (i, v) in unmatched {
                match free.next() {
                    Some(j) => self.walk(&format!("{label}(all#{i})"), v, &ca[j], role),
                    // Dropping a part loosens what clients may receive; for params it only
                    // loosens what the daemon accepts.
                    None if role == Role::Other => {
                        self.out.insert(format!("{label}: allOf part removed"));
                    }
                    None => {}
                }
            }
        }
    }
}

/// Replace every `$ref` by its definition (cycles stay as the ref) so two schemas compare whole.
fn expand(defs: &Map<String, Value>, v: &Value, stack: &mut Vec<String>) -> Value {
    match v {
        Value::Object(o) => {
            if let Some(r) = o.get("$ref").and_then(Value::as_str) {
                let n = r.rsplit('/').next().unwrap_or(r).to_string();
                if let Some(d) = defs.get(&n) {
                    if stack.contains(&n) {
                        return v.clone();
                    }
                    stack.push(n);
                    let e = expand(defs, d, stack);
                    stack.pop();
                    return e;
                }
            }
            Value::Object(
                o.iter()
                    .map(|(k, x)| (k.clone(), expand(defs, x, stack)))
                    .collect(),
            )
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| expand(defs, x, stack)).collect()),
        other => other.clone(),
    }
}

/// Drop every `description` key (documentation, not shape) so schemas compare by content.
fn strip_descriptions(v: Value) -> Value {
    match v {
        Value::Object(o) => Value::Object(
            o.into_iter()
                .filter(|(k, x)| !(k == "description" && x.is_string()))
                .map(|(k, x)| (k, strip_descriptions(x)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.into_iter().map(strip_descriptions).collect()),
        other => other,
    }
}

fn method_schema<'a>(m: &'a Value, which: &str) -> Option<&'a Value> {
    match which {
        "params" => m.pointer("/params/0/schema"),
        _ => m.pointer("/result/schema"),
    }
}

/// One line per breaking change from `released` to `current`.
pub fn breaking_changes(released: &Value, current: &Value) -> Vec<String> {
    let mut w = Walk {
        released: defs(released),
        current: defs(current),
        out: BTreeSet::new(),
        stack: Vec::new(),
    };
    let empty = Vec::new();
    let cur_methods = current["methods"].as_array().unwrap_or(&empty);

    for rm in released["methods"].as_array().unwrap_or(&empty) {
        let name = rm["name"].as_str().unwrap_or("?");
        let Some(cm) = cur_methods.iter().find(|m| m["name"] == rm["name"]) else {
            w.out.insert(format!("{name}: method removed"));
            continue;
        };
        for which in ["params", "result"] {
            let (Some(rs), Some(cs)) = (method_schema(rm, which), method_schema(cm, which)) else {
                continue;
            };
            if rm["x-lifecycle"] == json!(true) {
                let a = strip_descriptions(expand(w.released, rs, &mut Vec::new()));
                let b = strip_descriptions(expand(w.current, cs, &mut Vec::new()));
                if a != b {
                    w.out
                        .insert(format!("{name}: lifecycle method {which} changed (frozen)"));
                }
            }
            let role = if which == "params" {
                Role::Params
            } else {
                Role::Other
            };
            w.walk(&format!("{name}.{which}"), rs, cs, role);
        }
    }

    if let Some(revents) = released["x-events"].as_object() {
        for (name, re) in revents {
            match current["x-events"].get(name) {
                None => {
                    w.out.insert(format!("event {name}: removed"));
                }
                Some(ce) => w.walk(
                    &format!("event {name}"),
                    &re["params"],
                    &ce["params"],
                    Role::Other,
                ),
            }
        }
    }

    let cur_errors = strings(current.get("x-errors"));
    for e in strings(released.get("x-errors")) {
        if !cur_errors.contains(&e) {
            w.out.insert(format!("ErrorKind: enum value {e} removed"));
        }
    }

    w.out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_values_reads_both_shapes() {
        let plain = json!({"type": "string", "enum": ["a", "b"]});
        let documented =
            json!({"oneOf": [{"const": "a", "type": "string"}, {"const": "b", "type": "string"}]});
        assert_eq!(enum_values(&plain), Some(vec![json!("a"), json!("b")]));
        assert_eq!(enum_values(&documented), Some(vec![json!("a"), json!("b")]));
        assert_eq!(enum_values(&json!({"type": "object"})), None);
    }

    #[test]
    fn info_version_is_the_protocol_generation() {
        assert_eq!(document()["info"]["version"], json!("1"));
    }
}
