//! Operations on a checklist or project: like 2Do, they act on the whole family
//! (`docs/reference/phase0/plan6-baseline.md` H5–H9). Each writes the parent and its children in one
//! journal entry: creates first, then updates, then removes. `uids` is the parent, then
//! every UID the op created or touched. Every loaded member the op does not write is held:
//! the entry is refused unless its file still holds the loaded bytes.
//!
//! The gate (rule 1b) has already refused a family with a child it has no case for, so each
//! child here is a supported shape. Cancelled children are left as they are by complete and
//! reopen, and go with the family on move and delete. A checklist's new date also removes
//! its items' own dates (`plan6-twydo-list1/01-phone-edit`).
//!
//! The loaded files' bytes do not show an item a sync added after the snapshot, so these ops
//! also rescan the task's list under the lock ([`Store::check_family`]) and are refused if
//! the task has an item that was not loaded, including a first item of a task that had none.

use std::collections::BTreeSet;
use std::fs;
use std::io;

use chrono::{DateTime, Utc};

use super::{Plan, Store, Target, WriteError, create};

/// A family as loaded: the parent's list, its UID and the vdir-relative files of the parent
/// and every item.
pub(super) struct Family {
    collection: String,
    parent_uid: String,
    loaded: BTreeSet<String>,
}
use crate::edit::Op;
use crate::edit::{
    Change, apply_changes, complete_member, complete_one_time, complete_recurring,
    drop_member_dates, member_copy, moved_copy, new_uid, reopen_member, reopen_one_time,
};
use crate::ical;
use crate::model::{Status, TaskKind};
use crate::views::Snapshot;

/// A loaded child and its status.
struct Child {
    uid: String,
    status: Status,
    target: Target,
}

impl Store {
    /// The task `op` acts on and the items loaded with it, for the rescan under the lock: every
    /// complete, reopen, move and delete, and a checklist's date change; `None` otherwise.
    pub(super) fn family_of(&self, snap: &Snapshot, op: &Op) -> Result<Option<Family>, WriteError> {
        let uid = match op {
            Op::Complete { uid }
            | Op::Reopen { uid }
            | Op::Move { uid, .. }
            | Op::Delete { uid } => uid,
            Op::Edit { uid, .. } => uid,
            Op::Create(_) => return Ok(None),
        };
        let Some(idx) = snap.find(uid) else {
            return Ok(None);
        };
        // Any task may have gained an item since the snapshot, even one that had none (an
        // empty checklist or project, or a plain task). An edit matters only for a checklist's
        // dates, which its items follow.
        if let Op::Edit { changes, .. } = op {
            let dates = changes
                .iter()
                .any(|c| matches!(c, Change::Due(_) | Change::Start(_)));
            if snap.tasks[idx].kind != TaskKind::Checklist || !dates {
                return Ok(None);
            }
        }
        let mut loaded = BTreeSet::new();
        for &i in std::iter::once(&idx).chain(snap.children(idx)) {
            loaded.insert(self.rel(&snap.tasks[i].path)?);
        }
        Ok(Some(Family {
            collection: snap.tasks[idx].collection.clone(),
            parent_uid: uid.clone(),
            loaded,
        }))
    }

    /// Refuse with [`WriteError::Stale`] (naming the file) if the parent's list now holds an
    /// item of the parent that was not loaded. The caller holds the lock.
    pub(super) fn check_family(&self, f: &Family) -> Result<(), WriteError> {
        let dir = self.vdir.join(&f.collection);
        for e in fs::read_dir(&dir)? {
            let path = e?.path();
            if path.extension().is_none_or(|x| x != "ics") {
                continue;
            }
            let rel = self.rel(&path)?;
            if f.loaded.contains(&rel) {
                continue;
            }
            let bytes = match fs::read(&path) {
                Ok(b) => b,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            let needle = f.parent_uid.as_bytes();
            if !bytes.windows(needle.len()).any(|w| w == needle) {
                continue;
            }
            // Mentions the parent: an item of it, or unreadable (then refuse to be safe).
            let joined = match ical::parse(&bytes) {
                Ok(cal) => cal.root.children_named("VTODO").any(|t| {
                    t.props_named("RELATED-TO")
                        .any(|p| p.value.trim() == f.parent_uid)
                }),
                Err(_) => true,
            };
            if joined {
                return Err(WriteError::Stale(path));
            }
        }
        Ok(())
    }

    fn children(&self, snap: &Snapshot, idx: usize) -> Result<Vec<Child>, WriteError> {
        snap.children(idx)
            .iter()
            .map(|&c| {
                let t = &snap.tasks[c];
                Ok(Child {
                    uid: t.uid.clone(),
                    status: t.status,
                    target: self.target(snap, &t.uid)?,
                })
            })
            .collect()
    }

    /// Complete the parent `p` (at `idx`) and every open child at the same time (H5). A
    /// repeating project instead gets 2Do's copy-and-advance for the family (H6).
    pub(super) fn plan_family_complete(
        &self,
        snap: &Snapshot,
        idx: usize,
        p: Target,
        now: DateTime<Utc>,
    ) -> Result<Plan, WriteError> {
        let parent_uid = snap.tasks[idx].uid.clone();
        let children = self.children(snap, idx)?;
        if p.repeats {
            return plan_recurring_complete(parent_uid, p, children, now);
        }
        let mut cal = p.cal.clone();
        complete_one_time(&mut cal, now)?;
        let mut uids = vec![parent_uid];
        let mut writes = vec![p.update(&cal)?];
        let mut holds = Vec::new();
        for c in children {
            if c.status != Status::Open {
                holds.push(c.target.hold());
                continue;
            }
            let mut cal = c.target.cal.clone();
            complete_member(&mut cal, now, now)?;
            uids.push(c.uid);
            writes.push(c.target.update(&cal)?);
        }
        Ok(Plan {
            kind: "complete",
            uids,
            writes,
            holds,
        })
    }

    /// Reopen the parent and every completed child (H7).
    pub(super) fn plan_family_reopen(
        &self,
        snap: &Snapshot,
        idx: usize,
        p: Target,
        now: DateTime<Utc>,
    ) -> Result<Plan, WriteError> {
        let mut cal = p.cal.clone();
        reopen_one_time(&mut cal, now)?;
        let mut uids = vec![snap.tasks[idx].uid.clone()];
        let mut writes = vec![p.update(&cal)?];
        let mut holds = Vec::new();
        for c in self.children(snap, idx)? {
            if c.status != Status::Completed {
                holds.push(c.target.hold());
                continue;
            }
            let mut cal = c.target.cal.clone();
            reopen_member(&mut cal, now)?;
            uids.push(c.uid);
            writes.push(c.target.update(&cal)?);
        }
        Ok(Plan {
            kind: "reopen",
            uids,
            writes,
            holds,
        })
    }

    /// Recreate the family in `to` under new UIDs, the children pointing at the new parent,
    /// then remove every source file (H8).
    pub(super) fn plan_family_move(
        &self,
        snap: &Snapshot,
        idx: usize,
        p: Target,
        to: &str,
        now: DateTime<Utc>,
    ) -> Result<Plan, WriteError> {
        let new_parent = new_uid();
        let mut uids = vec![snap.tasks[idx].uid.clone(), new_parent.clone()];
        let mut writes = vec![create(
            to,
            &new_parent,
            &moved_copy(&p.cal, &new_parent, None, now)?,
        )?];
        let children = self.children(snap, idx)?;
        for c in &children {
            let new = new_uid();
            let copy = moved_copy(&c.target.cal, &new, Some(&new_parent), now)?;
            writes.push(create(to, &new, &copy)?);
            uids.push(c.uid.clone());
            uids.push(new);
        }
        writes.push(p.remove());
        writes.extend(children.into_iter().map(|c| c.target.remove()));
        Ok(Plan {
            kind: "move",
            uids,
            writes,
            holds: Vec::new(),
        })
    }

    /// A checklist's date change: the checklist's `changes`, and every item with a date of
    /// its own loses it, as 2Do does (the gate allows only a new date-only due here). Items
    /// without a date are held.
    pub(super) fn plan_checklist_date(
        &self,
        snap: &Snapshot,
        idx: usize,
        p: Target,
        changes: &[Change],
        now: DateTime<Utc>,
    ) -> Result<Plan, WriteError> {
        let mut cal = p.cal.clone();
        apply_changes(&mut cal, changes, now)?;
        let mut uids = vec![snap.tasks[idx].uid.clone()];
        let mut writes = vec![p.update(&cal)?];
        let mut holds = Vec::new();
        for (&c, child) in snap.children(idx).iter().zip(self.children(snap, idx)?) {
            if snap.tasks[c].due.is_none() {
                holds.push(child.target.hold());
                continue;
            }
            let mut cal = child.target.cal.clone();
            drop_member_dates(&mut cal, now)?;
            uids.push(child.uid);
            writes.push(child.target.update(&cal)?);
        }
        Ok(Plan {
            kind: "edit",
            uids,
            writes,
            holds,
        })
    }

    /// Remove the parent and every child (H9).
    pub(super) fn plan_family_delete(
        &self,
        snap: &Snapshot,
        idx: usize,
        p: Target,
    ) -> Result<Plan, WriteError> {
        let mut uids = vec![snap.tasks[idx].uid.clone()];
        let mut writes = vec![p.remove()];
        for c in self.children(snap, idx)? {
            uids.push(c.uid);
            writes.push(c.target.remove());
        }
        Ok(Plan {
            kind: "delete",
            uids,
            writes,
            holds: Vec::new(),
        })
    }
}

/// A repeating project's completion (H6): a completed copy of the parent and a copy of every
/// child under it (an open child's completed now, a completed one's keeping its time), then
/// the advanced parent and its completed children reopened. Open children are untouched.
fn plan_recurring_complete(
    parent_uid: String,
    p: Target,
    children: Vec<Child>,
    now: DateTime<Utc>,
) -> Result<Plan, WriteError> {
    let copy_uid = new_uid();
    let (live, copy) = complete_recurring(&p.cal, now, &copy_uid)?;
    let mut writes = vec![create(&p.collection, &copy_uid, &copy)?];
    let mut uids = vec![parent_uid, copy_uid.clone()];
    let mut updates = vec![p.update(&live)?];
    let mut holds = Vec::new();
    for c in children {
        if c.status == Status::Cancelled {
            holds.push(c.target.hold());
            continue;
        }
        let child_copy = new_uid();
        let cal = member_copy(&c.target.cal, &child_copy, &copy_uid, now, now)?;
        writes.push(create(&c.target.collection, &child_copy, &cal)?);
        uids.push(child_copy);
        if c.status == Status::Completed {
            let mut cal = c.target.cal.clone();
            reopen_member(&mut cal, now)?;
            uids.push(c.uid);
            updates.push(c.target.update(&cal)?);
        } else {
            // Copied from its loaded bytes, left as it is.
            holds.push(c.target.hold());
        }
    }
    writes.extend(updates);
    Ok(Plan {
        kind: "complete",
        uids,
        writes,
        holds,
    })
}
