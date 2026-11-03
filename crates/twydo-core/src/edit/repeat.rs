//! 2Do's numeric encoding of a repeat rule (`docs/reference/2do/01-features.md` §6, "Exact
//! encoding"): the JSON `RecurrenceType` and `RecurrenceValue`.

use chrono::Weekday;

use crate::model::repeat::{Freq, RRule};

/// Weekday bit in a `RecurrenceType` 1..=127 mask: Sun 1, Sat 2, Fri 4, Thu 8, Wed 16, Tue 32,
/// Mon 64.
fn day_bit(w: Weekday) -> i64 {
    match w {
        Weekday::Sun => 1,
        Weekday::Sat => 2,
        Weekday::Fri => 4,
        Weekday::Thu => 8,
        Weekday::Wed => 16,
        Weekday::Tue => 32,
        Weekday::Mon => 64,
    }
}

const WEEKDAYS: [Weekday; 5] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
];
const WEEKEND: [Weekday; 2] = [Weekday::Sat, Weekday::Sun];

fn is_set(by_day: &[(Option<i8>, Weekday)], set: &[Weekday]) -> bool {
    by_day.len() == set.len() && set.iter().all(|w| by_day.iter().any(|(_, d)| d == w))
}

/// `(RecurrenceType, RecurrenceValue)` for `r`. A rule 2Do has no code for (such as a plain
/// `FREQ=MONTHLY` with `BYMONTHDAY=5`) falls back to its plain every-N-units code.
pub fn twodo_codes(r: &RRule) -> (i64, i64) {
    let interval = i64::from(r.interval);
    match r.freq {
        Freq::Daily => (0x100, interval),
        Freq::Weekly if r.by_day.is_empty() => (0x101, interval),
        Freq::Weekly => (r.by_day.iter().map(|(_, w)| day_bit(*w)).sum(), interval),
        Freq::Yearly => (0x103, interval),
        Freq::Monthly => {
            let pos = r.by_set_pos.first().map_or(-1, |p| i64::from(*p));
            if r.by_month_day == [-1] {
                (0x10B, interval)
            } else if let [(Some(n), w)] = r.by_day[..] {
                (0x104 + i64::from(w.num_days_from_monday()), i64::from(n))
            } else if is_set(&r.by_day, &WEEKDAYS) && !r.by_set_pos.is_empty() {
                (0x10C, pos)
            } else if is_set(&r.by_day, &WEEKEND) && !r.by_set_pos.is_empty() {
                (0x10D, pos)
            } else {
                (0x102, interval)
            }
        }
    }
}
