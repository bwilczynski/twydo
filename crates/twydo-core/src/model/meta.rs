//! 2Do's `X-2DOAPP-METADATA` free-text token string (`@key` / `@key:value`).

use std::collections::BTreeMap;

/// Parsed metadata. `tokens` maps key to `Some(value)` or `None` for a bare flag.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    pub raw: Option<String>,
    pub tokens: BTreeMap<String, Option<String>>,
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
    }
}
