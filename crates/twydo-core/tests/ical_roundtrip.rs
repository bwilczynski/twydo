use twydo_core::ical::{ParseError, parse, serialize};

fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/data/ical/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn assert_roundtrip(name: &str) {
    let b = data(name);
    let cal = parse(&b).unwrap();
    assert_eq!(serialize(&cal), b, "{name} must round-trip byte-for-byte");
}

#[test]
fn roundtrip_crlf_folded() {
    let b = data("crlf_folded.ics");
    assert!(
        b.windows(3).any(|w| w == b"\r\n "),
        "fixture must contain folds"
    );
    assert_roundtrip("crlf_folded.ics");
    let cal = parse(&b).unwrap();
    let todo = cal.root.children_named("VTODO").next().unwrap();
    assert!(
        todo.prop("SUMMARY")
            .unwrap()
            .value
            .starts_with("Synthetic long summary abcdefghij ")
    );
    assert!(todo.prop("SUMMARY").unwrap().value.len() > 100);
}

#[test]
fn roundtrip_fold_inside_utf8_sequence() {
    assert_roundtrip("fold_in_utf8.ics");
    let cal = parse(&data("fold_in_utf8.ics")).unwrap();
    let todo = cal.root.children_named("VTODO").next().unwrap();
    assert_eq!(todo.prop("SUMMARY").unwrap().value, "Zażółć gęślą jaźń 🙂");
}

#[test]
fn roundtrip_lf_only_and_tab_continuation() {
    assert_roundtrip("lf_tab.ics");
    let cal = parse(&data("lf_tab.ics")).unwrap();
    let todo = cal.root.children_named("VTODO").next().unwrap();
    assert_eq!(
        todo.prop("DESCRIPTION").unwrap().value,
        "first part, second part and third"
    );
}

#[test]
fn roundtrip_unknown_props_params_components() {
    assert_roundtrip("unknown.ics");
    let cal = parse(&data("unknown.ics")).unwrap();
    assert!(cal.root.prop("X-2DOAPP-METADATA").is_some());
    assert!(cal.root.children_named("VTIMEZONE").next().is_some());
    assert!(cal.root.children_named("X-THING").next().is_some());
    let todo = cal.root.children_named("VTODO").next().unwrap();
    assert_eq!(todo.children_named("VALARM").count(), 1);
}

#[test]
fn roundtrip_interleaved_children_and_props_without_final_eol() {
    assert_roundtrip("interleaved_no_eol.ics");
}

#[test]
fn quoted_param_values_parse() {
    let cal = parse(&data("unknown.ics")).unwrap();
    let p = cal.root.props_named("X-FOO").next().unwrap();
    assert_eq!(p.name, "X-FOO");
    assert_eq!(p.param("X-BAR"), Some("a:b;c,d"));
    assert_eq!(p.param("x-bar"), Some("a:b;c,d"));
    assert_eq!(p.value, "v:with:colons");
    let list = &p.params.iter().find(|x| x.name == "X-LIST").unwrap().values;
    assert_eq!(list, &["one", "two,2", "three"]);
    let todo = cal.root.children_named("VTODO").next().unwrap();
    assert_eq!(todo.prop("X-FOO").unwrap().param("X-BAR"), Some("a:b;c,d"));
}

#[test]
fn missing_end_is_error() {
    let err = parse(&data("truncated.ics")).unwrap_err();
    assert!(matches!(err, ParseError::At { line: 5, .. }), "{err:?}");
}

#[test]
fn malformed_inputs_are_errors() {
    for bad in [
        &b""[..],
        b"VERSION:2.0\r\n",
        b"BEGIN:VCALENDAR\r\nEND:VTODO\r\n",
        b"BEGIN:VCALENDAR\r\nnocolon\r\nEND:VCALENDAR\r\n",
        b"BEGIN:VCALENDAR\r\nX;P=\"unterminated:v\r\nEND:VCALENDAR\r\n",
        b"BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\nX:after\r\n",
        b"BEGIN:VCALENDAR\r\nX:\xff\r\nEND:VCALENDAR\r\n",
    ] {
        assert!(parse(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
    }
}
