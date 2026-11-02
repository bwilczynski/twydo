//! Fixture comparison helpers shared by the edit tests.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::Path;

use chrono::{DateTime, Utc};
use twydo_core::ical::{Calendar, Component, Param, parse};

pub fn fixture(scenario: &str, step: &str, uid: &str) -> Vec<u8> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/phase0/v1")
        .join(scenario)
        .join(step)
        .join(format!("{uid}.ics"));
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

pub type Line = (String, String, Vec<Param>, String);

fn flat(c: &Component, path: &str, out: &mut Vec<Line>) {
    let path = format!("{path}/{}", c.name);
    for p in &c.props {
        out.push((
            path.clone(),
            p.name.clone(),
            p.params.clone(),
            p.value.clone(),
        ));
    }
    for ch in &c.children {
        flat(ch, &path, out);
    }
}

/// Every property as (component path, name, params, value), in document order.
pub fn lines(cal: &Calendar) -> Vec<Line> {
    let mut out = Vec::new();
    flat(&cal.root, "", &mut out);
    out
}

pub fn sorted(mut v: Vec<Line>) -> Vec<Line> {
    v.sort_by_cached_key(|l| format!("{l:?}"));
    v
}

pub fn is_stamp(name: &str) -> bool {
    name == "DTSTAMP" || name == "LAST-MODIFIED"
}

/// Raw logical lines (folds and terminator kept).
fn raw_logical(b: &[u8]) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    for phys in b.split_inclusive(|&c| c == b'\n') {
        match (phys.first(), out.last_mut()) {
            (Some(b' ' | b'\t'), Some(prev)) => prev.extend_from_slice(phys),
            _ => out.push(phys.to_vec()),
        }
    }
    out
}

fn raw_name(l: &[u8]) -> String {
    let s = String::from_utf8_lossy(l);
    let s = s.trim_end();
    let name = s
        .split([';', ':'])
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    if name == "BEGIN" || name == "END" {
        s.to_ascii_uppercase()
    } else {
        name
    }
}

pub fn todo(cal: &Calendar) -> &Component {
    cal.root.children_named("VTODO").next().unwrap()
}

fn utc_prop(cal: &Calendar, name: &str) -> DateTime<Utc> {
    let v = &todo(cal).prop(name).unwrap().value;
    chrono::NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%SZ")
        .unwrap()
        .and_utc()
}

pub fn dtstamp(cal: &Calendar) -> DateTime<Utc> {
    utc_prop(cal, "DTSTAMP")
}

pub fn completed(cal: &Calendar) -> DateTime<Utc> {
    utc_prop(cal, "COMPLETED")
}

pub fn stamp(now: DateTime<Utc>) -> String {
    now.format("%Y%m%dT%H%M%SZ").to_string()
}

/// The evidence with `COMPLETED` set to `now`: 2Do's completion time is a few seconds
/// before its sync stamp, Twydo uses one `now` for both.
pub fn with_completed_now(mut v: Vec<Line>, now: DateTime<Utc>) -> Vec<Line> {
    for l in &mut v {
        if l.1 == "COMPLETED" {
            l.3 = stamp(now);
        }
    }
    v
}

/// Names whose lines differ between two property lists.
fn diff_names(a: &[Line], b: &[Line]) -> BTreeSet<String> {
    a.iter()
        .filter(|l| !b.contains(l))
        .chain(b.iter().filter(|l| !a.contains(l)))
        .map(|l| l.1.clone())
        .collect()
}

/// [`check_lines`] against `evidence` with `COMPLETED` = `now`.
pub fn check_in_place(
    ctx: &str,
    input: &[u8],
    out: &[u8],
    evidence: &Calendar,
    now: DateTime<Utc>,
    changed: &[&str],
) {
    let want = with_completed_now(lines(evidence), now);
    check_lines(ctx, input, out, want, changed, &[]);
}

/// Check `out` (from `input`) against the expected lines `want`, leaving out properties
/// named in `ignore` from the comparisons with `want`:
/// - `input` and `want` differ exactly in `changed` + the two stamps;
/// - lines not in `changed` equal `want`'s, in order (stamps by value only);
/// - lines in `changed` equal `want`'s as a multiset;
/// - every untouched line of `input` keeps its original bytes and order in `out`.
pub fn check_lines(
    ctx: &str,
    input: &[u8],
    out: &[u8],
    want: Vec<Line>,
    changed: &[&str],
    ignore: &[&str],
) {
    let drop = |v: Vec<Line>| -> Vec<Line> {
        v.into_iter()
            .filter(|l| !ignore.contains(&l.1.as_str()))
            .collect()
    };
    let before = drop(lines(&parse(input).unwrap()));
    let want = drop(want);
    let got = drop(lines(&parse(out).unwrap()));

    let mut expect: BTreeSet<String> = changed.iter().map(|n| n.to_string()).collect();
    expect.extend(["DTSTAMP".to_string(), "LAST-MODIFIED".to_string()]);
    assert_eq!(diff_names(&before, &want), expect, "{ctx}: fixture changes");

    let fixed = |v: &[Line]| -> Vec<Line> {
        v.iter()
            .filter(|l| !is_stamp(&l.1) && !changed.contains(&l.1.as_str()))
            .cloned()
            .collect()
    };
    let moved = |v: &[Line]| -> Vec<Line> {
        sorted(
            v.iter()
                .filter(|l| is_stamp(&l.1) || changed.contains(&l.1.as_str()))
                .cloned()
                .collect(),
        )
    };
    assert_eq!(fixed(&got), fixed(&want), "{ctx}: untouched lines");
    assert_eq!(moved(&got), moved(&want), "{ctx}: changed lines");

    let keep = |v: Vec<Vec<u8>>| -> Vec<Vec<u8>> {
        v.into_iter()
            .filter(|l| {
                let n = raw_name(l);
                !is_stamp(&n) && !changed.contains(&n.as_str())
            })
            .collect()
    };
    assert_eq!(
        keep(raw_logical(out)),
        keep(raw_logical(input)),
        "{ctx}: preserved bytes"
    );
}
