use super::{tokens, Change, ChangeKind, EvidenceBundle, Manuscript, Paragraph, Token};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq)]
pub enum ViolationKind {
    ClaimChanged,
    NumberChanged,
    CitationChanged,
    NewClaim,
    StrengthenedClaim,
    LiteralNumber,
    StructureChanged,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StyleViolation {
    pub location: String,
    pub kind: ViolationKind,
    pub detail: String,
}

pub fn revise_instructions() -> String {
    crate::engine::asset("paper/prompts/revise.md")
        .unwrap_or_default()
        .to_owned()
}
pub fn style_instructions() -> String {
    let mut prompt = crate::engine::asset("paper/prompts/style.md")
        .unwrap_or_default()
        .to_owned();
    if let Some(humanizer) = crate::engine::asset("paper/academic-humanizer/SKILL.md") {
        prompt.push_str("\n\n");
        prompt.push_str(humanizer);
    }
    prompt
}

fn violation(errors: &mut Vec<StyleViolation>, location: &str, kind: ViolationKind, detail: &str) {
    errors.push(StyleViolation {
        location: location.into(),
        kind,
        detail: detail.into(),
    });
}

fn compare_maps<T: PartialEq>(
    before: &BTreeMap<String, T>,
    after: &BTreeMap<String, T>,
    kind: ViolationKind,
    errors: &mut Vec<StyleViolation>,
) {
    for location in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
        if before.get(location) != after.get(location) {
            violation(errors, location, kind.clone(), "invariant changed");
        }
    }
}

fn numerals(text: &str) -> BTreeMap<String, usize> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"\d+(?:[.,]\d+)*").unwrap());
    let mut stripped = String::new();
    let mut end = 0;
    for (range, _) in tokens(text) {
        stripped.push_str(&text[end..range.start]);
        stripped.push(' ');
        end = range.end;
    }
    stripped.push_str(&text[end..]);
    let mut counts = BTreeMap::new();
    for m in re.find_iter(&stripped) {
        *counts.entry(m.as_str().to_owned()).or_insert(0) += 1;
    }
    counts
}
fn text_locations(m: &Manuscript) -> Vec<(String, &str)> {
    let mut texts = vec![("title".into(), m.title.as_str())];
    texts.extend(
        m.sections
            .iter()
            .map(|s| (format!("sec:{}/heading", s.id), s.heading.as_str())),
    );
    texts.extend(
        m.paragraphs()
            .into_iter()
            .map(|(location, p)| (location, p.text.as_str())),
    );
    texts
}
fn excess(before: &BTreeMap<String, usize>, after: &BTreeMap<String, usize>) -> bool {
    after
        .iter()
        .any(|(number, count)| *count > before.get(number).copied().unwrap_or(0))
}
fn hedge_counts(p: &Paragraph) -> (usize, usize) {
    static HEDGES: OnceLock<regex::Regex> = OnceLock::new();
    static BOOSTERS: OnceLock<regex::Regex> = OnceLock::new();
    let h = HEDGES.get_or_init(|| {
        regex::Regex::new(r"(?i)\b(?:may|might|suggests|could|appears|likely)\b|可能|或許|顯示")
            .unwrap()
    });
    let b = BOOSTERS.get_or_init(|| {
        regex::Regex::new(
            r"(?i)\b(?:proves|demonstrates conclusively|always|never|clearly)\b|證明|必然",
        )
        .unwrap()
    });
    (h.find_iter(&p.text).count(), b.find_iter(&p.text).count())
}
fn strengthened(before: (usize, usize), after: (usize, usize)) -> bool {
    after.0 < before.0 || after.1 > before.1
}

pub fn check_style_only(
    before: &Manuscript,
    after: &Manuscript,
) -> Result<(), Vec<StyleViolation>> {
    let mut errors = Vec::new();
    let (a, b) = (before.invariants(), after.invariants());
    compare_maps(
        &a.claims,
        &b.claims,
        ViolationKind::ClaimChanged,
        &mut errors,
    );
    compare_maps(
        &a.quantities,
        &b.quantities,
        ViolationKind::NumberChanged,
        &mut errors,
    );
    compare_maps(
        &a.citations,
        &b.citations,
        ViolationKind::CitationChanged,
        &mut errors,
    );
    let section_ids = |m: &Manuscript| {
        m.sections
            .iter()
            .map(|s| {
                (
                    s.id.clone(),
                    s.paragraphs
                        .iter()
                        .map(|p| p.id.clone())
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
    };
    if section_ids(before) != section_ids(after)
        || before.abstract_text.id != after.abstract_text.id
    {
        violation(
            &mut errors,
            "manuscript",
            ViolationKind::StructureChanged,
            "section or paragraph ids changed",
        );
    }
    let mut bf = before.figures.clone();
    let mut af = after.figures.clone();
    for f in bf.iter_mut().chain(af.iter_mut()) {
        f.caption.text.clear();
    }
    let mut bt = before.tables.clone();
    let mut at = after.tables.clone();
    for t in bt.iter_mut().chain(at.iter_mut()) {
        t.caption.text.clear();
    }
    if bf != af {
        violation(
            &mut errors,
            "figures",
            ViolationKind::StructureChanged,
            "figure specifications changed",
        );
    }
    if bt != at {
        violation(
            &mut errors,
            "tables",
            ViolationKind::StructureChanged,
            "table specifications changed",
        );
    }
    let old_texts: BTreeMap<_, _> = text_locations(before).into_iter().collect();
    for (location, text) in text_locations(after) {
        if excess(
            &numerals(old_texts.get(&location).copied().unwrap_or("")),
            &numerals(text),
        ) {
            violation(
                &mut errors,
                &location,
                ViolationKind::LiteralNumber,
                "new literal numeral",
            );
        }
    }
    let old: BTreeMap<_, _> = before.paragraphs().into_iter().collect();
    for (location, p) in after.paragraphs() {
        if let Some(previous) = old.get(&location) {
            if !p.claims.is_empty() && strengthened(hedge_counts(previous), hedge_counts(p)) {
                violation(
                    &mut errors,
                    &location,
                    ViolationKind::StrengthenedClaim,
                    "hedges decreased or boosters increased",
                );
            }
        } else {
            violation(
                &mut errors,
                &location,
                ViolationKind::StructureChanged,
                "new paragraph location",
            );
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn check_revision(
    before: &Manuscript,
    after: &Manuscript,
    bundle: &EvidenceBundle,
    change_log: &[Change],
) -> Result<(), Vec<StyleViolation>> {
    let mut errors = Vec::new();
    let claims: BTreeSet<_> = bundle.claims.iter().map(|c| c.id.as_str()).collect();
    let evidence: BTreeSet<_> = bundle.evidence.iter().map(|e| e.id.as_str()).collect();
    let quantities: BTreeSet<_> = bundle.quantities.iter().map(|q| q.id.as_str()).collect();
    let mut citations: BTreeSet<String> = bundle.references.iter().map(|r| r.key.clone()).collect();
    for (_, text) in text_locations(before) {
        for (_, token) in tokens(text) {
            if let Token::Cite(keys) = token {
                citations.extend(keys);
            }
        }
    }
    let before_claims: BTreeSet<_> = before
        .paragraphs()
        .into_iter()
        .flat_map(|(_, p)| p.claims.iter().map(|c| c.claim.as_str()))
        .collect();
    let mut after_claims = BTreeSet::new();
    for (location, p) in after.paragraphs() {
        for link in &p.claims {
            after_claims.insert(link.claim.as_str());
            if !claims.contains(link.claim.as_str()) {
                violation(
                    &mut errors,
                    &location,
                    ViolationKind::NewClaim,
                    &format!("unknown claim {}", link.claim),
                );
            }
            for id in &link.evidence {
                if !evidence.contains(id.as_str()) {
                    violation(
                        &mut errors,
                        &location,
                        ViolationKind::NewClaim,
                        &format!("unknown evidence {id}"),
                    );
                }
            }
        }
    }
    for (location, text) in text_locations(after) {
        for (_, token) in tokens(text) {
            match token {
                Token::Quantity(id) if !quantities.contains(id.as_str()) => violation(
                    &mut errors,
                    &location,
                    ViolationKind::NumberChanged,
                    &format!("unknown quantity {id}"),
                ),
                Token::Cite(keys) => {
                    for key in keys {
                        if !citations.contains(&key) {
                            violation(
                                &mut errors,
                                &location,
                                ViolationKind::CitationChanged,
                                &format!("unknown citation {key}"),
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }
    for claim in before_claims.difference(&after_claims) {
        if !change_log.iter().any(|c| {
            c.kind == ChangeKind::Removed
                && (c.reason.contains(claim) || c.location.contains(claim))
        }) {
            violation(
                &mut errors,
                claim,
                ViolationKind::ClaimChanged,
                "claim removed without a removal entry",
            );
        }
    }
    let mut old_numbers = BTreeSet::new();
    for (_, text) in text_locations(before) {
        old_numbers.extend(numerals(text).into_keys());
    }
    for (location, text) in text_locations(after) {
        for n in numerals(text).into_keys() {
            if !old_numbers.contains(&n) {
                violation(
                    &mut errors,
                    &location,
                    ViolationKind::LiteralNumber,
                    &format!("new literal numeral {n}"),
                );
            }
        }
    }
    let totals = |m: &Manuscript| {
        m.paragraphs()
            .into_iter()
            .filter(|(_, p)| !p.claims.is_empty())
            .map(|(_, p)| hedge_counts(p))
            .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
    };
    if strengthened(totals(before), totals(after)) {
        violation(
            &mut errors,
            "manuscript",
            ViolationKind::StrengthenedClaim,
            "hedges decreased or boosters increased",
        );
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
