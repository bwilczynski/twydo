use chrono::{Duration, NaiveDate};
use chrono_tz::Tz;
use twydo_core::ical::parse;
use twydo_core::ical::value::{
    IcalDateTime, Resolved, parse_date_time, parse_duration, unescape_text,
};

const NY: Tz = chrono_tz::America::New_York;

fn dt(line: &str) -> IcalDateTime {
    let src = format!("BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\n{line}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n");
    let cal = parse(src.as_bytes()).unwrap();
    let todo = cal.root.children_named("VTODO").next().unwrap();
    parse_date_time(&todo.props[0]).unwrap()
}

fn inst(d: &IcalDateTime, local: Tz) -> (String, Option<String>) {
    match d.resolve(local) {
        (Resolved::Instant(i), w) => (i.to_rfc3339(), w),
        other => panic!("expected instant, got {other:?}"),
    }
}

#[test]
fn date_value() {
    let d = dt("DUE;VALUE=DATE:20261104");
    assert_eq!(
        d,
        IcalDateTime::Date(NaiveDate::from_ymd_opt(2026, 11, 4).unwrap())
    );
    assert!(d.is_date_only());
    assert_eq!(dt("DUE:20261104"), d);
    assert_eq!(
        d.local_date(NY),
        NaiveDate::from_ymd_opt(2026, 11, 4).unwrap()
    );
}

#[test]
fn utc_floating_zoned() {
    assert!(matches!(dt("DUE:20261104T150000Z"), IcalDateTime::Utc(_)));
    assert!(matches!(
        dt("DUE:20261104T150000"),
        IcalDateTime::Floating(_)
    ));
    let z = dt("DUE;TZID=America/New_York:20261104T150000");
    assert!(matches!(z, IcalDateTime::Zoned { .. }));
    assert_eq!(
        inst(&z, Tz::UTC),
        ("2026-11-04T15:00:00-05:00".into(), None)
    );
    assert_eq!(
        inst(&dt("DUE:20261104T150000"), NY).0,
        "2026-11-04T15:00:00-05:00"
    );
    // Utc late evening lands on previous local date.
    assert_eq!(
        dt("DUE:20261105T020000Z").local_date(NY),
        NaiveDate::from_ymd_opt(2026, 11, 4).unwrap()
    );
    assert!(!z.is_date_only());
}

#[test]
fn tzid_with_trailing_z_is_utc() {
    let d = dt("DUE;TZID=UTC:20261104T150000Z");
    assert!(matches!(d, IcalDateTime::Utc(_)));
}

#[test]
fn dst_gap_moves_forward() {
    let z = dt("DUE;TZID=America/New_York:20260308T023000");
    let (s, w) = inst(&z, NY);
    assert_eq!(s, "2026-03-08T03:30:00-04:00");
    assert!(w.is_none());
}

#[test]
fn dst_ambiguous_takes_earlier() {
    let z = dt("DUE;TZID=America/New_York:20261101T013000");
    assert_eq!(inst(&z, NY).0, "2026-11-01T01:30:00-04:00");
}

#[test]
fn unknown_tzid_falls_back_to_local_with_warning() {
    let z = dt("DUE;TZID=W. Europe Standard Time:20261104T150000");
    let (s, w) = inst(&z, NY);
    assert_eq!(s, "2026-11-04T15:00:00-05:00");
    assert!(w.unwrap().contains("unknown TZID"));
}

#[test]
fn invalid_values_error() {
    for line in [
        "DUE:garbage",
        "DUE:20261304",
        "DUE:20261104T250000",
        "DUE;VALUE=DATE:2026110",
    ] {
        let src =
            format!("BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\n{line}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n");
        let cal = parse(src.as_bytes()).unwrap();
        let todo = cal.root.children_named("VTODO").next().unwrap();
        assert!(parse_date_time(&todo.props[0]).is_err(), "{line}");
    }
}

#[test]
fn durations() {
    assert_eq!(parse_duration("-PT15M").unwrap(), Duration::minutes(-15));
    assert_eq!(parse_duration("P1W").unwrap(), Duration::days(7));
    assert_eq!(parse_duration("-P1DT2H").unwrap(), Duration::hours(-26));
    assert_eq!(parse_duration("PT0S").unwrap(), Duration::zero());
    assert_eq!(
        parse_duration("+PT1H30M10S").unwrap(),
        Duration::seconds(5410)
    );
    for bad in ["", "P", "PT", "15M", "P1H", "PT1D", "P1DT"] {
        assert!(parse_duration(bad).is_err(), "{bad}");
    }
}

#[test]
fn unescape() {
    assert_eq!(unescape_text(r"a\,b\;c\nd\\"), "a,b;c\nd\\");
    assert_eq!(unescape_text(r"x\Ny"), "x\ny");
    assert_eq!(unescape_text(r"trail\"), "trail\\");
}
