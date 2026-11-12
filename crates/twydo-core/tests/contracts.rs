//! Public compatibility schema behavior, independent of captured task layout.
use twydo_core::compat::contracts::{ContractId, Matrix};
use twydo_core::config::WriteMode;

fn entry(extra: &str) -> String {
    format!(
        "matrix_version = 2\n[[contract]]\nid = \"edit-title.resource\"\niphone_baseline = \"pass\"\ntwydo_to_iphone = \"unverified\"\niphone_to_twydo = \"pass\"\n{extra}"
    )
}

#[test]
fn missing_and_incomplete_contracts_refuse_by_mode() {
    let m = Matrix::from_toml_str(&entry("")).unwrap();
    assert!(
        m.require(ContractId::EditTitleResource, WriteMode::Verified)
            .is_err()
    );
    assert!(
        m.require(ContractId::EditTitleResource, WriteMode::Test)
            .is_ok()
    );
    assert!(
        m.require(ContractId::EditNotesResource, WriteMode::Test)
            .is_err()
    );
}

#[test]
fn version_is_checked_before_v2_fields() {
    let e = Matrix::from_toml_str("matrix_version = 1\n[[rule]]\nclass = 'days'\n").unwrap_err();
    assert!(e.to_string().contains("version 1"));
    assert!(e.to_string().contains("migrate"));
    assert!(Matrix::from_toml_str("matrix_version = 3").is_err());
}

#[test]
fn unknown_duplicate_and_misplaced_fields_are_rejected() {
    assert!(Matrix::from_toml_str(&entry("surprise = true")).is_err());
    assert!(Matrix::from_toml_str(&entry("atomicity = 'fail'")).is_err());
    assert!(
        Matrix::from_toml_str(&entry("").replace("edit-title.resource", "edit-title.anything"))
            .is_err()
    );
    let duplicate = format!("{}\n[[contract]]\nid = 'edit-title.resource'\n", entry(""));
    assert!(Matrix::from_toml_str(&duplicate).is_err());
}

#[test]
fn ordering_exception_is_bounded_and_does_not_ignore_failed_evidence() {
    let text = entry("")
        .replace("edit-title.resource", "reorder.items")
        .replace(
            "iphone_baseline = \"pass\"",
            "iphone_baseline = \"unsupported\"",
        )
        .replace(
            "twydo_to_iphone = \"unverified\"",
            "twydo_to_iphone = \"pass\"",
        );
    let m = Matrix::from_toml_str(&text).unwrap();
    assert!(
        m.require(ContractId::ReorderItems, WriteMode::Verified)
            .is_ok()
    );
    let m = Matrix::from_toml_str(&text.replace("unsupported", "fail")).unwrap();
    assert!(
        m.require(ContractId::ReorderItems, WriteMode::Test)
            .is_err()
    );
    let m = Matrix::from_toml_str(&entry("").replace(
        "iphone_baseline = \"pass\"",
        "iphone_baseline = \"unsupported\"",
    ))
    .unwrap();
    assert!(
        m.require(ContractId::EditTitleResource, WriteMode::Test)
            .is_err()
    );
}

#[test]
fn collection_interoperability_does_not_prove_atomicity() {
    let text = entry("")
        .replace("edit-title.resource", "list-metadata.collection")
        .replace("unverified", "pass");
    let m = Matrix::from_toml_str(&text).unwrap();
    for mode in [WriteMode::Verified, WriteMode::Test] {
        assert!(m.require(ContractId::ListMetadata, mode).is_err());
    }
    let m = Matrix::from_toml_str(&(text.clone() + "atomicity = 'pass'\n")).unwrap();
    assert!(
        m.require(ContractId::ListMetadata, WriteMode::Verified)
            .is_ok()
    );
    let m = Matrix::from_toml_str(&(text + "atomicity = 'fail'\n")).unwrap();
    assert!(
        m.require(ContractId::ListMetadata, WriteMode::Test)
            .is_err()
    );
}

#[test]
fn refusals_name_the_write_its_verdict_and_the_matrix_entry() {
    let text = "matrix_version = 2\n\
        [[contract]]\nid = \"convert.project-checklist\"\niphone_baseline = \"unsupported\"\n\
        [[contract]]\nid = \"complete.task\"\niphone_baseline = \"pass\"\ntwydo_to_iphone = \"fail\"\niphone_to_twydo = \"pass\"\n\
        [[contract]]\nid = \"reopen.resource\"\niphone_baseline = \"pass\"\ntwydo_to_iphone = \"unverified\"\niphone_to_twydo = \"pass\"\n";
    let m = Matrix::from_toml_str(text).unwrap();
    let message = |id| m.require(id, WriteMode::Verified).unwrap_err().to_string();
    assert_eq!(
        message(ContractId::ConvertProjectChecklist),
        "converting a project with items to a checklist is not supported by 2Do \
         (convert.project-checklist: iphone_baseline)"
    );
    assert_eq!(
        message(ContractId::CompleteTask),
        "completing a task fails with 2Do (complete.task: twydo_to_iphone)"
    );
    assert_eq!(
        message(ContractId::ReopenResource),
        "reopening a task is not verified yet (reopen.resource: twydo_to_iphone)"
    );
    assert_eq!(
        message(ContractId::DeleteResource),
        "deleting a task is not verified yet (delete.resource: no matrix entry)"
    );
}

#[test]
fn schedule_verdict_reports_the_worst_part() {
    use twydo_core::compat::{OperationId, Verdict};
    let m = Matrix::embedded();
    assert_eq!(
        m.schedule_verdict(OperationId::Complete, "days+until"),
        Verdict::Pass
    );
    assert_eq!(
        m.schedule_verdict(OperationId::SetRepeat, "months-last-day"),
        Verdict::Fail
    );
    assert_eq!(
        m.schedule_verdict(OperationId::Complete, "unrecognized"),
        Verdict::Unverified
    );
}

fn list_row(id: &str, extra: &str) -> String {
    format!(
        "matrix_version = 2\n[[contract]]\nid = \"{id}\"\natomicity = \"fail\"\naccepted_risk = \"best-effort\"\n{extra}"
    )
}

#[test]
fn accepted_risk_is_only_valid_on_list_metadata_and_delete() {
    assert!(Matrix::from_toml_str(&entry("accepted_risk = 'best-effort'")).is_err());
    for id in ["list-metadata.collection", "list-delete.collection"] {
        assert!(Matrix::from_toml_str(&list_row(id, "")).is_ok(), "{id}");
    }
    assert!(
        Matrix::from_toml_str(
            &list_row("list-metadata.collection", "").replace("best-effort", "anything")
        )
        .is_err()
    );
}

#[test]
fn best_effort_admits_failed_atomicity_and_unverified_directions_in_both_modes() {
    for (id, contract) in [
        ("list-metadata.collection", ContractId::ListMetadata),
        ("list-delete.collection", ContractId::ListDelete),
    ] {
        let m = Matrix::from_toml_str(&list_row(id, "")).unwrap();
        for mode in [WriteMode::Verified, WriteMode::Test] {
            assert!(m.require(contract, mode).is_ok(), "{id} {mode:?}");
        }
    }
}

#[test]
fn best_effort_still_refuses_fail_or_unsupported() {
    let refused = |extra: &str, direction: &str| {
        let m = Matrix::from_toml_str(&list_row("list-metadata.collection", extra)).unwrap();
        for mode in [WriteMode::Verified, WriteMode::Test] {
            let e = m.require(ContractId::ListMetadata, mode).unwrap_err();
            assert_eq!(e.direction, Some(direction), "{extra} {mode:?}");
        }
    };
    refused("twydo_to_iphone = 'fail'\n", "twydo_to_iphone");
    refused("iphone_to_twydo = 'unsupported'\n", "iphone_to_twydo");
    refused("iphone_baseline = 'unsupported'\n", "iphone_baseline");
    let m = Matrix::from_toml_str(
        &list_row("list-metadata.collection", "")
            .replace("atomicity = \"fail\"", "atomicity = \"unsupported\""),
    )
    .unwrap();
    let e = m
        .require(ContractId::ListMetadata, WriteMode::Test)
        .unwrap_err();
    assert_eq!(e.direction, Some("atomicity"));
}

#[test]
fn without_acceptance_list_rows_refuse_as_before() {
    for (id, contract) in [
        ("list-metadata.collection", ContractId::ListMetadata),
        ("list-delete.collection", ContractId::ListDelete),
    ] {
        let m = Matrix::from_toml_str(
            &list_row(id, "").replace("accepted_risk = \"best-effort\"\n", ""),
        )
        .unwrap();
        for mode in [WriteMode::Verified, WriteMode::Test] {
            let e = m.require(contract, mode).unwrap_err();
            assert_eq!(e.direction, Some("atomicity"), "{id} {mode:?}");
        }
    }
}

#[test]
fn embedded_matrix_admits_list_metadata_and_delete() {
    for contract in [ContractId::ListMetadata, ContractId::ListDelete] {
        assert!(
            Matrix::embedded()
                .require(contract, WriteMode::Verified)
                .is_ok()
        );
    }
}
