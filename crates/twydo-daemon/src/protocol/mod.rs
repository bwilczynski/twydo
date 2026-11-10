pub mod admin;
pub mod events;
pub mod jobs;
pub mod lists;
pub mod methods;
pub mod rpc;
pub mod sync;
pub mod wire;

/// Marks an enum open (`x-open`): clients read a value they do not know as its `Unknown`
/// variant, so the daemon may add values without a new protocol generation (daemon spec
/// §6.2). Skipping `Unknown` makes schemars wrap a string enum in a one-element `oneOf`; that
/// is unwrapped so the schema keeps its plain `enum` shape.
pub(crate) fn open_enum(schema: &mut schemars::Schema) {
    if let Some(obj) = schema.as_object_mut() {
        if let Some(serde_json::Value::Array(one)) = obj.get("oneOf")
            && let [serde_json::Value::Object(only)] = one.as_slice()
            && only.contains_key("enum")
        {
            let only = only.clone();
            obj.remove("oneOf");
            obj.extend(only);
        }
        obj.insert("x-open".into(), serde_json::Value::Bool(true));
    }
}
