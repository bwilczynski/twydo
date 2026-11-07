//! Phase-0 compatibility matrix (`compat/matrix.toml`): what has been verified against 2Do.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct OperationEntry {
    pub case: String,
    pub op: String,
    pub iphone_baseline: Verdict,
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
#[serde(deny_unknown_fields)]
pub struct EdgeCase {
    pub id: String,
    #[serde(default)]
    pub observed: String,
    pub verdict: Verdict,
    #[serde(default)]
    pub evidence: Vec<String>,
}

/// Why matrix text was rejected.
#[derive(Debug, thiserror::Error)]
pub enum MatrixError {
    #[error(transparent)]
    Parse(toml::de::Error),
    #[error("{0}")]
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields)]
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
        Matrix::from_toml_str(&text).map_err(|e| match e {
            MatrixError::Parse(source) => ConfigError::Parse {
                path: p.to_path_buf(),
                source,
            },
            MatrixError::Invalid(message) => ConfigError::Invalid {
                path: p.to_path_buf(),
                message,
            },
        })
    }

    /// Parse a matrix from TOML text, rejecting duplicate `[[rule]]` classes.
    pub fn from_toml_str(text: &str) -> Result<Matrix, MatrixError> {
        let m: Matrix = toml::from_str(text).map_err(MatrixError::Parse)?;
        let mut seen = std::collections::BTreeSet::new();
        for r in &m.rule {
            if !seen.insert(r.class.as_str()) {
                return Err(MatrixError::Invalid(format!(
                    "duplicate [[rule]] class {:?}",
                    r.class
                )));
            }
        }
        Ok(m)
    }

    pub fn rule(&self, class: &str) -> Option<&RuleEntry> {
        self.rule.iter().find(|r| r.class == class)
    }

    /// The matrix compiled into the binary (`compat/matrix.toml`), parsed once.
    pub fn embedded() -> &'static Matrix {
        static EMBEDDED: OnceLock<Matrix> = OnceLock::new();
        EMBEDDED.get_or_init(|| {
            toml::from_str(include_str!("../../../compat/matrix.toml"))
                .expect("embedded compat/matrix.toml must parse")
        })
    }

    pub fn operation(&self, case: &str, op: &str) -> Option<&OperationEntry> {
        self.operation.iter().find(|o| o.case == case && o.op == op)
    }

    pub fn edge_case(&self, id: &str) -> Option<&EdgeCase> {
        self.edge_case.iter().find(|e| e.id == id)
    }
}

fn severity(v: Verdict) -> u8 {
    match v {
        Verdict::Pass => 0,
        Verdict::Unverified => 1,
        Verdict::Unsupported => 2,
        Verdict::Fail => 3,
    }
}

fn worst(a: Verdict, b: Verdict) -> Verdict {
    if severity(b) > severity(a) { b } else { a }
}

/// Worst (preserve, execute, edit) verdicts over the class's base row and one `mod:<name>` row
/// per `+modifier`. A missing row counts as unverified.
pub fn combined_verdicts(m: &Matrix, class: &str) -> (Verdict, Verdict, Verdict) {
    let mut parts = class.split('+');
    let base = parts.next().unwrap_or(class);
    let keys = std::iter::once(base.to_string()).chain(parts.map(|x| format!("mod:{x}")));
    let mut out = (Verdict::Pass, Verdict::Pass, Verdict::Pass);
    for key in keys {
        let (p, x, e) = m.rule(&key).map_or(
            (
                Verdict::Unverified,
                Verdict::Unverified,
                Verdict::Unverified,
            ),
            |r| (r.preserve, r.execute, r.edit),
        );
        out = (worst(out.0, p), worst(out.1, x), worst(out.2, e));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::repeat::RULE_CLASSES;

    #[test]
    fn matrix_template_loads() {
        // The real matrix is filled in during phase 0, so only its shape is fixed here.
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../compat/matrix.toml");
        let m = Matrix::load(&p).unwrap();
        assert_eq!(m.matrix_version, 1);
        for c in
            RULE_CLASSES
                .iter()
                .copied()
                .chain(["mod:from-completion", "mod:count", "mod:until"])
        {
            let r = m.rule(c).unwrap_or_else(|| panic!("missing {c}"));
            if c == "unrecognized" {
                let u = Verdict::Unsupported;
                assert_eq!((r.preserve, r.execute, r.edit), (u, u, u));
            }
        }
        assert_eq!(m.rule.len(), RULE_CLASSES.len() + 3);
        assert_eq!(m.operation.len(), 113);
        assert_eq!(m.edge_case.len(), 12);
    }

    #[test]
    fn embedded_matrix_has_baseline_column() {
        let m = Matrix::embedded();
        assert_eq!(m.operation.len(), 113);
        // Verified in the Plan 7 session B: baseline and both Twydo round trips.
        let e = m.operation("from-completion", "create").unwrap();
        assert_eq!(e.iphone_baseline, Verdict::Pass);
        // Plan 8 completed both Twydo/phone directions; Mac remains unverified.
        let e = m.operation("checklist", "repeat").unwrap();
        assert_eq!(e.iphone_baseline, Verdict::Pass);
        assert_eq!(e.twydo_to_iphone, Verdict::Pass);
        assert_eq!(e.mac_to_iphone, Verdict::Unverified);
        assert!(m.edge_case(&m.edge_case[0].id).is_some());
        assert!(m.operation("nope", "create").is_none());
    }

    #[test]
    fn combined_verdicts_takes_worst_over_modifiers() {
        let m = Matrix::embedded();
        let p = Verdict::Pass;
        assert_eq!(combined_verdicts(m, "months-day-1-28"), (p, p, p));
        // Since Plan 7 session C the count modifier executes, so the worst part is the class.
        assert_eq!(combined_verdicts(m, "months-day-1-28+count"), (p, p, p));
        let (preserve, execute, edit) = combined_verdicts(m, "months-last-day+count");
        assert_eq!(preserve, Verdict::Unverified);
        assert_eq!(execute, Verdict::Unverified);
        assert_eq!(edit, Verdict::Fail);
        // Until now passes within the gate's captured shape boundary.
        assert_eq!(combined_verdicts(m, "months-day-1-28+until"), (p, p, p));
        // A custom matrix can still make a modifier worse than its base class.
        let mut restricted = m.clone();
        restricted
            .rule
            .iter_mut()
            .find(|r| r.class == "mod:until")
            .unwrap()
            .execute = Verdict::Unverified;
        assert_eq!(
            combined_verdicts(&restricted, "months-day-1-28+until"),
            (p, Verdict::Unverified, p)
        );
    }

    #[test]
    fn duplicate_rule_and_unknown_field_rejected() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("m.toml");
        let rule = "[[rule]]\nclass = \"days\"\npreserve = \"pass\"\nexecute = \"pass\"\nedit = \"pass\"\n";
        std::fs::write(&p, format!("matrix_version = 1\n{rule}{rule}")).unwrap();
        assert!(matches!(Matrix::load(&p), Err(ConfigError::Invalid { .. })));
        std::fs::write(&p, format!("matrix_version = 1\n{rule}bogus = 1\n")).unwrap();
        assert!(Matrix::load(&p).is_err());
    }
}
