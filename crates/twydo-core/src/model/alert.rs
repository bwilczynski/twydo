//! A task's single editable alert, as 2Do writes it (alerts round 2026-11-14,
//! `fixtures/set-alert/phone-encodings`). Read side only; the writer is `edit::alert`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ical::Component;
use crate::ical::value::parse_duration;

/// Whole minutes before the due value, at least one.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "u32", into = "u32")]
pub struct AlertOffset(u32);

impl AlertOffset {
    pub fn new(minutes: u32) -> Option<Self> {
        (minutes > 0).then_some(Self(minutes))
    }

    pub fn minutes(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for AlertOffset {
    type Error = &'static str;
    fn try_from(minutes: u32) -> Result<Self, Self::Error> {
        Self::new(minutes).ok_or("an alert offset must be at least one minute")
    }
}

impl From<AlertOffset> for u32 {
    fn from(o: AlertOffset) -> u32 {
        o.0
    }
}

/// What Twydo can edit of a task's alerts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CustomAlert {
    /// No alert.
    #[default]
    None,
    /// Only 2Do's automatic at-due-time alert: shown as `at due`, removable, not re-added.
    AtDue,
    /// One 2Do alert before the due value.
    One(AlertOffset),
    /// Absolute, after-due, end-related, several or non-2Do alerts: kept, never edited.
    Unsupported,
}

/// The relative offset of a VALARM's `TRIGGER` in seconds, if it is a plain duration
/// without `VALUE=DATE-TIME` or `RELATED=END`.
fn relative_seconds(alarm: &Component) -> Option<i64> {
    let p = alarm.prop("TRIGGER")?;
    if p.param("VALUE")
        .is_some_and(|v| v.eq_ignore_ascii_case("DATE-TIME"))
        || p.param("RELATED")
            .is_some_and(|r| r.eq_ignore_ascii_case("END"))
    {
        return None;
    }
    parse_duration(p.value.trim()).ok().map(|d| d.num_seconds())
}

/// 2Do's automatic at-due-time alert: `X-2DOAPP-ISAUTO:1` and a zero offset. An edited
/// default alert keeps `ISAUTO:1` with its new offset and is a custom alert (A1).
pub fn is_auto_alarm(c: &Component) -> bool {
    c.name == "VALARM"
        && c.prop("X-2DOAPP-ISAUTO")
            .is_some_and(|p| p.value.trim() == "1")
        && c.prop("TRIGGER")
            .and_then(|p| parse_duration(p.value.trim()).ok())
            .is_some_and(|d| d.is_zero())
}

/// The VALARM shape 2Do writes for every alert: `ACTION:AUDIO` with `X-2DOAPP-SOUND`.
fn two_do_shaped(c: &Component) -> bool {
    c.name == "VALARM"
        && c.prop("ACTION")
            .is_some_and(|p| p.value.trim().eq_ignore_ascii_case("AUDIO"))
        && c.prop("X-2DOAPP-SOUND").is_some()
}

/// The offset of a 2Do-shaped alert before the due value (`ACTION:AUDIO`, `X-2DOAPP-SOUND`, a
/// negative whole-minute relative trigger), whatever its `ISAUTO`/`RELDUETIME`.
pub fn custom_offset(c: &Component) -> Option<AlertOffset> {
    if !two_do_shaped(c) {
        return None;
    }
    let secs = relative_seconds(c)?;
    if secs >= 0 || secs % 60 != 0 {
        return None;
    }
    u32::try_from(-secs / 60).ok().and_then(AlertOffset::new)
}

/// Classify a VTODO's VALARMs. 2Do keeps a single alert per task (alerts round Q2), so only one
/// 2Do-shaped VALARM is editable; anything else is kept as it is.
pub fn custom_alert(todo: &Component) -> CustomAlert {
    let alarms: Vec<_> = todo.children_named("VALARM").collect();
    match alarms.as_slice() {
        [] => CustomAlert::None,
        [a] if is_auto_alarm(a) && two_do_shaped(a) => CustomAlert::AtDue,
        [a] => custom_offset(a).map_or(CustomAlert::Unsupported, CustomAlert::One),
        _ => CustomAlert::Unsupported,
    }
}
