//! Edits of a checklist's or project's members, as the 2Do iPhone app writes them when the
//! parent changes (`plan6-list1`, `plan6-list2`; `docs/phase0/plan6-baseline.md`).
//!
//! A member points at its parent with `RELATED-TO:<parent UID>` and JSON `parentuid`.
//! - Completing the parent completes every open member at the parent's completion time; a
//!   repeating member keeps its `RRULE` and due date (`05-complete-parent` p3).
//! - Reopening the parent reopens every completed member, repeating or not
//!   (`06-reopen-parent` p3 and its history copy).
//! - Completing a repeating project copies each member under the project's history copy
//!   (`05-complete-parent` r1, r2): an open member's copy is completed, a completed one's
//!   keeps its `COMPLETED`.
//!
//! Each function edits one resource; the store applies them to a whole family.

use chrono::{DateTime, Utc};

use super::complete::{State, mark_completed, mark_open, state};
use super::{EditError, Meta, meta_of, the_todo, touch, unsupported, write_meta};
use crate::ical::{Calendar, Component, Property};

/// Complete an open member at `at` (its parent's completion time), stamps = `now`. A
/// repeating member keeps its `RRULE` and due date. All or nothing.
pub fn complete_member(
    cal: &mut Calendar,
    at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<(), EditError> {
    let mut work = cal.clone();
    let todo = the_todo(&mut work)?;
    match state(todo) {
        State::Open => {}
        State::Completed => return Err(unsupported("task is already completed")),
        State::Cancelled => {
            return Err(unsupported("completing a cancelled task is not supported"));
        }
    }
    mark_completed(todo, at);
    touch(todo, now);
    *cal = work;
    Ok(())
}

/// Reopen a completed member, repeating or not, as `reopen_one_time` does. All or nothing.
pub fn reopen_member(cal: &mut Calendar, now: DateTime<Utc>) -> Result<(), EditError> {
    let mut work = cal.clone();
    let todo = the_todo(&mut work)?;
    match state(todo) {
        State::Completed => {}
        State::Open => return Err(unsupported("task is not completed")),
        State::Cancelled => return Err(unsupported("reopening a cancelled task is not supported")),
    }
    mark_open(todo);
    touch(todo, now);
    *cal = work;
    Ok(())
}

/// The copy of a repeating project's member that goes under the project's history copy
/// `parent_uid`: `UID` = `copy_uid`, parent link = `parent_uid`, stamps = `now`; an open
/// member is completed at `at`, a completed one keeps its `COMPLETED`. Every other line keeps
/// its bytes; `cal` is not changed. A repeating member is refused (not captured).
pub fn member_copy(
    cal: &Calendar,
    copy_uid: &str,
    parent_uid: &str,
    at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<Calendar, EditError> {
    let mut work = cal.clone();
    let todo = the_todo(&mut work)?;
    let uid = todo
        .prop_mut("UID")
        .ok_or_else(|| unsupported("task has no UID"))?;
    if copy_uid.is_empty() || uid.value == copy_uid {
        return Err(unsupported("a member's copy needs a new UID"));
    }
    uid.set_value(copy_uid);
    if todo.prop("RRULE").is_some() {
        return Err(unsupported("copying a repeating member is not verified"));
    }
    match state(todo) {
        State::Open => mark_completed(todo, at),
        State::Completed => {}
        State::Cancelled => {
            return Err(unsupported("copying a cancelled member is not supported"));
        }
    }
    match meta_of(todo) {
        Meta::Json(orig) => {
            let mut m = orig.clone();
            m.set_str("parentuid", parent_uid);
            write_meta(todo, Some(&orig), &m);
        }
        Meta::UnreadableJson => return Err(unsupported("this task's 2Do metadata is unreadable")),
        Meta::Tokens => return Err(unsupported("this task's 2Do metadata format is read-only")),
        Meta::Absent => {}
    }
    set_related_to(todo, parent_uid)?;
    touch(todo, now);
    Ok(work)
}

/// Point `RELATED-TO` at `parent_uid`, keeping the line's bytes when it already does. A
/// missing one goes after `UID`, as 2Do places it. A link that is not to a parent (another
/// `RELTYPE`) or several links are refused.
pub(super) fn set_related_to(todo: &mut Component, parent_uid: &str) -> Result<(), EditError> {
    let links: Vec<_> = todo.props_named("RELATED-TO").collect();
    if links.len() > 1 {
        return Err(unsupported("several RELATED-TO links are not supported"));
    }
    let not_parent = links.first().is_some_and(|p| {
        p.params.iter().any(|q| {
            q.name.eq_ignore_ascii_case("RELTYPE")
                && !q.values.iter().all(|v| v.eq_ignore_ascii_case("PARENT"))
        })
    });
    if not_parent {
        return Err(unsupported(
            "a RELATED-TO link other than PARENT is not supported",
        ));
    }
    match todo.prop_mut("RELATED-TO") {
        Some(p) if p.value == parent_uid => {}
        Some(p) => p.set_value(parent_uid),
        None => todo.insert_prop(
            Property::new("RELATED-TO", Vec::new(), parent_uid),
            Some("UID"),
        ),
    }
    Ok(())
}
