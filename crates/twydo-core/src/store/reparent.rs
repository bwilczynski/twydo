//! Candidate single-leaf relocation with durable dependencies on both families.
use super::journal::FamilyGuard;
use super::plan::{JournalReads, Plan, PlanReads, Planner, create};
use super::{WriteError, parent_json_uid};
use crate::edit::{new_uid, reparented_copy};
use crate::model::{TaskKind, meta_edit::MetaJson};
use crate::views::Snapshot;
use chrono::{DateTime, Utc};
use std::collections::BTreeSet;

impl Planner<'_> {
    pub(super) fn plan_reparent(
        &self,
        snap: &Snapshot,
        uid: &str,
        collection: &str,
        parent: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Plan, WriteError> {
        let i = snap.find(uid).expect("gate checked leaf");
        let p = parent.and_then(|uid| snap.find(uid));
        let clear = p.is_some_and(|p| snap.tasks[p].kind == TaskKind::Checklist)
            || (p.is_none()
                && snap
                    .parent(i)
                    .is_some_and(|p| snap.tasks[p].kind == TaskKind::Checklist));
        let target = self.target(snap, uid)?;
        let new = new_uid();
        let mut cal = reparented_copy(&target.cal, &new, parent, clear, now)?;
        let todo = cal.root.children_named_mut("VTODO").next().unwrap();
        let prop = todo.prop_mut("X-2DOAPP-METADATA").unwrap();
        let mut meta = MetaJson::parse(&prop.value).unwrap();
        if let Some(parent) = parent {
            meta.set_str(
                "parentuid",
                parent_json_uid(snap, parent)
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(parent),
            );
        }
        // Append without rewriting any sibling's bytes. Refuse overflow rather than wrap.
        let siblings: Vec<_> = if let Some(p) = p {
            snap.children(p).to_vec()
        } else {
            snap.tasks
                .iter()
                .enumerate()
                .filter(|(_, t)| t.collection == collection && t.parent.is_none())
                .map(|(n, _)| n)
                .collect()
        };
        let max = siblings
            .iter()
            .map(|&n| snap.tasks[n].meta.display_order())
            .max()
            .unwrap_or(-1);
        let rank = max.checked_add(1).ok_or_else(|| {
            super::io_error(std::io::ErrorKind::InvalidData, "item rank overflow")
        })?;
        meta.set_i64("DisplayOrder", rank);
        prop.set_value(&meta.encode());
        let mut holds = Vec::new();
        let mut held = BTreeSet::new();
        for p in snap.parent(i).into_iter().chain(p) {
            for n in std::iter::once(p).chain(snap.children(p).iter().copied()) {
                if n != i && held.insert(n) {
                    holds.push(self.target(snap, &snap.tasks[n].uid)?.hold());
                }
            }
        }
        let created = create(collection, &new, &cal)?;
        let families = self.reparent_families(snap, i, p, &created.path, &new, collection)?;
        Ok(Plan {
            reads: PlanReads {
                families,
                journal: JournalReads::Holds,
                ..Default::default()
            },
            kind: "reparent",
            uids: vec![uid.into(), new],
            writes: vec![created, target.remove()],
            holds,
        })
    }

    fn reparent_families(
        &self,
        snap: &Snapshot,
        i: usize,
        destination: Option<usize>,
        created: &str,
        new: &str,
        collection: &str,
    ) -> Result<Vec<FamilyGuard>, WriteError> {
        let mut guards = Vec::new();
        let mut seen = BTreeSet::new();
        for p in snap
            .parent(i)
            .into_iter()
            .chain(destination)
            .chain(std::iter::once(i))
        {
            if !seen.insert(p) {
                continue;
            }
            let mut guard = self.loaded_family(snap, p)?;
            if Some(p) == destination || p == i {
                guard.loaded.insert(created.to_owned());
            }
            guards.push(guard);
        }
        guards.push(FamilyGuard {
            collection: collection.into(),
            parent_uid: new.into(),
            parent_aliases: {
                let mut aliases = crate::gate::relationships::aliases(&snap.tasks[i]);
                aliases.remove(&snap.tasks[i].uid);
                aliases.insert(new.into());
                aliases
            },
            loaded: BTreeSet::from([self.rel(&snap.tasks[i].path)?, created.to_owned()]),
        });
        Ok(guards)
    }
}
