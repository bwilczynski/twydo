//! 2Do's `X-2DOAPP-METADATA`. Two encodings exist:
//! - the `@key` / `@key:value` token string (`TaskNoteMetaData`, as in Todoist notes);
//! - `<2Do Meta>` + URL-encoded JSON + `</2Do Meta>`, which 2Do 3.91 writes over CalDAV.
//!
//! JSON fields Twydo uses are mapped onto the token keys, so callers read one shape.

use std::collections::BTreeMap;

use chrono::DateTime;

const JSON_OPEN: &str = "<2Do Meta>";
const JSON_CLOSE: &str = "</2Do Meta>";

/// Which encoding the metadata used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MetaFormat {
    #[default]
    Tokens,
    /// `<2Do Meta>` JSON. It is the full field set, so a missing field means "not set"
    /// (for example no `StartDate` = no start date, whatever DTSTART says).
    Json,
}

/// Parsed metadata. `tokens` maps key to `Some(value)` or `None` for a bare flag.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    pub raw: Option<String>,
    pub tokens: BTreeMap<String, Option<String>>,
    pub format: MetaFormat,
    /// The value looked like `<2Do Meta>` JSON but did not decode; `tokens` is empty.
    pub unreadable: bool,
}

impl Metadata {
    /// Value of `@key:value`; `None` when absent or a bare flag.
    pub fn value(&self, key: &str) -> Option<&str> {
        self.tokens.get(key).and_then(|v| v.as_deref())
    }

    /// Whether `@key` appears at all (with or without a value).
    pub fn has(&self, key: &str) -> bool {
        self.tokens.contains_key(key)
    }
}

/// Byte offset of the next token start at or after `from`: an `@` preceded by whitespace
/// (or at the start), followed by letters and then `:`, whitespace or end of text.
fn is_token_start(s: &str, at: usize) -> bool {
    let b = s.as_bytes();
    if b[at] != b'@' {
        return false;
    }
    if at > 0 && !(b[at - 1] as char).is_whitespace() {
        return false;
    }
    let mut i = at + 1;
    while i < b.len() && b[i].is_ascii_alphabetic() {
        i += 1;
    }
    i > at + 1 && (i == b.len() || b[i] == b':' || (b[i] as char).is_whitespace())
}

/// Tokenise a metadata string. A value runs until whitespace followed by the next token,
/// so `@tags:a, b @starred` yields `tags = "a, b"`. Later duplicates replace earlier ones.
pub fn parse_metadata(raw: &str) -> Metadata {
    if raw.trim_start().starts_with(JSON_OPEN) {
        return parse_json_metadata(raw);
    }
    let mut tokens = BTreeMap::new();
    let starts: Vec<usize> = raw
        .char_indices()
        .map(|(i, _)| i)
        .filter(|&i| is_token_start(raw, i))
        .collect();
    for (n, &s) in starts.iter().enumerate() {
        let end = starts.get(n + 1).copied().unwrap_or(raw.len());
        let body = raw[s + 1..end].trim_end();
        let (key, value) = match body.split_once(':') {
            Some((k, v)) => (k, Some(v.trim().to_string())),
            None => (body, None),
        };
        tokens.insert(key.to_string(), value);
    }
    Metadata {
        raw: Some(raw.to_string()),
        tokens,
        format: MetaFormat::Tokens,
        unreadable: false,
    }
}

/// `<2Do Meta>%7B...%7D</2Do Meta>`: percent-decode the body and read it as a JSON object.
/// Mapped fields (observed in 2Do 3.91 CalDAV uploads):
/// - `TaskType` 1 / 2 -> `tasktype` `checklist` / `project` (0 = task, no token);
/// - `StartDate` epoch seconds -> `startdate` `YYYY-MM-DD`. Every value seen is a UTC
///   midnight naming the start day, so only the UTC date is taken;
/// - `IsStarred` 1 -> `starred`;
/// - `RecurrenceFrom` 2 -> `repeatfromcompletion` (same enum as the `.tod` `recurrence`
///   field, where 2 = from completion; not yet seen in a CalDAV upload).
///
/// Other fields are kept only in `raw`.
fn parse_json_metadata(raw: &str) -> Metadata {
    let mut meta = Metadata {
        raw: Some(raw.to_string()),
        format: MetaFormat::Json,
        ..Metadata::default()
    };
    let body = raw.trim_start()[JSON_OPEN.len()..]
        .split(JSON_CLOSE)
        .next()
        .unwrap_or("");
    let obj = percent_decode(body)
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| match v {
            serde_json::Value::Object(o) => Some(o),
            _ => None,
        });
    let Some(obj) = obj else {
        meta.unreadable = true;
        return meta;
    };
    let int = |key: &str| {
        obj.get(key).and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64))
                .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        })
    };
    let t = &mut meta.tokens;
    let kind = match int("TaskType") {
        Some(1) => Some("checklist"),
        Some(2) => Some("project"),
        _ => None,
    };
    if let Some(k) = kind {
        t.insert("tasktype".into(), Some(k.into()));
    }
    if let Some(day) = int("StartDate")
        .filter(|&n| n > 0)
        .and_then(|n| DateTime::from_timestamp(n, 0))
    {
        t.insert(
            "startdate".into(),
            Some(day.date_naive().format("%Y-%m-%d").to_string()),
        );
    }
    if int("IsStarred") == Some(1) {
        t.insert("starred".into(), None);
    }
    if int("RecurrenceFrom") == Some(2) {
        t.insert("repeatfromcompletion".into(), None);
    }
    meta
}

/// `%XX` -> byte; `None` on a bad escape or invalid UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}
