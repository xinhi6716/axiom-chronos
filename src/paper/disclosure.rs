use super::{AiPolicy, Disclosure, JournalRecord, ModelUse, PassId, PassRecord, PolicyKind};
use crate::base::{Failure, Res};
use serde::Deserialize;

#[derive(Deserialize)]
struct Catalog {
    policies: Vec<AiPolicy>,
}

pub fn policies() -> Res<Vec<AiPolicy>> {
    let asset = crate::engine::asset("paper/policies/ai-use.json")
        .ok_or_else(|| Failure::new("policy_unknown", "AI-use policy catalog is missing"))?;
    serde_json::from_str::<Catalog>(asset)
        .map(|catalog| catalog.policies)
        .map_err(|e| Failure::new("malformed", format!("invalid AI-use policy catalog: {e}")))
}
pub fn policy_for(publisher: &str) -> Option<AiPolicy> {
    find_policy(&policies().ok()?, publisher).cloned()
}

fn find_policy<'a>(list: &'a [AiPolicy], publisher: &str) -> Option<&'a AiPolicy> {
    let publisher = publisher.trim();
    list.iter().find(|p| {
        p.publisher.eq_ignore_ascii_case(publisher)
            || (publisher.eq_ignore_ascii_case("Springer")
                && p.publisher.eq_ignore_ascii_case("Springer Nature"))
    })
}
pub fn generate(
    passes: &[PassRecord],
    reviewer: Option<&ModelUse>,
    journal: &JournalRecord,
) -> Res<Disclosure> {
    generate_with_policies(&policies()?, passes, reviewer, journal)
}

pub fn generate_with_policies(
    list: &[AiPolicy],
    passes: &[PassRecord],
    reviewer: Option<&ModelUse>,
    journal: &JournalRecord,
) -> Res<Disclosure> {
    let publisher = journal
        .publisher
        .value()
        .ok_or_else(|| Failure::new("policy_unknown", "publisher is unknown"))?;
    let policy = find_policy(list, publisher).cloned().ok_or_else(|| {
        Failure::new(
            "policy_unknown",
            format!("no AI-use policy for {publisher}"),
        )
    })?;
    if policy.policy == PolicyKind::Prohibited {
        return Err(Failure::new(
            "policy_prohibits",
            format!("{} prohibits AI use", policy.publisher),
        ));
    }
    let mut uses = Vec::new();
    for record in passes
        .iter()
        .filter(|p| p.status == crate::council::Status::Ok && p.validation.is_empty())
    {
        let model = record.model.clone().ok_or_else(|| {
            Failure::new(
                "missing_model",
                format!("successful pass {:?} has no model", record.pass),
            )
        })?;
        let purpose = match record.pass {
            PassId::AstraDraft => "drafting the manuscript from the evidence bundle",
            PassId::FableRevise => "substantive revision",
            PassId::GeminiRevise => "critique and revision",
            PassId::FableStyle => "style editing of wording only",
        };
        uses.push(ModelUse {
            pass: Some(record.pass),
            member: record.member.clone(),
            model,
            provider: record.provider.clone(),
            purpose: purpose.into(),
        });
    }
    if let Some(reviewer) = reviewer {
        uses.push(reviewer.clone());
    }
    let mut text = String::from("The authors used the following AI systems:\n");
    for usage in &uses {
        text.push_str(&format!(
            "- {} ({}): {}.\n",
            usage.model, usage.provider, usage.purpose
        ));
    }
    text.push_str("The authors reviewed the content and take responsibility for it.\n");
    text.push_str(&format!(
        "Publisher policy: {} (fetched {}).\nRequired items: {}.",
        policy.url,
        policy.fetched_at,
        policy.required_items.join("; ")
    ));
    Ok(Disclosure {
        text,
        placement: policy.placement.clone(),
        uses,
        policy,
    })
}
