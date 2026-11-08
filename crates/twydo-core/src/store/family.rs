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
//! A change of type is rescanned too: whether a task may become a task, checklist or project
//! depends on its items.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};

use super::{Plan, Store, Target, WriteError, create};

use super::journal::{FamilyGuard as Family, check_family};
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
    /// complete, reopen, move and delete, a checklist's date change and a change of type
    /// (also as part of a save); `None` otherwise.
    pub(super) fn family_of(&self, snap: &Snapshot, op: &Op) -> Result<Option<Family>, WriteError> {
        let uid = match op {
            Op::Complete { uid }
            | Op::Reopen { uid }
            | Op::Move { uid, .. }
            | Op::Delete { uid } => uid,
            Op::Edit { uid, .. } | Op::Reorder { uid, .. } => uid,
            Op::Create(task) | Op::CreateAt { task, .. } => match &task.parent {
                Some(parent) => parent,
                None => return Ok(None),
            },
            // A save's move rescans the whole family, which covers its edit; without one, its
            // edit may.
            Op::Save { uid, changes, to } => {
                let part = match to {
                    Some(to) => Op::Move {
                        uid: uid.clone(),
                        to: to.clone(),
                    },
                    None => Op::Edit {
                        uid: uid.clone(),
                        changes: changes.clone(),
                    },
                };
                return self.family_of(snap, &part);
            }
        };
        let Some(idx) = snap.find(uid) else {
            return Ok(None);
        };
        // Individual deletion also depends on the loaded parent and sibling set.
        let idx = if matches!(op, Op::Delete { .. } | Op::Reorder { .. }) {
            snap.parent(idx).unwrap_or(idx)
        } else {
            idx
        };
        // Any task may have gained an item since the snapshot, even one that had none (an
        // empty checklist or project, or a plain task). An edit matters for a checklist's
        // dates, which its items follow, and for a change of type, which the gate allows or
        // refuses by the items the task has.
        if let Op::Edit { changes, .. } = op {
            let dates = changes.iter().any(|c| {
                matches!(
                    c,
                    Change::Due(_)
                        | Change::Start(_)
                        | Change::RepeatInterval(_)
                        | Change::Repeat(_)
                )
            });
            let kind = changes.iter().any(|c| matches!(c, Change::Kind(_)));
            if !kind && (snap.tasks[idx].kind != TaskKind::Checklist || !dates) {
                return Ok(None);
            }
        }
        let mut loaded = BTreeSet::new();
        for &i in std::iter::once(&idx).chain(snap.children(idx)) {
            loaded.insert(self.rel(&snap.tasks[i].path)?);
        }
        Ok(Some(Family {
            collection: snap.tasks[idx].collection.clone(),
            parent_uid: snap.tasks[idx].uid.clone(),
            loaded,
            parent_aliases: crate::gate::relationships::aliases(&snap.tasks[idx]),
        }))
    }

    /// Persist both source and destination membership for queue, recovery and inverses.
    pub(super) fn relocation_families(
        &self,
        snap: &Snapshot,
        op: &Op,
        plan: &Plan,
        source: Option<&Family>,
    ) -> Result<Vec<Family>, WriteError> {
        let uid = match op {
            Op::Move { uid, .. }
            | Op::Save {
                uid, to: Some(_), ..
            } => uid,
            _ => return Ok(Vec::new()),
        };
        let Some(source) = source else {
            return Ok(Vec::new());
        };
        let first = plan
            .writes
            .iter()
            .find(|w| w.before.is_none() && w.after.is_some())
            .expect("a move creates its destination first");
        let cal = ical::parse(first.after.as_ref().unwrap().as_bytes())
            .map_err(|e| super::io_error(std::io::ErrorKind::InvalidData, e.to_string()))?;
        let new_uid = cal
            .root
            .children_named("VTODO")
            .next()
            .unwrap()
            .prop("UID")
            .unwrap()
            .value
            .clone();
        let collection = first.path.split('/').next().unwrap().to_string();
        let loaded = plan
            .writes
            .iter()
            .filter(|w| w.before.is_none())
            .map(|w| w.path.clone())
            .collect();
        let mut parent_aliases =
            crate::gate::relationships::aliases(&snap.tasks[snap.find(uid).unwrap()]);
        parent_aliases.remove(uid);
        parent_aliases.insert(new_uid.clone());
        let guards = vec![
            source.clone(),
            Family {
                collection,
                parent_uid: new_uid,
                parent_aliases,
                loaded,
            },
        ];
        Ok(guards)
    }

    /// Refuse with [`WriteError::Stale`] (naming the file) if the parent's list now holds an
    /// item of the parent that was not loaded. The caller holds the lock.
    pub(super) fn check_family(&self, f: &Family) -> Result<(), WriteError> {
        check_family(&self.vdir, f).map_err(|err| match err {
            super::journal::ApplyError::Conflict { path } => {
                WriteError::Stale(self.vdir.join(path))
            }
            super::journal::ApplyError::Io(err) => err.into(),
            err => super::io_error(std::io::ErrorKind::InvalidInput, err.to_string()),
        })
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
        let cal = if p.repeats {
            let copy_uid = new_uid();
            let today = now.with_timezone(&snap.tz()).date_naive();
            match complete_recurring(&p.cal, now, today, &copy_uid)? {
                (live, Some(copy)) => {
                    return plan_recurring_complete(
                        parent_uid,
                        p,
                        children,
                        RecurringCompletion {
                            live,
                            copy_uid,
                            copy,
                        },
                        snap.tasks[idx].kind == TaskKind::Checklist,
                        now,
                    );
                }
                // The last occurrence of an ending series: the project is completed in place
                // with its open children, as a plain project is.
                (done, None) => done,
            }
        } else {
            let mut cal = p.cal.clone();
            complete_one_time(&mut cal, now)?;
            cal
        };
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
        let json_parent = super::parent_json_uid(snap, &snap.tasks[idx].uid)
            .filter(|u| u != &snap.tasks[idx].uid)
            .unwrap_or_else(|| new_parent.clone());
        for c in &children {
            let new = new_uid();
            let copy = crate::edit::moved_member_copy(
                &c.target.cal,
                &new,
                &new_parent,
                &json_parent,
                now,
            )?;
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

/// A repeating parent's completion (H6, Plan 8): a completed copy of the parent and a copy of every
/// child under it (an open child's completed now, a completed one's keeping its time), then
/// the advanced parent and its completed children reopened. A checklist also drops live
/// inherited item dates after copying them to history; a project's open children are untouched.
struct RecurringCompletion {
    live: ical::Calendar,
    copy_uid: String,
    copy: ical::Calendar,
}

fn plan_recurring_complete(
    parent_uid: String,
    p: Target,
    children: Vec<Child>,
    completion: RecurringCompletion,
    checklist: bool,
    now: DateTime<Utc>,
) -> Result<Plan, WriteError> {
    let RecurringCompletion {
        live,
        copy_uid,
        copy,
    } = completion;
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
        let drop_dates =
            checklist && crate::edit::family::member_has_inherited_dates(&c.target.cal)?;
        if c.status == Status::Completed || drop_dates {
            let mut cal = c.target.cal.clone();
            if c.status == Status::Completed {
                reopen_member(&mut cal, now)?;
            }
            if drop_dates {
                // Copy first: history retains the old inherited dates, live items lose them.
                drop_member_dates(&mut cal, now)?;
            }
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
