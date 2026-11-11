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

    /// The more severe of two verdicts: pass < unverified < unsupported < fail.
    pub fn worst(self, other: Self) -> Self {
        let severity = |v| match v {
            Verdict::Pass => 0,
            Verdict::Unverified => 1,
            Verdict::Unsupported => 2,
            Verdict::Fail => 3,
        };
        if severity(other) > severity(self) {
            other
        } else {
            self
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
