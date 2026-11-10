//! Advanced research: Astra, Fable and Argon reason about one brief in two rounds, and the
//! engine settles what they agree on mechanically. The switch is explicit: a session gets the
//! bridge's `council` tool only when `--advanced-research` launched it.
mod converge;
mod member;
pub use converge::{digest, stage_note, DIGEST_MAX_CHARS};

use crate::base::{self, Failure, Res};
use member::{ask_argon, ask_fable, Answer, Ask, Meter};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, OnceLock},
    time::Duration,
};

pub const ENV: &str = "AXIOM_ADVANCED_RESEARCH";
pub const FABLE_MODEL: &str = "claude-fable-5-2";
pub const ARGON_MODEL: &str = "gemini-4-argon-high";
pub const DEFAULT_FABLE_CAP_USD: f64 = 5.0;
pub const TOOL: &str = "mcp__plugin_chronos_bridge__council";
pub const SYSTEM_PROMPT: &str = "Advanced research is on for this session: run the council bullet of the wish or research skill with the bridge `council` tool.";
pub const NOTICE: &str = "advanced research is on: Astra, Fable 5.2 and Argon reason about this together; Fable is capped at $5.00";
pub(crate) const MEMBERS: [&str; 3] = ["astra", "fable", "argon"];

/// Fable's round 2 costs about $0.40 (30k in, 4k out at $8 and $40 per million). Below this
/// margin it would be killed part way and buy nothing.
const ROUND2_FLOOR_USD: f64 = 0.50;
const CLAIMS_PER_MEMBER: usize = 10;
const CLAIM_TEXT_CHARS: usize = 400;
const CLAIM_EVIDENCE_CHARS: usize = 300;
const BRIEF_ECHO_CHARS: usize = 2_000;
const ROUND1_TOOLS: &str = "Read,Glob,Grep";

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    Missing,
    Quota,
    Timeout,
    BudgetCut,
    Malformed,
    Failed,
}
impl Status {
    pub fn word(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Missing => "missing",
            Status::Quota => "out of quota",
            Status::Timeout => "timed out",
            Status::BudgetCut => "cut by its cap",
            Status::Malformed => "malformed output",
            Status::Failed => "failed",
        }
    }
}
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Stance {
    Author,
    Agree,
    Concede,
    Disagree,
    /// Disagreed without checkable evidence: recorded, but it counts for nothing.
    Unsupported,
    Absent,
}
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ClaimStatus {
    Consensus,
    Majority,
    Disputed,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Member {
    pub name: String,
    pub model: String,
    pub family: String,
    pub status: Status,
    pub tokens: u64,
    pub usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Position {
    pub member: String,
    pub path: PathBuf,
    pub sha256: String,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Round {
    pub n: u32,
    pub positions: Vec<Position>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Dissent {
    pub member: String,
    pub evidence: String,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Ruling {
    pub decision: String,
    pub reason: String,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Claim {
    pub id: String,
    pub text: String,
    pub evidence: String,
    pub stances: BTreeMap<String, Stance>,
    #[serde(default)]
    pub dissent: Vec<Dissent>,
    pub status: ClaimStatus,
    pub ruling: Option<Ruling>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Cost {
    pub usd: f64,
    pub cap_usd: f64,
    #[serde(default)]
    pub unpriced_tokens: u64,
    pub cut: Vec<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Council {
    pub key: String,
    pub question: String,
    pub members: Vec<Member>,
    pub rounds: Vec<Round>,
    pub claims: Vec<Claim>,
    pub cost: Cost,
    #[serde(default)]
    pub dropped: usize,
    pub degraded: bool,
}

pub struct Setup {
    pub fable: Res<Vec<String>>,
    pub argon: Res<Vec<String>>,
    pub fable_cap_usd: f64,
    pub astra_token_cap: u64,
    pub argon_token_cap: u64,
    pub round1_timeout: Duration,
    pub round2_timeout: Duration,
}
impl Setup {
    pub fn new(fable: Res<Vec<String>>, argon: Res<Vec<String>>) -> Self {
        Self {
            fable,
            argon,
            fable_cap_usd: DEFAULT_FABLE_CAP_USD,
            astra_token_cap: 200_000,
            argon_token_cap: 400_000,
            round1_timeout: Duration::from_secs(1800),
            round2_timeout: Duration::from_secs(900),
        }
    }
    /// Present only in a session launched with the switch; there is no other way on.
    pub fn from_env() -> Option<Self> {
        (std::env::var(ENV).as_deref() == Ok("1")).then(|| {
            Self::new(
                base::resolve_command("AXIOM_CLAUDE_CMD", "claude"),
                base::resolve_command("AXIOM_AGY_CMD", "agy"),
            )
        })
    }
}

pub struct Request {
    pub brief: String,
    pub cwd: PathBuf,
    pub plan_path: PathBuf,
    pub schema_path: PathBuf,
}
pub struct CrossRequest {
    pub prompt: String,
    pub schema: Value,
    pub timeout_s: u64,
    pub token_cap: u64,
}
pub struct CrossReply {
    pub status: Status,
    pub output: Option<Value>,
    pub tokens: u64,
    pub model: Option<String>,
    pub detail: Option<String>,
}

fn io(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn family(model: &str) -> &'static str {
    match base::family_of(model) {
        base::Family::OpenAi => "openai",
        base::Family::Google => "google",
        base::Family::Anthropic => "anthropic",
        base::Family::Unknown => "unknown",
    }
}
fn round2_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["stances"],
        "properties": {"stances": {"type": "array", "items": {
            "type": "object",
            "additionalProperties": false,
            "required": ["claim", "stance", "evidence"],
            "properties": {
                "claim": {"type": "string"},
                "stance": {"type": "string", "enum": ["agree", "disagree", "concede"]},
                "evidence": {"type": "string"}
            }
        }}}
    })
}

/// The file the switch leaves behind so a resumed session still counts as opted in. Nothing
/// in a wish's words can create it.
fn switch_file(home: &Path, session: &str) -> Option<PathBuf> {
    base::require_slug("session", session).ok()?;
    Some(home.join("council-switch").join(session))
}
pub fn remember_switch(home: &Path, session: &str) {
    if let Some(path) = switch_file(home, session) {
        let _ = path
            .parent()
            .map(std::fs::create_dir_all)
            .transpose()
            .and_then(|_| base::atomic_write(&path, b"on"));
    }
}
pub fn switch_remembered(home: &Path, session: &str) -> bool {
    switch_file(home, session).is_some_and(|p| p.is_file())
}

fn claims_of(position: &Value) -> Vec<(String, String)> {
    let field = |v: &Value, k: &str| v[k].as_str().unwrap_or("").to_owned();
    if let Some(changes) = position["changes"].as_array() {
        return changes
            .iter()
            .map(|c| {
                (
                    format!(
                        "{}: {} (because: {})",
                        field(c, "location"),
                        field(c, "change"),
                        field(c, "problem")
                    ),
                    field(c, "evidence"),
                )
            })
            .collect();
    }
    position["claims"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| (field(c, "claim"), field(c, "evidence")))
        .collect()
}

/// Member names cannot reach a reader through the text of another member's claim.
fn anonymised(text: &str) -> String {
    static NAMES: OnceLock<regex::Regex> = OnceLock::new();
    NAMES
        .get_or_init(|| {
            regex::Regex::new(
                r"(?i)(astra|fable|argon|gemini|claude|anthropic|codex|openai|gpt-?\d*)",
            )
            .unwrap()
        })
        .replace_all(text, "[member]")
        .into_owned()
}

fn round1_prompt(brief: &str, schema: &Value) -> String {
    format!(
        "{brief}\n\nYou work alone and read-only: change no file and run nothing that writes. Answer with one JSON object matching this schema and nothing else:\n{schema}"
    )
}

struct Seat {
    name: &'static str,
    model: String,
    status: Status,
    tokens: u64,
    usd: Option<f64>,
    detail: Option<String>,
    position: Option<Value>,
    round2: bool,
}
impl Seat {
    fn seated(name: &'static str, model: &str, answer: Answer) -> Self {
        Self {
            name,
            model: model.to_owned(),
            status: answer.status,
            tokens: answer.tokens,
            usd: None,
            detail: answer.detail,
            round2: answer.output.is_some(),
            position: answer.output,
        }
    }
    fn absorb(&mut self, answer: Answer) -> Option<Value> {
        self.tokens += answer.tokens;
        if self.status == Status::Ok && answer.status != Status::Ok {
            self.status = answer.status;
            self.detail = answer.detail;
        }
        answer.output
    }
    fn cut_round2(&mut self, detail: String) {
        self.status = Status::BudgetCut;
        self.detail = Some(detail);
        self.round2 = false;
    }
}

fn write_position(dir: &Path, member: &str, round: u32, value: &Value) -> Res<Position> {
    let bytes = serde_json::to_vec_pretty(value).map_err(io)?;
    let path = dir.join(format!("r{round}-{member}.json"));
    base::atomic_write(&path, &bytes).map_err(io)?;
    Ok(Position {
        member: member.to_owned(),
        path,
        sha256: sha256(&bytes),
    })
}

fn load_council(path: &Path) -> Option<Council> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}
fn save_council(path: &Path, council: &Council) -> Res<()> {
    let bytes = serde_json::to_vec_pretty(council).map_err(io)?;
    base::atomic_write(path, &bytes).map_err(io)
}
fn finish(dir: &Path, council: &Council, reused: bool) -> Res<Value> {
    let mut result = digest(council, &dir.join("council.json"), reused);
    let digest_path = dir.join("council-digest.json");
    base::atomic_write(&digest_path, result.to_string().as_bytes()).map_err(io)?;
    result["digest_path"] = json!(digest_path);
    Ok(result)
}
fn joined(result: std::thread::Result<Answer>) -> Answer {
    result.unwrap_or_else(|_| Answer {
        status: Status::Failed,
        output: None,
        tokens: 0,
        detail: Some("member thread panicked".to_owned()),
    })
}

/// Runs both rounds and returns the bounded digest. Astra's round 1 is the plan the wish
/// already made; without it nothing runs, so no Fable money is spent on a blocked wish.
pub fn run(
    setup: &Setup,
    request: &Request,
    astra: &(dyn Fn(&CrossRequest) -> CrossReply + Sync),
    stop: &AtomicBool,
) -> Res<Value> {
    let invalid = |m: String| Failure::new("invalid_argument", m);
    if request.brief.trim().is_empty() {
        return Err(invalid("brief is empty".into()));
    }
    let schema: Value = std::fs::read(&request.schema_path)
        .map_err(|e| e.to_string())
        .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()))
        .map_err(|e| invalid(format!("{}: {e}", request.schema_path.display())))?;
    if schema.pointer("/properties/changes").is_none()
        && schema.pointer("/properties/claims").is_none()
    {
        return Err(invalid(
            "the schema has neither `changes` nor `claims` to compare".into(),
        ));
    }
    let astra_missing = |m: String| {
        Failure::new(
            "astra_missing",
            format!("{}: {m}", request.plan_path.display()),
        )
    };
    let plan_bytes = std::fs::read(&request.plan_path).map_err(|e| astra_missing(e.to_string()))?;
    let plan: Value =
        serde_json::from_slice(&plan_bytes).map_err(|e| astra_missing(e.to_string()))?;
    base::schema::validate(&schema, &plan).map_err(astra_missing)?;
    let dir = request
        .plan_path
        .parent()
        .ok_or_else(|| invalid("plan_path has no parent".into()))?
        .to_path_buf();
    let key = sha256(format!("{}\0{}", request.brief, sha256(&plan_bytes)).as_bytes());
    let council_path = dir.join("council.json");
    if let Some(existing) = load_council(&council_path).filter(|c| c.key == key) {
        return finish(&dir, &existing, true);
    }
    let work = dir.join("council");
    std::fs::create_dir_all(&work).map_err(io)?;
    let mut meter = Meter::open(
        &dir.join("council-spend.json"),
        &sha256(request.brief.as_bytes()),
        setup.fable_cap_usd,
    )?;
    let cwd = request.cwd.as_path();
    let (fable_r1, argon_r1) = {
        let prompt = round1_prompt(&request.brief, &schema);
        let (fable_prompt, argon_prompt) = (prompt.clone(), prompt);
        let timeout = setup.round1_timeout;
        let (schema, meter) = (&schema, &mut meter);
        std::thread::scope(|scope| {
            let fable = scope.spawn(move || {
                let ask = Ask {
                    prompt: fable_prompt,
                    cwd,
                    schema,
                    timeout,
                    stop,
                };
                ask_fable(&setup.fable, ask, ROUND1_TOOLS, meter)
            });
            let argon = scope.spawn(move || {
                let ask = Ask {
                    prompt: argon_prompt,
                    cwd,
                    schema,
                    timeout,
                    stop,
                };
                ask_argon(&setup.argon, ask, &request.schema_path)
            });
            (joined(fable.join()), joined(argon.join()))
        })
    };
    let mut cut: Vec<String> = vec![];
    let mut astra_seat = Seat {
        name: "astra",
        model: "astra".to_owned(),
        status: Status::Ok,
        tokens: 0,
        usd: None,
        detail: None,
        position: Some(plan),
        round2: true,
    };
    let mut fable_seat = Seat::seated("fable", FABLE_MODEL, fable_r1);
    let mut argon_seat = Seat::seated("argon", ARGON_MODEL, argon_r1);
    if fable_seat.status == Status::BudgetCut {
        cut.push("fable:round1".to_owned());
    }
    if argon_seat.status == Status::Ok && argon_seat.tokens >= setup.argon_token_cap {
        // `agy --output-format json` reports only at the end, so this cap can stop round 2
        // but not round 1.
        argon_seat.cut_round2(format!(
            "{} tokens reached the {} cap",
            argon_seat.tokens, setup.argon_token_cap
        ));
        cut.push("argon:round2".to_owned());
    }
    if fable_seat.status == Status::Ok {
        let reason = if !meter.observed() {
            Some("no usage was reported, so its cost cannot be bounded".to_owned())
        } else if meter.exceeded() || meter.remaining_usd() < ROUND2_FLOOR_USD {
            Some(format!(
                "${:.2} left of the ${:.2} cap",
                meter.remaining_usd().max(0.0),
                meter.cap_usd()
            ))
        } else {
            None
        };
        if let Some(reason) = reason {
            fable_seat.cut_round2(reason);
            cut.push("fable:round2".to_owned());
        }
    }
    let mut first = vec![Position {
        member: "astra".to_owned(),
        path: request.plan_path.clone(),
        sha256: sha256(&plan_bytes),
    }];
    for seat in [&fable_seat, &argon_seat] {
        if let Some(position) = &seat.position {
            first.push(write_position(&work, seat.name, 1, position)?);
        }
    }
    let sitting = first.len();
    let mut rounds = vec![Round {
        n: 1,
        positions: first,
    }];
    let mut claims = vec![];
    let mut dropped = 0;
    if sitting > 1 {
        let mut pending: Vec<(&str, usize, String, String)> = vec![];
        for seat in [&astra_seat, &fable_seat, &argon_seat] {
            let list = seat.position.as_ref().map(claims_of).unwrap_or_default();
            dropped += list.len().saturating_sub(CLAIMS_PER_MEMBER);
            for (index, (text, evidence)) in list.into_iter().take(CLAIMS_PER_MEMBER).enumerate() {
                pending.push((seat.name, index, text, evidence));
            }
        }
        // Numbered in a keyed order, so an id says nothing about who wrote the claim.
        pending.sort_by_key(|(name, index, ..)| sha256(format!("{key}{name}{index}").as_bytes()));
        for (number, (author, _, text, evidence)) in pending.into_iter().enumerate() {
            claims.push(Claim {
                id: format!("C{:02}", number + 1),
                text: converge::cut(&text, CLAIM_TEXT_CHARS),
                evidence: converge::cut(&evidence, CLAIM_EVIDENCE_CHARS),
                stances: MEMBERS
                    .iter()
                    .map(|m| {
                        let stance = if *m == author {
                            Stance::Author
                        } else {
                            Stance::Absent
                        };
                        ((*m).to_owned(), stance)
                    })
                    .collect(),
                dissent: vec![],
                status: ClaimStatus::Disputed,
                ruling: None,
            });
        }
        let schema2 = round2_schema();
        let schema2_path = work.join("round2.schema.json");
        base::atomic_write(&schema2_path, schema2.to_string().as_bytes()).map_err(io)?;
        let prompt_for =
            |reader: &str| cross_prompt(&request.brief, &claims, reader, &key, &schema2);
        let (astra_prompt, fable_prompt, argon_prompt) = (
            prompt_for("astra"),
            prompt_for("fable"),
            prompt_for("argon"),
        );
        let run_fable = fable_seat.position.is_some() && fable_seat.round2;
        let run_argon = argon_seat.position.is_some() && argon_seat.round2;
        let timeout = setup.round2_timeout;
        let (astra_r2, fable_r2, argon_r2) = {
            let (schema2, schema2_path, meter) = (&schema2, schema2_path.as_path(), &mut meter);
            std::thread::scope(|scope| {
                let astra_handle = scope.spawn(move || {
                    astra(&CrossRequest {
                        prompt: astra_prompt,
                        schema: schema2.clone(),
                        timeout_s: timeout.as_secs().max(10),
                        token_cap: setup.astra_token_cap,
                    })
                });
                let fable_handle = run_fable.then(|| {
                    scope.spawn(move || {
                        let ask = Ask {
                            prompt: fable_prompt,
                            cwd,
                            schema: schema2,
                            timeout,
                            stop,
                        };
                        ask_fable(&setup.fable, ask, "", meter)
                    })
                });
                let argon_handle = run_argon.then(|| {
                    scope.spawn(move || {
                        let ask = Ask {
                            prompt: argon_prompt,
                            cwd,
                            schema: schema2,
                            timeout,
                            stop,
                        };
                        ask_argon(&setup.argon, ask, schema2_path)
                    })
                });
                let astra_reply = astra_handle.join().unwrap_or_else(|_| CrossReply {
                    status: Status::Failed,
                    output: None,
                    tokens: 0,
                    model: None,
                    detail: Some("panicked".to_owned()),
                });
                (
                    astra_reply,
                    fable_handle.map(|h| joined(h.join())),
                    argon_handle.map(|h| joined(h.join())),
                )
            })
        };
        if let Some(model) = &astra_r2.model {
            astra_seat.model = model.clone();
        }
        let astra_answer = Answer {
            status: astra_r2.status,
            output: astra_r2.output,
            tokens: astra_r2.tokens,
            detail: astra_r2.detail,
        };
        let mut positions = vec![];
        let mut record = |seat: &mut Seat, answer: Option<Answer>| -> Res<()> {
            let Some(answer) = answer else { return Ok(()) };
            if answer.status == Status::BudgetCut {
                cut.push(format!("{}:round2", seat.name));
            }
            if let Some(stances) = seat.absorb(answer) {
                positions.push(write_position(&work, seat.name, 2, &stances)?);
                apply_stances(&mut claims, seat.name, &stances);
            }
            Ok(())
        };
        record(&mut astra_seat, Some(astra_answer))?;
        record(&mut fable_seat, fable_r2)?;
        record(&mut argon_seat, argon_r2)?;
        rounds.push(Round { n: 2, positions });
        claims.iter_mut().for_each(converge::settle);
    }
    fable_seat.tokens = meter.tokens();
    fable_seat.usd = Some(meter.spent_usd());
    let seats = [astra_seat, fable_seat, argon_seat];
    let degraded = seats.iter().any(|s| s.status != Status::Ok);
    let council = Council {
        key,
        question: request.brief.clone(),
        members: seats
            .iter()
            .map(|s| Member {
                name: s.name.to_owned(),
                family: family(&s.model).to_owned(),
                model: s.model.clone(),
                status: s.status,
                tokens: s.tokens,
                usd: s.usd,
                detail: s.detail.clone(),
            })
            .collect(),
        rounds,
        claims,
        cost: Cost {
            usd: meter.spent_usd(),
            cap_usd: meter.cap_usd(),
            unpriced_tokens: meter.unpriced_tokens(),
            cut,
        },
        dropped,
        degraded,
    };
    save_council(&council_path, &council)?;
    finish(&dir, &council, false)
}

/// Round 2 shows a member the other members' claims under neutral labels. A member's own
/// claims are left out: it has already stated its stance on them.
fn cross_prompt(brief: &str, claims: &[Claim], reader: &str, key: &str, schema: &Value) -> String {
    let mut others: Vec<&str> = MEMBERS.iter().copied().filter(|m| *m != reader).collect();
    others.sort_by_key(|m| sha256(format!("{key}{reader}{m}").as_bytes()));
    let mut listing = String::new();
    for claim in claims {
        let author = MEMBERS
            .iter()
            .copied()
            .find(|m| claim.stances.get(*m) == Some(&Stance::Author));
        if author == Some(reader) {
            continue;
        }
        let label = if author == others.first().copied() {
            "P"
        } else {
            "Q"
        };
        listing.push_str(&format!(
            "{} [Position {label}]\n  claim: {}\n  evidence: {}\n",
            claim.id,
            anonymised(&claim.text),
            anonymised(&claim.evidence)
        ));
    }
    format!(
        "CROSS-EXAMINATION\nThe brief was:\n{}\n\nYou answered this brief independently. Below are the other members' claims, anonymised. For each claim id give one stance: `agree`, `concede` (you held otherwise and now accept it), or `disagree` with concrete evidence in `evidence` (a file and line, or a fact someone can check). A disagreement without evidence does not count. Make no new proposals and use no tools. Answer with one JSON object matching this schema and nothing else:\n{schema}\n\nCLAIMS\n{listing}",
        anonymised(&converge::cut(brief, BRIEF_ECHO_CHARS))
    )
}

fn apply_stances(claims: &mut [Claim], member: &str, answer: &Value) {
    for item in answer["stances"].as_array().into_iter().flatten() {
        let id = item["claim"].as_str().unwrap_or("");
        let evidence = item["evidence"].as_str().unwrap_or("");
        let Some(claim) = claims.iter_mut().find(|c| c.id == id) else {
            continue;
        };
        if claim.stances.get(member) != Some(&Stance::Absent) {
            continue;
        }
        if let Some(stance) = converge::stance_of(item["stance"].as_str().unwrap_or(""), evidence) {
            claim.stances.insert(member.to_owned(), stance);
            if stance == Stance::Disagree {
                claim.dissent.push(Dissent {
                    member: member.to_owned(),
                    evidence: converge::cut(evidence, CLAIM_EVIDENCE_CHARS),
                });
            }
        }
    }
}

const RULINGS: [&str; 3] = ["upheld", "rejected", "narrowed"];

/// Records the core's ruling on open claims; a consensus claim takes none.
pub fn rule(plan_path: &Path, rulings: &[(String, String, String)]) -> Res<Value> {
    let invalid = |m: String| Failure::new("invalid_argument", m);
    let path = plan_path
        .parent()
        .ok_or_else(|| invalid("plan_path has no parent".into()))?
        .join("council.json");
    let mut council =
        load_council(&path).ok_or_else(|| invalid(format!("no council at {}", path.display())))?;
    for (id, decision, reason) in rulings {
        if !RULINGS.contains(&decision.as_str()) || reason.trim().is_empty() {
            return Err(invalid(format!(
                "{id}: a ruling needs a decision of upheld, rejected or narrowed and a reason"
            )));
        }
        let claim = council
            .claims
            .iter_mut()
            .find(|c| c.id == *id)
            .ok_or_else(|| invalid(format!("unknown claim {id}")))?;
        if claim.status == ClaimStatus::Consensus {
            return Err(invalid(format!("{id} is settled and takes no ruling")));
        }
        claim.ruling = Some(Ruling {
            decision: decision.clone(),
            reason: reason.clone(),
        });
    }
    save_council(&path, &council)?;
    let unruled: Vec<&str> = council
        .claims
        .iter()
        .filter(|c| c.status != ClaimStatus::Consensus && c.ruling.is_none())
        .map(|c| c.id.as_str())
        .collect();
    Ok(json!({"status":"completed","unruled":unruled}))
}

/// The one-line state for a report or the app; None when no council ran.
pub fn stage_note_at(dir: &Path) -> Option<String> {
    Some(stage_note(&load_council(&dir.join("council.json"))?))
}
