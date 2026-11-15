//! Classify recurring failures and audit harness rules.
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    MissingContext,
    WrongTool,
    BadOutput,
    RepeatedLoop,
    UnsafeAction,
    LostDecision,
    Unknown,
}

impl FailureClass {
    pub const ALL: [Self; 7] = [
        Self::MissingContext,
        Self::WrongTool,
        Self::BadOutput,
        Self::RepeatedLoop,
        Self::UnsafeAction,
        Self::LostDecision,
        Self::Unknown,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::MissingContext => "MISSING CONTEXT",
            Self::WrongTool => "WRONG TOOL",
            Self::BadOutput => "BAD OUTPUT",
            Self::RepeatedLoop => "REPEATED LOOP",
            Self::UnsafeAction => "UNSAFE ACTION",
            Self::LostDecision => "LOST DECISION",
            Self::Unknown => "UNKNOWN",
        }
    }
    pub fn change_type(self) -> &'static str {
        match self {
            Self::MissingContext => "a map or retrieval rule",
            Self::WrongTool => "a better tool description or routing",
            Self::BadOutput => "a validator or stronger contract",
            Self::RepeatedLoop => "a retry cap and escalation",
            Self::UnsafeAction => "a permission gate",
            Self::LostDecision => "durable state",
            Self::Unknown => "tracing and evidence capture",
        }
    }
}

pub fn classify(p: &super::digest::Problem) -> FailureClass {
    if p.family == "lint" {
        FailureClass::MissingContext
    } else if p.id == "hook.refusal" {
        FailureClass::UnsafeAction
    } else if (p.id.starts_with("orch.") && p.id.ends_with("-failed"))
        || p.id.starts_with("review.finding")
        || p.family.starts_with("review.finding")
    {
        FailureClass::BadOutput
    } else if p.id == "orch.re-review" {
        FailureClass::RepeatedLoop
    } else if p.family == "handoff" {
        FailureClass::LostDecision
    } else {
        FailureClass::Unknown
    }
}

pub const CHECKLIST: [&str; 12] = [
    "success defined up front",
    "the map, not everything",
    "tool contracts",
    "isolation",
    "durable decisions",
    "evidence on risky steps",
    "approval for irreversible actions",
    "retry caps and budgets",
    "resume",
    "explainable traces",
    "failures updating guides or tests",
    "rollback",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    Passed,
    Gap,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecklistItem {
    pub name: &'static str,
    pub status: ItemStatus,
    pub evidence: Option<String>,
}

pub fn audit(evidence: &BTreeMap<String, (bool, String)>) -> Vec<ChecklistItem> {
    CHECKLIST
        .iter()
        .map(|&name| {
            let (status, evidence) = match evidence.get(name) {
                Some((passed, text)) => (
                    if *passed {
                        ItemStatus::Passed
                    } else {
                        ItemStatus::Gap
                    },
                    Some(text.clone()),
                ),
                None => (ItemStatus::Unknown, None),
            };
            ChecklistItem {
                name,
                status,
                evidence,
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    Test,
    Lint,
    Hook,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MechanicalCheck {
    pub kind: CheckKind,
    pub id: String,
    pub failed_before: Option<String>,
    pub passed_after: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleProposal {
    pub guide: Option<String>,
    pub check: Option<MechanicalCheck>,
}

pub fn judge_rule(r: &RuleProposal) -> Result<(), String> {
    fn present(value: &Option<String>) -> bool {
        value.as_deref().is_some_and(|s| !s.trim().is_empty())
    }
    if !present(&r.guide) {
        return Err("missing or blank guide".into());
    }
    let check = r.check.as_ref().ok_or("missing mechanical check")?;
    if !present(&check.failed_before) {
        return Err("missing or blank failed-before evidence".into());
    }
    if !present(&check.passed_after) {
        return Err("missing or blank passed-after evidence".into());
    }
    Ok(())
}
