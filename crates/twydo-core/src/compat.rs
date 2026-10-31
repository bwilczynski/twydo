//! Phase-0 compatibility matrix (`compat/matrix.toml`): what has been verified against 2Do.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::config::ConfigError;

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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RuleEntry {
    pub class: String,
    pub preserve: Verdict,
    pub execute: Verdict,
    pub edit: Verdict,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OperationEntry {
    pub case: String,
    pub op: String,
    pub mac_to_iphone: Verdict,
    pub iphone_to_mac: Verdict,
    pub twydo_to_iphone: Verdict,
    pub iphone_to_twydo: Verdict,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct EdgeCase {
    pub id: String,
    #[serde(default)]
    pub observed: String,
    pub verdict: Verdict,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Matrix {
    pub matrix_version: u32,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub rule: Vec<RuleEntry>,
    #[serde(default)]
    pub operation: Vec<OperationEntry>,
    #[serde(default)]
    pub edge_case: Vec<EdgeCase>,
}

impl Matrix {
    pub fn load(p: &Path) -> Result<Matrix, ConfigError> {
        let text = std::fs::read_to_string(p).map_err(|source| ConfigError::Read {
            path: p.to_path_buf(),
            source,
        })?;
        toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: p.to_path_buf(),
            source,
        })
    }

    pub fn rule(&self, class: &str) -> Option<&RuleEntry> {
        self.rule.iter().find(|r| r.class == class)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::repeat::RULE_CLASSES;

    #[test]
    fn matrix_template_loads() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../compat/matrix.toml");
        let m = Matrix::load(&p).unwrap();
        assert_eq!(m.matrix_version, 1);
        for c in RULE_CLASSES {
            let r = m.rule(c).unwrap_or_else(|| panic!("missing {c}"));
            let want = if *c == "unrecognized" {
                Verdict::Unsupported
            } else {
                Verdict::Unverified
            };
            assert_eq!((r.preserve, r.execute, r.edit), (want, want, want));
        }
        assert_eq!(m.rule.len(), RULE_CLASSES.len());
        assert_eq!(m.operation.len(), 66);
        assert_eq!(m.edge_case.len(), 11);
    }
}
