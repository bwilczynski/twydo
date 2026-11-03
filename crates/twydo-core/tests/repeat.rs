mod common;

use chrono::{NaiveDate, Weekday};
use twydo_core::config::WriteMode;
use twydo_core::model::repeat::{
    End, Freq, RULE_CLASSES, describe, parse_rrule, rule_class, short_hint,
};
use twydo_core::model::task::Task;

fn d(s: &str) -> Option<NaiveDate> {
    if s == "-" {
        None
    } else {
        Some(NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap())
    }
}

#[test]
fn describe_and_classify() {
    // (rrule, anchor, from_completion, describe, hint, class)
    let rows: &[(&str, &str, bool, &str, &str, &str)] = &[
        ("FREQ=DAILY", "-", false, "Daily", "daily", "days"),
        (
            "FREQ=DAILY;INTERVAL=3",
            "-",
            false,
            "Every 3 Days",
            "every 3d",
            "days",
        ),
        (
            "FREQ=WEEKLY;BYDAY=MO,TH",
            "-",
            false,
            "Every Mon, Thu",
            "weekly",
            "weekdays-set",
        ),
        (
            "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR,SA,SU",
            "-",
            false,
            "Every Day",
            "daily",
            "weekdays-set",
        ),
        (
            "FREQ=WEEKLY;INTERVAL=2",
            "-",
            false,
            "Bi-weekly",
            "every 2w",
            "weeks",
        ),
        (
            "FREQ=MONTHLY",
            "2026-11-25",
            false,
            "Monthly on the 25th",
            "monthly",
            "months-day-1-28",
        ),
        (
            "FREQ=MONTHLY",
            "2026-10-31",
            false,
            "Monthly on the 31st",
            "monthly",
            "months-day-29-31",
        ),
        (
            "FREQ=MONTHLY;INTERVAL=6",
            "2026-11-01",
            false,
            "Semi-yearly on the 1st",
            "every 6m",
            "months-day-1-28",
        ),
        (
            "FREQ=MONTHLY;BYDAY=2TU",
            "-",
            false,
            "The second Tue of each month",
            "monthly",
            "months-nth-weekday",
        ),
        (
            "FREQ=MONTHLY;BYMONTHDAY=-1;INTERVAL=2",
            "-",
            false,
            "On the last day of every 2 months",
            "every 2m",
            "months-last-day",
        ),
        (
            "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1",
            "-",
            false,
            "The last weekday of each month",
            "monthly",
            "months-nth-workday",
        ),
        (
            "FREQ=MONTHLY;BYDAY=SA,SU;BYSETPOS=1",
            "-",
            false,
            "The first weekend day of each month",
            "monthly",
            "months-nth-weekend-day",
        ),
        (
            "FREQ=YEARLY",
            "2028-02-29",
            false,
            "Yearly on 29 Feb",
            "yearly",
            "years-feb-29",
        ),
        (
            "FREQ=MONTHLY;COUNT=3",
            "2026-11-15",
            true,
            "Monthly on the 15th, from completion, 3 times",
            "monthly",
            "months-day-1-28+from-completion+count",
        ),
        (
            "FREQ=YEARLY;UNTIL=20271231",
            "2026-11-15",
            false,
            "Yearly on 15 Nov, until 31 Dec 2027",
            "yearly",
            "years+until",
        ),
        (
            "FREQ=MONTHLY;BYDAY=-1FR",
            "-",
            false,
            "The last Fri of each month",
            "monthly",
            "months-nth-weekday",
        ),
        // 2Do encodes only the 1st..5th and the last weekday of a month.
        (
            "FREQ=MONTHLY;BYDAY=7MO",
            "-",
            false,
            "The nth Mon of each month",
            "monthly",
            "unrecognized",
        ),
        (
            "FREQ=MONTHLY;BYDAY=-3TU",
            "-",
            false,
            "The nth Tue of each month",
            "monthly",
            "unrecognized",
        ),
        // Without BYMONTHDAY or an anchor the day of the month is unknown.
        (
            "FREQ=MONTHLY",
            "-",
            false,
            "Monthly",
            "monthly",
            "unrecognized",
        ),
        (
            "FREQ=MONTHLY;BYMONTHDAY=15",
            "-",
            false,
            "Monthly on the 15th",
            "monthly",
            "months-day-1-28",
        ),
        // Without an anchor the day of the year is unknown.
        (
            "FREQ=YEARLY",
            "-",
            false,
            "Yearly",
            "yearly",
            "unrecognized",
        ),
        (
            "FREQ=YEARLY;BYMONTH=3",
            "-",
            false,
            "Yearly",
            "yearly",
            "unrecognized",
        ),
        (
            "FREQ=MONTHLY;BYHOUR=9",
            "-",
            false,
            "Monthly (unrecognised parts: BYHOUR=9)",
            "monthly",
            "unrecognized",
        ),
    ];
    for (rule, anchor, fc, desc, hint, class) in rows {
        let r = parse_rrule(rule).unwrap_or_else(|e| panic!("{rule}: {e}"));
        let a = d(anchor);
        assert_eq!(describe(&r, a, *fc), *desc, "describe {rule}");
        assert_eq!(short_hint(&r), *hint, "hint {rule}");
        let c = rule_class(&r, a, *fc);
        assert_eq!(c, *class, "class {rule}");
        let base = c.split('+').next().unwrap();
        assert!(
            RULE_CLASSES.contains(&base),
            "{base} missing from RULE_CLASSES"
        );
    }
}

#[test]
fn rule_classes_are_complete_and_unique() {
    assert_eq!(RULE_CLASSES.len(), 12);
    let mut v = RULE_CLASSES.to_vec();
    v.sort();
    v.dedup();
    assert_eq!(v.len(), 12);
}

#[test]
fn parse_fields() {
    let r = parse_rrule("RRULE:FREQ=MONTHLY;INTERVAL=2;BYDAY=-1FR;BYMONTH=3;COUNT=4").unwrap();
    assert_eq!(r.freq, Freq::Monthly);
    assert_eq!(r.interval, 2);
    assert_eq!(r.by_day, vec![(Some(-1), Weekday::Fri)]);
    assert_eq!(r.by_month, vec![3]);
    assert_eq!(r.end, End::Count(4));
    assert!(r.unknown.is_empty());
}

#[test]
fn parse_errors() {
    assert!(parse_rrule("FREQ=HOURLY").is_err());
    assert!(parse_rrule("INTERVAL=2").is_err());
    assert!(parse_rrule("FREQ=DAILY;INTERVAL=0").is_err());
    assert!(parse_rrule("FREQ=WEEKLY;BYDAY=XX").is_err());
    assert!(parse_rrule("").is_err());
}

#[test]
fn until_date_time_and_anchorless_defaults() {
    let r = parse_rrule("FREQ=YEARLY;UNTIL=20271231T235959Z").unwrap();
    assert!(matches!(r.end, End::Until(_)));
    assert_eq!(rule_class(&r, None, false), "unrecognized");
    let anchor = NaiveDate::from_ymd_opt(2026, 11, 15);
    assert_eq!(rule_class(&r, anchor, false), "years+until");
    assert_eq!(describe(&r, None, false), "Yearly, until 31 Dec 2027");
    let m = parse_rrule("FREQ=MONTHLY").unwrap();
    assert_eq!(rule_class(&m, None, false), "unrecognized");
    assert_eq!(describe(&m, None, false), "Monthly");
}

/// What the iPhone's 2Do wrote for each repeat kind (fixtures `plan7-list1`), read back as
/// 2Do labels it (01-features §6). Monthly and yearly labels name the day (deliberate: see
/// `describe_and_classify`), so "Monthly" reads "Monthly on the 31st".
#[test]
fn plan7_captures_read_as_2do_shows_them() {
    // (summary, describe, class)
    let rows: &[(&str, &str, &str)] = &[
        ("R01", "Daily", "days"),
        ("R02", "Daily", "days"),
        ("R03", "Every 3 Days", "days"),
        ("R04", "Weekly", "weeks"),
        ("R05", "Bi-weekly", "weeks"),
        ("R06", "Every Mon, Thu", "weekdays-set"),
        ("R07", "Every Mon, Tue, Wed, Thu, Fri", "weekdays-set"),
        ("R08", "Monthly on the 31st", "months-day-29-31"),
        ("R10", "The second Tue of each month", "months-nth-weekday"),
        ("R13", "Yearly on 29 Feb", "years-feb-29"),
        (
            "R14",
            "Monthly on the 7th, from completion",
            "months-day-1-28+from-completion",
        ),
        (
            "R15",
            "Monthly on the 7th, 3 times",
            "months-day-1-28+count",
        ),
        (
            "R16",
            "Monthly on the 7th, until 30 Jun 2027",
            "months-day-1-28+until",
        ),
    ];
    let phone = [
        ("plan7-list1/01-phone-repeat", rows),
        (
            "plan7-list1/02-phone-repeat-no-due",
            &[("R17", "Weekly", "weeks")][..],
        ),
    ];
    for (fixture, rows) in phone {
        let env = common::Env::new(WriteMode::Test, fixture);
        let snap = env.snap();
        for (summary, desc, class) in rows {
            let t: &Task = snap
                .tasks
                .iter()
                .find(|t| t.summary == *summary)
                .unwrap_or_else(|| panic!("{summary} missing in {fixture}"));
            let r = parse_rrule(t.rrule.as_deref().unwrap()).unwrap();
            let anchor = t.due.as_ref().map(|d| d.local_date(snap.tz()));
            assert_eq!(
                describe(&r, anchor, t.from_completion),
                *desc,
                "describe {summary}"
            );
            assert_eq!(
                twydo_core::views::task_rule_class(t, &snap).as_deref(),
                Some(*class),
                "class {summary}"
            );
        }
    }
    // The picker offered no repeat for these (R09, R11, R12): they stay plain tasks.
    let env = common::Env::new(WriteMode::Test, "plan7-list1/01-phone-repeat");
    let snap = env.snap();
    for s in ["R09", "R11", "R12"] {
        let t = snap.tasks.iter().find(|t| t.summary == s).unwrap();
        assert!(t.rrule.is_none(), "{s}");
    }
}
