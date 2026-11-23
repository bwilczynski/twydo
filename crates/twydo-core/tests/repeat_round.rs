//! The 2026-11-23 repeat round (spec `2026-11-23-repeat-unblock-design.md`): Twydo-written
//! twins completed by Twydo must equal what the phone wrote for the same completion, and the
//! phone's own project repeat change must read back as written.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use chrono::{DateTime, TimeZone, Utc};

use common::{Env, Line, lines};
use twydo_core::compat::Matrix;
use twydo_core::config::WriteMode;
use twydo_core::edit::Op;
use twydo_core::ical::parse;
use twydo_core::model::meta_edit::MetaJson;

const P1A: &str = "c511b26ff399443eaa1801c2db191199";
const P1B: &str = "c99d955f09064717b1dfc59c39d688c4";
const P2: &str = "3ffd78c3af534f14bec0180a67236491";
const C1A: &str = "272a3f701d4147c7900e87d5a13ae72c";
const C1B: &str = "b41d0a85a25f4e219d52783579bbb048";
const C2A: &str = "ad20e262e69e4a3e905112e0c027fee1";
const C2B: &str = "8c395f835f0144518d97ca065bff62b2";
const C3A: &str = "b97bcb9acff149d7b4836ae7bee8a415";
const C3B: &str = "f0b80a36a0364c1f8633526c2c37fbd4";
const T1A: &str = "d47489e33b1e4c40b46cb238f9fc98d5";
const T1B: &str = "64b25f8a414840f79634d4d116c35f10";

/// When the twins were completed (the phone at 19:58, Twydo at 19:59 UTC): the same local day.
fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 23, 19, 59, 0).unwrap()
}

/// Properties that identify a resource or record when it was written, not what it is.
fn identity(name: &str) -> bool {
    common::is_stamp(name)
        || matches!(
            name,
            "UID"
                | "SUMMARY"
                | "RELATED-TO"
                | "COMPLETED"
                | "CREATED"
                | "SEQUENCE"
                | "PRODID"
                | "X-2DOAPP-METADATA"
                | "X-2DOAPP-AUDIO-DURATION"
                | "X-2DOAPP-SYNC-AUDIO"
                | "X-2DOAPP-SYNC-IMAGE"
        )
}

/// One resource without its identity: its other lines, and its metadata with every link
/// replaced by the role it points at (`head`, `copy` of the head, or `other`).
type Shape = (Vec<Line>, Option<MetaJson>);

/// The family of `head` in `files`: the head, its completed copies (same `RUID`), and the
/// items of either.
fn family(files: &BTreeMap<String, Vec<u8>>, head: &str) -> Vec<Shape> {
    let parsed: Vec<_> = files
        .values()
        .map(|b| parse(b).unwrap())
        .map(|cal| {
            let t = common::todo(&cal).clone();
            let uid = t.prop("UID").unwrap().value.clone();
            let meta = t
                .prop("X-2DOAPP-METADATA")
                .and_then(|p| MetaJson::parse(&p.value));
            let related: Vec<String> = t
                .props_named("RELATED-TO")
                .map(|p| p.value.clone())
                .collect();
            (uid, meta, related, cal)
        })
        .collect();
    let mut roles: BTreeMap<String, &str> = BTreeMap::new();
    roles.insert(head.to_string(), "head");
    for (uid, meta, _, _) in &parsed {
        if uid != head && meta.as_ref().and_then(|m| m.get_str("RUID")) == Some(head) {
            roles.insert(uid.clone(), "copy");
        }
    }
    let parents: Vec<String> = roles.keys().cloned().collect();
    let mut out = Vec::new();
    for (uid, meta, related, cal) in &parsed {
        let item = related.iter().any(|r| parents.contains(r))
            || meta
                .as_ref()
                .and_then(|m| m.get_str("parentuid"))
                .is_some_and(|p| parents.iter().any(|x| x == p));
        if !roles.contains_key(uid) && !item {
            continue;
        }
        let shape_lines: Vec<Line> = lines(cal).into_iter().filter(|l| !identity(&l.1)).collect();
        let meta = meta.clone().map(|mut m| {
            for key in ["uid", "RUID", "parentuid"] {
                if let Some(v) = m.get_str(key).map(str::to_string) {
                    let role = roles.get(&v).copied().unwrap_or("other");
                    m.set_str(key, role);
                }
            }
            m
        });
        out.push((common::sorted(shape_lines), meta));
    }
    out
}

fn state_files(case: &str, state: &str) -> BTreeMap<String, Vec<u8>> {
    let dir = common::case_state(case, state).join("list1");
    fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "ics"))
        .map(|p| (p.display().to_string(), fs::read(&p).unwrap()))
        .collect()
}

/// The same multiset of shapes, whatever the order.
fn same_family(ctx: &str, mut got: Vec<Shape>, want: Vec<Shape>) {
    assert_eq!(got.len(), want.len(), "{ctx}: resource count");
    for w in want {
        let i = got
            .iter()
            .position(|g| *g == w)
            .unwrap_or_else(|| panic!("{ctx}: no Twydo resource matches the phone's {w:?}"));
        got.remove(i);
    }
}

/// Completes the Twydo-seeded `b` with the shipped matrix and compares its family with the
/// phone's completion of `a`.
fn twin_matches(case: &str, a: &str, b: &str) {
    let env = Env::new(
        WriteMode::Verified,
        &format!("{}/list1", case_rel(case, "twydo-seeded")),
    );
    let mut store = env.store(Matrix::embedded().clone());
    store
        .apply(&env.snap(), &Op::Complete { uid: b.into() }, now())
        .unwrap_or_else(|e| panic!("{case}: Twydo completion of {b}: {e}"));
    let ours = family(&env.files(), b);
    let phone = family(&state_files(case, "phone-completed"), a);
    same_family(case, ours, phone);
}

fn case_rel(case: &str, state: &str) -> String {
    let full = common::case_state(case, state);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    full.strip_prefix(&root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| {
            // case_state joins through `..`; strip textually.
            full.display()
                .to_string()
                .split("fixtures/")
                .last()
                .unwrap()
                .to_string()
        })
}

#[test]
fn twin_p1_after_done_project_matches_the_phone() {
    twin_matches("complete/project-from-completion", P1A, P1B);
}

#[test]
fn twin_c1_biweekly_checklist_matches_the_phone() {
    twin_matches("complete/checklist-variants", C1A, C1B);
}

#[test]
fn twin_c2_after_done_checklist_matches_the_phone() {
    twin_matches("complete/checklist-variants", C2A, C2B);
}

#[test]
fn twin_c3_ending_checklist_matches_the_phone() {
    twin_matches("complete/checklist-variants", C3A, C3B);
}

#[test]
fn twin_t1_after_done_with_start_matches_the_phone() {
    twin_matches("complete/from-completion-start", T1A, T1B);
}

/// The phone's own change of P2 (plain monthly project) to every 3 months after completion
/// reads as that rule, with its items and links untouched, and equals what Twydo writes for P1.
#[test]
fn project_repeat_set_on_the_phone_reads_as_3m_after_done() {
    let case = "set-repeat/project-roundtrip";
    let env = Env::new(
        WriteMode::Verified,
        &format!("{}/list1", case_rel(case, "phone-p2-and-view")),
    );
    let s = env.snap();
    let p2 = &s.tasks[s.find(P2).unwrap()];
    assert_eq!(p2.rrule.as_deref(), Some("FREQ=MONTHLY;INTERVAL=3"));
    assert!(p2.from_completion);
    assert_eq!(s.children(s.find(P2).unwrap()).len(), 2);
    let seeded = state_files(case, "twydo-seeded");
    let viewed = state_files(case, "phone-p2-and-view");
    // Items unchanged byte for byte; P1's resources untouched by viewing them.
    for (path, bytes) in &seeded {
        let name = Path::new(path).file_name().unwrap();
        let after = viewed
            .iter()
            .find(|(p, _)| Path::new(p).file_name() == Some(name))
            .map(|(_, b)| b)
            .unwrap();
        if !path.ends_with(&format!("{P2}.ics")) {
            assert_eq!(after, bytes, "{path}");
        }
    }
    let shape = |uid: &str| {
        family(&viewed, uid)
            .into_iter()
            .find(|(_, m)| m.as_ref().and_then(|m| m.get_str("uid")) == Some("head"))
            .unwrap()
    };
    assert_eq!(
        shape(P2),
        shape(P1A),
        "phone-written P2 equals Twydo-written P1"
    );
}
