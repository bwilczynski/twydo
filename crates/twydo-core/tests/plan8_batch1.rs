//! Plan 8 batch 1: compare local writers with each captured phone transition.
//! Candidate matrices here are test-only; they do not enable unverified production writes.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use chrono::{DateTime, Utc};
use twydo_core::compat::{Matrix, OperationEntry, Verdict};
use twydo_core::config::WriteMode;
use twydo_core::edit::{self, Change, Op};
use twydo_core::ical::{Calendar, Property, parse, serialize};
use twydo_core::model::meta_edit::MetaJson;
use twydo_core::store::Store;
use twydo_core::store::journal::{Journal, Stage};

use common::{Env, Line, completed, dtstamp, fixture, is_stamp, lines, sorted, stamp, todo};

const SCENARIO: &str = "plan8-tui-a";
const SEED: &str = "001-after-upload";
const SET: &str = "010-first-phone-actions";
const ADVANCE: &str = "020-boundary-and-checklist-complete";
const END: &str = "030-end-series-and-items";
const REOPEN: &str = "040-reopen-ended-and-complete-parent";
const REMOVE: &str = "050-remove-repeat-and-refinish";
const HISTORY: &str = "060-remove-checklist-repeat-and-history-reopen";
const FC: &str = "61ed74be3ba34212ab7476b698de1263";
const U1: &str = "91fb1d1636754afda633eb2bda234417";
const U2: &str = "be99b147d81b4df6a63dc134e559dab6";
const RD: &str = "db95528b16fb4aa2b6a47e56a0a1000b";
const RY: &str = "033d6526b6ba46999f3f510edecf888b";

fn cal(step: &str, uid: &str) -> Calendar {
    parse(&fixture(SCENARIO, step, uid)).unwrap()
}

fn step_files(step: &str) -> BTreeMap<String, Calendar> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/phase0/v1")
        .join(SCENARIO)
        .join(step);
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "ics"))
        .map(|p| {
            let cal = parse(&fs::read(p).unwrap()).unwrap();
            (todo(&cal).prop("UID").unwrap().value.clone(), cal)
        })
        .collect()
}

/// The only excluded properties are phone layout, modification stamps (asserted separately),
/// and CREATED (the phone's first-receipt time differs from the copied source's creation).
/// COMPLETED, all recurrence/end fields, alarms and identity links remain in the comparison.
fn comparable(cal: &Calendar, ids: &BTreeMap<String, String>) -> (Vec<Line>, MetaJson) {
    let mut meta = None;
    let mut result = Vec::new();
    for mut line in lines(cal) {
        if is_stamp(&line.1)
            || matches!(
                line.1.as_str(),
                "PRODID"
                    | "X-2DOAPP-SYNC-IMAGE"
                    | "X-2DOAPP-SYNC-AUDIO"
                    | "X-2DOAPP-AUDIO-DURATION"
            )
        {
            continue;
        }
        for (from, to) in ids {
            line.3 = line.3.replace(from, to);
        }
        match line.1.as_str() {
            "CREATED" => line.3 = "<creation time>".into(),
            "X-2DOAPP-METADATA" => {
                meta = MetaJson::parse(&line.3);
                line.3 = "<metadata compared as JSON>".into();
            }
            _ => {}
        }
        result.push(line);
    }
    (sorted(result), meta.expect("readable metadata"))
}

fn same(ctx: &str, got: &Calendar, want: &Calendar, ids: &BTreeMap<String, String>) {
    assert_eq!(
        comparable(got, ids),
        comparable(want, &BTreeMap::new()),
        "{ctx}"
    );
}

fn stamped(cal: &Calendar, now: DateTime<Utc>) {
    for key in ["DTSTAMP", "LAST-MODIFIED"] {
        assert_eq!(todo(cal).prop(key).unwrap().value, stamp(now), "{key}");
    }
}

fn preserved_created(before: &Calendar, after: &Calendar) {
    assert_eq!(todo(before).prop("CREATED"), todo(after).prop("CREATED"));
}

/// This is deliberately a candidate, not the embedded matrix. Device verification of the
/// fixed Twydo output must precede shipping any new operation/rule verdict.
fn candidate() -> Matrix {
    let mut matrix = Matrix::embedded().clone();
    matrix
        .rule
        .iter_mut()
        .find(|r| r.class == "mod:until")
        .unwrap()
        .execute = Verdict::Pass;
    for (case, op) in [
        ("recurring-checklist", "complete"),
        ("ended-until-daily", "reopen"),
        ("ended-until-monthly", "reopen"),
    ] {
        let entry = OperationEntry {
            case: case.into(),
            op: op.into(),
            iphone_baseline: Verdict::Pass,
            mac_to_iphone: Verdict::Unverified,
            iphone_to_mac: Verdict::Unverified,
            twydo_to_iphone: Verdict::Pass,
            iphone_to_twydo: Verdict::Pass,
            evidence: Vec::new(),
            notes: "Local candidate only".into(),
        };
        if let Some(existing) = matrix
            .operation
            .iter_mut()
            .find(|r| r.case == case && r.op == op)
        {
            *existing = entry;
        } else {
            matrix.operation.push(entry);
        }
    }
    matrix
}

#[test]
fn repeat_setting_reshaping_and_removal_match_each_phone_step() {
    for (uid, changes) in [
        (
            RD,
            vec![
                (SEED, SET, "daily"),
                (SET, ADVANCE, "3d"),
                (ADVANCE, END, "weekly"),
                (END, REOPEN, "daily"),
                (REOPEN, REMOVE, "none"),
            ],
        ),
        (
            RY,
            vec![
                (SEED, SET, "yearly"),
                (SET, ADVANCE, "2y"),
                (ADVANCE, END, "monthly"),
                (END, REOPEN, "yearly"),
                (REOPEN, REMOVE, "none"),
            ],
        ),
        (
            FC,
            vec![
                (SEED, SET, "monthly"),
                (REOPEN, REMOVE, "weekly"),
                (REMOVE, HISTORY, "none"),
            ],
        ),
    ] {
        for (from, to, text) in changes {
            let before = cal(from, uid);
            let want = cal(to, uid);
            let now = dtstamp(&want);
            let repeat = twydo_core::repeatinput::parse(text, now.date_naive(), &[]).unwrap();
            let mut got = before.clone();
            edit::apply_changes(&mut got, &[Change::Repeat(repeat)], now).unwrap();
            let mut ids = BTreeMap::new();
            if from == SEED && uid != FC {
                // For plain Twydo-created tasks the phone assigns its private series ID.
                // Twydo must use its known UID, not fabricate that opaque phone ID.
                let get = |c: &Calendar| {
                    MetaJson::parse(&todo(c).prop("X-2DOAPP-METADATA").unwrap().value).unwrap()
                };
                assert_eq!(get(&before).get_str("RUID"), Some(""));
                assert_eq!(get(&got).get_str("RUID"), Some(uid));
                ids.insert(
                    uid.to_string(),
                    get(&want).get_str("RUID").unwrap().to_string(),
                );
                // Map only the RUID, not the resource identity itself.
                let t = got.root.children_named_mut("VTODO").next().unwrap();
                let p = t.prop_mut("X-2DOAPP-METADATA").unwrap();
                let mut meta = MetaJson::parse(&p.value).unwrap();
                meta.set_str("RUID", ids.get(uid).unwrap());
                p.set_value(&meta.encode());
                ids.clear();
            }
            same(&format!("{uid} {from} -> {to}"), &got, &want, &ids);
            stamped(&got, now);
            preserved_created(&before, &got);
        }
    }
}

#[test]
fn until_continuation_inclusive_boundary_end_reopen_and_recompletion_match_phone() {
    for uid in [U1, U2] {
        let mut input = cal(SEED, uid);
        for (from, to) in [
            (SEED, SET),
            (SET, ADVANCE),
            (ADVANCE, END),
            (REOPEN, REMOVE),
        ] {
            if from == REOPEN {
                let want = cal(REOPEN, uid);
                let now = dtstamp(&want);
                edit::reopen_ended_until(&mut input, now).unwrap();
                same("ended until reopen", &input, &want, &BTreeMap::new());
                stamped(&input, now);
            }
            let before_files = step_files(from);
            let after_files = step_files(to);
            let copies: Vec<_> = after_files
                .iter()
                .filter(|(id, c)| {
                    !before_files.contains_key(*id)
                        && todo(c).prop("SUMMARY").unwrap().value
                            == todo(&input).prop("SUMMARY").unwrap().value
                })
                .collect();
            let want = cal(to, uid);
            let now = copies
                .first()
                .map_or_else(|| completed(&want), |(_, c)| completed(c));
            let copy_uid = copies
                .first()
                .map_or("0123456789abcdef0123456789abcdef", |(id, _)| id.as_str());
            let (live, copy) =
                edit::complete_recurring(&input, now, now.date_naive(), copy_uid).unwrap();
            same(&format!("{uid} {to} live"), &live, &want, &BTreeMap::new());
            stamped(&live, now);
            preserved_created(&input, &live);
            match (copies.first(), copy) {
                (Some((_, want)), Some(got)) => {
                    same("until history", &got, want, &BTreeMap::new());
                    stamped(&got, now);
                    preserved_created(&input, &got);
                }
                (None, None) => {}
                _ => panic!("wrong history-copy count for {uid} {to}"),
            }
            input = live;
        }
    }
}

#[test]
fn reopening_until_history_does_not_reopen_the_ended_original() {
    for (uid, history_uid) in [
        (U1, "7435cbd64d9443888f9eb8ebc84518f4"),
        (U2, "84df3db1133a47c583278e7ec8c07bcc"),
    ] {
        let before = cal(REMOVE, history_uid);
        let want = cal(HISTORY, history_uid);
        let mut got = before.clone();
        edit::reopen_one_time(&mut got, dtstamp(&want)).unwrap();
        same("history reopen", &got, &want, &BTreeMap::new());
        assert_eq!(
            fixture(SCENARIO, REMOVE, uid),
            fixture(SCENARIO, HISTORY, uid)
        );
    }
}

#[test]
fn recurring_checklist_family_completion_matches_both_phone_occurrences_and_undo() {
    for (from, to) in [(SET, ADVANCE), (END, REOPEN)] {
        let before_phone = step_files(from);
        let after_phone = step_files(to);
        let copies: BTreeMap<_, _> = after_phone
            .iter()
            .filter(|(id, c)| {
                !before_phone.contains_key(*id)
                    && todo(c).prop("SUMMARY").unwrap().value.starts_with("FCa")
            })
            .map(|(uid, c)| (todo(c).prop("SUMMARY").unwrap().value.clone(), uid.clone()))
            .collect();
        assert_eq!(copies.len(), 4);
        let now = completed(&after_phone[&copies["FCa"]]);
        let env = Env::new(WriteMode::Verified, &format!("{SCENARIO}/{from}"));
        let mut store = env.store(candidate());
        let before = env.files();
        store
            .apply(&env.snap(), &Op::Complete { uid: FC.into() }, now)
            .unwrap();
        let after = env.files();
        let journal = env.journal();
        assert_eq!(journal.len(), 1);
        let entry = &journal[0];
        assert_eq!(
            entry.writes.len(),
            8,
            "four history copies and four live updates"
        );
        let ids: BTreeMap<_, _> = entry
            .writes
            .iter()
            .filter(|w| w.before.is_none())
            .map(|w| {
                let c = parse(w.after.as_ref().unwrap().as_bytes()).unwrap();
                (
                    todo(&c).prop("UID").unwrap().value.clone(),
                    copies[&todo(&c).prop("SUMMARY").unwrap().value].clone(),
                )
            })
            .collect();
        for write in &entry.writes {
            let got = parse(&after[&write.path]).unwrap();
            let ours = &todo(&got).prop("UID").unwrap().value;
            let theirs = ids.get(ours).unwrap_or(ours);
            same(
                &format!("checklist {to} {theirs}"),
                &got,
                &after_phone[theirs],
                &ids,
            );
            stamped(&got, now);
            let title = &todo(&got).prop("SUMMARY").unwrap().value;
            let seeds = step_files(SEED);
            let source_uid = seeds
                .iter()
                .find(|(_, c)| &todo(c).prop("SUMMARY").unwrap().value == title)
                .unwrap()
                .0;
            preserved_created(&before_phone[source_uid], &got);
        }
        for (path, bytes) in &before {
            if !entry.writes.iter().any(|w| &w.path == path) {
                assert_eq!(&after[path], bytes);
            }
        }
        store.undo_last(now).unwrap();
        assert_eq!(
            env.files(),
            before,
            "family undo restores exact source bytes"
        );
        store.redo_last(now).unwrap();
        assert_eq!(env.files(), after, "redo reuses history identities");

        // Interrupt after two history creates: recovery must finish all eight resources,
        // not generate a second family or expose half of the reset indefinitely.
        let recovering = Env::new(WriteMode::Verified, &format!("{SCENARIO}/{from}"));
        drop(recovering.store(candidate()));
        let mut interrupted = entry.clone();
        interrupted.stage = Stage::Prepared;
        Journal::open(&recovering.state)
            .unwrap()
            .prepare(&interrupted)
            .unwrap();
        for write in entry.writes.iter().take(2) {
            fs::write(
                recovering.vdir.join(&write.path),
                write.after.as_ref().unwrap(),
            )
            .unwrap();
        }
        let (_, report) = Store::open_with(&recovering.cfg, candidate()).unwrap();
        assert!(report.stuck.is_empty());
        assert_eq!(
            recovering.files(),
            after,
            "recovery uses the recorded copy identities"
        );
    }
}

#[test]
fn captured_until_reopen_is_gated_by_its_specific_operation_row() {
    let env = Env::new(WriteMode::Verified, &format!("{SCENARIO}/{END}"));
    let snap = env.snap();
    for (uid, case) in [(U1, "ended-until-daily"), (U2, "ended-until-monthly")] {
        let op = Op::Reopen { uid: uid.into() };
        let mut matrix = candidate();
        assert!(
            twydo_core::gate::check(
                &snap,
                &matrix,
                WriteMode::Verified,
                &op,
                dtstamp(&cal(END, uid)).date_naive()
            )
            .is_ok()
        );
        matrix
            .operation
            .iter_mut()
            .find(|r| r.case == case && r.op == "reopen")
            .unwrap()
            .twydo_to_iphone = Verdict::Unverified;
        assert!(
            twydo_core::gate::check(
                &snap,
                &matrix,
                WriteMode::Verified,
                &op,
                dtstamp(&cal(END, uid)).date_naive()
            )
            .is_err()
        );
    }
}

#[test]
fn phone_baselines_do_not_enable_unchecked_embedded_writes() {
    for (step, uid, op) in [
        (SET, FC, Op::Complete { uid: FC.into() }),
        (SEED, U1, Op::Complete { uid: U1.into() }),
        (END, U1, Op::Reopen { uid: U1.into() }),
    ] {
        let env = Env::new(WriteMode::Verified, &format!("{SCENARIO}/{step}"));
        let mut store = env.store(Matrix::embedded().clone());
        let before = env.files();
        assert!(
            store
                .apply(&env.snap(), &op, dtstamp(&cal(step, uid)))
                .is_err()
        );
        assert_eq!(env.files(), before);
        assert!(env.journal().is_empty());
    }
}

#[test]
fn ended_until_reopen_store_is_journalled_and_undoable() {
    for uid in [U1, U2] {
        let env = Env::new(WriteMode::Verified, &format!("{SCENARIO}/{END}"));
        let mut store = env.store(candidate());
        let before = env.files();
        let want = cal(REOPEN, uid);
        store
            .apply(&env.snap(), &Op::Reopen { uid: uid.into() }, dtstamp(&want))
            .unwrap();
        let after = env.files();
        same(
            "store ended reopen",
            &parse(&after[&format!("list1/{uid}.ics")]).unwrap(),
            &want,
            &BTreeMap::new(),
        );
        assert_eq!(env.journal()[0].writes.len(), 1);
        store.undo_last(dtstamp(&want)).unwrap();
        assert_eq!(env.files(), before);
        store.redo_last(dtstamp(&want)).unwrap();
        assert_eq!(env.files(), after);
    }
}

#[test]
fn ended_until_reopen_refuses_uncaptured_or_inconsistent_shapes_atomically() {
    for case in [
        "count",
        "yearly",
        "open",
        "not-final",
        "timed",
        "parent",
        "metadata-parent",
        "from-completion",
        "wrong-end",
        "duplicate-rule",
    ] {
        let mut input = cal(END, U1);
        let t = input.root.children_named_mut("VTODO").next().unwrap();
        match case {
            "count" => t.prop_mut("RRULE").unwrap().set_value("FREQ=DAILY;COUNT=1"),
            "yearly" => t
                .prop_mut("RRULE")
                .unwrap()
                .set_value("FREQ=YEARLY;UNTIL=20261120"),
            "open" => t.prop_mut("STATUS").unwrap().set_value("NEEDS-ACTION"),
            "not-final" => t.prop_mut("DUE").unwrap().set_value("20261119"),
            "timed" => {
                t.prop_mut("DUE").unwrap().set_params(Vec::new());
                t.prop_mut("DUE").unwrap().set_value("20261120T090000");
            }
            "parent" => t.insert_prop(
                Property::new("RELATED-TO", Vec::new(), "some-parent"),
                Some("UID"),
            ),
            "duplicate-rule" => t.insert_prop(
                Property::new("RRULE", Vec::new(), "FREQ=DAILY;UNTIL=20261120"),
                Some("DUE"),
            ),
            _ => {
                let p = t.prop_mut("X-2DOAPP-METADATA").unwrap();
                let mut m = MetaJson::parse(&p.value).unwrap();
                match case {
                    "metadata-parent" => m.set_str("parentuid", "some-parent"),
                    "from-completion" => m.set_i64("RecurrenceFrom", 2),
                    "wrong-end" => m.set_i64("RecurrenceEndDate", 1_795_262_400),
                    _ => unreachable!(),
                }
                p.set_value(&m.encode());
            }
        }
        let before = input.clone();
        let now = dtstamp(&input);
        assert!(edit::reopen_ended_until(&mut input, now).is_err(), "{case}");
        assert_eq!(input, before, "{case}: no partial edit");
    }
}

#[test]
fn recurring_checklist_refuses_uncaptured_item_shapes_without_partial_history() {
    for case in [
        "start-only",
        "metadata-start-only",
        "metadata-time-only",
        "cancelled",
        "timed",
    ] {
        let env = Env::new(WriteMode::Verified, &format!("{SCENARIO}/{SET}"));
        let mut store = env.store(candidate());
        let snap = env.snap();
        let child = snap.children(snap.find(FC).unwrap())[0];
        let path = &snap.tasks[child].path;
        let mut input = parse(&fs::read(path).unwrap()).unwrap();
        let now = dtstamp(&input);
        if case == "cancelled" {
            input
                .root
                .children_named_mut("VTODO")
                .next()
                .unwrap()
                .prop_mut("STATUS")
                .unwrap()
                .set_value("CANCELLED");
        } else if case.ends_with("only") {
            let t = input.root.children_named_mut("VTODO").next().unwrap();
            t.remove_props("DUE");
            if case != "start-only" {
                t.remove_props("DTSTART");
                let p = t.prop_mut("X-2DOAPP-METADATA").unwrap();
                let mut m = MetaJson::parse(&p.value).unwrap();
                let key = if case == "metadata-start-only" {
                    "StartDate"
                } else {
                    "duetime"
                };
                m.set_i64(key, 1_795_262_400);
                p.set_value(&m.encode());
            }
        } else {
            edit::apply_changes(
                &mut input,
                &[Change::Due(Some(edit::Due::Time(now.naive_utc())))],
                now,
            )
            .unwrap();
        }
        fs::write(path, serialize(&input)).unwrap();
        let before = env.files();
        assert!(
            store
                .apply(&env.snap(), &Op::Complete { uid: FC.into() }, now)
                .is_err(),
            "{case}"
        );
        assert_eq!(env.files(), before, "{case}");
        assert!(env.journal().is_empty(), "{case}");
    }
}

#[test]
fn every_batch1_resource_roundtrips_byte_equal() {
    for step in [
        SEED,
        SET,
        ADVANCE,
        END,
        REOPEN,
        REMOVE,
        HISTORY,
        "070-twydo-first-transitions",
        "071-twydo-first-upload",
        "080-phone-check-twydo-first",
    ] {
        let files = step_files(step);
        for (uid, c) in files {
            assert_eq!(serialize(&c), fixture(SCENARIO, step, &uid));
        }
    }
}

/// Exercise the same writers against the captured Twydo upload and the phone's subsequent
/// edits/completions. The phone is acting on Twydo-created series identities here.
#[test]
fn first_twydo_upload_and_phone_followup_match_the_writers() {
    const LOCAL: &str = "070-twydo-first-transitions";
    const UPLOADED: &str = "071-twydo-first-upload";
    const PHONE: &str = "080-phone-check-twydo-first";
    let seeds = step_files(SEED);
    let uid = |title: &str| {
        seeds
            .iter()
            .find(|(_, c)| todo(c).prop("SUMMARY").unwrap().value == title)
            .unwrap()
            .0
            .clone()
    };
    for (from, to, edits) in [
        (
            HISTORY,
            LOCAL,
            [("FCb", "monthly"), ("RDb", "daily"), ("RYb", "yearly")],
        ),
        (
            UPLOADED,
            PHONE,
            [("FCb", "note"), ("RDb", "3d"), ("RYb", "2y")],
        ),
    ] {
        for (title, text) in edits {
            let id = uid(title);
            let before = cal(from, &id);
            let want = cal(to, &id);
            let now = dtstamp(&want);
            let change = if text == "note" {
                Change::Description(Some("x".into()))
            } else {
                Change::Repeat(twydo_core::repeatinput::parse(text, now.date_naive(), &[]).unwrap())
            };
            let mut got = before.clone();
            edit::apply_changes(&mut got, &[change], now).unwrap();
            same(
                &format!("{title} {from} -> {to}"),
                &got,
                &want,
                &BTreeMap::new(),
            );
            stamped(&got, now);
            preserved_created(&before, &got);
        }
        let before_files = step_files(from);
        let after_files = step_files(to);
        for title in ["U1b", "U2b"] {
            let id = uid(title);
            let copies: Vec<_> = after_files
                .iter()
                .filter(|(id, c)| {
                    !before_files.contains_key(*id)
                        && todo(c).prop("SUMMARY").unwrap().value == title
                })
                .collect();
            assert_eq!(copies.len(), 1, "{title}: one completed occurrence");
            let (copy_id, want_copy) = copies[0];
            let now = completed(want_copy);
            let (live, copy) =
                edit::complete_recurring(&before_files[&id], now, now.date_naive(), copy_id)
                    .unwrap();
            same(
                &format!("{title} {to} live"),
                &live,
                &after_files[&id],
                &BTreeMap::new(),
            );
            same(
                &format!("{title} {to} history"),
                &copy.unwrap(),
                want_copy,
                &BTreeMap::new(),
            );
            stamped(&live, now);
            preserved_created(&before_files[&id], &live);
        }
    }
    let local = step_files(LOCAL);
    let uploaded = step_files(UPLOADED);
    assert_eq!(
        local.keys().collect::<Vec<_>>(),
        uploaded.keys().collect::<Vec<_>>()
    );
    for id in local.keys() {
        assert_eq!(
            fixture(SCENARIO, LOCAL, id),
            fixture(SCENARIO, UPLOADED, id),
            "upload preserves {id}"
        );
    }
}
