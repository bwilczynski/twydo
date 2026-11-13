//! Explicit sibling ordering: one guarded, recoverable family operation.

use chrono::{DateTime, Utc};

use super::plan::{Plan, Planner, create};
use super::{WriteError, parent_json_uid};
use crate::edit::{Due, EditError, Op, Placement, new_task, new_uid, touch};
use crate::ical::value::IcalDateTime;
use crate::model::{TaskKind, meta_edit::MetaJson};
use crate::views::{Snapshot, compare_items};

fn invalid(message: &str) -> WriteError {
    EditError::Unsupported(message.into()).into()
}

fn rank(cal: &mut crate::ical::Calendar, value: i64) -> Result<(), WriteError> {
    let todo = cal
        .root
        .children_named_mut("VTODO")
        .next()
        .ok_or(EditError::NoTodo)?;
    let prop = todo
        .prop_mut("X-2DOAPP-METADATA")
        .ok_or_else(|| invalid("item ordering needs editable JSON metadata"))?;
    let mut meta = MetaJson::parse(&prop.value)
        .ok_or_else(|| invalid("item ordering needs editable JSON metadata"))?;
    meta.set_i64("DisplayOrder", value);
    prop.set_value(&meta.encode());
    Ok(())
}

impl Planner<'_> {
    pub(super) fn plan_order(
        &self,
        snap: &Snapshot,
        op: &Op,
        now: DateTime<Utc>,
    ) -> Result<Plan, WriteError> {
        let (anchor, placement) = match op {
            Op::Reorder {
                anchor, placement, ..
            }
            | Op::CreateAt {
                anchor, placement, ..
            } => (anchor, placement),
            _ => unreachable!("ordering op"),
        };
        let a = snap
            .find(anchor)
            .ok_or_else(|| invalid("item anchor no longer exists"))?;
        let p = snap
            .parent(a)
            .ok_or_else(|| invalid("item anchor has no parent"))?;
        let mut sequence = snap.children(p).to_vec();
        sequence.sort_by(|&a, &b| compare_items(snap, snap.tz(), a, b));
        let (uid, mut fresh) = match op {
            Op::Reorder { uid, .. } => {
                let i = snap
                    .find(uid)
                    .ok_or_else(|| invalid("item no longer exists"))?;
                sequence.retain(|&n| n != i);
                (uid.clone(), None)
            }
            Op::CreateAt { task, .. } => {
                let uid = new_uid();
                let parent = &snap.tasks[p];
                let mut inherited = task.clone();
                if parent.kind == TaskKind::Checklist
                    && parent.rrule.is_some()
                    && let Some(IcalDateTime::Date(due)) = parent.due
                {
                    inherited.due = Some(Due::Date(due));
                }
                let cal = new_task(
                    &inherited,
                    &uid,
                    parent_json_uid(snap, &parent.uid).as_deref(),
                    now,
                )?;
                (uid, Some((task.collection.clone(), cal)))
            }
            _ => unreachable!(),
        };
        let at = sequence
            .iter()
            .position(|&i| i == a)
            .ok_or_else(|| invalid("an item cannot be its own anchor"))?
            + usize::from(*placement == Placement::After);
        let mut uids: Vec<String> = sequence
            .iter()
            .map(|&i| snap.tasks[i].uid.clone())
            .collect();
        uids.insert(at, uid.clone());
        let mut plan = Plan {
            kind: if fresh.is_some() { "insert" } else { "reorder" },
            uids: vec![uid.clone()],
            writes: Vec::new(),
            holds: vec![self.target(snap, &snap.tasks[p].uid)?.hold()],
        };
        for (position, member) in uids.iter().enumerate() {
            let position = i64::try_from(position)
                .map_err(|_| invalid("item order exceeds the supported range"))?;
            if member == &uid
                && let Some((collection, mut cal)) = fresh.take()
            {
                rank(&mut cal, position)?;
                plan.writes.push(create(&collection, member, &cal)?);
                continue;
            }
            let target = self.target(snap, member)?;
            // The reader's zero fallback is not a saved numeric rank. Explicit ordering
            // must materialize zero too when the shared value is absent or invalid.
            let saved = target
                .cal
                .root
                .children_named("VTODO")
                .next()
                .and_then(|todo| todo.prop("X-2DOAPP-METADATA"))
                .and_then(|prop| MetaJson::parse(&prop.value))
                .and_then(|meta| meta.get_i64("DisplayOrder"));
            if saved == Some(position) {
                plan.holds.push(target.hold());
                continue;
            }
            let mut cal = target.cal.clone();
            rank(&mut cal, position)?;
            touch(
                cal.root
                    .children_named_mut("VTODO")
                    .next()
                    .expect("target todo"),
                now,
            );
            plan.writes.push(target.update(&cal)?);
            if member != &uid {
                plan.uids.push(member.clone());
            }
        }
        Ok(plan)
    }
}
