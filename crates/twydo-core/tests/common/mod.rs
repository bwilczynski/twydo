//! Fixture comparison helpers shared by the edit tests, and the store test environment.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use twydo_core::compat::Matrix;
use twydo_core::config::{Config, WriteMode};
use twydo_core::ical::{Calendar, Component, Param, parse};
use twydo_core::store::Store;
use twydo_core::store::journal::{Entry, Journal};
use twydo_core::views::Snapshot;

const ALL_OPS: [&str; 6] = ["create", "edit", "complete", "reopen", "move", "delete"];

/// Every op on every case in `cases` has baseline `pass` and the twydo columns `twydo`; the
/// monthly rule class passes; each `(id, verdict)` in `edges` is an edge case.
pub fn matrix(cases: &[&str], twydo: &str, edges: &[(&str, &str)]) -> Matrix {
    let mut s = String::from("matrix_version = 1\n");
    for case in cases {
        for op in ALL_OPS {
            s.push_str(&format!(
                "[[operation]]\ncase = \"{case}\"\nop = \"{op}\"\niphone_baseline = \"pass\"\n\
                 mac_to_iphone = \"unverified\"\niphone_to_mac = \"unverified\"\n\
                 twydo_to_iphone = \"{twydo}\"\niphone_to_twydo = \"{twydo}\"\n"
            ));
        }
    }
    s.push_str(
        "[[rule]]\nclass = \"months-day-1-28\"\npreserve = \"pass\"\nexecute = \"pass\"\nedit = \"pass\"\n",
    );
    for (id, verdict) in edges {
        s.push_str(&format!(
            "[[edge_case]]\nid = \"{id}\"\nverdict = \"{verdict}\"\n"
        ));
    }
    Matrix::from_toml_str(&s).unwrap()
}

/// A vdir in a tempdir with lists `list1` (the `.ics` files of fixture step `fixture`, e.g.
/// `plan4-list1/02-edit`) and an empty `list2`, and a state dir beside it.
pub struct Env {
    _tmp: tempfile::TempDir,
    pub vdir: PathBuf,
    pub state: PathBuf,
    pub cfg: Config,
}

impl Env {
    pub fn new(mode: WriteMode, fixture: &str) -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let vdir = tmp.path().join("vdir");
        let state = tmp.path().join("state");
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/phase0/v1")
            .join(fixture);
        let list1 = vdir.join("list1");
        fs::create_dir_all(&list1).unwrap();
        fs::create_dir_all(vdir.join("list2")).unwrap();
        for e in fs::read_dir(&src).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "ics") {
                fs::copy(&p, list1.join(p.file_name().unwrap())).unwrap();
            }
        }
        let mut cfg = Config::with_vdir(vdir.clone());
        cfg.timezone = Some("America/New_York".to_string());
        cfg.state_dir = Some(state.clone());
        // The store keeps its journal in the vdir's own state dir.
        let state = twydo_core::store::vdir_state_dir(&state, &vdir).unwrap();
        cfg.writes.mode = mode;
        Env {
            _tmp: tmp,
            vdir,
            state,
            cfg,
        }
    }

    pub fn snap(&self) -> Snapshot {
        let report = twydo_core::vdir::load(&self.vdir).unwrap();
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        Snapshot::new(report, self.cfg.clone())
    }

    pub fn store(&self, m: Matrix) -> Store {
        let (store, report) = Store::open_with(&self.cfg, m).unwrap();
        assert!(report.stuck.is_empty(), "{report:?}");
        store
    }

    pub fn file(&self, uid: &str) -> PathBuf {
        self.vdir.join("list1").join(format!("{uid}.ics"))
    }

    /// Every file under the vdir (relative path → bytes).
    pub fn files(&self) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        for list in ["list1", "list2"] {
            for e in fs::read_dir(self.vdir.join(list)).unwrap() {
                let p = e.unwrap().path();
                let name = format!("{list}/{}", p.file_name().unwrap().to_string_lossy());
                out.insert(name, fs::read(&p).unwrap());
            }
        }
        out
    }

    pub fn journal(&self) -> Vec<Entry> {
        if !self.state.join("journal").is_dir() {
            return Vec::new();
        }
        Journal::open(&self.state).unwrap().list().unwrap()
    }
}

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

pub mod recurrence;

pub mod item_deletion;
