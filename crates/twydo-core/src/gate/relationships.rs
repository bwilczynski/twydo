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

/// Validate a top-level family's links, including references omitted from the indexed
/// child list. This does not authorize an operation or validate its task shapes.
pub(crate) fn check(snap: &Snapshot, parent: usize) -> Result<(), Blocked> {
    let head = &snap.tasks[parent];
    validate_metadata_source(snap, parent)?;
    let meta = json_meta(head);
    if head.parent.is_some()
        || meta.as_ref().is_some_and(|m| {
            m.has("parentuid") && m.get_str("parentuid").is_none_or(|s| !s.is_empty())
        })
    {
        return blocked("family operations need an unambiguous top-level parent");
    }
    if meta
        .as_ref()
        .is_some_and(|m| m.has("uid") && m.get_str("uid").is_none())
    {
        return blocked("family operations need a readable parent identity");
    }
    let identities = aliases(head);
    if snap
        .tasks
        .iter()
        .enumerate()
        .any(|(i, t)| i != parent && !identities.is_disjoint(&aliases(t)))
    {
        return blocked("family operations need unambiguous resource and internal identities");
    }
    let expected = meta
        .as_ref()
        .and_then(|m| m.get_str("uid"))
        .filter(|s| !s.is_empty())
        .unwrap_or(&head.uid);
    for (i, task) in snap.tasks.iter().enumerate() {
        if i == parent {
            continue;
        }
        let standard = task.parent.as_deref() == Some(head.uid.as_str());
        // The reader exposes the first metadata property. A later blob must not
        // hide a membership reference from permission probes.
        let raw_link = snap
            .source(&task.path)
            .and_then(|b| crate::ical::parse(b).ok())
            .is_some_and(|cal| {
                cal.root.children_named("VTODO").any(|t| {
                    t.props_named("X-2DOAPP-METADATA").any(|p| {
                        crate::model::meta_edit::MetaJson::parse(&p.value)
                            .and_then(|m| m.get_str("parentuid").map(str::to_owned))
                            .is_some_and(|p| identities.contains(&p))
                    })
                })
            });
        if raw_link && !standard {
            return blocked("family operations need agreeing standard and JSON parent links");
        }
        let child_meta = json_meta(task);
        let json_link = child_meta
            .as_ref()
            .and_then(|m| m.get_str("parentuid"))
            .filter(|s| !s.is_empty());
        let standard_reference = task.parent.as_ref().is_some_and(|s| identities.contains(s));
        if (standard_reference || json_link.is_some_and(|s| identities.contains(s))) && !standard {
            return blocked("family operations need agreeing standard and JSON parent links");
        }
        if standard {
            validate_metadata_source(snap, i)?;
            if task.collection != head.collection {
                return blocked("an item must be in the list of its checklist or project");
            }
            if child_meta
                .as_ref()
                .is_some_and(|m| m.has("parentuid") && m.get_str("parentuid").is_none())
                || json_link.is_some_and(|s| s != expected)
            {
                return blocked("family operations need agreeing standard and JSON parent links");
            }
        }
    }
    Ok(())
}

fn validate_metadata_source(snap: &Snapshot, i: usize) -> Result<(), Blocked> {
    let task = &snap.tasks[i];
    let Some(cal) = snap
        .source(&task.path)
        .and_then(|b| crate::ical::parse(b).ok())
    else {
        return blocked("family source is unreadable");
    };
    if cal
        .root
        .children_named("VTODO")
        .any(|todo| todo.props_named("X-2DOAPP-METADATA").count() > 1)
    {
        return blocked("family operations need unambiguous metadata");
    }
    Ok(())
}
