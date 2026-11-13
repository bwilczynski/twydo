//! The pure planner: the gate and the plan of one operation on a loaded snapshot, with no
//! I/O. [`plan`] produces the [`Prepared`] entry; [`Store`](super::Store) adds only what reads
//! the state directory (the collection fence) and what belongs to the store (the request).

use std::io;
use std::path::{Component, Path};

use chrono::{DateTime, Utc};

use super::journal::{Entry, FORMAT, Stage, Write, new_op_id};
use super::queue::Prepared;
use super::{WriteError, io_error, parent_json_uid};
use crate::compat::Matrix;
use crate::config::WriteMode;
use crate::edit::{
    Change, EditError, Op, apply_changes, complete_one_time, complete_recurring, file_name,
    moved_copy, new_task, new_uid, reopen_ended_until, reopen_one_time,
};
use crate::gate::{self, Blocked};
use crate::ical::{self, Calendar};
use crate::model::meta_edit::MetaJson;
use crate::views::Snapshot;

/// The write policy an operation is gated and planned under.
#[derive(Clone, Copy)]
pub struct Policy<'a> {
    pub matrix: &'a Matrix,
    pub mode: WriteMode,
}

/// The gate and the plan: the journal entry `op` produces on the files loaded in `snap`,
/// with the holds and family it must find unchanged when it is committed. Reads nothing
/// beyond `snap`. The entry carries no request; the store stamps it.
pub fn plan(
    snap: &Snapshot,
    op: &Op,
    policy: Policy<'_>,
    now: DateTime<Utc>,
) -> Result<Prepared, WriteError> {
    plan_in(&snap.config.vdir, snap, op, policy, now)
}

/// [`plan`] with the vdir named by the caller (the writer's own) instead of the snapshot's
/// configuration.
pub(super) fn plan_in(
    vdir: &Path,
    snap: &Snapshot,
    op: &Op,
    policy: Policy<'_>,
    now: DateTime<Utc>,
) -> Result<Prepared, WriteError> {
    let today = now.with_timezone(&snap.tz()).date_naive();
    let planner = Planner { vdir, policy };
    planner.gate(snap, op, today)?;
    let mut plan = planner.plan(snap, op, now)?;
    let mut holds = std::mem::take(&mut plan.holds);
    let text_head = planner.text_edit_head(snap, op);
    if let Some(head) = text_head {
        for i in std::iter::once(head).chain(snap.children(head).iter().copied()) {
            let hold = planner.target(snap, &snap.tasks[i].uid)?.hold();
            if !plan.writes.iter().any(|w| w.path == hold.0) && !holds.iter().any(|h| h.0 == hold.0)
            {
                holds.push(hold);
            }
        }
    }
    let member_parent = match op {
        Op::Create(n) | Op::CreateAt { task: n, .. } => {
            n.parent.as_deref().and_then(|uid| snap.find(uid))
        }
        Op::Edit { uid, .. }
        | Op::Save { uid, .. }
        | Op::Delete { uid }
        | Op::Reorder { uid, .. } => snap.find(uid).and_then(|i| snap.parent(i)),
        _ => None,
    };
    let creating_item =
        matches!(op, Op::Create(_) | Op::CreateAt { .. }) && member_parent.is_some();
    let checklist_schedule = match op {
        Op::Edit { uid, changes } | Op::Save { uid, changes, .. } => {
            snap.find(uid)
                .is_some_and(|i| snap.tasks[i].kind == crate::model::TaskKind::Checklist)
                && changes.iter().any(|c| {
                    matches!(
                        c,
                        Change::Due(_)
                            | Change::Start(_)
                            | Change::RepeatInterval(_)
                            | Change::Repeat(_)
                    )
                })
        }
        _ => false,
    };
    if checklist_schedule {
        let uid = match op {
            Op::Edit { uid, .. } | Op::Save { uid, .. } => uid,
            _ => unreachable!(),
        };
        let i = snap.find(uid).expect("gate checked target");
        for &c in snap.children(i) {
            let hold = planner.target(snap, &snap.tasks[c].uid)?.hold();
            if !plan.writes.iter().any(|w| w.path == hold.0) && !holds.iter().any(|h| h.0 == hold.0)
            {
                holds.push(hold);
            }
        }
    }
    let durable_holds = text_head.is_some()
        || checklist_schedule
        || matches!(
            op,
            Op::Delete { .. } | Op::Reopen { .. } | Op::Reparent { .. }
        )
        || member_parent.is_some_and(|p| {
            creating_item
                || matches!(op, Op::Reorder { .. } | Op::Delete { .. })
                || (snap.tasks[p].kind == crate::model::TaskKind::Checklist
                    && snap.tasks[p].rrule.is_some())
        });
    let mut family = planner.family_of(snap, op)?;
    let relocating = matches!(op, Op::Move { .. } | Op::Save { to: Some(_), .. });
    if (creating_item || relocating)
        && let Some(family) = family.as_mut()
    {
        // Recovery may find our own create already on disk. It is a known member,
        // while any additional sibling must still make the snapshot stale.
        family
            .loaded
            .extend(plan.writes.iter().map(|w| w.path.clone()));
    }
    let families = planner.relocation_families(snap, op, &plan, family.as_ref())?;
    let entry = Entry {
        format: FORMAT,
        id: new_op_id(now),
        created: now,
        kind: plan.kind.to_string(),
        uids: plan.uids,
        stage: Stage::Prepared,
        inverse_of: None,
        from_queue: None,
        request: None,
        synced: None,
        holds: if durable_holds {
            holds.clone()
        } else {
            Vec::new()
        },
        family: if text_head.is_some()
            || checklist_schedule
            || (durable_holds
                && matches!(
                    op,
                    Op::Create(_)
                        | Op::CreateAt { .. }
                        | Op::Reorder { .. }
                        | Op::Delete { .. }
                        | Op::Reopen { .. }
                )) {
            family.clone()
        } else {
            None
        },
        families: families.clone(),
        writes: plan.writes,
    };
    Ok(Prepared {
        entry,
        holds,
        family,
        families,
    })
}

/// Plans operations against one snapshot; the `plan_*` methods of the sibling modules extend it.
pub(super) struct Planner<'a> {
    pub vdir: &'a Path,
    pub policy: Policy<'a>,
}

/// The parts of a journal entry an operation produces.
pub(super) struct Plan {
    pub(super) kind: &'static str,
    pub(super) uids: Vec<String>,
    pub(super) writes: Vec<Write>,
    /// Loaded files the op reads but does not write (`rel`, loaded bytes): a family member
    /// left as it is. Each must still hold its bytes when the entry is applied.
    pub(super) holds: Vec<(String, String)>,
}

/// The loaded resource an operation targets.
pub(super) struct Target {
    pub(super) collection: String,
    pub(super) rel: String,
    pub(super) source: String,
    pub(super) cal: Calendar,
    pub(super) repeats: bool,
}

impl Target {
    pub(super) fn update(self, cal: &Calendar) -> Result<Write, WriteError> {
        Ok(Write {
            path: self.rel,
            before: Some(self.source),
            after: Some(text(cal)?),
        })
    }

    pub(super) fn remove(self) -> Write {
        Write {
            path: self.rel,
            before: Some(self.source),
            after: None,
        }
    }

    /// The file is left as it is, but must still hold the loaded bytes.
    pub(super) fn hold(self) -> (String, String) {
        (self.rel, self.source)
    }
}

pub(super) fn text(cal: &Calendar) -> Result<String, WriteError> {
    String::from_utf8(ical::serialize(cal))
        .map_err(|_| io_error(io::ErrorKind::InvalidData, "resource is not UTF-8"))
}

pub(super) fn create(collection: &str, uid: &str, cal: &Calendar) -> Result<Write, WriteError> {
    Ok(Write {
        path: format!("{collection}/{}", file_name(uid)),
        before: None,
        after: Some(text(cal)?),
    })
}

impl Planner<'_> {
    /// Refuse with [`WriteError::Blocked`] what the gate refuses under this policy.
    pub(super) fn gate(
        &self,
        snap: &Snapshot,
        op: &Op,
        today: chrono::NaiveDate,
    ) -> Result<(), WriteError> {
        gate::check(snap, self.policy.matrix, self.policy.mode, op, today)
            .map_err(WriteError::Blocked)
    }

    /// `<collection>/<file>` of a loaded task's absolute path.
    pub(super) fn rel(&self, path: &Path) -> Result<String, WriteError> {
        let outside = || {
            io_error(
                io::ErrorKind::InvalidInput,
                format!("{} is not in the vdir", path.display()),
            )
        };
        let rest = path.strip_prefix(self.vdir).map_err(|_| outside())?;
        let parts = rest
            .components()
            .map(|c| match c {
                Component::Normal(s) => s.to_str().ok_or_else(outside),
                _ => Err(outside()),
            })
            .collect::<Result<Vec<&str>, _>>()?;
        if parts.len() != 2 {
            return Err(outside());
        }
        Ok(parts.join("/"))
    }

    pub(super) fn target(&self, snap: &Snapshot, uid: &str) -> Result<Target, WriteError> {
        let i = snap
            .find(uid)
            .ok_or_else(|| WriteError::Blocked(Blocked(format!("no task with UID {uid}"))))?;
        let t = &snap.tasks[i];
        let bytes = snap
            .source(&t.path)
            .ok_or_else(|| WriteError::Stale(t.path.clone()))?;
        let bad = |why: String| io_error(io::ErrorKind::InvalidData, why);
        let source = String::from_utf8(bytes.to_vec())
            .map_err(|_| bad(format!("{} is not UTF-8", t.path.display())))?;
        let cal = ical::parse(bytes).map_err(|e| bad(format!("{}: {e}", t.path.display())))?;
        Ok(Target {
            collection: t.collection.clone(),
            rel: self.rel(&t.path)?,
            source,
            cal,
            repeats: t.rrule.is_some(),
        })
    }

    /// A save (kind "save"): the edit's writes, then the move's, planned on the edited files.
    /// A path written twice keeps the first `before` and the last `after`, at its later place
    /// (a move creates the copy before it removes the source). Either part may be empty.
    fn plan_save(
        &self,
        snap: &Snapshot,
        uid: &str,
        changes: &[Change],
        to: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Plan, WriteError> {
        let mut plan = Plan {
            kind: "save",
            uids: Vec::new(),
            writes: Vec::new(),
            holds: Vec::new(),
        };
        let mut edited = snap.clone();
        if !changes.is_empty() || to.is_none() {
            let edit = Op::Edit {
                uid: uid.to_string(),
                changes: changes.to_vec(),
            };
            let part = self.plan(snap, &edit, now)?;
            for w in &part.writes {
                if let Some(after) = &w.after {
                    edited.set_source(&self.vdir.join(&w.path), after.clone().into_bytes());
                }
            }
            plan.uids = part.uids;
            plan.writes = part.writes;
            plan.holds = part.holds;
        }
        let Some(to) = to else {
            return Ok(plan);
        };
        let mv = Op::Move {
            uid: uid.to_string(),
            to: to.to_string(),
        };
        let part = self.plan(&edited, &mv, now)?;
        for w in part.writes {
            match plan.writes.iter().position(|e| e.path == w.path) {
                Some(k) => {
                    let first = plan.writes.remove(k);
                    plan.writes.push(Write {
                        before: first.before,
                        ..w
                    });
                }
                None => plan.writes.push(w),
            }
        }
        for u in part.uids {
            if !plan.uids.contains(&u) {
                plan.uids.push(u);
            }
        }
        plan.holds.extend(part.holds);
        let writes = &plan.writes;
        plan.holds
            .retain(|(rel, _)| writes.iter().all(|w| &w.path != rel));
        Ok(plan)
    }

    pub(super) fn plan(
        &self,
        snap: &Snapshot,
        op: &Op,
        now: DateTime<Utc>,
    ) -> Result<Plan, WriteError> {
        let one = |kind, uid: &String, w| Plan {
            kind,
            uids: vec![uid.clone()],
            writes: vec![w],
            holds: Vec::new(),
        };
        // A checklist or project with children: the op acts on the whole family.
        let parent = |uid: &str| snap.find(uid).filter(|&i| !snap.children(i).is_empty());
        Ok(match op {
            Op::Reorder { .. } | Op::CreateAt { .. } => self.plan_order(snap, op, now)?,
            Op::Create(n) => {
                let uid = new_uid();
                let parent_json_uid = n.parent.as_deref().and_then(|p| parent_json_uid(snap, p));
                // The C29 phone baseline copies a recurring checklist's due onto a
                // newly added item. Its dates remain inherited, not independent input.
                let mut inherited = n.clone();
                if let Some(p) = n.parent.as_deref().and_then(|p| snap.find(p))
                    && snap.tasks[p].kind == crate::model::TaskKind::Checklist
                    && snap.tasks[p].rrule.is_some()
                    && let Some(crate::ical::value::IcalDateTime::Date(due)) = snap.tasks[p].due
                {
                    inherited.due = Some(crate::edit::Due::Date(due));
                }
                let mut cal = new_task(&inherited, &uid, parent_json_uid.as_deref(), now)?;
                if let Some(p) = n.parent.as_deref().and_then(|p| snap.find(p)) {
                    let kids = snap.children(p);
                    let count = i64::try_from(kids.len()).map_err(|_| {
                        io_error(
                            io::ErrorKind::InvalidData,
                            "item order exceeds the supported range",
                        )
                    })?;
                    let next = match kids
                        .iter()
                        .map(|&i| snap.tasks[i].meta.display_order())
                        .max()
                    {
                        Some(max) => max.checked_add(1).ok_or_else(|| {
                            io_error(
                                io::ErrorKind::InvalidData,
                                "item order exceeds the supported range",
                            )
                        })?,
                        None => 0,
                    };
                    let todo = cal
                        .root
                        .children_named_mut("VTODO")
                        .next()
                        .expect("new task");
                    let prop = todo
                        .prop_mut("X-2DOAPP-METADATA")
                        .expect("new task metadata");
                    let mut meta = MetaJson::parse(&prop.value).expect("new task JSON");
                    meta.set_i64("DisplayOrder", next.max(count));
                    prop.set_value(&meta.encode());
                }
                let w = create(&n.collection, &uid, &cal)?;
                let mut plan = one("create", &uid, w);
                // The parent the gate checked must still be as loaded: not removed, nor
                // turned into a task that cannot have items.
                if let Some(p) = &n.parent {
                    plan.holds.push(self.target(snap, p)?.hold());
                    let idx = snap.find(p).expect("parent checked by gate");
                    for &i in snap.children(idx) {
                        plan.holds
                            .push(self.target(snap, &snap.tasks[i].uid)?.hold());
                    }
                }
                plan
            }
            Op::Edit { uid, changes }
                if let Some(i) = parent(uid)
                    && crate::gate::checklist_date_change(snap, i, op) =>
            {
                self.plan_checklist_date(snap, i, self.target(snap, uid)?, changes, now)?
            }
            Op::Edit { uid, changes } => {
                let t = self.target(snap, uid)?;
                let mut cal = t.cal.clone();
                apply_changes(&mut cal, changes, now)?;
                let mut plan = one("edit", uid, t.update(&cal)?);
                if let Some(p) = snap.find(uid).and_then(|i| snap.parent(i))
                    && snap.tasks[p].kind == crate::model::TaskKind::Checklist
                    && snap.tasks[p].rrule.is_some()
                {
                    plan.holds
                        .push(self.target(snap, &snap.tasks[p].uid)?.hold());
                }
                plan
            }
            Op::Complete { uid } if let Some(i) = parent(uid) => {
                self.plan_family_complete(snap, i, self.target(snap, uid)?, now)?
            }
            Op::Reopen { uid } if let Some(i) = parent(uid) => {
                self.plan_family_reopen(snap, i, self.target(snap, uid)?, now)?
            }
            Op::Delete { uid } if let Some(i) = parent(uid) => {
                self.plan_family_delete(snap, i, self.target(snap, uid)?)?
            }
            Op::Complete { uid } => {
                let t = self.target(snap, uid)?;
                if t.repeats {
                    // 2Do's copy-and-advance: the history copy first, then the live task.
                    // The last occurrence of an ending series has no copy.
                    let copy_uid = new_uid();
                    let today = now.with_timezone(&snap.tz()).date_naive();
                    match complete_recurring(&t.cal, now, today, &copy_uid)? {
                        (live, Some(copy)) => {
                            let copy = create(&t.collection, &copy_uid, &copy)?;
                            Plan {
                                kind: "complete",
                                uids: vec![uid.clone(), copy_uid],
                                writes: vec![copy, t.update(&live)?],
                                holds: Vec::new(),
                            }
                        }
                        (live, None) => one("complete", uid, t.update(&live)?),
                    }
                } else {
                    let mut cal = t.cal.clone();
                    complete_one_time(&mut cal, now)?;
                    one("complete", uid, t.update(&cal)?)
                }
            }
            Op::Reopen { uid } => {
                let t = self.target(snap, uid)?;
                let mut cal = t.cal.clone();
                if t.repeats {
                    reopen_ended_until(&mut cal, now)?;
                } else {
                    reopen_one_time(&mut cal, now)?;
                }
                one("reopen", uid, t.update(&cal)?)
            }
            Op::Reparent {
                uid,
                collection,
                parent,
            } => self.plan_reparent(snap, uid, collection, parent.as_deref(), now)?,
            Op::Move { uid, to } => {
                // H5 as captured: a new resource under a new UID, then the source removed.
                let t = self.target(snap, uid)?;
                if &t.collection == to {
                    return Err(EditError::Unsupported(format!("task is already in {to}")).into());
                }
                if let Some(i) = parent(uid) {
                    return self.plan_family_move(snap, i, t, to, now);
                }
                let new = new_uid();
                let copy = create(to, &new, &moved_copy(&t.cal, &new, None, now)?)?;
                Plan {
                    kind: "move",
                    uids: vec![uid.clone(), new],
                    writes: vec![copy, t.remove()],
                    holds: Vec::new(),
                }
            }
            Op::Save { uid, changes, to } => {
                self.plan_save(snap, uid, changes, to.as_deref(), now)?
            }
            Op::Delete { uid } => {
                let t = self.target(snap, uid)?;
                let mut plan = one("delete", uid, t.remove());
                if let Some(p) = snap.find(uid).and_then(|i| snap.parent(i)) {
                    // Preserve the loaded parent and siblings, including their schedules
                    // and states, until deletion commits under the shared lock.
                    for &i in std::iter::once(&p).chain(snap.children(p)) {
                        if snap.tasks[i].uid != *uid {
                            plan.holds
                                .push(self.target(snap, &snap.tasks[i].uid)?.hold());
                        }
                    }
                }
                plan
            }
        })
    }
}
