use serde_json::{Value, json};
use twydo_daemon::schema;

fn load(name: &str) -> Value {
    let path = format!("{}/../../schema/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn def_mut<'a>(doc: &'a mut Value, name: &str) -> &'a mut Value {
    doc.pointer_mut(&format!("/components/schemas/{name}"))
        .unwrap_or_else(|| panic!("no definition {name}"))
}

#[test]
fn checked_in_schema_is_current() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../schema/tdo.openrpc.json");
    let current = serde_json::to_string_pretty(&schema::document()).unwrap() + "\n";
    if std::env::var_os("TWYDO_BLESS").is_some() {
        std::fs::write(path, &current).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        current,
        "schema/tdo.openrpc.json is stale: run TWYDO_BLESS=1 cargo test -p twydo-daemon --test schema"
    );
}

#[test]
fn document_is_deterministic() {
    assert_eq!(
        serde_json::to_string_pretty(&schema::document()).unwrap(),
        serde_json::to_string_pretty(&schema::document()).unwrap()
    );
}

#[test]
fn no_breaking_change_against_the_released_schema() {
    let released = load("released.openrpc.json");
    assert_eq!(
        schema::breaking_changes(&released, &schema::document()),
        Vec::<String>::new()
    );
}

#[test]
fn removing_a_result_field_is_a_break() {
    let released = schema::document();
    let mut current = released.clone();
    def_mut(&mut current, "WriteResult")["properties"]
        .as_object_mut()
        .unwrap()
        .remove("revision")
        .unwrap();
    let lines = schema::breaking_changes(&released, &current);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("revision"), "{lines:?}");
}

#[test]
fn a_new_required_param_is_a_break() {
    let released = schema::document();
    let mut current = released.clone();
    def_mut(&mut current, "SubmitParams")["required"]
        .as_array_mut()
        .unwrap()
        .push(json!("x"));
    let lines = schema::breaking_changes(&released, &current);
    assert_eq!(lines, vec!["SubmitParams.x: now required".to_string()]);
}

#[test]
fn a_new_optional_param_is_not_a_break() {
    let released = schema::document();
    let mut current = released.clone();
    def_mut(&mut current, "SubmitParams")["properties"]["x"] = json!({"type": "string"});
    assert_eq!(
        schema::breaking_changes(&released, &current),
        Vec::<String>::new()
    );
}

#[test]
fn a_lost_error_kind_is_a_break() {
    let released = schema::document();
    let mut current = released.clone();
    let values = def_mut(&mut current, "ErrorKind")["enum"]
        .as_array_mut()
        .unwrap();
    values.retain(|v| v != "stale");
    let lines = schema::breaking_changes(&released, &current);
    assert!(lines.iter().any(|l| l.contains("stale")), "{lines:?}");
}

#[test]
fn a_gained_value_of_an_open_enum_is_not_a_break() {
    // ErrorKind is open: clients read a value they do not know as `unknown`.
    let released = schema::document();
    assert_eq!(
        def_mut(&mut released.clone(), "ErrorKind")["x-open"],
        json!(true)
    );
    let mut current = released.clone();
    def_mut(&mut current, "ErrorKind")["enum"]
        .as_array_mut()
        .unwrap()
        .push(json!("brand_new"));
    assert_eq!(
        schema::breaking_changes(&released, &current),
        Vec::<String>::new()
    );
}

#[test]
fn a_gained_value_of_a_closed_enum_in_a_reply_is_a_break() {
    // WriteMode reaches clients in the snapshot's config and is not open.
    let released = schema::document();
    let mut current = released.clone();
    let def = def_mut(&mut current, "WriteMode");
    assert!(def.get("x-open").is_none());
    def["enum"].as_array_mut().unwrap().push(json!("brand_new"));
    let b = schema::breaking_changes(&released, &current);
    assert!(
        b.iter()
            .any(|l| l.contains("WriteMode") && l.contains("\"brand_new\" added")),
        "{b:?}"
    );
}

#[test]
fn a_gained_variant_of_a_closed_tagged_enum_in_a_reply_is_a_break() {
    let mut released = schema::document();
    def_mut(&mut released, "OpState")
        .as_object_mut()
        .unwrap()
        .remove("x-open");
    let mut current = released.clone();
    def_mut(&mut current, "OpState")["oneOf"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "type": "object",
            "properties": {"state": {"type": "string", "const": "brand_new"}},
            "required": ["state"]
        }));
    let b = schema::breaking_changes(&released, &current);
    assert!(
        b.iter()
            .any(|l| l.contains("OpState") && l.contains("added")),
        "{b:?}"
    );
    // The same variant in the open OpState is fine.
    let released = schema::document();
    let mut current = released.clone();
    def_mut(&mut current, "OpState")["oneOf"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "type": "object",
            "properties": {"state": {"type": "string", "const": "brand_new"}},
            "required": ["state"]
        }));
    assert_eq!(
        schema::breaking_changes(&released, &current),
        Vec::<String>::new()
    );
}

#[test]
fn a_gained_value_of_a_closed_enum_in_params_is_not_a_break() {
    let doc = |values: Value| {
        json!({
            "methods": [{
                "name": "m",
                "params": [{"name": "params", "schema": {"$ref": "#/components/schemas/Mode"}}],
                "result": {"name": "result", "schema": {"type": "object"}}
            }],
            "components": {"schemas": {"Mode": {"type": "string", "enum": values}}}
        })
    };
    let released = doc(json!(["a"]));
    assert_eq!(
        schema::breaking_changes(&released, &doc(json!(["a", "b"]))),
        Vec::<String>::new()
    );
    assert_eq!(
        schema::breaking_changes(&released, &doc(json!([]))),
        vec!["Mode: enum value \"a\" removed".to_string()]
    );
}

#[test]
fn every_open_enum_reads_an_unknown_value() {
    use twydo_daemon::protocol::admin::{
        WireConflictKind, WireConflictState, WireResolution, WireSide, WireStage,
    };
    use twydo_daemon::protocol::events::{Cause, Event, EventEnvelope};
    use twydo_daemon::protocol::jobs::{JobKind, JobResult, JobState};
    use twydo_daemon::protocol::methods::{OpState, SyncState, WriteOutcome};
    use twydo_daemon::protocol::rpc::ErrorKind;
    use twydo_daemon::protocol::sync::{SyncOutcome, SyncReason};
    let doc = schema::document();
    let open: Vec<&str> = doc["components"]["schemas"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, d)| d["x-open"] == json!(true))
        .map(|(n, _)| n.as_str())
        .collect();
    assert_eq!(
        open,
        [
            "Cause",
            "ErrorKind",
            "EventEnvelope",
            "JobKind",
            "JobResult",
            "JobState",
            "OpState",
            "SyncOutcome",
            "SyncReason",
            "SyncState",
            "WireConflictKind",
            "WireConflictState",
            "WireResolution",
            "WireSide",
            "WireStage",
            "WriteOutcome"
        ]
    );
    assert_eq!(
        serde_json::from_value::<WriteOutcome>(json!("brand_new")).unwrap(),
        WriteOutcome::Unknown
    );
    assert_eq!(
        serde_json::from_value::<ErrorKind>(json!("brand_new")).unwrap(),
        ErrorKind::Unknown
    );
    assert_eq!(
        serde_json::from_value::<Cause>(json!("brand_new")).unwrap(),
        Cause::Unknown
    );
    assert_eq!(
        serde_json::from_value::<SyncState>(json!("brand_new")).unwrap(),
        SyncState::Unknown
    );
    assert_eq!(
        serde_json::from_value::<OpState>(json!({"state": "brand_new", "x": 1})).unwrap(),
        OpState::Unknown
    );
    assert_eq!(
        serde_json::from_value::<JobKind>(json!("brand_new")).unwrap(),
        JobKind::Unknown
    );
    assert_eq!(
        serde_json::from_value::<SyncOutcome>(json!("brand_new")).unwrap(),
        SyncOutcome::Unknown
    );
    assert_eq!(
        serde_json::from_value::<SyncReason>(json!("brand_new")).unwrap(),
        SyncReason::Unknown
    );
    assert_eq!(
        serde_json::from_value::<JobState>(json!({"state": "brand_new", "x": 1})).unwrap(),
        JobState::Unknown
    );
    assert_eq!(
        serde_json::from_value::<JobResult>(json!({"type": "brand_new", "x": 1})).unwrap(),
        JobResult::Unknown
    );
    assert_eq!(
        serde_json::from_value::<WireStage>(json!("brand_new")).unwrap(),
        WireStage::Unknown
    );
    assert_eq!(
        serde_json::from_value::<WireConflictKind>(json!("brand_new")).unwrap(),
        WireConflictKind::Unknown
    );
    assert_eq!(
        serde_json::from_value::<WireConflictState>(json!("brand_new")).unwrap(),
        WireConflictState::Unknown
    );
    assert_eq!(
        serde_json::from_value::<WireResolution>(json!("brand_new")).unwrap(),
        WireResolution::Unknown
    );
    assert_eq!(
        serde_json::from_value::<WireSide>(json!("brand_new")).unwrap(),
        WireSide::Unknown
    );
    let ev: EventEnvelope =
        serde_json::from_value(json!({"seq": 3, "type": "brand.new", "x": 1})).unwrap();
    assert_eq!(ev.event, Event::Unknown);
    assert_eq!(ev.seq, 3);
    // The schema never offers `unknown`: the daemon does not send it.
    for name in open {
        let def = doc["components"]["schemas"][name].to_string();
        assert!(!def.contains("\"unknown\""), "{name}: {def}");
    }
}

#[test]
fn a_removed_method_is_a_break() {
    let released = schema::document();
    let mut current = released.clone();
    current["methods"]
        .as_array_mut()
        .unwrap()
        .retain(|m| m["name"] != "op.check");
    let lines = schema::breaking_changes(&released, &current);
    assert!(lines.iter().any(|l| l.contains("op.check")), "{lines:?}");
}

#[test]
fn any_change_to_a_lifecycle_method_is_a_break() {
    let released = schema::document();
    let mut current = released.clone();
    let m = current["methods"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|m| m["name"] == "daemon.info")
        .unwrap();
    m["params"][0]["schema"] =
        json!({"type": "object", "properties": {"extra": {"type": "string"}}});
    let lines = schema::breaking_changes(&released, &current);
    assert!(lines.iter().any(|l| l.contains("daemon.info")), "{lines:?}");
}

fn breaks(released: &Value, current: &Value) -> Vec<String> {
    schema::breaking_changes(released, current)
}

#[test]
fn a_param_type_change_is_a_break() {
    let released = schema::document();
    let mut current = released.clone();
    def_mut(&mut current, "SubmitParams")["properties"]["session"] = json!({"type": "integer"});
    let lines = breaks(&released, &current);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("SubmitParams.session") && l.contains("string")),
        "{lines:?}"
    );
}

#[test]
fn a_param_losing_null_is_a_break() {
    let released = schema::document();
    let before = released
        .pointer("/components/schemas/OpGetParams/properties/op_id")
        .unwrap();
    let mut current = released.clone();
    def_mut(&mut current, "OpGetParams")["properties"]["op_id"] = json!({"type": "string"});
    assert_ne!(
        before,
        &json!({"type": "string"}),
        "fixture assumption: op_id is nullable"
    );
    let lines = breaks(&released, &current);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("OpGetParams.op_id") && l.contains("null")),
        "{lines:?}"
    );
}

#[test]
fn a_result_gaining_null_is_a_break() {
    let released = schema::document();
    let mut current = released.clone();
    def_mut(&mut current, "WriteResult")["properties"]["message"] =
        json!({"type": ["string", "null"]});
    let lines = breaks(&released, &current);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("WriteResult.message") && l.contains("null")),
        "{lines:?}"
    );
}

#[test]
fn a_result_losing_null_is_not_a_break() {
    let released = schema::document();
    let mut current = released.clone();
    def_mut(&mut current, "CheckResult")["properties"]["reason"] = json!({"type": "string"});
    assert_eq!(breaks(&released, &current), Vec::<String>::new());
}

#[test]
fn closing_a_params_object_is_a_break() {
    let released = schema::document();
    let mut current = released.clone();
    def_mut(&mut current, "SubmitParams")["additionalProperties"] = json!(false);
    let lines = breaks(&released, &current);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("SubmitParams") && l.contains("additional")),
        "{lines:?}"
    );
}

fn to_const_shape(def: &mut Value) {
    let values = def["enum"].as_array().unwrap().clone();
    let one_of: Vec<Value> = values
        .into_iter()
        .map(|v| json!({"const": v, "type": "string"}))
        .collect();
    let o = def.as_object_mut().unwrap();
    o.remove("enum");
    o.remove("type");
    o.insert("oneOf".into(), Value::Array(one_of));
}

#[test]
fn a_lost_const_variant_is_a_break() {
    let mut released = schema::document();
    to_const_shape(def_mut(&mut released, "ErrorKind"));
    let mut current = released.clone();
    def_mut(&mut current, "ErrorKind")["oneOf"]
        .as_array_mut()
        .unwrap()
        .retain(|v| v["const"] != "stale");
    let lines = breaks(&released, &current);
    assert!(lines.iter().any(|l| l.contains("stale")), "{lines:?}");
}

#[test]
fn enum_to_const_shape_alone_is_not_a_break() {
    let released = schema::document();
    let mut current = released.clone();
    to_const_shape(def_mut(&mut current, "ErrorKind"));
    assert_eq!(breaks(&released, &current), Vec::<String>::new());
}

#[test]
fn swapping_one_of_for_any_of_still_checks_variants() {
    let released = schema::document();
    let mut current = released.clone();
    let def = def_mut(&mut current, "OpState").as_object_mut().unwrap();
    let mut variants = def.remove("oneOf").expect("OpState is a oneOf");
    variants.as_array_mut().unwrap().pop();
    def.insert("anyOf".into(), variants);
    let lines = breaks(&released, &current);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("OpState") && l.contains("removed")),
        "{lines:?}"
    );
}

#[test]
fn dropping_all_variants_reports_them() {
    let released = schema::document();
    let mut current = released.clone();
    def_mut(&mut current, "OpState")
        .as_object_mut()
        .unwrap()
        .remove("oneOf");
    let lines = breaks(&released, &current);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("OpState") && l.contains("removed")),
        "{lines:?}"
    );
}

#[test]
fn opening_an_enum_and_adding_a_value_at_once_is_a_break() {
    let doc = |values: Value, open: bool| {
        let mut mode = json!({"type": "string", "enum": values});
        if open {
            mode["x-open"] = json!(true);
        }
        json!({
            "methods": [{
                "name": "m",
                "params": [{"name": "params", "schema": {"type": "object"}}],
                "result": {"name": "result", "schema": {"$ref": "#/components/schemas/Mode"}}
            }],
            "components": {"schemas": {"Mode": mode}}
        })
    };
    let b = schema::breaking_changes(&doc(json!(["a"]), false), &doc(json!(["a", "b"]), true));
    assert_eq!(
        b,
        vec!["Mode: enum value \"b\" added (clients cannot read it)".to_string()]
    );
    // Opening it alone is fine, and once released open, it may gain values.
    assert_eq!(
        schema::breaking_changes(&doc(json!(["a"]), false), &doc(json!(["a"]), true)),
        Vec::<String>::new()
    );
    assert_eq!(
        schema::breaking_changes(&doc(json!(["a"]), true), &doc(json!(["a", "b"]), true)),
        Vec::<String>::new()
    );
}
