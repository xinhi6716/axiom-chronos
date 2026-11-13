use super::*;
use crate::{desktop::ornament::Celestial, paper::PassId};
use std::prelude::v1::test;
#[test]
fn pass_mapping() {
    for (p, n, g) in [
        (PassId::AstraDraft, "Astra", Celestial::Star),
        (PassId::FableRevise, "Fable", Celestial::Jupiter),
        (PassId::ArgonRevise, "Argon", Celestial::Gemini),
        (PassId::FableStyle, "Fable", Celestial::Jupiter),
    ] {
        let (name, glyph) = progress::glyph(p);
        assert_eq!(name, n);
        assert_eq!(glyph, g);
    }
}
#[test]
fn gate_requires_choice_estimate_and_ack() {
    assert!(!can_start(false, true, true, false));
    assert!(!can_start(true, false, true, false));
    assert!(!can_start(true, true, false, false));
    assert!(!can_start(true, true, true, true));
    assert!(can_start(true, true, true, false));
}
#[test]
fn no_preselection_and_stale_choice_are_rejected() {
    use crate::paper::{Chooser, JournalChoice};
    assert!(!is_chosen(None, "first", "current"));
    let choice = JournalChoice {
        journal_id: "second".into(),
        shortlist_hash: "current".into(),
        chosen_by: Chooser::Person,
        at: String::new(),
    };
    assert!(!is_chosen(Some(&choice), "first", "current"));
    assert!(!is_chosen(Some(&choice), "second", "stale"));
    assert!(is_chosen(Some(&choice), "second", "current"));
}

#[test]
fn form_conversion_covers_requirements_and_jcr() {
    use crate::paper::{AccessWanted, Money, Quartile, QuartileScheme};
    let mut fields: Vec<String> = [
        "abstract",
        " alpha, , beta ",
        " 123 ",
        " USD ",
        " medicine, biology,, ",
        " fast, open ",
        " reports/jcr.csv ",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let (req, path) = form::convert(
        &fields,
        Some(Quartile::Q2),
        QuartileScheme::Jcr,
        AccessWanted::Subscription,
        true,
    )
    .unwrap();
    assert_eq!(req.abstract_text, "abstract");
    assert_eq!(req.keywords, ["alpha", "beta"]);
    assert_eq!(req.categories, ["medicine", "biology"]);
    assert_eq!(req.free_text, ["fast", "open"]);
    assert_eq!(req.max_quartile, Some(Quartile::Q2));
    assert_eq!(req.tier_scheme, QuartileScheme::Jcr);
    assert_eq!(req.access, AccessWanted::Subscription);
    assert!(req.require_no_fee);
    assert_eq!(
        req.apc_limit,
        Some(Money {
            amount_minor: 123,
            currency: "USD".into()
        })
    );
    assert_eq!(path, Some(std::path::PathBuf::from("reports/jcr.csv")));
    fields[3] = " ".into();
    assert!(form::convert(
        &fields,
        None,
        QuartileScheme::Sjr,
        AccessWanted::Either,
        false
    )
    .is_err());
    fields[2] = "not a number".into();
    fields[3] = "USD".into();
    assert!(form::convert(
        &fields,
        None,
        QuartileScheme::Sjr,
        AccessWanted::OpenAccess,
        false
    )
    .is_err());
    fields[2] = " ".into();
    fields[6] = " ".into();
    let (req, path) = form::convert(
        &fields,
        None,
        QuartileScheme::Sjr,
        AccessWanted::OpenAccess,
        false,
    )
    .unwrap();
    assert_eq!(req.apc_limit, None);
    assert_eq!(req.access, AccessWanted::OpenAccess);
    assert_eq!(path, None);
}

#[test]
fn evidence_question_is_loaded_separately_from_status_hash() {
    use crate::desktop::ux::real::paper::SessionPaper;
    use crate::paper::{evidence, EvidenceBundle};
    let home = tempfile::tempdir().unwrap();
    let session = "paper-question";
    let goal = crate::goal::open(
        home.path(),
        session,
        "wish",
        Some(home.path().to_str().unwrap()),
    )
    .unwrap();
    let store = crate::orch::store_for_goal(home.path(), &goal).unwrap();
    let bundle = EvidenceBundle {
        version: 1,
        program: "program".into(),
        question: "Research question?".into(),
        artifacts: vec![],
        claims: vec![],
        evidence: vec![],
        quantities: vec![],
        references: vec![],
        hash: "evidence-hash".into(),
    };
    evidence::save(&store, &bundle).unwrap();
    let service = SessionPaper::new(home.path());
    assert_eq!(
        service.status(session).unwrap().evidence.as_deref(),
        Some("evidence-hash")
    );
    assert_eq!(
        evidence_question(home.path(), session).unwrap().as_deref(),
        Some("Research question?")
    );
}
