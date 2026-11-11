use twydo_core::ical::value::{IcalDateTime, escape_text, format_date_time, unescape_text};
use twydo_core::ical::{Component, Param, Property, new_calendar, parse, serialize};

fn fixtures() -> Vec<std::path::PathBuf> {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "ics") {
                out.push(p);
            }
        }
    }
    let root = format!("{}/../../fixtures/phase0/v1", env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    walk(std::path::Path::new(&root), &mut out);
    out.sort();
    out
}

/// Logical (unfolded-group) lines, each with its raw bytes including folds and terminator.
fn logical(b: &[u8]) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    for phys in b.split_inclusive(|&c| c == b'\n') {
        match (phys.first(), out.last_mut()) {
            (Some(b' ' | b'\t'), Some(prev)) => prev.extend_from_slice(phys),
            _ => out.push(phys.to_vec()),
        }
    }
    out
}

fn first_with(name: &str) -> (std::path::PathBuf, Vec<u8>) {
    let p = fixtures()
        .into_iter()
        .find(|p| p.to_string_lossy().contains(name))
        .unwrap();
    let b = std::fs::read(&p).unwrap();
    (p, b)
}

fn todo(cal: &mut twydo_core::ical::Calendar) -> &mut Component {
    cal.root.children_named_mut("VTODO").next().unwrap()
}

#[test]
fn unedited_calendar_is_byte_equal() {
    let all = fixtures();
    assert!(!all.is_empty());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase0/v1");
    // The former subset scans also failed if a required capture directory disappeared.
    // Keep that guard without parsing any resource a second time.
    for scenario in [
        "plan7-list1",
        "issue35-checklist-order",
        "issue35-project-order",
    ] {
        assert!(root.join(scenario).is_dir(), "missing capture {scenario}");
    }
    for step in [
        "001-after-upload",
        "010-first-phone-actions",
        "020-boundary-and-checklist-complete",
        "030-end-series-and-items",
        "040-reopen-ended-and-complete-parent",
        "050-remove-repeat-and-refinish",
        "060-remove-checklist-repeat-and-history-reopen",
        "070-twydo-first-transitions",
        "080-phone-check-twydo-first",
        "090-twydo-checklist-reset-and-until-end",
        "100-phone-items-and-ended-reopen",
        "110-twydo-second-reset-reshape-and-end",
        "111-twydo-ended-originals-reopened",
        "120-phone-second-round-followup",
        "130-twydo-repeat-removal-and-history-reopen",
        "131-phone-versions-kept-after-conflict",
        "141-prepared-repeat-and-history-states",
        "142-twydo-repeat-removal-and-history-reopen",
        "144-phone-removal-and-history-followup",
        "151-twydo-checklist-repeat-removed",
        "153-phone-checklist-removal-followup",
    ] {
        assert!(
            root.join("plan8-tui-a").join(step).is_dir(),
            "missing capture {step}"
        );
    }
    let plan7 = all
        .iter()
        .filter(|p| p.components().any(|c| c.as_os_str() == "plan7-list1"))
        .count();
    assert!(plan7 > 100, "expected the plan 7 captures, found {plan7}");
    let item_order = all
        .iter()
        .filter(|p| {
            p.components().any(|c| {
                c.as_os_str() == "issue35-checklist-order"
                    || c.as_os_str() == "issue35-project-order"
            })
        })
        .count();
    assert!(
        item_order >= 16,
        "expected both families and their append captures"
    );
    for p in all {
        let b = std::fs::read(&p).unwrap();
        let mut cal = parse(&b).unwrap();
        for t in cal.root.children_named_mut("VTODO") {
            let _ = t.prop_mut("SUMMARY");
        }
        assert_eq!(serialize(&cal), b, "{}", p.display());
    }
}

#[test]
fn edit_changes_only_that_line() {
    let (_, b) = first_with("rule-years/01-create/c3e7");
    let mut cal = parse(&b).unwrap();
    todo(&mut cal)
        .prop_mut("SUMMARY")
        .unwrap()
        .set_value("Changed");
    let out = serialize(&cal);
    let (a, c) = (logical(&b), logical(&out));
    assert_eq!(a.len(), c.len());
    let diff: Vec<usize> = (0..a.len()).filter(|&i| a[i] != c[i]).collect();
    assert_eq!(diff.len(), 1);
    assert!(c[diff[0]].starts_with(b"SUMMARY"));
    assert!(c[diff[0]].ends_with(b":Changed\n"));
}

fn fixture_with_status_rrule_alarm() -> Vec<u8> {
    for p in fixtures() {
        let b = std::fs::read(&p).unwrap();
        let s = String::from_utf8_lossy(&b);
        if s.contains("\nSTATUS:") && s.contains("\nRRULE:") && s.contains("BEGIN:VALARM") {
            return b;
        }
    }
    // Synthetic fallback.
    b"BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VTODO\nUID:x\nSTATUS:NEEDS-ACTION\nRRULE:FREQ=DAILY\nSUMMARY:s\nBEGIN:VALARM\nACTION:DISPLAY\nEND:VALARM\nEND:VTODO\nEND:VCALENDAR\n".to_vec()
}

#[test]
fn insert_and_remove_keep_other_bytes() {
    let b = fixture_with_status_rrule_alarm();
    let mut cal = parse(&b).unwrap();
    let t = todo(&mut cal);
    t.insert_prop(
        Property::new("PERCENT-COMPLETE", vec![], "100"),
        Some("STATUS"),
    );
    assert_eq!(t.remove_props("RRULE"), 1);
    let out = serialize(&cal);
    let (a, c) = (logical(&b), logical(&out));
    let mut expect: Vec<Vec<u8>> = Vec::new();
    for l in &a {
        if l.starts_with(b"RRULE") {
            continue;
        }
        expect.push(l.clone());
        if l.starts_with(b"STATUS") {
            expect.push(b"PERCENT-COMPLETE:100\n".to_vec());
        }
    }
    assert_eq!(c, expect);
    let s = String::from_utf8(out).unwrap();
    assert!(s.find("BEGIN:VALARM").unwrap() > s.find("PERCENT-COMPLETE").unwrap());
}

#[test]
fn insert_default_goes_after_last_prop_before_children() {
    let b = b"BEGIN:VCALENDAR\nBEGIN:VTODO\nUID:x\nSUMMARY:s\nBEGIN:VALARM\nEND:VALARM\nEND:VTODO\nEND:VCALENDAR\n";
    let mut cal = parse(b).unwrap();
    let t = todo(&mut cal);
    t.insert_prop(Property::new("X-A", vec![], "1"), Some("MISSING"));
    t.set_prop(Property::new("SUMMARY", vec![], "new"));
    let s = String::from_utf8(serialize(&cal)).unwrap();
    assert_eq!(
        s,
        "BEGIN:VCALENDAR\nBEGIN:VTODO\nUID:x\nSUMMARY:new\nX-A:1\nBEGIN:VALARM\nEND:VALARM\nEND:VTODO\nEND:VCALENDAR\n"
    );
}

#[test]
fn new_lines_use_file_line_ending() {
    let crlf = b"BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:x\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
    let mut cal = parse(crlf).unwrap();
    assert_eq!(cal.line_ending(), "\r\n");
    todo(&mut cal).set_prop(Property::new("SUMMARY", vec![], "a"));
    assert_eq!(
        serialize(&cal),
        b"BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:x\r\nSUMMARY:a\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
    );
    let lf = crlf
        .iter()
        .filter(|&&c| c != b'\r')
        .copied()
        .collect::<Vec<_>>();
    let mut cal = parse(&lf).unwrap();
    assert_eq!(cal.line_ending(), "\n");
    todo(&mut cal).set_prop(Property::new("SUMMARY", vec![], "a"));
    assert!(!serialize(&cal).contains(&b'\r'));
}

#[test]
fn edited_text_escapes_folds_and_reads_back() {
    let original = "Zażółć, gęślą; jaźń\\ 🙂\nline two ".repeat(4);
    let b =
        b"BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:x\r\nSUMMARY:old\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
    let mut cal = parse(b).unwrap();
    todo(&mut cal)
        .prop_mut("SUMMARY")
        .unwrap()
        .set_value(&escape_text(&original));
    let out = serialize(&cal);
    for phys in out.split(|&c| c == b'\n') {
        let phys = phys.strip_suffix(b"\r").unwrap_or(phys);
        assert!(phys.len() <= 75, "{} octets", phys.len());
        assert!(std::str::from_utf8(phys).is_ok());
    }
    assert!(out.windows(3).any(|w| w == b"\r\n "));
    let mut cal2 = parse(&out).unwrap();
    let v = &todo(&mut cal2).prop("SUMMARY").unwrap().value;
    assert_eq!(unescape_text(v), original);
}

#[test]
fn params_are_quoted_when_needed() {
    let mut c = Component::new("VTODO");
    c.set_prop(Property::new(
        "X-FOO",
        vec![Param {
            name: "X-P".into(),
            values: vec!["a:b".into(), "plain".into()],
        }],
        "v",
    ));
    let mut cal = Component::new("VCALENDAR");
    cal.push_child(c);
    let out = String::from_utf8(serialize(&new_calendar(cal))).unwrap();
    assert_eq!(
        out,
        "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nX-FOO;X-P=\"a:b\",plain:v\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
    );
}

#[test]
fn format_date_time_forms() {
    let d = chrono::NaiveDate::from_ymd_opt(2026, 12, 3).unwrap();
    let dt = d.and_hms_opt(17, 30, 0).unwrap();
    let (p, v) = format_date_time(&IcalDateTime::Date(d));
    assert_eq!(
        (p, v),
        (
            vec![Param {
                name: "VALUE".into(),
                values: vec!["DATE".into()]
            }],
            "20261203".to_string()
        )
    );
    assert_eq!(
        format_date_time(&IcalDateTime::Floating(dt)),
        (vec![], "20261203T173000".to_string())
    );
    assert_eq!(
        format_date_time(&IcalDateTime::Utc(dt.and_utc())),
        (vec![], "20261203T173000Z".to_string())
    );
    let (p, v) = format_date_time(&IcalDateTime::Zoned {
        local: dt,
        tzid: "America/New_York".into(),
    });
    assert_eq!(p[0].name, "TZID");
    assert_eq!(v, "20261203T173000");
}

#[test]
fn remove_children_keeps_other_bytes_and_order() {
    let b = b"BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:x\r\nBEGIN:VALARM\r\nX-K:1\r\nEND:VALARM\r\nSUMMARY:s\r\n folded\r\n\r\nBEGIN:VALARM\r\nX-K:2\r\nEND:VALARM\r\nSTATUS:NEEDS-ACTION\r\nBEGIN:VALARM\r\nX-K:3\r\nEND:VALARM\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
    let mut cal = parse(b).unwrap();
    let t = todo(&mut cal);
    let gone = t.remove_children(|c| c.prop("X-K").is_some_and(|p| p.value == "2"));
    assert_eq!(gone, 1);
    assert_eq!(t.remove_children(|_| false), 0);
    t.insert_prop(Property::new("X-NEW", vec![], "n"), Some("STATUS"));
    let s = String::from_utf8(serialize(&cal)).unwrap();
    assert_eq!(
        s,
        "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:x\r\nBEGIN:VALARM\r\nX-K:1\r\nEND:VALARM\r\nSUMMARY:s\r\n folded\r\n\r\nSTATUS:NEEDS-ACTION\r\nX-NEW:n\r\nBEGIN:VALARM\r\nX-K:3\r\nEND:VALARM\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
    );
}
