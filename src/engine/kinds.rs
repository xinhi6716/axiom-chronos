pub const ROLE_SPEND: &str = "role_spend";
pub const BUDGET_EXCEEDED: &str = "budget_exceeded";
pub const ADJUDICATION: &str = "adjudication";
pub const ADJUDICATION_OVERTURNED: &str = "adjudication_overturned";
pub const PERSON_REQUEST: &str = "person_request";
pub const APPROVAL_REQUESTED: &str = "approval_requested";
pub const APPROVAL_APPROVED: &str = "approval_approved";
pub const APPROVAL_DENIED: &str = "approval_denied";
pub const APPROVAL_EXECUTING: &str = "approval_executing";
pub const APPROVAL_EXECUTED: &str = "approval_executed";
pub const INTERJECTION_SENT: &str = "interjection_sent";
pub const INTERJECTION_RECEIVED: &str = "interjection_received";
pub const INTERJECTION_CLASSIFIED: &str = "interjection_classified";
pub const INTERJECTION_APPLIED: &str = "interjection_applied";
pub const INTERJECTION_DECLINED: &str = "interjection_declined";
pub const FINDING_VERIFIED: &str = "finding_verified";
pub const REVIEW_RECORDED: &str = "review_recorded";
pub const DUAL_CORE: &str = "dual_core";
pub const PROMPT_HEAD: &str = "AXIOM-ENGINE";
pub fn prompt_head(role: &str, case: &str) -> String {
    format!("{PROMPT_HEAD} {role} {case}\n")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn event_names_and_prompt() {
        let kinds = [
            ROLE_SPEND,
            BUDGET_EXCEEDED,
            ADJUDICATION,
            ADJUDICATION_OVERTURNED,
            PERSON_REQUEST,
            APPROVAL_REQUESTED,
            APPROVAL_APPROVED,
            APPROVAL_DENIED,
            APPROVAL_EXECUTING,
            APPROVAL_EXECUTED,
            INTERJECTION_SENT,
            INTERJECTION_RECEIVED,
            INTERJECTION_CLASSIFIED,
            INTERJECTION_APPLIED,
            INTERJECTION_DECLINED,
            FINDING_VERIFIED,
            REVIEW_RECORDED,
            DUAL_CORE,
        ];
        assert_eq!(kinds.join(","), "role_spend,budget_exceeded,adjudication,adjudication_overturned,person_request,approval_requested,approval_approved,approval_denied,approval_executing,approval_executed,interjection_sent,interjection_received,interjection_classified,interjection_applied,interjection_declined,finding_verified,review_recorded,dual_core");
        assert_eq!(
            prompt_head("verifier", "case"),
            "AXIOM-ENGINE verifier case\n"
        );
    }
}
