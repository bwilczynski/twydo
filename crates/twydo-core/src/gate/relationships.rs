//! Family membership must agree in the standard and 2Do representations.

use std::collections::BTreeSet;

use crate::model::Task;
use crate::views::Snapshot;

use super::{Blocked, blocked, json_meta};

/// Both identities can appear in incoming relationship references. Keep them when the
/// parent disappears so journal recovery and inverses can still detect new members.
pub(crate) fn aliases(task: &Task) -> BTreeSet<String> {
    let mut out = BTreeSet::from([task.uid.clone()]);
    if let Some(uid) = json_meta(task)
        .and_then(|m| m.get_str("uid").map(str::to_owned))
        .filter(|s| !s.is_empty())
    {
        out.insert(uid);
    }
    out
}

/// Facts shared by checks against one immutable snapshot. Only construction scans
/// the collection and parses source calendars; checking a head visits its references.
#[derive(Debug)]
pub(super) struct Index {
    metadata: Vec<Option<crate::model::meta_edit::MetaJson>>,
    aliases: Vec<BTreeSet<String>>,
    owners: std::collections::HashMap<String, Vec<usize>>,
    references: std::collections::HashMap<String, BTreeSet<usize>>,
    readable_sources: Vec<bool>,
    duplicate_metadata: Vec<bool>,
}

impl Index {
    pub(super) fn new(snap: &Snapshot) -> Self {
        let mut out = Self {
            metadata: Vec::with_capacity(snap.tasks.len()),
            aliases: Vec::with_capacity(snap.tasks.len()),
            owners: Default::default(),
            references: Default::default(),
            readable_sources: Vec::with_capacity(snap.tasks.len()),
            duplicate_metadata: Vec::with_capacity(snap.tasks.len()),
        };
        for (i, task) in snap.tasks.iter().enumerate() {
            let meta = json_meta(task);
            let mut aliases = BTreeSet::from([task.uid.clone()]);
            if let Some(uid) = meta
                .as_ref()
                .and_then(|m| m.get_str("uid"))
                .filter(|s| !s.is_empty())
            {
                aliases.insert(uid.into());
            }
            for alias in &aliases {
                out.owners.entry(alias.clone()).or_default().push(i);
            }
            let mut references = BTreeSet::new();
            if let Some(parent) = &task.parent {
                references.insert(parent.clone());
            }
            if let Some(parent) = meta
                .as_ref()
                .and_then(|m| m.get_str("parentuid"))
                .filter(|s| !s.is_empty())
            {
                references.insert(parent.into());
            }
            let cal = snap
                .source(&task.path)
                .and_then(|b| crate::ical::parse(b).ok());
            out.readable_sources.push(cal.is_some());
            out.duplicate_metadata.push(cal.as_ref().is_some_and(|cal| {
                cal.root
                    .children_named("VTODO")
                    .any(|todo| todo.props_named("X-2DOAPP-METADATA").count() > 1)
            }));
            // Inspect every metadata blob, including later duplicates hidden by
            // the task reader, so a raw JSON-only member cannot escape the index.
            if let Some(cal) = &cal {
                for todo in cal.root.children_named("VTODO") {
                    for prop in todo.props_named("X-2DOAPP-METADATA") {
                        if let Some(parent) = crate::model::meta_edit::MetaJson::parse(&prop.value)
                            .and_then(|m| m.get_str("parentuid").map(str::to_owned))
                        {
                            references.insert(parent);
                        }
                    }
                }
            }
            for reference in references {
                out.references.entry(reference).or_default().insert(i);
            }
            out.metadata.push(meta);
            out.aliases.push(aliases);
        }
        out
    }

    /// Validate both link encodings without authorizing any operation or task shape.
    pub(super) fn check(&self, snap: &Snapshot, parent: usize) -> Result<(), Blocked> {
        let head = &snap.tasks[parent];
        self.validate_metadata_source(parent)?;
        let meta = self.metadata[parent].as_ref();
        if head.parent.is_some()
            || meta.is_some_and(|m| {
                m.has("parentuid") && m.get_str("parentuid").is_none_or(|s| !s.is_empty())
            })
        {
            return blocked("family operations need an unambiguous top-level parent");
        }
        if meta.is_some_and(|m| m.has("uid") && m.get_str("uid").is_none()) {
            return blocked("family operations need a readable parent identity");
        }
        let identities = &self.aliases[parent];
        if identities.iter().any(|alias| {
            self.owners
                .get(alias)
                .is_some_and(|owners| owners.iter().any(|&i| i != parent))
        }) {
            return blocked("family operations need unambiguous resource and internal identities");
        }
        let expected = meta
            .and_then(|m| m.get_str("uid"))
            .filter(|s| !s.is_empty())
            .unwrap_or(&head.uid);
        let linked: BTreeSet<usize> = identities
            .iter()
            .filter_map(|alias| self.references.get(alias))
            .flatten()
            .copied()
            .collect();
        for i in linked {
            if i == parent {
                continue;
            }
            let task = &snap.tasks[i];
            if task.parent.as_deref() != Some(head.uid.as_str()) {
                return blocked("family operations need agreeing standard and JSON parent links");
            }
            self.validate_metadata_source(i)?;
            if task.collection != head.collection {
                return blocked("an item must be in the list of its checklist or project");
            }
            let child_meta = self.metadata[i].as_ref();
            let json_link = child_meta
                .and_then(|m| m.get_str("parentuid"))
                .filter(|s| !s.is_empty());
            if child_meta.is_some_and(|m| m.has("parentuid") && m.get_str("parentuid").is_none())
                || json_link.is_some_and(|s| s != expected)
            {
                return blocked("family operations need agreeing standard and JSON parent links");
            }
        }
        Ok(())
    }

    fn validate_metadata_source(&self, i: usize) -> Result<(), Blocked> {
        if !self.readable_sources[i] {
            return blocked("family source is unreadable");
        }
        if self.duplicate_metadata[i] {
            return blocked("family operations need unambiguous metadata");
        }
        Ok(())
    }
}
