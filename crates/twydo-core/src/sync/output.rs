//! Parser for `vdirsyncer sync` stderr.
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Upload,
    Update,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transfer {
    pub action: Action,
    pub uid: String,
    pub storage: String,
    pub collection: String,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct SyncOutput {
    pub synced: BTreeSet<String>,
    pub transfers: Vec<Transfer>,
    pub conflict_items: Vec<String>,
    pub unhooked: BTreeMap<String, Vec<String>>,
    pub emptied: BTreeMap<String, String>,
    pub errors: BTreeMap<String, Vec<String>>,
    pub unattributed: Vec<String>,
}

/// Parse vdirsyncer's stderr for `pair`.
pub fn parse(pair: &str, stderr: &str) -> SyncOutput {
    let mut out = SyncOutput::default();
    let prefix = format!("{pair}/");
    // Collection of the last attributed error, while only error lines follow it.
    let mut current: Option<String> = None;
    // Collection of the last "One item changed on both sides" error.
    let mut conflict_coll: Option<String> = None;

    for line in stderr.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(msg) = line.strip_prefix("error: ") {
            if let Some((coll, text)) = attributed(&prefix, msg) {
                if let Some(storage) = emptied_storage(&text) {
                    out.emptied.insert(coll.clone(), storage);
                }
                conflict_coll = text
                    .starts_with("One item changed on both sides")
                    .then(|| coll.clone());
                out.errors.entry(coll.clone()).or_default().push(text);
                current = Some(coll);
                continue;
            }
            match &current {
                Some(coll) => {
                    if let (Some(uid), Some(cc)) = (msg.strip_prefix("Item ID: "), &conflict_coll) {
                        out.unhooked
                            .entry(cc.clone())
                            .or_default()
                            .push(uid.to_string());
                    }
                    if let Some(v) = out.errors.get_mut(coll) {
                        v.push(msg.to_string());
                    }
                }
                None => out.unattributed.push(msg.to_string()),
            }
            continue;
        }
        current = None;
        conflict_coll = None;
        if line.starts_with("warning:")
            || line.starts_with("critical: The two files are not completely equal")
        {
            continue;
        }
        if let Some(msg) = line.strip_prefix("critical: ") {
            out.unattributed.push(msg.to_string());
        } else if let Some(rest) = line.strip_prefix("Syncing ") {
            if let Some(coll) = rest.strip_prefix(prefix.as_str()) {
                out.synced.insert(coll.to_string());
            }
        } else if let Some(uid) = line
            .strip_prefix("Doing conflict resolution for item ")
            .and_then(|r| r.strip_suffix("..."))
        {
            out.conflict_items.push(uid.to_string());
        } else if let Some(t) = transfer(line) {
            out.transfers.push(t);
        }
    }
    out
}

fn transfer(line: &str) -> Option<Transfer> {
    let (action, rest) = if let Some(r) = line.strip_prefix("Copying (uploading) item ") {
        (Action::Upload, r)
    } else if let Some(r) = line.strip_prefix("Copying (updating) item ") {
        (Action::Update, r)
    } else {
        (Action::Delete, line.strip_prefix("Deleting item ")?)
    };
    let sep = if action == Action::Delete {
        " from "
    } else {
        " to "
    };
    let (uid, target) = rest.split_once(sep)?;
    let (storage, collection) = target.split_once('/')?;
    Some(Transfer {
        action,
        uid: uid.to_string(),
        storage: storage.to_string(),
        collection: collection.to_string(),
    })
}

/// Split `<pair>/<coll>: <text>` or `Unknown error occurred for <pair>/<coll>: <text>`
/// into the collection and the message.
fn attributed(prefix: &str, msg: &str) -> Option<(String, String)> {
    if let Some(rest) = msg.strip_prefix(prefix) {
        let (coll, text) = rest.split_once(": ")?;
        return Some((coll.to_string(), text.to_string()));
    }
    let rest = msg
        .strip_prefix("Unknown error occurred for ")?
        .strip_prefix(prefix)?;
    let (coll, text) = rest.split_once(": ")?;
    Some((coll.to_string(), format!("Unknown error occurred: {text}")))
}

/// Storage name from `Storage "<storage>/<coll>" was completely emptied. ...`.
fn emptied_storage(text: &str) -> Option<String> {
    let after = text.strip_prefix("Storage \"")?;
    let (storage_coll, _) = after.split_once("\" was completely emptied")?;
    Some(storage_coll.split('/').next()?.to_string())
}
