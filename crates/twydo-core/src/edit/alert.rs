//! Setting, changing and removing a task's custom alert the way 2Do does (spec "Capture
//! results"): an existing VALARM gets a new `TRIGGER` in place, a task without one gets A3's
//! shape, and removal deletes the VALARM without restoring the automatic alert.

use super::{EditError, unsupported};
use crate::ical::{Component, Param, Property};
use crate::model::alert::{AlertOffset, CustomAlert, custom_alert};

pub(crate) const NEEDS_DUE: &str = "an alert needs a due date";
pub(crate) const CANT_EDIT: &str = "this task has alerts Twydo can't edit";

/// `-PT15M`, `-PT1H30M`, `-P1D`, `-P1DT1H`: days when whole, then hours and minutes. Weeks are
/// written as days (2Do wrote `-P1D`; a `W` form was never observed).
pub(crate) fn trigger_value(offset: AlertOffset) -> String {
    let m = offset.minutes();
    let (days, rest) = (m / 1440, m % 1440);
    let (hours, mins) = (rest / 60, rest % 60);
    let mut s = String::from("-P");
    if days > 0 {
        s += &format!("{days}D");
    }
    if rest > 0 {
        s.push('T');
        if hours > 0 {
            s += &format!("{hours}H");
        }
        if mins > 0 {
            s += &format!("{mins}M");
        }
    }
    s
}

/// A3's VALARM: a new alert added on the phone.
fn new_alarm(summary: &str, offset: AlertOffset) -> Component {
    let global = || {
        vec![Param {
            name: "SHARE-SCOPE".into(),
            values: vec!["GLOBAL".into()],
        }]
    };
    let mut a = Component::new("VALARM");
    let trigger = trigger_value(offset);
    for (name, params, value) in [
        ("TRIGGER", Vec::new(), trigger.as_str()),
        ("ACTION", Vec::new(), "AUDIO"),
        ("DESCRIPTION", Vec::new(), summary),
        ("X-2DOAPP-SOUND", global(), "Basso"),
        ("X-2DOAPP-RELDUETIME", global(), "1"),
    ] {
        a.insert_prop(Property::new(name, params, value), None);
    }
    a
}

pub(crate) fn set_alert(todo: &mut Component, alert: Option<AlertOffset>) -> Result<(), EditError> {
    let current = custom_alert(todo);
    match (alert, current) {
        (_, CustomAlert::Unsupported) => Err(unsupported(CANT_EDIT)),
        (None, CustomAlert::None) => Ok(()),
        // Removing deletes the single VALARM; 2Do does not restore the automatic one (A2).
        (None, CustomAlert::One(_) | CustomAlert::AtDue) => {
            todo.remove_children(|c| c.name == "VALARM");
            Ok(())
        }
        (Some(_), _) if todo.prop("DUE").is_none() => Err(unsupported(NEEDS_DUE)),
        (Some(offset), CustomAlert::One(o)) if o == offset => Ok(()),
        // A new alert takes the automatic alert's place in place (A1).
        (Some(offset), CustomAlert::One(_) | CustomAlert::AtDue) => {
            let alarm = todo
                .children_named_mut("VALARM")
                .next()
                .ok_or_else(|| unsupported(CANT_EDIT))?;
            let p = alarm
                .prop_mut("TRIGGER")
                .ok_or_else(|| unsupported(CANT_EDIT))?;
            p.set_value(&trigger_value(offset));
            Ok(())
        }
        (Some(offset), CustomAlert::None) => {
            let summary = todo.prop("SUMMARY").map_or("", |p| p.value.as_str());
            let alarm = new_alarm(summary, offset);
            todo.push_child(alarm);
            Ok(())
        }
    }
}
