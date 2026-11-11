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
