//! The words for a write: the title of the task an operation touches and the success message.
//! Pure: reads the snapshot, does no I/O.

use crate::edit::Op;
use crate::model::Status;
use crate::views::Snapshot;

/// The title of the task an operation writes, as it is before the write.
pub fn op_title(snap: &Snapshot, op: &Op) -> String {
    let uid = match op {
        Op::Create(t) | Op::CreateAt { task: t, .. } => return t.summary.trim().to_string(),
        Op::Edit { uid, .. }
        | Op::Save { uid, .. }
        | Op::Complete { uid }
        | Op::Reopen { uid }
        | Op::Reparent { uid, .. }
        | Op::Move { uid, .. }
        | Op::Delete { uid }
        | Op::Reorder { uid, .. } => uid,
    };
    snap.find(uid)
        .map_or_else(|| uid.clone(), |i| snap.tasks[i].summary.clone())
}

/// How many children an operation writes with the task, from the snapshot before the write.
fn family_size(snap: &Snapshot, op: &Op) -> usize {
    let (Op::Complete { uid } | Op::Reopen { uid } | Op::Move { uid, .. } | Op::Delete { uid }) =
        op
    else {
        return 0;
    };
    let Some(idx) = snap.find(uid) else { return 0 };
    let kids = snap.children(idx);
    match op {
        Op::Complete { .. } if snap.tasks[idx].rrule.is_some() => kids.len(),
        Op::Complete { .. } => snap.open_items(idx),
        Op::Reopen { .. } => kids
            .iter()
            .filter(|&&c| snap.tasks[c].status == Status::Completed)
            .count(),
        _ => kids.len(),
    }
}

pub fn success_message(snap: &Snapshot, op: &Op) -> String {
    let n = family_size(snap, op);
    let title = op_title(snap, op);
    let items = if n == 1 { "item" } else { "items" };
    let with = |verb: &str| {
        if n > 0 {
            format!("{verb}: \"{title}\" and {n} {items}")
        } else {
            verb.to_string()
        }
    };
    match op {
        Op::Complete { .. } => with("done"),
        Op::Reopen { .. } => with("reopened"),
        Op::Delete { .. } if n > 0 => format!("deleted \"{title}\" and {n} {items} · u to undo"),
        Op::Delete { .. } => "deleted · u to undo".into(),
        Op::Edit { .. } | Op::Save { .. } => "saved".into(),
        Op::Create(_) | Op::CreateAt { .. } => "added".into(),
        Op::Reorder { .. } => "reordered · u to undo".into(),
        Op::Reparent { .. } => "relocated · u to undo".into(),
        Op::Move { to, .. } => {
            let name = snap
                .collections
                .iter()
                .find(|c| &c.id == to)
                .and_then(|c| c.display_name.clone())
                .unwrap_or_else(|| to.clone());
            if n > 0 {
                format!("moved \"{title}\" and {n} {items} to {name}")
            } else {
                format!("moved to {name}")
            }
        }
    }
}
