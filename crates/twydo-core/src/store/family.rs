//! Operations on a checklist or project: like 2Do, they act on the whole family
//! (`docs/phase0/plan6-baseline.md` H5–H9). Each writes the parent and its children in one
//! journal entry: creates first, then updates, then removes. `uids` is the parent, then
//! every UID the op created or touched.
//!
//! The gate (rule 1b) has already refused a family with a child it has no case for, so each
//! child here is a supported shape. Cancelled children are left as they are by complete and
//! reopen, and go with the family on move and delete.

use chrono::{DateTime, Utc};

use super::{Plan, Store, Target, WriteError, create};
use crate::edit::{
    complete_member, complete_one_time, complete_recurring, member_copy, moved_copy, new_uid,
    reopen_member, reopen_one_time,
};
use crate::model::Status;
use crate::views::Snapshot;

/// A loaded child and its status.
struct Child {
    uid: String,
    status: Status,
    target: Target,
}

impl Store {
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
        for c in children.into_iter().filter(|c| c.status == Status::Open) {
            let mut cal = c.target.cal.clone();
            complete_member(&mut cal, now, now)?;
            uids.push(c.uid);
            writes.push(c.target.update(&cal)?);
        }
        Ok(Plan {
            kind: "complete",
            uids,
            writes,
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
        for c in self.children(snap, idx)? {
            if c.status != Status::Completed {
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
    for c in children {
        if c.status == Status::Cancelled {
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
        }
    }
    writes.extend(updates);
    Ok(Plan {
        kind: "complete",
        uids,
        writes,
    })
}
