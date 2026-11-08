//! Candidate membership writer. Delete/recreate identities are borrowed from list moves;
//! actual reparent interoperability is not established and requires a dedicated gate row.
use super::*;

/// Recreate one plain leaf, replacing/removing both parent representations.
/// Foreign RUID and internal identities survive; self identities follow the new UID.
/// `clear_dates` allows only simple date-only due/mirrored start cleanup.
pub fn reparented_copy(
    cal: &Calendar,
    new_uid: &str,
    parent: Option<&str>,
    clear_dates: bool,
    now: DateTime<Utc>,
) -> Result<Calendar, EditError> {
    let mut source = cal.clone();
    let todo = the_todo(&mut source)?;
    for (name, max) in [
        ("UID", 1),
        ("RELATED-TO", 1),
        (META_PROP, 1),
        ("DUE", 1),
        ("DTSTART", 1),
        ("RRULE", 1),
    ] {
        let n = todo.props_named(name).count();
        if n > max || (matches!(name, "UID" | META_PROP) && n != 1) {
            return Err(unsupported(
                "ambiguous reparent identity, metadata or dates",
            ));
        }
    }
    if let Some(p) = todo.prop("RELATED-TO")
        && p.param("RELTYPE")
            .is_some_and(|s| !s.eq_ignore_ascii_case("PARENT"))
    {
        return Err(unsupported("unsupported reparent relationship type"));
    }
    let Meta::Json(meta) = meta_of(todo) else {
        return Err(unsupported(READ_ONLY));
    };
    for key in ["uid", "RUID", "parentuid"] {
        if meta.has(key) && meta.get_str(key).is_none() {
            return Err(unsupported("unreadable reparent identity"));
        }
    }
    if clear_dates {
        if todo.prop("RRULE").is_some()
            || todo.children_named("VALARM").next().is_some()
            || meta.has("duetime")
            || meta.has("StartDate")
        {
            return Err(unsupported(
                "checklist membership cannot clear recurrence, alarms or own start/time",
            ));
        }
        for name in ["DUE", "DTSTART"] {
            if let Some(p) = todo.prop(name)
                && !matches!(
                    crate::ical::value::parse_date_time(p),
                    Ok(IcalDateTime::Date(_))
                )
            {
                return Err(unsupported(
                    "checklist membership needs simple date-only dates",
                ));
            }
        }
        family::member_has_inherited_dates(&source)?;
        if todo_due(&source) {
            drop_member_dates(&mut source, now)?;
        }
    }
    let mut out = moved_copy(&source, new_uid, parent, now)?;
    let todo = the_todo(&mut out)?;
    if parent.is_none() {
        todo.remove_props("RELATED-TO");
        let Meta::Json(orig) = meta_of(todo) else {
            unreachable!()
        };
        let mut meta = orig.clone();
        meta.remove("parentuid");
        write_meta(todo, Some(&orig), &meta);
    }
    Ok(out)
}

fn todo_due(cal: &Calendar) -> bool {
    cal.root
        .children_named("VTODO")
        .next()
        .unwrap()
        .prop("DUE")
        .is_some()
}
