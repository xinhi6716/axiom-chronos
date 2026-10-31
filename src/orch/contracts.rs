//! Typed records exchanged between first-round orchestration stages.
//!
//! Optional metadata describes what was actually reported, not what was
//! requested. These records do not themselves enforce acceptance or binding.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunBinding {
    pub run: String,
    pub session: String,
    pub repo: String,
    pub round: u32,
    pub owner_generation: u64,
}

/// How the operation's result was transported, independently of its outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationTransport {
    Completed,
    Failed,
    Cancelled,
    Unconfirmed,
}

/// The domain-level state reported for an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationOutcome {
    AwaitingCore,
    Verified,
    Blocked,
    Failed,
    Cancelled,
    Unconfirmed,
    ProtocolIncomplete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationResult {
    pub transport: OperationTransport,
    pub outcome: OperationOutcome,
    pub termination: Option<String>,
    pub evidence: Option<String>,
    pub delivery: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageReceipt {
    pub stage: String,
    pub input: Option<String>,
    pub output: Option<String>,
    pub source: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub sandbox: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundAcceptance {
    pub candidate: Option<String>,
    pub tests: Option<String>,
    pub review: Option<String>,
    pub commit: Option<String>,
    pub remote: Option<String>,
    pub install: Option<String>,
    pub ack: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{de::DeserializeOwned, Serialize};
    use std::fmt::Debug;

    fn round_trip<T: Serialize + DeserializeOwned + PartialEq + Debug>(value: T) {
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(serde_json::from_str::<T>(&json).unwrap(), value);
    }

    #[test]
    fn records_round_trip() {
        round_trip(RunBinding {
            run: "run".into(),
            session: "session".into(),
            repo: "repo".into(),
            round: 1,
            owner_generation: 2,
        });
        round_trip(OperationResult {
            transport: OperationTransport::Completed,
            outcome: OperationOutcome::Verified,
            termination: Some("exit 0".into()),
            evidence: None,
            delivery: Some("delivered".into()),
        });
        round_trip(StageReceipt {
            stage: "review".into(),
            input: Some("candidate".into()),
            output: None,
            source: None,
            model: Some("model".into()),
            effort: None,
            sandbox: None,
        });
        round_trip(RoundAcceptance {
            candidate: Some("candidate".into()),
            tests: None,
            review: None,
            commit: None,
            remote: None,
            install: None,
            ack: None,
        });
    }

    #[test]
    fn all_enum_variants_round_trip() {
        for value in [
            OperationTransport::Completed,
            OperationTransport::Failed,
            OperationTransport::Cancelled,
            OperationTransport::Unconfirmed,
        ] {
            round_trip(value);
        }
        for value in [
            OperationOutcome::AwaitingCore,
            OperationOutcome::Verified,
            OperationOutcome::Blocked,
            OperationOutcome::Failed,
            OperationOutcome::Cancelled,
            OperationOutcome::Unconfirmed,
            OperationOutcome::ProtocolIncomplete,
        ] {
            round_trip(value);
        }
        assert_eq!(
            serde_json::to_string(&OperationOutcome::AwaitingCore).unwrap(),
            "\"awaiting_core\""
        );
        assert_eq!(
            serde_json::to_string(&OperationOutcome::ProtocolIncomplete).unwrap(),
            "\"protocol_incomplete\""
        );
    }

    #[test]
    fn binding_requires_every_field() {
        let complete = serde_json::json!({"run":"r", "session":"s", "repo":"p", "round":1, "owner_generation":2});
        for field in ["run", "session", "repo", "round", "owner_generation"] {
            let mut missing = complete.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<RunBinding>(missing).is_err(),
                "missing {field}"
            );
        }
    }

    #[test]
    fn unknown_enum_variants_are_rejected() {
        assert!(serde_json::from_str::<OperationTransport>("\"unknown\"").is_err());
        assert!(serde_json::from_str::<OperationOutcome>("\"unknown\"").is_err());
    }

    #[test]
    fn omitted_metadata_remains_absent() {
        let receipt: StageReceipt = serde_json::from_str(r#"{"stage":"review"}"#).unwrap();
        assert_eq!(
            (
                receipt.input,
                receipt.output,
                receipt.source,
                receipt.model,
                receipt.effort,
                receipt.sandbox
            ),
            (None, None, None, None, None, None)
        );
        let operation: OperationResult =
            serde_json::from_str(r#"{"transport":"completed","outcome":"protocol_incomplete"}"#)
                .unwrap();
        assert_eq!(
            (
                operation.termination,
                operation.evidence,
                operation.delivery
            ),
            (None, None, None)
        );
    }
}
