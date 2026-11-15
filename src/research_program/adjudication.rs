use super::{
    investigation::{MemberCall, ResearchMember},
    Stage,
};
use crate::{
    base::{self, schema, Failure, Res},
    council::Status,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub stage: Stage,
    pub item: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RulingDecision {
    Upheld,
    Overruled,
    Narrowed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ruling {
    pub finding: String,
    pub decision: RulingDecision,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adjudication {
    pub model: String,
    pub rulings: Vec<Ruling>,
    pub dropped_changes: Vec<String>,
}

pub trait Adjudicator: Sync {
    fn adjudicate(
        &self,
        plan: &Value,
        findings: &[Finding],
        stop: &AtomicBool,
    ) -> Res<Adjudication>;
}

pub struct MemberAdjudicator<'a>(pub &'a dyn ResearchMember);

// The seam receives values, not paths. Give the member a separate scratch directory instead of
// exposing the program or another member's files.
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl Adjudicator for MemberAdjudicator<'_> {
    fn adjudicate(
        &self,
        plan: &Value,
        findings: &[Finding],
        stop: &AtomicBool,
    ) -> Res<Adjudication> {
        if stop.load(Ordering::Relaxed) {
            return Err(Failure::new("interrupted", "adjudication interrupted"));
        }
        let expected: HashSet<&str> = findings.iter().map(|f| f.id.as_str()).collect();
        if expected.len() != findings.len() {
            return Err(Failure::new("malformed", "duplicate finding id"));
        }
        let schema = json!({
            "type": "object", "required": ["rulings", "dropped_changes"], "additionalProperties": false,
            "properties": {
                "rulings": {"type": "array", "items": {"type": "object", "required": ["finding", "decision", "reason"], "additionalProperties": false, "properties": {
                    "finding": {"type": "string"}, "decision": {"type": "string", "enum": ["upheld", "overruled", "narrowed"]}, "reason": {"type": "string"}
                }}},
                "dropped_changes": {"type": "array", "items": {"type": "string"}}
            }
        });
        let dir = std::env::temp_dir().join(format!(
            "research-adjudication-{}-{}",
            std::process::id(),
            base::unique()
        ));
        fs::create_dir(&dir).map_err(|e| Failure::new("io", format!("{}: {e}", dir.display())))?;
        let scratch = Scratch(dir);
        let call = MemberCall {
            member: self.0.name().into(), stage: "adjudication".into(),
            prompt: format!("Adjudicate each finding against the plan. Return exactly one ruling for each finding, with a reason; list dropped change IDs.\nPlan:\n{plan}\nFindings:\n{}\n", serde_json::to_string(findings).map_err(|e| Failure::new("json", e.to_string()))?),
            readable: vec![], out_dir: scratch.0.clone(), schema: schema.clone(), timeout_s: 900,
        };
        let reply = self.0.call(&call, stop);
        if stop.load(Ordering::Relaxed) {
            return Err(Failure::new("interrupted", "adjudication interrupted"));
        }
        if reply.status != Status::Ok {
            let kind = if reply.status == Status::Malformed {
                "malformed"
            } else {
                "member_failed"
            };
            return Err(Failure::new(
                kind,
                reply.detail.unwrap_or_else(|| reply.status.word().into()),
            ));
        }
        let output = reply
            .output
            .ok_or_else(|| Failure::new("malformed", "missing adjudication output"))?;
        schema::validate(&schema, &output).map_err(|e| Failure::new("malformed", e))?;
        #[derive(Deserialize)]
        struct Output {
            rulings: Vec<Ruling>,
            dropped_changes: Vec<String>,
        }
        let output: Output =
            serde_json::from_value(output).map_err(|e| Failure::new("malformed", e.to_string()))?;
        let mut seen = HashSet::new();
        for ruling in &output.rulings {
            if !expected.contains(ruling.finding.as_str()) || !seen.insert(ruling.finding.as_str())
            {
                return Err(Failure::new(
                    "malformed",
                    format!("unknown or duplicate ruling: {}", ruling.finding),
                ));
            }
        }
        if seen.len() != expected.len() {
            return Err(Failure::new("malformed", "missing ruling for finding"));
        }
        Ok(Adjudication {
            model: reply.model.unwrap_or_else(|| self.0.name().into()),
            rulings: output.rulings,
            dropped_changes: output.dropped_changes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research_program::investigation::MemberReply;

    struct Fake(Value);
    impl ResearchMember for Fake {
        fn name(&self) -> &str {
            "fake"
        }
        fn call(&self, call: &MemberCall, _: &AtomicBool) -> MemberReply {
            assert_eq!(call.stage, "adjudication");
            assert!(call.out_dir.is_dir());
            MemberReply {
                status: Status::Ok,
                output: Some(self.0.clone()),
                raw: None,
                model: Some("test-model".into()),
                provider: "fake".into(),
                usd: None,
                tokens: 0,
                detail: None,
            }
        }
    }
    fn findings() -> Vec<Finding> {
        vec![
            Finding {
                id: "R1".into(),
                stage: Stage::RedTeam,
                item: "a".into(),
                reason: "risk".into(),
            },
            Finding {
                id: "P1".into(),
                stage: Stage::PreMortem,
                item: "b".into(),
                reason: "risk".into(),
            },
        ]
    }
    #[test]
    fn missing_or_duplicate_rulings_are_malformed() {
        let r = json!({"finding":"R1", "decision":"upheld", "reason":"risk"});
        for rulings in [vec![r.clone()], vec![r.clone(), r.clone()]] {
            let member = Fake(json!({"rulings":rulings, "dropped_changes":[]}));
            let err = MemberAdjudicator(&member)
                .adjudicate(&json!({}), &findings(), &AtomicBool::new(false))
                .unwrap_err();
            assert_eq!(err.kind, "malformed");
        }
    }
    #[test]
    fn complete_rulings_use_member_model() {
        let member = Fake(
            json!({"rulings":[{"finding":"R1","decision":"upheld","reason":"risk"},{"finding":"P1","decision":"narrowed","reason":"risk"}], "dropped_changes":["C1"]}),
        );
        let result = MemberAdjudicator(&member)
            .adjudicate(&json!({}), &findings(), &AtomicBool::new(false))
            .unwrap();
        assert_eq!(result.model, "test-model");
        assert_eq!(result.rulings.len(), 2);
        assert_eq!(result.dropped_changes, ["C1"]);
    }
}
