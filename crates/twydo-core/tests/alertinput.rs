//! The typed alert offset shared by the edit card and `tdo op edit --alert`.

use twydo_core::alertinput::{EXAMPLES, format, parse};
use twydo_core::model::AlertOffset;

fn mins(s: &str) -> u32 {
    parse(s)
        .unwrap_or_else(|e| panic!("{s}: {e}"))
        .unwrap_or_else(|| panic!("{s}: removed"))
        .minutes()
}

#[test]
fn parses_units() {
    for (s, want) in [
        ("15m", 15),
        ("15min", 15),
        ("15mins", 15),
        ("4h", 240),
        ("1d", 1440),
        ("2w", 20160),
        ("1h30m", 90),
        ("1d2h", 1560),
        ("15m before", 15),
        ("15 m", 15),
        ("15 min", 15),
        ("15 minutes before", 15),
        ("1h 30m", 90),
        ("2 hours", 120),
        ("1 day", 1440),
        ("1 week", 10080),
        ("4H", 240),
        (" 30m ", 30),
    ] {
        assert_eq!(mins(s), want, "{s}");
    }
}

#[test]
fn removes() {
    for s in ["", "none", "  ", "None"] {
        assert_eq!(parse(s), Ok(None), "{s:?}");
    }
}

#[test]
fn rejects_ambiguous_input() {
    for s in [
        "90",
        "-15m",
        "0m",
        "15x",
        "m",
        "1.5h",
        "15m after",
        "before",
        "15m15m",
        "1 5m",
        "15 before 4h",
    ] {
        let e = parse(s).expect_err(s);
        assert!(e.contains(EXAMPLES), "{s}: {e}");
    }
}

#[test]
fn format_round_trips() {
    for n in [15, 30, 90, 240, 1440, 1560, 10080, 20160, 2880] {
        let o = AlertOffset::new(n).unwrap();
        assert_eq!(mins(&format(o)), n, "{n}");
    }
    let f = |n| format(AlertOffset::new(n).unwrap());
    assert_eq!(f(15), "15m");
    assert_eq!(f(90), "90m");
    assert_eq!(f(240), "4h");
    assert_eq!(f(1440), "1d");
    assert_eq!(f(1560), "26h");
    assert_eq!(f(10080), "1w");
    assert_eq!(f(2880), "2d");
}

#[test]
fn at_due_names_the_automatic_alert() {
    use twydo_core::alertinput::{AT_DUE_NOT_ADDED, is_at_due};
    for s in ["at due", "due", "At Due", "at due time"] {
        assert!(is_at_due(s), "{s}");
        assert_eq!(parse(s), Err(AT_DUE_NOT_ADDED.to_string()), "{s}");
    }
    assert!(!is_at_due("15m"));
}
