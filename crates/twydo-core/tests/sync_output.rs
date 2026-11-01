use std::collections::BTreeSet;
use twydo_core::sync::output::{Action, SyncOutput, Transfer, parse};

fn fx(name: &str) -> SyncOutput {
    let path = format!(
        "{}/tests/data/vdirsyncer/{name}.txt",
        env!("CARGO_MANIFEST_DIR")
    );
    parse("twydo", &std::fs::read_to_string(path).unwrap())
}

fn t(action: Action, uid: &str, storage: &str, coll: &str) -> Transfer {
    Transfer {
        action,
        uid: uid.into(),
        storage: storage.into(),
        collection: coll.into(),
    }
}

fn set(v: &[&str]) -> BTreeSet<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn clean_run() {
    let o = fx("clean");
    assert_eq!(o.synced, set(&["Twydo test", "c1"]));
    assert_eq!(
        o.transfers,
        vec![
            t(Action::Upload, "u1", "twydo_local", "c1"),
            t(Action::Upload, "u2", "twydo_local", "c1"),
            t(Action::Upload, "u3", "twydo_local", "c1"),
        ]
    );
    assert!(o.conflict_items.is_empty() && o.errors.is_empty() && o.unattributed.is_empty());
}

#[test]
fn hook_unresolved_drops_not_equal_line() {
    let o = fx("hook-unresolved");
    assert_eq!(o.conflict_items, vec!["u1"]);
    assert_eq!(
        o.transfers,
        vec![t(Action::Update, "u2", "twydo_local", "c1")]
    );
    assert!(o.unattributed.is_empty() && o.errors.is_empty() && o.unhooked.is_empty());
}

#[test]
fn hook_resolved() {
    let o = fx("hook-resolved");
    assert_eq!(o.conflict_items, vec!["u1"]);
    assert_eq!(
        o.transfers,
        vec![t(Action::Update, "u1", "twydo_local", "c1")]
    );
    assert!(o.errors.is_empty() && o.unattributed.is_empty());
}

#[test]
fn same_content() {
    let o = fx("same-content");
    assert_eq!(o.conflict_items, vec!["u1"]);
    assert!(o.transfers.is_empty() && o.errors.is_empty() && o.unattributed.is_empty());
}

#[test]
fn delete_vs_edit_local() {
    let o = fx("delete-vs-edit-local");
    assert_eq!(
        o.transfers,
        vec![t(Action::Upload, "u2", "twydo_local", "c1")]
    );
    assert!(o.errors.is_empty() && o.conflict_items.is_empty());
}

#[test]
fn delete_vs_edit_remote() {
    let o = fx("delete-vs-edit-remote");
    assert_eq!(
        o.transfers,
        vec![t(Action::Upload, "u3", "twydo_remote", "c1")]
    );
    assert!(o.errors.is_empty() && o.conflict_items.is_empty());
}

#[test]
fn hook_failed_is_collection_error() {
    let o = fx("hook-failed");
    assert_eq!(o.conflict_items, vec!["u1"]);
    let e = &o.errors["c1"];
    assert_eq!(e.len(), 2);
    assert!(e[0].starts_with("Unknown error occurred"));
    assert!(e[0].contains("returned non-zero exit status 3"));
    assert!(e[1].contains("-vdebug"));
    assert!(o.unattributed.is_empty() && o.unhooked.is_empty());
}

#[test]
fn no_hook_conflict_folds_continuations() {
    let o = fx("no-hook-conflict");
    assert_eq!(o.unhooked["c1"], vec!["u1"]);
    assert!(o.unattributed.is_empty());
    assert_eq!(o.errors["c1"].len(), 5);
    assert!(o.errors["c1"][0].starts_with("One item changed on both sides"));
}

#[test]
fn emptied_names_storage() {
    let o = fx("emptied-local");
    assert_eq!(o.emptied["c1"], "twydo_local");
    assert_eq!(o.errors["c1"].len(), 1);
    assert!(o.unattributed.is_empty());
}

#[test]
fn force_delete() {
    let o = fx("force-delete");
    assert_eq!(o.synced, set(&["c1"]));
    assert_eq!(o.transfers.len(), 3);
    assert_eq!(
        o.transfers[0],
        t(Action::Delete, "u1", "twydo_remote", "c1")
    );
    assert!(o.errors.is_empty());
}

#[test]
fn two_collections() {
    let o = fx("two-collections");
    assert_eq!(o.synced, set(&["Twydo test", "c1"]));
    assert_eq!(
        o.transfers,
        vec![
            t(Action::Upload, "t1", "twydo_local", "Twydo test"),
            t(Action::Upload, "u9", "twydo_local", "c1"),
        ]
    );
}

#[test]
fn collection_with_space() {
    let o = parse(
        "twydo",
        "Syncing twydo/Twydo test\nerror: twydo/Twydo test: boom\nerror: more\n",
    );
    assert!(o.synced.contains("Twydo test"));
    assert_eq!(o.errors["Twydo test"], vec!["boom", "more"]);
}

#[test]
fn unknown_error_line_is_unattributed() {
    let o = parse("twydo", "error: Connection refused\n");
    assert_eq!(o.unattributed, vec!["Connection refused"]);
    assert!(o.errors.is_empty());
}

#[test]
fn error_after_non_error_line_is_unattributed() {
    let o = parse("twydo", "error: twydo/c1: x\nSyncing twydo/c2\nerror: y\n");
    assert_eq!(o.errors["c1"], vec!["x"]);
    assert_eq!(o.unattributed, vec!["y"]);
}

#[test]
fn collection_name_containing_to_and_from() {
    let o = parse(
        "twydo",
        "Deleting item u1 from twydo_remote/Things to do\nCopying (uploading) item u2 to twydo_local/Things to do\nCopying (updating) item u3 to twydo_local/Mail from Bob\n",
    );
    assert_eq!(
        o.transfers,
        vec![
            t(Action::Delete, "u1", "twydo_remote", "Things to do"),
            t(Action::Upload, "u2", "twydo_local", "Things to do"),
            t(Action::Update, "u3", "twydo_local", "Mail from Bob"),
        ]
    );
}
