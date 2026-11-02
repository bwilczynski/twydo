use chrono::{NaiveDate, NaiveTime};
use chrono_tz::Tz;
use twydo_core::dateinput::{DateInput, DueShift, describe, input_text, parse, shift_due};
use twydo_core::edit::Due;
use twydo_core::ical::value::IcalDateTime;

const NY: Tz = chrono_tz::America::New_York;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn t(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).unwrap()
}

fn di(date: NaiveDate, time: Option<NaiveTime>) -> DateInput {
    DateInput { date, time }
}

fn today() -> NaiveDate {
    d(2026, 11, 4) // a Wednesday
}

fn p(s: &str) -> Option<DateInput> {
    parse(s, today()).unwrap()
}

#[test]
fn empty_input_clears() {
    assert_eq!(p(""), None);
    assert_eq!(p("  "), None);
}

#[test]
fn day_words() {
    assert_eq!(p("today"), Some(di(d(2026, 11, 4), None)));
    assert_eq!(p("tod"), Some(di(d(2026, 11, 4), None)));
    assert_eq!(p("TOM"), Some(di(d(2026, 11, 5), None)));
    assert_eq!(p("tomorrow"), Some(di(d(2026, 11, 5), None)));
}

#[test]
fn relative_units() {
    assert_eq!(p("3d"), Some(di(d(2026, 11, 7), None)));
    assert_eq!(p("2w"), Some(di(d(2026, 11, 18), None)));
    assert_eq!(p("1m").unwrap().date, d(2026, 12, 4));
    assert_eq!(
        parse("1m", d(2027, 1, 31)).unwrap().unwrap().date,
        d(2027, 2, 28)
    );
    assert_eq!(p("1y").unwrap().date, d(2027, 11, 4));
}

#[test]
fn ordinal_day() {
    assert_eq!(p("15th").unwrap().date, d(2026, 11, 15));
    assert_eq!(p("4th").unwrap().date, d(2026, 12, 4));
    assert_eq!(p("1st").unwrap().date, d(2026, 12, 1));
    assert_eq!(p("31st").unwrap().date, d(2026, 12, 31));
}

#[test]
fn weekday_is_never_today() {
    assert_eq!(p("wed").unwrap().date, d(2026, 11, 11));
    assert_eq!(p("thu").unwrap().date, d(2026, 11, 5));
    assert_eq!(p("Sunday").unwrap().date, d(2026, 11, 8));
}

#[test]
fn times() {
    assert_eq!(p("fri 5pm"), Some(di(d(2026, 11, 6), Some(t(17, 0)))));
    assert_eq!(p("friday 9:30 pm").unwrap().time, Some(t(21, 30)));
    assert_eq!(p("3d 930a").unwrap().time, Some(t(9, 30)));
    assert_eq!(p("12a").unwrap(), di(d(2026, 11, 4), Some(t(0, 0))));
    assert_eq!(p("12p").unwrap().time, Some(t(12, 0)));
    assert_eq!(p("noon").unwrap().time, Some(t(12, 0)));
    assert_eq!(
        p("tom midnight").unwrap(),
        di(d(2026, 11, 5), Some(t(0, 0)))
    );
    assert_eq!(p("14:30").unwrap(), di(d(2026, 11, 4), Some(t(14, 30))));
    assert_eq!(
        p("2026-11-20 14:30"),
        Some(di(d(2026, 11, 20), Some(t(14, 30))))
    );
    assert_eq!(p("2026-11-20"), Some(di(d(2026, 11, 20), None)));
}

#[test]
fn unreadable_input() {
    for bad in [
        "9",
        "someday",
        "32nd",
        "2026-02-30",
        "fri 25:00",
        "0d",
        "13pm",
        "fri fri",
    ] {
        assert_eq!(
            parse(bad, today()),
            Err(format!("can't read date \"{bad}\"")),
            "{bad}"
        );
    }
}

#[test]
fn due_conversion() {
    assert_eq!(di(d(2026, 11, 6), None).due(), Due::Date(d(2026, 11, 6)));
    assert_eq!(
        di(d(2026, 11, 6), Some(t(9, 0))).due(),
        Due::Time(d(2026, 11, 6).and_time(t(9, 0)))
    );
}

#[test]
fn describe_formats() {
    assert_eq!(describe(&di(d(2026, 11, 6), None), today()), "Fri 6 Nov");
    assert_eq!(
        describe(&di(d(2026, 11, 6), Some(t(9, 0))), today()),
        "Fri 6 Nov 09:00"
    );
    assert_eq!(
        describe(&di(d(2027, 11, 4), None), today()),
        "Thu 4 Nov 2027"
    );
}

#[test]
fn input_text_round_trips() {
    let date = IcalDateTime::Date(d(2026, 11, 6));
    assert_eq!(input_text(&date, NY), "2026-11-06");
    assert_eq!(p(&input_text(&date, NY)), Some(di(d(2026, 11, 6), None)));

    let floating = IcalDateTime::Floating(d(2026, 11, 6).and_time(t(9, 0)));
    assert_eq!(input_text(&floating, NY), "2026-11-06 09:00");
    assert_eq!(
        p(&input_text(&floating, NY)),
        Some(di(d(2026, 11, 6), Some(t(9, 0))))
    );

    // 15:00 in Warsaw (UTC+1 in November) is 09:00 in New York (UTC-5).
    let zoned = IcalDateTime::Zoned {
        local: d(2026, 11, 6).and_time(t(15, 0)),
        tzid: "Europe/Warsaw".into(),
    };
    assert_eq!(input_text(&zoned, NY), "2026-11-06 09:00");
}

#[test]
fn shift_due_cases() {
    let today = today();
    assert_eq!(
        shift_due(None, NY, today, DueShift::Days(1)),
        Due::Date(d(2026, 11, 5))
    );
    let timed = IcalDateTime::Floating(d(2026, 11, 1).and_time(t(9, 0)));
    assert_eq!(
        shift_due(Some(&timed), NY, today, DueShift::Days(-1)),
        Due::Time(d(2026, 10, 31).and_time(t(9, 0)))
    );
    let date_only = IcalDateTime::Date(d(2026, 10, 1));
    assert_eq!(
        shift_due(Some(&date_only), NY, today, DueShift::Today),
        Due::Date(today)
    );
    assert_eq!(
        shift_due(Some(&timed), NY, today, DueShift::Tomorrow),
        Due::Time(d(2026, 11, 5).and_time(t(9, 0)))
    );
    let before_dst = IcalDateTime::Floating(d(2026, 10, 28).and_time(t(9, 0)));
    assert_eq!(
        shift_due(Some(&before_dst), NY, today, DueShift::Days(7)),
        Due::Time(d(2026, 11, 4).and_time(t(9, 0)))
    );
    let zoned = IcalDateTime::Zoned {
        local: d(2026, 11, 6).and_time(t(15, 0)),
        tzid: "Europe/Warsaw".into(),
    };
    assert_eq!(
        shift_due(Some(&zoned), NY, today, DueShift::Days(1)),
        Due::Time(d(2026, 11, 7).and_time(t(9, 0)))
    );
}
