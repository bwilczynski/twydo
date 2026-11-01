//! Field-level editing of 2Do's `<2Do Meta>` JSON metadata.
//!
//! The value is `<2Do Meta>` + percent-encoded JSON + `</2Do Meta>`, optionally with text
//! around it (2Do ends it with a TEXT-escaped `\n`). Editing keeps the JSON key order and
//! the text outside the markers byte for byte, so only the targeted field changes.

use serde_json::{Map, Value};

const OPEN: &str = "<2Do Meta>";
const CLOSE: &str = "</2Do Meta>";

/// A parsed `<2Do Meta>` value that re-encodes to the same bytes unless edited.
#[derive(Debug, Clone, PartialEq)]
pub struct MetaJson {
    prefix: String,
    obj: Map<String, Value>,
    suffix: String,
}

impl MetaJson {
    /// Parse the raw (still TEXT-escaped, unfolded) property value. `None` unless it holds a
    /// `<2Do Meta>` JSON object.
    pub fn parse(raw_value: &str) -> Option<MetaJson> {
        let start = raw_value.find(OPEN)?;
        let body_start = start + OPEN.len();
        let end = body_start + raw_value[body_start..].find(CLOSE)?;
        let json = super::meta::percent_decode(&raw_value[body_start..end])?;
        let Value::Object(obj) = serde_json::from_str::<Value>(&json).ok()? else {
            return None;
        };
        Some(MetaJson {
            prefix: raw_value[..start].to_string(),
            obj,
            suffix: raw_value[end + CLOSE.len()..].to_string(),
        })
    }

    /// Inverse of [`MetaJson::parse`].
    pub fn encode(&self) -> String {
        let json = serde_json::to_string(&self.obj).expect("a JSON object serialises");
        format!(
            "{}{OPEN}{}{CLOSE}{}",
            self.prefix,
            percent_encode(&json),
            self.suffix
        )
    }

    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.obj.get(key).and_then(Value::as_i64)
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.obj.get(key).and_then(Value::as_str)
    }

    /// Replace in place. An absent `StartDate` goes right after `RecurrenceEndRepetitionsOrig`;
    /// any other absent key goes last (as 2Do orders them).
    pub fn set_i64(&mut self, key: &str, v: i64) {
        self.set(key, Value::from(v));
    }

    pub fn set_str(&mut self, key: &str, v: &str) {
        self.set(key, Value::from(v));
    }

    pub fn remove(&mut self, key: &str) {
        self.obj.shift_remove(key);
    }

    fn set(&mut self, key: &str, v: Value) {
        if let Some(slot) = self.obj.get_mut(key) {
            *slot = v;
            return;
        }
        if key != "StartDate" || !self.obj.contains_key("RecurrenceEndRepetitionsOrig") {
            self.obj.insert(key.to_string(), v);
            return;
        }
        let after = "RecurrenceEndRepetitionsOrig";
        // serde_json's Map has no positional insert: rebuild it with the new key in place.
        let mut rebuilt = Map::new();
        for (k, old) in std::mem::take(&mut self.obj) {
            let hit = k == after;
            rebuilt.insert(k, old);
            if hit {
                rebuilt.insert(key.to_string(), v.clone());
            }
        }
        self.obj = rebuilt;
    }
}

/// 2Do's plain-task key set and order (hypothesis H10, as captured), zero values, `RUID` empty.
pub fn default_task_meta() -> MetaJson {
    let mut obj = Map::new();
    for key in [
        "RecurrenceValue",
        "RecurrenceEndRepetitionsOrig",
        "RecurrenceEndRepetitions",
        "TaskDuration",
        "TaskType",
        "RecurrenceType",
        "StartDayDelay",
        "isExpandedToShowChildProjects",
        "RecurrenceFrom",
        "RecurrenceEndType",
        "IsStarred",
    ] {
        obj.insert(key.to_string(), Value::from(0));
    }
    obj.insert("RUID".to_string(), Value::from(""));
    obj.insert("DisplayOrder".to_string(), Value::from(0));
    MetaJson {
        prefix: String::new(),
        obj,
        suffix: "\\n".to_string(),
    }
}

/// Percent-encode every byte except ASCII letters, digits and `-._~`.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
