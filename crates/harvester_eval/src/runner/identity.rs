//! Pure run-identity compatibility checks.

use serde::{Deserialize, Serialize};

use crate::config::{DatasetSelection, Transport};

/// Inputs that define a non-mixable result stream.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunIdentity {
    pub run_id: String,
    pub tool_version: String,
    pub manifest_hash: String,
    pub config_hash: String,
    pub prompt_identity_hash: String,
    pub rubric_sha256: String,
    pub transport_kind: Transport,
    pub split: DatasetSelection,
    pub repeat: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeDecision {
    Compatible,
    Incompatible {
        field: &'static str,
        existing: String,
        resolved: String,
    },
}

/// Returns the first incompatible field in a stable, user-facing order.
pub fn check_resume(existing: &RunIdentity, resolved: &RunIdentity) -> ResumeDecision {
    macro_rules! differs {
        ($field:ident) => {
            if existing.$field != resolved.$field {
                return ResumeDecision::Incompatible {
                    field: stringify!($field),
                    existing: format!("{:?}", existing.$field),
                    resolved: format!("{:?}", resolved.$field),
                };
            }
        };
    }
    differs!(manifest_hash);
    differs!(prompt_identity_hash);
    differs!(rubric_sha256);
    differs!(transport_kind);
    differs!(split);
    differs!(repeat);
    differs!(tool_version);
    differs!(config_hash);
    ResumeDecision::Compatible
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity() -> RunIdentity {
        RunIdentity {
            run_id: "one".into(),
            tool_version: "1".into(),
            manifest_hash: "manifest".into(),
            config_hash: "config".into(),
            prompt_identity_hash: "prompt".into(),
            rubric_sha256: "rubric".into(),
            transport_kind: Transport::Fake,
            split: DatasetSelection::Dev,
            repeat: 1,
        }
    }
    #[test]
    fn rejects_every_result_mixing_input() {
        assert_eq!(
            check_resume(&identity(), &identity()),
            ResumeDecision::Compatible
        );
        for field in [
            "manifest_hash",
            "config_hash",
            "prompt_identity_hash",
            "rubric_sha256",
            "transport_kind",
            "split",
            "repeat",
            "tool_version",
        ] {
            let old = identity();
            let mut new = old.clone();
            match field {
                "manifest_hash" => new.manifest_hash = "x".into(),
                "config_hash" => new.config_hash = "x".into(),
                "prompt_identity_hash" => new.prompt_identity_hash = "x".into(),
                "rubric_sha256" => new.rubric_sha256 = "x".into(),
                "transport_kind" => new.transport_kind = Transport::Live,
                "split" => new.split = DatasetSelection::All,
                "repeat" => new.repeat = 2,
                "tool_version" => new.tool_version = "2".into(),
                _ => unreachable!(),
            }
            assert!(
                matches!(check_resume(&old, &new), ResumeDecision::Incompatible { field: actual, .. } if actual == field)
            );
        }
    }

    #[test]
    fn names_specific_field_before_catch_all_config_hash() {
        let old = identity();
        let mut new = old.clone();
        new.transport_kind = Transport::Live;
        new.config_hash = "also-changed".into();
        assert!(matches!(
            check_resume(&old, &new),
            ResumeDecision::Incompatible {
                field: "transport_kind",
                ..
            }
        ));
    }
}
