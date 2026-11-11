//! Operation-scoped compatibility evidence.
pub mod contracts;
pub use contracts::{Contract, ContractId, Matrix, OperationId, RequirementFailure};
use serde::Deserialize;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Unverified,
    Pass,
    Fail,
    Unsupported,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Unverified => "unverified",
            Verdict::Pass => "pass",
            Verdict::Fail => "fail",
            Verdict::Unsupported => "unsupported",
        }
    }
}

/// Why matrix text was rejected.
#[derive(Debug, thiserror::Error)]
pub enum MatrixError {
    #[error(transparent)]
    Parse(toml::de::Error),
    #[error("{0}")]
    Invalid(String),
}
