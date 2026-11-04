use chrono::{NaiveDate, Weekday};
use twydo_core::ical::value::IcalDateTime;
use twydo_core::model::repeat::{End, Freq, parse_rrule, rule_class};
use twydo_core::repeatinput::{RepeatSpec, describe, parse, to_text};

const WORK: [Weekday; 5] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
];

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 3).unwrap()
}

fn spec(input: &str) -> RepeatSpec {
    parse(input, today(), &WORK).unwrap().unwrap()
}

/// (typed forms, RRULE text, from completion, class with the due date on the 3rd)
type Row = (&'static [&'static str], &'static str, bool, &'static str);

const ROWS: &[Row] = &[
    (&["daily", "every day", "1d"], "FREQ=DAILY", false, "days"),
    (
        &["3d", "every 3 days"],
        "FREQ=DAILY;INTERVAL=3",
        false,
        "days",
    ),
    (
        &["weekly", "every week", "1w"],
        "FREQ=WEEKLY",
        false,
        "weeks",
    ),
    (
        &["biweekly", "2w", "every 2 weeks"],
        "FREQ=WEEKLY;INTERVAL=2",
        false,
        "weeks",
    ),
    (
        &["monthly", "every month"],
        "FREQ=MONTHLY",
        false,
        "months-day-1-28",
    ),
    (
        &["3m", "every 3 months"],
        "FREQ=MONTHLY;INTERVAL=3",
        false,
        "months-day-1-28",
    ),
    (&["yearly", "every year"], "FREQ=YEARLY", false, "years"),
    (
        &["2y", "every 2 years"],
        "FREQ=YEARLY;INTERVAL=2",
        false,
        "years",
    ),
    (
        &["mon thu", "mon,thu", "every mon and thu", "Monday Thursday"],
        "FREQ=WEEKLY;BYDAY=MO,TH",
        false,
        "weekdays-set",
    ),
    (
        &["workdays"],
        "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR",
        false,
        "weekdays-set",
    ),
    (
        &["last day"],
        "FREQ=MONTHLY;BYMONTHDAY=-1",
        false,
        "months-last-day",
    ),
    (
        &["last day every 2m", "last day every 2 months"],
        "FREQ=MONTHLY;INTERVAL=2;BYMONTHDAY=-1",
        false,
        "months-last-day",
    ),
    (
        &["2nd tue"],
        "FREQ=MONTHLY;BYDAY=2TU",
        false,
        "months-nth-weekday",
    ),
    (
        &["last fri"],
        "FREQ=MONTHLY;BYDAY=-1FR",
        false,
        "months-nth-weekday",
    ),
    (
        &["1st sun"],
        "FREQ=MONTHLY;BYDAY=1SU",
        false,
        "months-nth-weekday",
    ),
    (
        &["1st weekday"],
        "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=1",
        false,
        "months-nth-workday",
    ),
    (
        &["last weekday"],
        "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1",
        false,
        "months-nth-workday",
    ),
    (
        &["1st weekend day"],
        "FREQ=MONTHLY;BYDAY=SA,SU;BYSETPOS=1",
        false,
        "months-nth-weekend-day",
    ),
    (
        &["last weekend day"],
        "FREQ=MONTHLY;BYDAY=SA,SU;BYSETPOS=-1",
        false,
        "months-nth-weekend-day",
    ),
    (
        &["monthly after done"],
        "FREQ=MONTHLY",
        true,
        "months-day-1-28+from-completion",
    ),
    (
        &["weekly until 2027-06-01"],
        "FREQ=WEEKLY;UNTIL=20270601",
        false,
        "weeks+until",
    ),
    (
        &["monthly 10 times", "monthly 10 time"],
        "FREQ=MONTHLY;COUNT=10",
        false,
        "months-day-1-28+count",
    ),
    (
        &["weekly until 2027-06-01 after done"],
        "FREQ=WEEKLY;UNTIL=20270601",
        true,
        "weeks+from-completion+until",
    ),
];

#[test]
fn parses_every_row_of_the_spec_table() {
    assert_eq!(parse("", today(), &WORK), Ok(None));
    assert_eq!(parse("  ", today(), &WORK), Ok(None));
    assert_eq!(parse("none", today(), &WORK), Ok(None));
    let anchor = NaiveDate::from_ymd_opt(2026, 11, 3);
    for (typed, rrule, completion, class) in ROWS {
        let want = parse_rrule(rrule).unwrap();
        for t in *typed {
            let got = spec(t);
            assert_eq!(got.rule, want, "{t}");
            assert_eq!(got.from_completion, *completion, "{t}");
            assert_eq!(
                rule_class(&got.rule, anchor, got.from_completion),
                *class,
                "{t}"
            );
        }
    }
}

#[test]
fn encodings_match_the_phone_capture() {
    let daily = spec("daily").rule;
    assert_eq!((daily.freq, daily.interval), (Freq::Daily, 1));
    assert!(daily.by_day.is_empty());
    let until = spec("weekly until 2027-06-30").rule;
    assert_eq!(
        until.end,
        End::Until(IcalDateTime::Date(
            NaiveDate::from_ymd_opt(2027, 6, 30).unwrap()
        ))
    );
    assert_eq!(spec("weekly 3 times").rule.end, End::Count(3));
    // `until` takes any Due-style date.
    assert_eq!(
        spec("monthly until tomorrow").rule.end,
        End::Until(IcalDateTime::Date(
            NaiveDate::from_ymd_opt(2026, 11, 4).unwrap()
        ))
    );
}

#[test]
fn workdays_follow_the_configured_days() {
    let got = parse(
        "workdays",
        today(),
        &[Weekday::Sun, Weekday::Mon, Weekday::Tue],
    )
    .unwrap()
    .unwrap();
    let days: Vec<_> = got.rule.by_day.iter().map(|(_, w)| *w).collect();
    assert_eq!(days, vec![Weekday::Mon, Weekday::Tue, Weekday::Sun]);
}

#[test]
fn rejects_with_reason() {
    for (input, needle) in [
        ("every", "needs a rule"),
        ("6th tue", "6th"),
        ("0d", "at least 1"),
        ("monthly 15th", "set it in Due"),
        ("blah", "can't read"),
        ("2nd", "after a position"),
        ("last day every 2w", "months"),
        ("until", "needs a date"),
        ("monthly until", "needs a date"),
        ("monthly 0 times", "count"),
        ("monthly until nonsense", "can't read date"),
        ("after done", "before it"),
    ] {
        let e = parse(input, today(), &WORK).unwrap_err();
        assert!(e.contains(needle), "{input}: {e}");
    }
}

#[test]
fn to_text_round_trips() {
    for (typed, rrule, completion, _) in ROWS {
        let text = to_text(rrule, *completion).unwrap_or_else(|| panic!("no text for {rrule}"));
        assert_eq!(spec(&text), spec(typed[0]), "{rrule} -> {text}");
    }
    assert_eq!(to_text("FREQ=DAILY", false).as_deref(), Some("daily"));
    assert_eq!(
        to_text("FREQ=WEEKLY;UNTIL=20270601", true).as_deref(),
        Some("weekly until 2027-06-01 after done")
    );
}

#[test]
fn to_text_is_none_for_rules_the_syntax_cannot_say() {
    for r in [
        "FREQ=MONTHLY;BYMONTHDAY=15",
        "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO",
        "FREQ=YEARLY;BYMONTH=3",
        "FREQ=MONTHLY;BYDAY=2TU;INTERVAL=2",
        "FREQ=DAILY;X-FOO=1",
        "FREQ=WEEKLY;UNTIL=20270601T100000Z",
        "garbage",
    ] {
        assert_eq!(to_text(r, false), None, "{r}");
    }
}

#[test]
fn describe_matches_2do_labels() {
    let anchor = NaiveDate::from_ymd_opt(2026, 11, 1);
    for (typed, want) in [
        ("daily", "Daily · from due date"),
        ("biweekly", "Bi-weekly · from due date"),
        ("monthly", "Monthly on the 1st · from due date"),
        ("2nd tue", "The second Tue of each month · from due date"),
        ("last day", "On the last day of each month · from due date"),
        ("monthly after done", "Monthly on the 1st · after done"),
        ("weekly 3 times", "Weekly, 3 times · from due date"),
    ] {
        assert_eq!(describe(&spec(typed), anchor), want, "{typed}");
    }
}

/// A weekday set repeats every week only (Twydo cannot complete it with an interval), so no
/// typed form gives it another interval.
#[test]
fn weekday_sets_take_no_interval() {
    for input in [
        "every 2 weeks on mon thu",
        "every 2 weeks mon thu",
        "2w mon thu",
        "mon thu every 2w",
        "every 2 mon thu",
        "biweekly mon",
        "workdays every 2 weeks",
    ] {
        match parse(input, today(), &WORK) {
            Err(_) => {}
            Ok(Some(s)) => assert!(
                s.rule.by_day.is_empty() || s.rule.interval == 1,
                "{input}: {s:?}"
            ),
            Ok(None) => panic!("{input}: no repeat"),
        }
    }
}

/// Non-ASCII text, typed one key at a time into the card, is an error, never a panic (a
/// split inside a multi-byte character): a CJK unit, an accented letter, an emoji.
#[test]
fn non_ascii_input_is_an_error_not_a_panic() {
    for bad in [
        "2週",
        "週",
        "3日",
        "2😀",
        "😀",
        "every 2週",
        "2週 until 3日",
        "2nd 週",
        "2ndą",
        "1ą",
        "12ę",
        "pią",
        "monthly 2😀 times",
        "１２d",
    ] {
        let got = std::panic::catch_unwind(|| parse(bad, today(), &WORK));
        match got {
            Ok(r) => assert!(r.is_err(), "{bad}: {r:?}"),
            Err(_) => panic!("{bad}: panicked"),
        }
    }
}
