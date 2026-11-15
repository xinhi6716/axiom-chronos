use super::{
    asset,
    budgets::{self, Role},
    calls::{self, EngineCall},
    kinds, policy,
};
use crate::{
    base::{self, Failure, Res},
    codex::AppServer,
    orch::{
        self,
        events::{self, Event},
    },
};
use regex::Regex;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub enum Case {
    NeedsCheck {
        unit: String,
        finding: usize,
        needs: String,
    },
    FixRounds {
        unit: String,
    },
    CloseAsk {
        question: String,
    },
    CouncilDispute {
        claim: String,
        positions: Vec<String>,
    },
}
#[derive(Debug, Clone)]
pub struct Evidence {
    pub id: String,
    pub path: Option<PathBuf>,
    pub text: String,
}
#[derive(Debug, Clone)]
pub enum Outcome {
    Ruled(Ruling),
    Person { reason: String },
}
#[derive(Debug, Clone)]
pub struct Ruling {
    pub id: String,
    pub decision: String,
    pub reason: String,
    pub evidence: Vec<String>,
    pub answer: Option<String>,
}

impl Case {
    fn name(&self) -> &'static str {
        match self {
            Self::NeedsCheck { .. } => "needs_check",
            Self::FixRounds { .. } => "fix_rounds",
            Self::CloseAsk { .. } => "close_ask",
            Self::CouncilDispute { .. } => "council_dispute",
        }
    }
    fn unit(&self) -> Option<&str> {
        match self {
            Self::NeedsCheck { unit, .. } | Self::FixRounds { unit } => Some(unit),
            _ => None,
        }
    }
    fn subject(&self) -> String {
        match self {
            Self::NeedsCheck { unit, finding, .. } => format!("{unit}#{finding}"),
            Self::FixRounds { unit } => unit.clone(),
            Self::CloseAsk { question } => question.clone(),
            Self::CouncilDispute { claim, .. } => claim.clone(),
        }
    }
    fn description(&self) -> String {
        match self {
            Self::NeedsCheck {
                unit,
                finding,
                needs,
            } => format!("Needs check: unit {unit}, finding {finding}, needs {needs}"),
            Self::FixRounds { unit } => format!("Fix rounds: unit {unit}"),
            Self::CloseAsk { question } => format!("Question: {question}"),
            Self::CouncilDispute { claim, positions } => {
                format!("Claim: {claim}\nPositions:\n{}", positions.join("\n"))
            }
        }
    }
    fn allowed(&self, d: &str) -> bool {
        match self {
            Self::NeedsCheck { .. } => matches!(d, "pass" | "block"),
            Self::FixRounds { .. } => matches!(d, "waive" | "block"),
            _ => matches!(d, "answer" | "block"),
        }
    }
}
fn person(store: &Path, case: &Case, reason: &str) -> Res<Outcome> {
    events::append_event(
        store,
        kinds::PERSON_REQUEST,
        None,
        case.unit(),
        json!({"case":case.name(),"reason":reason}),
    )?;
    Ok(Outcome::Person {
        reason: reason.into(),
    })
}
fn line(store: &Path, text: &str) -> Res<()> {
    fs::create_dir_all(store).map_err(|e| Failure::new("io", e.to_string()))?;
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(store.join("decisions.md"))
        .map_err(|e| Failure::new("io", e.to_string()))?;
    writeln!(f, "{}", text.replace(['\r', '\n'], " "))
        .map_err(|e| Failure::new("io", e.to_string()))
}
pub fn adjudicate(
    codex: &AppServer,
    store: &Path,
    repo: &Path,
    case: &Case,
    evidence: Vec<Evidence>,
    reserved: bool,
) -> Res<Outcome> {
    if reserved
        || match case {
            Case::CloseAsk { question } => is_reserved(question),
            Case::CouncilDispute { claim, .. } => is_reserved(claim),
            _ => false,
        }
    {
        return person(store, case, "reserved");
    }
    let p = policy::load(store)?.adjudicator;
    if !p.enabled {
        return person(store, case, "disabled");
    }
    let count = events::read_events(store, 0)?
        .iter()
        .filter(|e| e.kind == kinds::ADJUDICATION)
        .count();
    if count >= p.max_adjudications as usize {
        return person(store, case, "cap_count");
    }
    let spend = budgets::spent(store, Role::Adjudicator)?;
    if spend.unknown || spend.usd >= p.max_usd {
        return person(store, case, "cap_spend");
    }
    if evidence.is_empty() {
        return person(store, case, "missing_evidence");
    }
    let mut pack = format!(
        "{}\n{}",
        asset("roles/adjudicator.md").unwrap_or_default(),
        case.description()
    );
    for item in &evidence {
        pack.push_str(&format!("\n[{}] {}", item.id, item.text));
    }
    let schema = json!({"type":"object","properties":{"decision":{"type":"string"},"reason":{"type":"string"},"evidence":{"type":"array","items":{"type":"string"}},"answer":{"type":"string"}},"required":["decision","reason","evidence"],"additionalProperties":false});
    let call = EngineCall {
        role: Role::Adjudicator,
        case: case.name(),
        pack,
        cwd: repo,
        schema,
        timeout_s: 600,
    };
    let value = match calls::astra_max(codex, store, &call) {
        Ok(v) => v,
        Err(e) => {
            return person(
                store,
                case,
                if e.kind == "budget_exceeded" {
                    "budget_exceeded"
                } else {
                    "failed"
                },
            )
        }
    };
    let parsed = || -> Option<(String, String, Vec<String>, Option<String>)> {
        let decision = value.get("decision")?.as_str()?.to_owned();
        let reason = value.get("reason")?.as_str()?.to_owned();
        let ids = value
            .get("evidence")?
            .as_array()?
            .iter()
            .map(|v| v.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()?;
        let answer = match value.get("answer") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_str()?.to_owned()),
        };
        Some((decision, reason, ids, answer))
    };
    let Some((decision, reason, ids, answer)) = parsed() else {
        return person(store, case, "failed");
    };
    if !case.allowed(&decision) {
        return person(store, case, "failed");
    }
    if decision == "pass" && !ids.iter().any(|id| evidence.iter().any(|e| &e.id == id)) {
        return person(store, case, "missing_evidence");
    }
    let id = format!("adj-{}", count + 1);
    let subject = case.subject();
    events::append_event(
        store,
        kinds::ADJUDICATION,
        None,
        case.unit(),
        json!({"id":id,"case":case.name(),"subject":subject,"decision":decision,"reason":reason,"evidence":ids,"on_behalf_of_person":true}),
    )?;
    line(
        store,
        &format!(
            "- [adjudicator {id}] {} {subject}: {decision}: {reason} (evidence: {})",
            case.name(),
            ids.join(", ")
        ),
    )?;
    Ok(Outcome::Ruled(Ruling {
        id,
        decision,
        reason,
        evidence: ids,
        answer,
    }))
}
pub fn is_reserved(text: &str) -> bool {
    Regex::new(r"(?i)\b(?:push|publish|merge to main|delete data|pay|order|purchase|credential|login|password|token|licence|license|goal change|scope change|change the goal|change the scope)\b").unwrap().is_match(text)
}
fn cap(text: &str) -> String {
    let mut end = text.len().min(8192);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}
fn excluded(path: &Path) -> bool {
    path.components().any(|c| {
        let s = c.as_os_str().to_string_lossy().to_ascii_lowercase();
        s.contains("brief")
            || s.contains("handoff")
            || s.contains("dispatcher")
            || s.contains("core-notes")
            || s.contains("core_notes")
            || s.starts_with("core.")
            || s == "core"
    })
}
fn add_file(pack: &mut Vec<Evidence>, seen: &mut HashSet<PathBuf>, path: PathBuf) -> Res<()> {
    if excluded(&path) || !path.is_file() {
        return Ok(());
    }
    let canonical = fs::canonicalize(&path).map_err(|e| Failure::new("io", e.to_string()))?;
    if excluded(&canonical) || !seen.insert(canonical) {
        return Ok(());
    }
    let bytes = fs::read(&path).map_err(|e| Failure::new("io", e.to_string()))?;
    pack.push(Evidence {
        id: format!("E{}", pack.len() + 1),
        path: Some(path),
        text: cap(&String::from_utf8_lossy(&bytes)),
    });
    Ok(())
}
pub fn evidence_for(store: &Path, repo: &Path, case: &Case) -> Res<Vec<Evidence>> {
    let mut pack = Vec::new();
    let mut seen = HashSet::new();
    let Some(unit) = case.unit() else {
        return Ok(pack);
    };
    base::require_slug("unit", unit)?;
    if let Some(verdict) = orch::read_verdict(store, unit)? {
        let paths = Regex::new(r#"[^\s`'\"<>|,;()\[\]{}]+"#).unwrap();
        for finding in &verdict.findings {
            if matches!(case,Case::NeedsCheck{finding:n,..} if *n != finding.number) {
                continue;
            }
            for m in paths.find_iter(&finding.evidence) {
                let name = m.as_str().trim_end_matches(|c: char| c == ':' || c == '.');
                if name.is_empty() {
                    continue;
                }
                let path = Path::new(name);
                if path.is_absolute() {
                    add_file(&mut pack, &mut seen, path.to_path_buf())?;
                } else {
                    add_file(&mut pack, &mut seen, repo.join(path))?;
                    add_file(&mut pack, &mut seen, store.join(path))?;
                }
            }
        }
    }
    add_file(
        &mut pack,
        &mut seen,
        store.join("reports").join(format!("{unit}.verify.txt")),
    )?;
    for event in events::read_events(store, 0)?
        .into_iter()
        .filter(|e| e.unit.as_deref() == Some(unit))
    {
        let text = serde_json::to_string(&event)
            .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
        pack.push(Evidence {
            id: format!("E{}", pack.len() + 1),
            path: None,
            text: cap(&text),
        });
    }
    Ok(pack)
}
pub fn apply(store: &Path, case: &Case, ruling: &Ruling) -> Res<()> {
    let Case::FixRounds { unit } = case else {
        return Ok(());
    };
    if ruling.decision != "waive" {
        return Ok(());
    }
    let units = orch::read_units(store)?;
    let original = units
        .iter()
        .find(|u| &u.id == unit)
        .ok_or_else(|| Failure::new("not_found", format!("unit {unit} not found")))?;
    let standing = orch::standing_unit(&units, original)
        .ok_or_else(|| Failure::new("not_found", "standing unit not found"))?;
    let rows = orch::evidence(store, &standing.id)?;
    let review = rows
        .iter()
        .rfind(|r| r.kind == "review")
        .ok_or_else(|| Failure::new("invalid_argument", "no review to waive"))?;
    let verdict = orch::read_verdict(store, &standing.id)?
        .ok_or_else(|| Failure::new("invalid_argument", "no verdict file to waive"))?;
    let waived: HashSet<_> = orch::waivers(&rows).into_iter().map(|w| w.number).collect();
    let remaining: Vec<_> = verdict
        .findings
        .iter()
        .filter(|f| f.fails() && !waived.contains(&f.number))
        .collect();
    for (i, f) in remaining.iter().enumerate() {
        orch::append_evidence(
            store,
            &standing.id,
            &orch::waive_row(
                f.number,
                &format!("adjudicator {}: {}", ruling.id, ruling.reason),
                &review.commit,
                i + 1 == remaining.len(),
            ),
        )?;
    }
    orch::evidence::update_unit_state(store, &standing.id)?;
    Ok(())
}
pub fn overturn(store: &Path, ruling_id: &str, reason: &str) -> Res<Event> {
    if !events::read_events(store, 0)?
        .iter()
        .any(|e| e.kind == kinds::ADJUDICATION && e.data["id"] == ruling_id)
    {
        return Err(Failure::new(
            "not_found",
            format!("adjudication {ruling_id} not found"),
        ));
    }
    let event = events::append_event(
        store,
        kinds::ADJUDICATION_OVERTURNED,
        None,
        None,
        json!({"id":ruling_id,"reason":reason,"on_behalf_of_person":false}),
    )?;
    line(
        store,
        &format!("- [person] overturned {ruling_id}: {reason}"),
    )?;
    Ok(event)
}
