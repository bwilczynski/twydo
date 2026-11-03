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

    /// Replace in place. An absent `StartDate` goes right after `RecurrenceEndRepetitionsOrig`
    /// and an absent `RecurrenceEndDate` right after `RecurrenceEndType`; any other absent key
    /// goes last (as 2Do orders them).
    pub fn set_i64(&mut self, key: &str, v: i64) {
        self.set(key, Value::from(v));
    }

    pub fn set_str(&mut self, key: &str, v: &str) {
        self.set(key, Value::from(v));
    }

    pub fn remove(&mut self, key: &str) {
        self.obj.shift_remove(key);
    }

    /// The `TaskType` code (0 task, 1 checklist, 2 project); an absent key counts as a task.
    pub fn task_type(&self) -> i64 {
        self.get_i64("TaskType").unwrap_or(0)
    }

    /// Change `TaskType` the way the phone's type selector does (`plan7-list1/03-phone-convert`,
    /// `04-phone-convert-empty`). Between checklist and project only the code changes. A
    /// task becoming a checklist or project gets `uid` = `own_uid` right after
    /// `RecurrenceEndRepetitionsOrig` and `TaskType` before `TaskDuration`; the reverse drops
    /// `uid` and puts `TaskType` after `TaskDuration`. A missing anchor key appends.
    pub fn set_task_type(&mut self, task_type: i64, own_uid: &str) {
        let was_parent = self.task_type() != 0;
        let is_parent = task_type != 0;
        if was_parent == is_parent {
            self.set_i64("TaskType", task_type);
            return;
        }
        let mut pairs: Vec<(String, Value)> = std::mem::take(&mut self.obj)
            .into_iter()
            .filter(|(k, _)| k != "uid" && k != "TaskType")
            .collect();
        let insert =
            |pairs: &mut Vec<(String, Value)>, key: &str, v: Value, anchor: &str, before: bool| {
                let at = pairs.iter().position(|(k, _)| k == anchor);
                let at = at.map_or(pairs.len(), |i| if before { i } else { i + 1 });
                pairs.insert(at, (key.to_string(), v));
            };
        if is_parent {
            insert(
                &mut pairs,
                "uid",
                Value::from(own_uid),
                "RecurrenceEndRepetitionsOrig",
                false,
            );
            insert(
                &mut pairs,
                "TaskType",
                Value::from(task_type),
                "TaskDuration",
                true,
            );
        } else {
            insert(
                &mut pairs,
                "TaskType",
                Value::from(task_type),
                "TaskDuration",
                false,
            );
        }
        self.obj = pairs.into_iter().collect();
    }

    fn set(&mut self, key: &str, v: Value) {
        if let Some(slot) = self.obj.get_mut(key) {
            *slot = v;
            return;
        }
        let after = match key {
            "StartDate" => "RecurrenceEndRepetitionsOrig",
            "RecurrenceEndDate" => "RecurrenceEndType",
            _ => "",
        };
        if !self.obj.contains_key(after) {
            self.obj.insert(key.to_string(), v);
            return;
        }
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
    new_task_meta(0, None, None)
}

/// The key set of a new task as the phone writes it. `task_type` is 0 task, 1 checklist,
/// 2 project. A checklist or project gets `uid` (its own id) after
/// `RecurrenceEndRepetitionsOrig` and `TaskType` before `TaskDuration`; an item gets
/// `parentuid` before `RUID` (`plan6-list1/01-create`).
pub fn new_task_meta(task_type: i64, uid: Option<&str>, parent_uid: Option<&str>) -> MetaJson {
    let mut obj = Map::new();
    let zero = |obj: &mut Map<String, Value>, keys: &[&str]| {
        for key in keys {
            obj.insert((*key).to_string(), Value::from(0));
        }
    };
    zero(
        &mut obj,
        &["RecurrenceValue", "RecurrenceEndRepetitionsOrig"],
    );
    if let Some(uid) = uid {
        obj.insert("uid".to_string(), Value::from(uid));
    }
    zero(&mut obj, &["RecurrenceEndRepetitions"]);
    if uid.is_some() {
        obj.insert("TaskType".to_string(), Value::from(task_type));
        zero(&mut obj, &["TaskDuration"]);
    } else {
        zero(&mut obj, &["TaskDuration"]);
        obj.insert("TaskType".to_string(), Value::from(task_type));
    }
    zero(
        &mut obj,
        &[
            "RecurrenceType",
            "StartDayDelay",
            "isExpandedToShowChildProjects",
            "RecurrenceFrom",
            "RecurrenceEndType",
            "IsStarred",
        ],
    );
    if let Some(p) = parent_uid {
        obj.insert("parentuid".to_string(), Value::from(p));
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
