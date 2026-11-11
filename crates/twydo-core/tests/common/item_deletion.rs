//! Representative phone deletion fixtures and test-only capability verdicts.
use super::Env;
use twydo_core::{compat::Matrix, config::WriteMode};

pub const SEED: &str = "seeded/list1";
pub const FIRST: &str = "phone-delete-q/list1";
pub const CLOSED: &str = "phone-complete-project/list1";
pub const EMPTY: &str = "phone-delete-last-items/list1";

pub fn env(step: &str) -> Env {
    Env::new(WriteMode::Verified, &format!("delete/family-items/{step}"))
}

/// Deliberately has no task/family operation or recurrence-rule rows: removal
/// must depend on its own evidence and preserve the rest without executing it.
pub fn matrix(twydo: &str) -> Matrix {
    let mut text = String::from("matrix_version = 1\n");
    for case in ["checklist-item", "project-item"] {
        text.push_str(&format!(
            "[[operation]]\ncase = \"{case}\"\nop = \"delete\"\n\
             iphone_baseline = \"pass\"\nmac_to_iphone = \"unverified\"\n\
             iphone_to_mac = \"unverified\"\ntwydo_to_iphone = \"{twydo}\"\n\
             iphone_to_twydo = \"{twydo}\"\n"
        ));
    }
    Matrix::from_toml_str(&text).unwrap()
}
