//! The pure planner: the gate and the plan of one operation on a loaded snapshot, with no
//! I/O. [`plan`] produces the [`Prepared`] entry; the [`Writer`](super::Writer) adds what reads
//! the state directory (the collection fence) and the request.

use std::io;
use std::path::{Component, Path};

use chrono::{DateTime, Utc};

use super::journal::{Entry, FORMAT, FamilyGuard, Stage, Write, new_op_id};
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
use crate::model::{Task, TaskKind};
use crate::views::Snapshot;

/// The write policy an operation is gated and planned under.
#[derive(Clone, Copy)]
pub struct Policy<'a> {
    pub matrix: &'a Matrix,
    pub mode: WriteMode,
}

/// The gate and the plan: the journal entry `op` produces on the files loaded in `snap`,
/// with the holds and family it must find unchanged when it is committed. Reads nothing
/// beyond `snap`. The entry carries no request; the writer stamps it.
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
    let holds = std::mem::take(&mut plan.holds);
    let mut family = plan.reads.family.take();
    if let Op::Move { uid, .. }
    | Op::Save {
        uid, to: Some(_), ..
    } = op
    {
        let idx = snap.find(uid).expect("gate checked move target");
        let mut source = planner.loaded_family(snap, idx)?;
        source
            .loaded
            .extend(plan.writes.iter().map(|w| w.path.clone()));
        family = Some(source);
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
        holds: match plan.reads.journal {
            JournalReads::None => Vec::new(),
            JournalReads::Holds | JournalReads::FamilyAndHolds => holds.clone(),
        },
        family: match plan.reads.journal {
            JournalReads::FamilyAndHolds => family.clone(),
            JournalReads::None | JournalReads::Holds => None,
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

/// Which read dependencies survive the initial commit for recovery and inverses.
#[derive(Default)]
pub(super) enum JournalReads {
    #[default]
    None,
    Holds,
    FamilyAndHolds,
}

#[derive(Default)]
pub(super) struct PlanReads {
    pub(super) family: Option<FamilyGuard>,
    pub(super) journal: JournalReads,
}

/// The parts of a journal entry an operation produces.
pub(super) struct Plan {
    pub(super) reads: PlanReads,
    pub(super) kind: &'static str,
    pub(super) uids: Vec<String>,
    pub(super) writes: Vec<Write>,
    /// Loaded files the op reads but does not write (`rel`, loaded bytes): a family member
    /// left as it is. Each must still hold its bytes when the entry is applied.
    pub(super) holds: Vec<(String, String)>,
}

impl Plan {
    fn one(kind: &'static str, uid: &str, write: Write) -> Plan {
        Plan {
            reads: Default::default(),
            kind,
            uids: vec![uid.to_owned()],
            writes: vec![write],
            holds: Vec::new(),
        }
    }

    /// For an operation whose holds and writes together cover one complete family.
    /// Including a created path lets replay recognize an already-applied creation.
    pub(super) fn with_family_guard(mut self, head: &Task) -> Self {
        let loaded = self
            .holds
            .iter()
            .map(|(path, _)| path.clone())
            .chain(self.writes.iter().map(|w| w.path.clone()))
            .collect();
        self.reads.journal = JournalReads::FamilyAndHolds;
        self.reads.family = Some(FamilyGuard {
            collection: head.collection.clone(),
            parent_uid: head.uid.clone(),
            parent_aliases: crate::gate::relationships::aliases(head),
            loaded,
        });
        self
    }
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

    fn complete_leaf(
        &self,
        snap: &Snapshot,
        uid: &str,
        t: Target,
        now: DateTime<Utc>,
    ) -> Result<Plan, WriteError> {
        Ok(if t.repeats {
            // 2Do's copy-and-advance: the history copy first, then the live task.
            // The last occurrence of an ending series has no copy.
            let copy_uid = new_uid();
            let today = now.with_timezone(&snap.tz()).date_naive();
            match complete_recurring(&t.cal, now, today, &copy_uid)? {
                (live, Some(copy)) => {
                    let copy = create(&t.collection, &copy_uid, &copy)?;
                    Plan {
                        reads: Default::default(),
                        kind: "complete",
                        uids: vec![uid.to_owned(), copy_uid],
                        writes: vec![copy, t.update(&live)?],
                        holds: Vec::new(),
                    }
                }
                (live, None) => Plan::one("complete", uid, t.update(&live)?),
            }
        } else {
            let mut cal = t.cal.clone();
            complete_one_time(&mut cal, now)?;
            Plan::one("complete", uid, t.update(&cal)?)
        })
    }

    fn edit_fields(
        &self,
        snap: &Snapshot,
        idx: usize,
        changes: &[Change],
        now: DateTime<Utc>,
    ) -> Result<Plan, WriteError> {
        let uid = &snap.tasks[idx].uid;
        let target = self.target(snap, uid)?;
        if crate::gate::checklist_date_changes(snap, idx, changes) {
            return self.plan_checklist_date(snap, idx, target, changes, now);
        }
        let mut cal = target.cal.clone();
        apply_changes(&mut cal, changes, now)?;
        let mut plan = Plan::one("edit", uid, target.update(&cal)?);
        if let Some(parent) = snap.parent(idx) {
            let parent = &snap.tasks[parent];
            if parent.kind == TaskKind::Checklist && parent.rrule.is_some() {
                plan.holds.push(self.target(snap, &parent.uid)?.hold());
                plan.reads.journal = JournalReads::Holds;
            }
        }
        Ok(plan)
    }

    fn hold_unwritten_members(
        &self,
        snap: &Snapshot,
        head: usize,
        plan: &mut Plan,
    ) -> Result<(), WriteError> {
        for i in std::iter::once(head).chain(snap.children(head).iter().copied()) {
            let member = &snap.tasks[i];
            let path = self.rel(&member.path)?;
            if !plan.writes.iter().any(|w| w.path == path)
                && !plan.holds.iter().any(|(held, _)| *held == path)
            {
                plan.holds.push(self.target(snap, &member.uid)?.hold());
            }
        }
        Ok(())
    }

    fn checklist_schedule(task: &Task, changes: &[Change]) -> bool {
        task.kind == TaskKind::Checklist
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

    fn plan_edit(
        &self,
        snap: &Snapshot,
        uid: &str,
        changes: &[Change],
        now: DateTime<Utc>,
    ) -> Result<Plan, WriteError> {
        let idx = snap.find(uid).expect("gate checked edit target");
        let mut plan = self.edit_fields(snap, idx, changes, now)?;
        if crate::edit::text_only(changes) {
            let head = snap.parent(idx).unwrap_or(idx);
            self.hold_unwritten_members(snap, head, &mut plan)?;
            let mut family = self.loaded_family(snap, head)?;
            // Text admission also checks for hidden descendants and duplicate identities of siblings.
            for &child in snap.children(head) {
                family
                    .parent_aliases
                    .extend(crate::gate::relationships::aliases(&snap.tasks[child]));
            }
            plan.reads = PlanReads {
                family: Some(family),
                journal: JournalReads::FamilyAndHolds,
            };
        } else if Self::checklist_schedule(&snap.tasks[idx], changes) {
            self.hold_unwritten_members(snap, idx, &mut plan)?;
            plan = plan.with_family_guard(&snap.tasks[idx]);
        } else if changes.iter().any(|c| matches!(c, Change::Kind(_))) {
            plan.reads.family = Some(self.loaded_family(snap, idx)?);
        }
        Ok(plan)
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
        let Some(to) = to else {
            let mut edit = self.plan_edit(snap, uid, changes, now)?;
            edit.kind = "save";
            return Ok(edit);
        };
        let idx = snap.find(uid).expect("gate checked save target");
        let mut plan = Plan {
            reads: Default::default(),
            kind: "save",
            uids: Vec::new(),
            writes: Vec::new(),
            holds: Vec::new(),
        };
        let mut edited = snap.clone();
        if !changes.is_empty() {
            let part = self.edit_fields(snap, idx, changes, now)?;
            plan.reads.journal = part.reads.journal;
            for w in &part.writes {
                if let Some(after) = &w.after {
                    edited.set_source(&self.vdir.join(&w.path), after.clone().into_bytes());
                }
            }
            plan.uids = part.uids;
            plan.writes = part.writes;
            plan.holds = part.holds;
        }
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
        if Self::checklist_schedule(&snap.tasks[idx], changes) {
            self.hold_unwritten_members(snap, idx, &mut plan)?;
            plan.reads.journal = JournalReads::FamilyAndHolds;
        }
        Ok(plan)
    }

    pub(super) fn plan(
        &self,
        snap: &Snapshot,
        op: &Op,
        now: DateTime<Utc>,
    ) -> Result<Plan, WriteError> {
        // A checklist or project with children: the op acts on the whole family.
        let parent = |uid: &str| snap.find(uid).filter(|&i| !snap.children(i).is_empty());
        Ok(match op {
            Op::Reorder { .. } | Op::CreateAt { .. } => self.plan_order(snap, op, now)?,
            Op::Create(n) => {
                let uid = new_uid();
                let parent_idx = n.parent.as_deref().and_then(|uid| snap.find(uid));
                let parent_json_uid = n.parent.as_deref().and_then(|p| parent_json_uid(snap, p));
                // The C29 phone baseline copies a recurring checklist's due onto a
                // newly added item. Its dates remain inherited, not independent input.
                let mut inherited = n.clone();
                if let Some(p) = parent_idx
                    && snap.tasks[p].kind == crate::model::TaskKind::Checklist
                    && snap.tasks[p].rrule.is_some()
                    && let Some(crate::ical::value::IcalDateTime::Date(due)) = snap.tasks[p].due
                {
                    inherited.due = Some(crate::edit::Due::Date(due));
                }
                let mut cal = new_task(&inherited, &uid, parent_json_uid.as_deref(), now)?;
                if let Some(p) = parent_idx {
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
                let mut plan = Plan::one("create", &uid, w);
                if let Some(idx) = parent_idx {
                    for i in std::iter::once(idx).chain(snap.children(idx).iter().copied()) {
                        plan.holds
                            .push(self.target(snap, &snap.tasks[i].uid)?.hold());
                    }
                    plan = plan.with_family_guard(&snap.tasks[idx]);
                }
                plan
            }
            Op::Edit { uid, changes } => self.plan_edit(snap, uid, changes, now)?,
            Op::Complete { uid } => {
                let target = self.target(snap, uid)?;
                let idx = snap.find(uid).expect("target exists");
                let mut plan = if snap.children(idx).is_empty() {
                    self.complete_leaf(snap, uid, target, now)?
                } else {
                    self.plan_family_complete(snap, idx, target, now)?
                };
                plan.reads.family = Some(self.loaded_family(snap, idx)?);
                plan
            }
            Op::Reopen { uid } => {
                let t = self.target(snap, uid)?;
                let idx = snap.find(uid).expect("target exists");
                let plan = if snap.children(idx).is_empty() {
                    let mut cal = t.cal.clone();
                    if t.repeats {
                        reopen_ended_until(&mut cal, now)?;
                    } else {
                        reopen_one_time(&mut cal, now)?;
                    }
                    Plan::one("reopen", uid, t.update(&cal)?)
                } else {
                    self.plan_family_reopen(snap, idx, t, now)?
                };
                plan.with_family_guard(&snap.tasks[idx])
            }
            Op::Delete { uid } if let Some(i) = parent(uid) => {
                self.plan_family_delete(snap, i, self.target(snap, uid)?)?
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
                    reads: Default::default(),
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
                let mut plan = Plan::one("delete", uid, t.remove());
                let idx = snap.find(uid).expect("target exists");
                let head = snap.parent(idx).unwrap_or(idx);
                if head != idx {
                    // Preserve the loaded parent and siblings, including their schedules
                    // and states, until deletion commits under the shared lock.
                    for &i in std::iter::once(&head).chain(snap.children(head)) {
                        if snap.tasks[i].uid != *uid {
                            plan.holds
                                .push(self.target(snap, &snap.tasks[i].uid)?.hold());
                        }
                    }
                }
                plan.with_family_guard(&snap.tasks[head])
            }
        })
    }
}
