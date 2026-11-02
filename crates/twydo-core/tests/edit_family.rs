//! Edits of checklist and project members (complete, reopen, copy, move) against the iPhone
//! captures in `plan6-list1` and `plan6-list2`.

mod common;

use chrono::{DateTime, Utc};
use twydo_core::edit::{
    EditError, complete_member, drop_member_dates, member_copy, moved_copy, reopen_member,
};
use twydo_core::ical::{Calendar, parse, serialize};
use twydo_core::model::meta_edit::MetaJson;

use common::{check_in_place, check_lines, completed, dtstamp, fixture, lines, sorted, todo};

const L1: &str = "plan6-list1";
const L2: &str = "plan6-list2";
const K: &str = "a8dcf93bcaa14211a07481eb87af8f11";
const K_MOVED: &str = "55c46cbc93fc41b695492a0ea63dec78";
const K1B: &str = "28ae80e7369d40feaba54fe228eda2c6";
const K1B_MOVED: &str = "b07b5cf6793342bfaefae99b45ca12eb";
const K2: &str = "c57dfd30e9574d6ba195117e73dcc6d9";
const P: &str = "09c49c2cd76f483498fc2ff3a0f66160";
const P_MOVED: &str = "39527f9217d641998ebebafc875d7187";
const P3: &str = "6a41bc3350d04ca2abffeebf69893007";
const P3_COPY: &str = "905ce14bad6e4f849d697df5da10ad48";
const P3_MOVED: &str = "ebf6fc389b304cc69bb33f29b76cdab7";
const R_COPY: &str = "14b5a37a52224e8c959b5002e2275fc9";
const R1: &str = "35baad226105494080416dc20825f49c";
const R1_COPY: &str = "780885dd967342ec8953c6824f1d674b";
const R2: &str = "72e9476833d14a77a76c64cafeedfa97";
const R2_COPY: &str = "dd8f672c8feb47ccae449f8488918975";

const STATE: &[&str] = &["STATUS", "PERCENT-COMPLETE", "COMPLETED"];

fn unsupported(err: &EditError) -> &str {
    match err {
        EditError::Unsupported(m) => m,
        e => panic!("expected Unsupported, got {e:?}"),
    }
}

fn meta(cal: &Calendar) -> MetaJson {
    MetaJson::parse(&todo(cal).prop("X-2DOAPP-METADATA").unwrap().value).unwrap()
}

/// Complete `uid` from `04-reopen-child` as its parent's completion did in
/// `05-complete-parent`: `COMPLETED` = the parent's, stamps = the member's.
fn check_completed_member(uid: &str, parent: &str) {
    let input = fixture(L1, "04-reopen-child", uid);
    let evidence = parse(&fixture(L1, "05-complete-parent", uid)).unwrap();
    let at = completed(&parse(&fixture(L1, "05-complete-parent", parent)).unwrap());
    assert_eq!(completed(&evidence), at, "{uid}: completed with its parent");
    let now = dtstamp(&evidence);
    let mut cal = parse(&input).unwrap();
    complete_member(&mut cal, at, now).unwrap();
    let out = serialize(&cal);
    check_lines(uid, &input, &out, lines(&evidence), STATE, &[]);
}

#[test]
fn completing_checklist_completes_open_item() {
    check_completed_member(K2, K);
}

#[test]
fn completing_project_completes_repeating_child_keeping_rrule() {
    check_completed_member(P3, P);
    let mut cal = parse(&fixture(L1, "04-reopen-child", P3)).unwrap();
    let rrule = todo(&cal).prop("RRULE").unwrap().clone();
    let t = dtstamp(&cal);
    complete_member(&mut cal, t, t).unwrap();
    assert_eq!(todo(&cal).prop("RRULE"), Some(&rrule));
}

#[test]
fn reopening_parent_reopens_repeating_child_and_history_copy() {
    for uid in [P3, P3_COPY] {
        let input = fixture(L1, "05-complete-parent", uid);
        let evidence = parse(&fixture(L1, "06-reopen-parent", uid)).unwrap();
        let now = dtstamp(&evidence);
        let mut cal = parse(&input).unwrap();
        reopen_member(&mut cal, now).unwrap();
        let out = serialize(&cal);
        check_in_place(uid, &input, &out, &evidence, now, STATE);
    }
}

#[test]
fn recurring_project_child_copies_match_2do() {
    let at = completed(&parse(&fixture(L1, "05-complete-parent", R_COPY)).unwrap());
    for (uid, copy_uid) in [(R1, R1_COPY), (R2, R2_COPY)] {
        let cal = parse(&fixture(L1, "04-reopen-child", uid)).unwrap();
        let before = cal.clone();
        let evidence = parse(&fixture(L1, "05-complete-parent", copy_uid)).unwrap();
        let now = dtstamp(&evidence);
        let copy = member_copy(&cal, copy_uid, R_COPY, at, now).unwrap();
        assert_eq!(cal, before, "{uid}: input untouched");
        let copy = parse(&serialize(&copy)).unwrap();
        assert_eq!(
            sorted(lines(&copy)),
            sorted(lines(&evidence)),
            "{uid}: copy"
        );
    }
}

/// Move `uid` from `plan6-list1/06-reopen-parent` to `moved` in `plan6-list2/07-move`.
/// 2Do dropped `PERCENT-COMPLETE:0` there; Twydo keeps the line, so it is left out of the
/// comparison with the evidence (and must keep its bytes).
fn check_move(uid: &str, moved: &str, parent: Option<&str>, changed: &[&str]) {
    let input = fixture(L1, "06-reopen-parent", uid);
    let evidence = parse(&fixture(L2, "07-move", moved)).unwrap();
    let now = dtstamp(&evidence);
    let cal = parse(&input).unwrap();
    let out = serialize(&moved_copy(&cal, moved, parent, now).unwrap());
    check_lines(
        &format!("{uid} -> {moved}"),
        &input,
        &out,
        lines(&evidence),
        changed,
        &["PERCENT-COMPLETE"],
    );
}

#[test]
fn moving_parent_rewrites_json_uid() {
    check_move(K, K_MOVED, None, &["UID", "X-2DOAPP-METADATA"]);
}

#[test]
fn moving_child_points_at_new_parent_and_keeps_ruid() {
    let changed = &["UID", "RELATED-TO", "X-2DOAPP-METADATA"];
    check_move(K1B, K1B_MOVED, Some(K_MOVED), changed);
    check_move(P3, P3_MOVED, Some(P_MOVED), changed);
    let cal = parse(&fixture(L1, "06-reopen-parent", P3)).unwrap();
    let moved = moved_copy(&cal, P3_MOVED, Some(P_MOVED), dtstamp(&cal)).unwrap();
    assert_eq!(meta(&moved).get_str("RUID"), Some(P3));
}

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-02T16:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn member_state_checks() {
    // Completing a completed member, reopening an open one: refused, `cal` untouched.
    let mut cal = parse(&fixture(L1, "05-complete-parent", K2)).unwrap();
    let before = cal.clone();
    let err = complete_member(&mut cal, t0(), t0()).unwrap_err();
    assert_eq!(unsupported(&err), "task is already completed");
    assert_eq!(cal, before);

    let mut cal = parse(&fixture(L1, "04-reopen-child", K2)).unwrap();
    let before = cal.clone();
    let err = reopen_member(&mut cal, t0()).unwrap_err();
    assert_eq!(unsupported(&err), "task is not completed");
    assert_eq!(cal, before);
}

#[test]
fn member_copy_refusals() {
    let r2 = parse(&fixture(L1, "04-reopen-child", R2)).unwrap();
    for bad in ["", R2] {
        let err = member_copy(&r2, bad, R_COPY, t0(), t0()).unwrap_err();
        assert!(matches!(err, EditError::Unsupported(_)), "{bad:?}: {err:?}");
    }
    // A repeating member's copy is not captured.
    let p3 = parse(&fixture(L1, "04-reopen-child", P3)).unwrap();
    let err = member_copy(&p3, R2_COPY, R_COPY, t0(), t0()).unwrap_err();
    assert_eq!(
        unsupported(&err),
        "copying a repeating member is not verified"
    );
}

/// r1 with its JSON `key` set to its own UID.
fn r1_naming_itself(key: &str) -> Calendar {
    let mut cal = parse(&fixture(L1, "04-reopen-child", R1)).unwrap();
    let mut m = meta(&cal);
    m.set_str(key, R1);
    let todo = cal.root.children_named_mut("VTODO").next().unwrap();
    todo.prop_mut("X-2DOAPP-METADATA")
        .unwrap()
        .set_value(&m.encode());
    cal
}

#[test]
fn member_copy_refused_when_metadata_names_itself() {
    for key in ["RUID", "uid"] {
        let cal = r1_naming_itself(key);
        assert_eq!(meta(&cal).get_str(key), Some(R1));
        let err = member_copy(&cal, R1_COPY, R_COPY, t0(), t0()).unwrap_err();
        assert_eq!(
            unsupported(&err),
            "copying an item that refers to itself is not verified yet",
            "{key}"
        );
    }
}

#[test]
fn moving_without_parent_keeps_related_to() {
    // `None` leaves the parent link as it is (the store decides what a lone move means).
    let cal = parse(&fixture(L1, "06-reopen-parent", K1B)).unwrap();
    let moved = moved_copy(&cal, K1B_MOVED, None, t0()).unwrap();
    assert_eq!(todo(&moved).prop("RELATED-TO").unwrap().value, K);
    assert_eq!(meta(&moved).get_str("parentuid"), Some(K));
}

// Plan 6 Task 7 round 01: the phone moved checklist C's date and removed its items' own dates.
const T1: &str = "plan6-twydo-list1";
const C1: &str = "f2bcae42b8f84b37b48c18b7f89c5ac0";

#[test]
fn dropping_item_dates_matches_2do() {
    let input = fixture(T1, "00-setup", C1);
    let evidence = parse(&fixture(T1, "01-phone-edit", C1)).unwrap();
    let mut cal = parse(&input).unwrap();
    drop_member_dates(&mut cal, dtstamp(&evidence)).unwrap();
    let out = serialize(&cal);
    check_lines(C1, &input, &out, lines(&evidence), &["DUE", "DTSTART"], &[]);
}

#[test]
fn dropping_item_dates_refuses_an_item_without_a_date() {
    let mut cal = parse(&fixture(T1, "01-phone-edit", C1)).unwrap();
    let before = cal.clone();
    let err = drop_member_dates(&mut cal, t0()).unwrap_err();
    assert_eq!(unsupported(&err), "item has no date of its own");
    assert_eq!(cal, before);
}

#[test]
fn dropping_item_dates_refuses_a_timed_or_started_item() {
    for (key, value) in [("duetime", 900), ("StartDate", 1796083200)] {
        let mut cal = parse(&fixture(T1, "00-setup", C1)).unwrap();
        let mut m = meta(&cal);
        m.set_i64(key, value);
        let todo = cal.root.children_named_mut("VTODO").next().unwrap();
        todo.prop_mut("X-2DOAPP-METADATA")
            .unwrap()
            .set_value(&m.encode());
        let before = cal.clone();
        let err = drop_member_dates(&mut cal, t0()).unwrap_err();
        assert_eq!(
            unsupported(&err),
            "removing an item's due time or start date is not verified yet",
            "{key}"
        );
        assert_eq!(cal, before);
    }
}
