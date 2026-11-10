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

fn all_collections() -> BTreeSet<String> {
    (1..=14).map(|i| format!("c{i}")).collect()
}

#[test]
fn caldav_hook_conflict() {
    let o = fx("caldav-hook-conflict");
    assert_eq!(o.synced, all_collections());
    assert_eq!(o.conflict_items, vec!["u1"]);
    assert!(o.transfers.is_empty());
    assert!(o.errors.is_empty() && o.unattributed.is_empty() && o.unhooked.is_empty());
}

#[test]
fn caldav_same_content() {
    let o = fx("caldav-same-content");
    assert_eq!(o.synced, all_collections());
    assert_eq!(o.conflict_items, vec!["u2"]);
    assert_eq!(
        o.transfers,
        vec![
            t(Action::Upload, "u3", "twydo_local", "c13"),
            t(Action::Upload, "u4", "twydo_remote", "c13"),
        ]
    );
    assert!(o.errors.is_empty() && o.unattributed.is_empty());
}

#[test]
fn caldav_delete_vs_edit_local() {
    let o = fx("caldav-delete-vs-edit-local");
    assert_eq!(o.synced, all_collections());
    assert_eq!(
        o.transfers,
        vec![t(Action::Upload, "u5", "twydo_local", "c13")]
    );
    assert!(o.conflict_items.is_empty() && o.errors.is_empty() && o.unattributed.is_empty());
}

#[test]
fn caldav_delete_no_etag() {
    let o = fx("caldav-delete-no-etag");
    assert_eq!(o.synced, all_collections());
    assert_eq!(
        o.transfers,
        vec![t(Action::Delete, "u5", "twydo_remote", "c13")]
    );
    assert!(o.conflict_items.is_empty() && o.errors.is_empty() && o.unattributed.is_empty());
}

#[test]
fn caldav_delete_vs_edit_remote() {
    let o = fx("caldav-delete-vs-edit-remote");
    assert_eq!(o.synced, all_collections());
    assert_eq!(
        o.transfers,
        vec![t(Action::Upload, "u6", "twydo_remote", "c13")]
    );
    assert!(o.conflict_items.is_empty() && o.errors.is_empty() && o.unattributed.is_empty());
}

#[test]
fn caldav_emptied_remote() {
    let o = fx("caldav-emptied-remote");
    assert_eq!(o.synced, all_collections());
    assert_eq!(o.emptied.len(), 1);
    assert_eq!(o.emptied["c14"], "twydo_remote");
    assert_eq!(o.errors.len(), 1);
    assert!(o.errors["c14"][0].starts_with("Storage \"twydo_remote/c14\" was completely emptied."));
    assert!(o.transfers.is_empty() && o.unattributed.is_empty());
}

#[test]
fn caldav_emptied_local() {
    let o = fx("caldav-emptied-local");
    assert_eq!(o.synced, all_collections());
    assert_eq!(o.emptied.len(), 1);
    assert_eq!(o.emptied["c14"], "twydo_local");
    assert_eq!(o.errors.len(), 1);
    assert_eq!(
        o.transfers,
        vec![t(Action::Upload, "u9", "twydo_remote", "c13")]
    );
    assert!(o.unattributed.is_empty());
}

#[test]
fn caldav_offline_attributes_every_collection() {
    let o = fx("caldav-offline");
    assert_eq!(o.synced, all_collections());
    assert_eq!(
        o.errors.keys().cloned().collect::<BTreeSet<_>>(),
        all_collections()
    );
    for (coll, lines) in &o.errors {
        assert_eq!(lines.len(), 2, "{coll}");
        assert!(
            lines[0].starts_with("Unknown error occurred: Cannot connect to host 127.0.0.1:9"),
            "{coll}"
        );
        assert_eq!(lines[1], "Use `-vdebug` to see the full traceback.");
    }
    assert!(o.emptied.is_empty() && o.transfers.is_empty() && o.unattributed.is_empty());
}

#[test]
fn caldav_config_changed_is_unattributed() {
    let o = fx("caldav-config-changed");
    assert_eq!(
        o.unattributed,
        vec!["Detected change in config file, please run `vdirsyncer discover twydo`."]
    );
    assert!(o.synced.is_empty() && o.errors.is_empty() && o.transfers.is_empty());
}

/// The lines `tdo sync` printed before its wording moved to `twydo_core::sync::report`.
#[test]
fn messages_keep_the_tdo_sync_wording() {
    use std::collections::BTreeMap;
    use twydo_core::config::Config;
    use twydo_core::sync::report::messages;
    use twydo_core::sync::run::RunReport;
    use twydo_core::sync::status::CollState;

    let dir = tempfile::tempdir().unwrap();
    let vdir = dir.path().join("vdir");
    for c in ["c1", "c2", "c3"] {
        std::fs::create_dir_all(vdir.join(c)).unwrap();
    }
    std::fs::write(vdir.join("c1/displayname"), "Groceries\n").unwrap();
    std::fs::write(vdir.join("c3/displayname"), "Chores\n").unwrap();
    let mut cfg = Config::with_vdir(vdir);
    cfg.state_dir = Some(dir.path().join("state"));

    let r = RunReport {
        collections: BTreeMap::from([
            ("c1".into(), CollState::EmptiedLocal),
            (
                "c2".into(),
                CollState::Error {
                    message: "boom".into(),
                },
            ),
        ]),
        unexpected: vec!["c1/u1.ics".into()],
        superseded: vec!["c2/u2.ics".into()],
        warnings: vec!["the manifest is unreadable".into()],
        auto_forced: vec!["c3".into()],
        ..RunReport::default()
    };
    let m = messages(&cfg, &r);
    assert_eq!(
        m.problems,
        vec![
            "list \"Groceries\" was emptied here; to apply that on the server run: tdo sync --force-delete \"Groceries\"".to_string(),
            "list \"c2\": error: boom".to_string(),
        ]
    );
    assert_eq!(
        m.warnings,
        vec![
            "the manifest is unreadable".to_string(),
            "c1/u1.ics changed before its edit synced; the edit stays pending".to_string(),
            "c2/u2.ics was changed on the server after its edit synced; keeping the server's version".to_string(),
        ]
    );
    assert_eq!(
        m.notes,
        vec!["applied the emptied list \"Chores\" on the server".to_string()]
    );
}
