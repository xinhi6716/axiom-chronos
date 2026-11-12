use axiom::{
    council::Status,
    paper::{
        disclosure,
        style::{self, StyleViolation, ViolationKind as V},
        *,
    },
};
use serde_json::json;
use std::{collections::BTreeSet, fs, path::PathBuf};

fn manuscript(name: &str) -> Manuscript {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/paper/style")
        .join(format!("{name}.json"));
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn bundle() -> EvidenceBundle {
    serde_json::from_value(json!({
        "version":1, "program":"test", "question":"outcome", "artifacts":[],
        "claims":[{"id":"C1","text":"first","evidence":["E1"],"confidence":"high"},
                  {"id":"C2","text":"second","evidence":["E1"],"confidence":"high"}],
        "evidence":[{"id":"E1","source":"data","artifact":null,"locator":null}],
        "quantities":[{"id":"q1","claim_ids":["C1"],"value":2.5,"unit":null,"n":null,"uncertainty":null,"test":null,"p_value":null,"precision":1,"locator":{"artifact":"data.csv","column":null,"row":null,"aggregate":null}}],
        "references":[{"key":"smith","doi":null,"openalex":null,"url":null,"evidence":[]}], "hash":"test"
    })).unwrap()
}

fn contains_kind(violations: &[StyleViolation], kind: V) {
    assert!(
        violations.iter().any(|v| v.kind == kind),
        "missing {kind:?} in {violations:?}"
    );
}

#[test]
fn style_accepts_wording_and_existing_literal() {
    let before = manuscript("before");
    assert!(style::check_style_only(&before, &manuscript("wording")).is_ok());
    assert!(style::check_style_only(&before, &manuscript("existing_literal")).is_ok());
    assert!(style::check_style_only(&before, &manuscript("caption_rewritten")).is_ok());
}

macro_rules! style_rejection {
    ($test:ident, $fixture:literal, $kind:expr) => {
        #[test]
        fn $test() {
            let result = style::check_style_only(&manuscript("before"), &manuscript($fixture));
            assert!(result.is_err(), "{} must be rejected", $fixture);
            contains_kind(&result.unwrap_err(), $kind);
        }
    };
}
style_rejection!(style_rejects_claim_change, "claim_changed", V::ClaimChanged);
style_rejection!(
    style_rejects_quantity_change,
    "number_changed",
    V::NumberChanged
);
style_rejection!(
    style_rejects_citation_change,
    "citation_changed",
    V::CitationChanged
);
style_rejection!(
    style_rejects_removed_hedge,
    "hedge_removed",
    V::StrengthenedClaim
);
style_rejection!(
    style_rejects_english_booster,
    "booster_english",
    V::StrengthenedClaim
);
style_rejection!(
    style_rejects_chinese_booster,
    "booster_chinese",
    V::StrengthenedClaim
);
style_rejection!(
    style_rejects_literal_number,
    "literal_number",
    V::LiteralNumber
);
style_rejection!(
    style_rejects_renamed_paragraph,
    "paragraph_renamed",
    V::StructureChanged
);
style_rejection!(
    style_rejects_dropped_section,
    "section_dropped",
    V::StructureChanged
);

style_rejection!(
    style_rejects_changed_table_column,
    "table_column_changed",
    V::StructureChanged
);

#[test]
fn style_rejects_added_paragraph_as_structure_change() {
    let errors =
        style::check_style_only(&manuscript("before"), &manuscript("paragraph_added")).unwrap_err();
    contains_kind(&errors, V::StructureChanged);
    assert!(!errors.iter().any(|v| v.kind == V::NewClaim));
}

#[test]
fn style_rejects_literal_numbers_in_title_and_heading() {
    for (fixture, location) in [
        ("title_literal", "title"),
        ("heading_literal", "sec:results/heading"),
    ] {
        let errors =
            style::check_style_only(&manuscript("before"), &manuscript(fixture)).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|v| v.kind == V::LiteralNumber && v.location == location),
            "{errors:?}"
        );
    }
}

#[test]
fn style_collects_violations() {
    let errors = style::check_style_only(&manuscript("before"), &manuscript("two_violations"))
        .expect_err("both changes must be rejected");
    contains_kind(&errors, V::NumberChanged);
    contains_kind(&errors, V::CitationChanged);
}

#[test]
fn revision_allows_restructure_with_documented_removal() {
    let before = manuscript("before");
    let after = manuscript("revision_removed");
    let log = [Change {
        location: "C1".into(),
        kind: ChangeKind::Removed,
        reason: "removed unsupported claim".into(),
    }];
    assert!(style::check_revision(&before, &after, &bundle(), &log).is_ok());
    let result = style::check_revision(&before, &after, &bundle(), &[]);
    assert!(result.is_err());
    contains_kind(&result.unwrap_err(), V::ClaimChanged);
    let reason_only = [Change {
        location: "results".into(),
        kind: ChangeKind::Removed,
        reason: "removed C1 as unsupported".into(),
    }];
    assert!(style::check_revision(&before, &after, &bundle(), &reason_only).is_ok());
    let wrong_kind = [Change {
        kind: ChangeKind::Edited,
        ..reason_only[0].clone()
    }];
    let errors = style::check_revision(&before, &after, &bundle(), &wrong_kind).unwrap_err();
    contains_kind(&errors, V::ClaimChanged);
}

macro_rules! revision_rejection {
    ($test:ident, $fixture:literal, $kind:expr) => {
        #[test]
        fn $test() {
            let result =
                style::check_revision(&manuscript("before"), &manuscript($fixture), &bundle(), &[]);
            assert!(result.is_err(), "{} must be rejected", $fixture);
            contains_kind(&result.unwrap_err(), $kind);
        }
    };
}
revision_rejection!(
    revision_rejects_new_claim,
    "revision_new_claim",
    V::NewClaim
);
revision_rejection!(
    revision_rejects_new_evidence,
    "revision_new_evidence",
    V::NewClaim
);
revision_rejection!(
    revision_rejects_unknown_quantity,
    "revision_unknown_quantity",
    V::NumberChanged
);
revision_rejection!(
    revision_rejects_unknown_citation,
    "revision_unknown_citation",
    V::CitationChanged
);
revision_rejection!(
    revision_rejects_literal_number,
    "revision_literal",
    V::LiteralNumber
);

revision_rejection!(
    revision_rejects_removed_hedge,
    "revision_hedge_removed",
    V::StrengthenedClaim
);
revision_rejection!(
    revision_rejects_added_booster,
    "revision_booster_added",
    V::StrengthenedClaim
);

#[test]
fn revision_allows_hedge_moved_between_claim_paragraphs() {
    assert!(style::check_revision(
        &manuscript("before"),
        &manuscript("revision_hedge_moved"),
        &bundle(),
        &[]
    )
    .is_ok());
}

fn no_evasion_terms(text: &str) {
    let lower = text.to_lowercase();
    for word in [
        "detector",
        "detect",
        "evade",
        "evasion",
        "undetectable",
        "bypass",
    ] {
        assert!(!lower.contains(word), "contains {word}");
    }
}

#[test]
fn instructions_include_audit_humanizer_and_invariants() {
    let asset = axiom::engine::asset("paper/prompts/style.md").expect("style prompt asset");
    for phrase in ["kind \"style\"", "{{q:", "[@", "hedge"] {
        assert!(asset.contains(phrase), "missing {phrase} in style asset");
    }
    let lower_asset = asset.to_lowercase();
    assert!(lower_asset.find("audit").unwrap() < lower_asset.find("rewrite").unwrap());
    let prompt = style::style_instructions();
    let lower = prompt.to_lowercase();
    for word in ["change_log", "style", "claim", "number", "citation"] {
        assert!(lower.contains(word), "missing {word}");
    }
    let skill_text =
        axiom::engine::asset("paper/academic-humanizer/SKILL.md").expect("humanizer asset");
    assert!(prompt.contains(skill_text));
    no_evasion_terms(&prompt.replace(skill_text, ""));
    for word in [
        "detector",
        "evade",
        "evasion",
        "undetectable",
        "bypass",
        "ai detect",
        "ai-detect",
    ] {
        assert!(!lower.contains(word), "contains {word}");
    }
    let revise = style::revise_instructions();
    assert!(!revise.trim().is_empty());
    assert_eq!(
        revise,
        axiom::engine::asset("paper/prompts/revise.md").unwrap()
    );
    no_evasion_terms(&revise);
}

fn journal(publisher: Option<&str>) -> JournalRecord {
    let unknown = json!({"unknown":{"reason":"not supplied"}});
    let publisher = publisher.map(|p| json!({"known":{"value":p,"source":{"kind":"official_page","url":null,"fetched_at":"2026-01-01","data_year":null,"raw":"test"}}})).unwrap_or_else(|| unknown.clone());
    serde_json::from_value(json!({"id":"j","title":"Journal","issns":[],"issn_l":null,"publisher":publisher,"homepage":unknown,"quartiles":[],"sjr":unknown,"access":unknown,"apc":unknown,"review_weeks":unknown,"topics":[],"works_count":unknown})).unwrap()
}

fn pass(
    pass: PassId,
    member: &str,
    model: Option<&str>,
    provider: &str,
    status: Status,
) -> PassRecord {
    PassRecord {
        pass,
        member: member.into(),
        model: model.map(str::to_owned),
        provider: provider.into(),
        status,
        evidence_hash: "e".into(),
        input_hash: "i".into(),
        output_hash: None,
        usd: None,
        tokens: 0,
        raw_path: "raw".into(),
        change_log: vec![],
        validation: vec![],
        at: "2026-01-01".into(),
    }
}

fn passes() -> Vec<PassRecord> {
    vec![
        pass(
            PassId::AstraDraft,
            "astra",
            Some("gpt-test-astra"),
            "OpenAI",
            Status::Ok,
        ),
        pass(
            PassId::FableRevise,
            "fable",
            Some("claude-fable-5-2"),
            "Anthropic",
            Status::Ok,
        ),
        pass(
            PassId::ArgonRevise,
            "argon",
            Some("gemini-4-argon-high"),
            "Google",
            Status::Ok,
        ),
        pass(
            PassId::FableStyle,
            "fable",
            Some("claude-fable-5-2"),
            "Anthropic",
            Status::Ok,
        ),
    ]
}

#[test]
fn policy_catalog_and_lookup() {
    let policies = disclosure::policies().unwrap();
    let names: BTreeSet<_> = policies.iter().map(|p| p.publisher.as_str()).collect();
    assert_eq!(
        names,
        BTreeSet::from(["Elsevier", "IEEE", "Springer Nature", "ACS"])
    );
    let date = regex::Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap();
    let hash = regex::Regex::new(r"^[0-9a-fA-F]{64}$").unwrap();
    for policy in &policies {
        assert!(policy.url.starts_with("https://"));
        assert!(date.is_match(&policy.fetched_at));
        assert!(hash.is_match(&policy.sha256));
        assert!(!policy.placement.trim().is_empty());
        assert!(!policy.required_items.is_empty());
        assert_eq!(
            disclosure::policy_for(&format!("  {}  ", policy.publisher.to_uppercase())),
            Some(policy.clone())
        );
    }
    assert_eq!(
        disclosure::policy_for("springer"),
        disclosure::policy_for("Springer Nature")
    );
    assert!(disclosure::policy_for("not listed").is_none());
}

#[test]
fn disclosure_lists_only_successful_uses_and_policy() {
    let mut records = passes();
    records.push(pass(
        PassId::ArgonRevise,
        "failed",
        Some("never-ran-model"),
        "other",
        Status::Failed,
    ));
    records.push(pass(
        PassId::FableStyle,
        "timed-out",
        Some("timed-out-model"),
        "other",
        Status::Timeout,
    ));
    let reviewer = ModelUse {
        pass: None,
        member: "reviewer".into(),
        model: "review-model".into(),
        provider: "review-provider".into(),
        purpose: "review".into(),
    };
    let policy = disclosure::policies()
        .unwrap()
        .into_iter()
        .find(|p| p.policy != PolicyKind::Prohibited)
        .expect("an allowed policy for disclosure test");
    let result =
        disclosure::generate(&records, Some(&reviewer), &journal(Some(&policy.publisher))).unwrap();
    assert_eq!(result.policy, policy);
    assert_eq!(result.placement, policy.placement);
    assert_eq!(result.uses.len(), 5);
    for (i, (record, purpose)) in records[..4]
        .iter()
        .zip([
            "drafting the manuscript from the evidence bundle",
            "substantive revision",
            "critique and revision",
            "style editing of wording only",
        ])
        .enumerate()
    {
        let usage = &result.uses[i];
        assert_eq!(usage.pass, Some(record.pass));
        assert_eq!(usage.member, record.member);
        assert_eq!(usage.model, record.model.as_ref().unwrap().as_str());
        assert_eq!(usage.provider, record.provider);
        assert_eq!(usage.purpose, purpose);
        for value in [&usage.model, &usage.provider, &usage.purpose] {
            assert!(result.text.contains(value), "missing {value}");
        }
    }
    assert_eq!(result.uses[4], reviewer);
    for value in [
        &reviewer.model,
        &reviewer.provider,
        &reviewer.purpose,
        &policy.url,
        &policy.fetched_at,
    ] {
        assert!(result.text.contains(value), "missing {value}");
    }
    for item in &policy.required_items {
        assert!(result.text.contains(item), "missing {item}");
    }
    assert!(!result.text.contains("never-ran-model"));
    assert!(!result.uses.iter().any(|u| u.model == "never-ran-model"));
    assert!(!result.text.contains("timed-out-model"));
    assert!(!result.uses.iter().any(|u| u.model == "timed-out-model"));
    let lower = result.text.to_lowercase();
    assert!(lower.contains("author") && lower.contains("review") && lower.contains("responsib"));
}

#[test]
fn disclosure_rejects_unknown_prohibited_and_missing_models() {
    assert_eq!(
        disclosure::generate(&passes(), None, &journal(None))
            .unwrap_err()
            .kind,
        "policy_unknown"
    );
    assert_eq!(
        disclosure::generate(&passes(), None, &journal(Some("unlisted publisher")))
            .unwrap_err()
            .kind,
        "policy_unknown"
    );
    let policies = disclosure::policies().unwrap();
    for policy in policies
        .iter()
        .filter(|p| p.policy == PolicyKind::Prohibited)
    {
        assert_eq!(
            disclosure::generate(&passes(), None, &journal(Some(&policy.publisher)))
                .unwrap_err()
                .kind,
            "policy_prohibits"
        );
    }
    let allowed = policies
        .iter()
        .find(|p| p.policy != PolicyKind::Prohibited)
        .expect("an allowed policy");
    let mut records = passes();
    records[0].model = None;
    assert_eq!(
        disclosure::generate(&records, None, &journal(Some(&allowed.publisher)))
            .unwrap_err()
            .kind,
        "missing_model"
    );
}

#[test]
fn disclosure_uses_injected_policy_catalog() {
    let mut prohibited = disclosure::policies().unwrap().remove(0);
    prohibited.publisher = "Springer Nature".into();
    prohibited.policy = PolicyKind::Prohibited;
    let mut allowed = prohibited.clone();
    allowed.publisher = "Allowed Publisher".into();
    allowed.policy = PolicyKind::Allowed;
    let list = [prohibited, allowed.clone()];
    assert_eq!(
        disclosure::generate_with_policies(&list, &passes(), None, &journal(Some("  sPrInGeR  ")))
            .unwrap_err()
            .kind,
        "policy_prohibits"
    );
    let result = disclosure::generate_with_policies(
        &list,
        &passes(),
        None,
        &journal(Some("  ALLOWED PUBLISHER  ")),
    )
    .unwrap();
    assert_eq!(result.policy, allowed);
    assert_eq!(
        disclosure::generate_with_policies(&list, &passes(), None, &journal(Some("missing")))
            .unwrap_err()
            .kind,
        "policy_unknown"
    );
}

#[test]
fn fixtures_and_supporting_data_deserialize() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/paper/style");
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            let _: Manuscript = serde_json::from_str(&fs::read_to_string(&path).unwrap())
                .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
        }
    }
    let _ = bundle();
    let _ = journal(None);
    let _ = journal(Some("Elsevier"));
}
