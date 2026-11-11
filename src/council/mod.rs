//! Advanced research: an ordered planning dialogue, one independent review, then replies. The
//! engine settles what they agree on mechanically. The switch is explicit: a session gets the
//! bridge's `council` tool only when `--advanced-research` launched it.
pub mod budgets;
pub use budgets::Budgets;
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
pub const NOTICE: &str = "advanced research is on: ";
pub(crate) const MEMBERS: [&str; 3] = ["astra", "fable", "argon"];

/// A later Fable turn costs about $0.40 (30k in, 4k out at $8 and $40 per million). Below this
/// margin it would be killed part way and buy nothing.
const NEXT_TURN_FLOOR_USD: f64 = 0.50;
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
    #[serde(default)]
    pub phase: String,
    pub n: u64,
    pub positions: Vec<Position>,
    #[serde(default)]
    pub turns: Vec<Turn>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Turn {
    pub member: String,
    pub status: Status,
    pub detail: Option<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Phase {
    pub phase: String,
    pub agreement: bool,
    pub reason: String,
    pub open: Vec<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ClaimTurn {
    pub phase: String,
    pub round: u64,
    pub member: String,
    pub stance: Stance,
    pub evidence: String,
    pub decision: Option<String>,
    pub change: Option<String>,
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
    pub phase: String,
    #[serde(default)]
    pub round: u64,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub history: Vec<ClaimTurn>,
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
    #[serde(default)]
    pub phases: Vec<Phase>,
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
    pub rounds: u64,
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
            rounds: 3,
            round1_timeout: Duration::from_secs(1800),
            round2_timeout: Duration::from_secs(900),
        }
    }
    /// Present only in a session launched with the switch; there is no other way on.
    pub fn from_env() -> Res<Option<Self>> {
        if std::env::var(ENV).as_deref() != Ok("1") {
            return Ok(None);
        }
        let budgets = Budgets::from_env()?;
        let mut setup = Self::new(
            base::resolve_command("AXIOM_CLAUDE_CMD", "claude"),
            base::resolve_command("AXIOM_AGY_CMD", "agy"),
        );
        setup.fable_cap_usd = budgets.fable_cap_usd;
        setup.astra_token_cap = budgets.astra_token_cap;
        setup.argon_token_cap = budgets.argon_token_cap;
        setup.rounds = budgets.rounds;
        Ok(Some(setup))
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

/// One agy call outside a council: `axiom review` asks Argon for a verdict on a diff. The
/// invocation and output parsing are the council's own, so the two cannot drift apart.
pub fn ask_agy(
    command: &Res<Vec<String>>,
    prompt: &str,
    cwd: &Path,
    schema: &Value,
    schema_path: &Path,
    timeout: Duration,
    stop: &AtomicBool,
) -> CrossReply {
    let answer = ask_argon(
        command,
        Ask {
            prompt: prompt.to_owned(),
            cwd,
            schema,
            timeout,
            stop,
        },
        schema_path,
    );
    CrossReply {
        status: answer.status,
        output: answer.output,
        tokens: answer.tokens,
        model: Some(ARGON_MODEL.to_owned()),
        detail: answer.detail,
    }
}
/// The longest prompt `ask_agy` accepts, in UTF-16 units; it travels on the command line.
pub const AGY_PROMPT_LIMIT: usize = member::ARGV_LIMIT_UTF16;

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
fn turn_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"required":["stances"],"properties":{
        "approved":{"type":"boolean"},
        "stances":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["claim","stance","evidence"],"properties":{
            "claim":{"type":"string"},"stance":{"type":"string","enum":["agree","disagree","concede"]},"evidence":{"type":"string"}}}},
        "claims":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["claim","evidence"],"properties":{"claim":{"type":"string"},"evidence":{"type":"string"}}}},
        "resolutions":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["claim","decision","reason","change"],"properties":{
            "claim":{"type":"string"},"decision":{"type":"string","enum":["accepted","rejected"]},"reason":{"type":"string"},"change":{"type":"string"}}}}
    }})
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

struct Seat {
    name: &'static str,
    model: String,
    status: Status,
    tokens: u64,
    usd: Option<f64>,
    detail: Option<String>,
}
impl Seat {
    fn new(name: &'static str, model: &str) -> Self {
        Self {
            name,
            model: model.into(),
            status: Status::Ok,
            tokens: 0,
            usd: None,
            detail: None,
        }
    }
    fn cut(&mut self, reason: impl Into<String>) {
        self.status = Status::BudgetCut;
        self.detail = Some(reason.into());
    }
}
fn write_position(
    dir: &Path,
    member: &str,
    phase: &str,
    round: u64,
    value: &Value,
) -> Res<Position> {
    let bytes = serde_json::to_vec_pretty(value).map_err(io)?;
    let path = dir.join(format!("{phase}-r{round}-{member}.json"));
    base::atomic_write(&path, &bytes).map_err(io)?;
    Ok(Position {
        member: member.into(),
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
/// Astra's existing plan is the first turn. No other member is paid without that plan.
pub fn run(
    setup: &Setup,
    request: &Request,
    astra: &(dyn Fn(&CrossRequest) -> CrossReply + Sync),
    stop: &AtomicBool,
) -> Res<Value> {
    Budgets {
        fable_cap_usd: setup.fable_cap_usd,
        astra_token_cap: setup.astra_token_cap,
        argon_token_cap: setup.argon_token_cap,
        rounds: setup.rounds,
    }
    .validate()?;
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
    let schema2 = turn_schema();
    let schema2_path = work.join("turn.schema.json");
    base::atomic_write(&schema2_path, schema2.to_string().as_bytes()).map_err(io)?;
    let mut seats = [
        Seat::new("astra", "astra"),
        Seat::new("fable", FABLE_MODEL),
        Seat::new("argon", ARGON_MODEL),
    ];
    let mut claims = Vec::new();
    let mut dropped = 0;
    intake(&mut claims, "astra", "A", 1, &plan, &mut dropped);
    let mut rounds = Vec::new();
    let mut phases = Vec::new();
    let mut cut = Vec::new();
    let mut objections = Vec::new();
    for phase in ["A", "B", "C"] {
        if phase == "C" && objections.is_empty() {
            break;
        }
        let mut agreement = false;
        let mut reason = "round cap reached".to_owned();
        let limit = if phase == "B" { 1 } else { setup.rounds };
        for n in 1..=limit {
            let mut round = Round {
                phase: phase.into(),
                n,
                positions: vec![],
                turns: vec![],
            };
            let speakers: &[usize] = if phase == "B" { &[2] } else { &[0, 1] };
            let mut failed = false;
            let mut approved = false;
            for &index in speakers {
                let seat = &mut seats[index];
                // The initial plan is retained under the same phase/round naming as every turn.
                if phase == "A" && n == 1 && index == 0 {
                    round
                        .positions
                        .push(write_position(&work, seat.name, phase, n, &plan)?);
                    round.turns.push(Turn {
                        member: seat.name.into(),
                        status: Status::Ok,
                        detail: None,
                    });
                    continue;
                }
                if seat.status != Status::Ok {
                    reason = format!("{}: {}", seat.name, seat.status.word());
                    failed = true;
                    break;
                }
                if index == 0 && seat.tokens >= setup.astra_token_cap {
                    seat.cut("token budget spent");
                }
                if index == 1
                    && (meter.exceeded()
                        || (meter.tokens() > 0
                            && (!meter.observed() || meter.remaining_usd() < NEXT_TURN_FLOOR_USD)))
                {
                    seat.cut(format!(
                        "${:.2} left of the ${:.2} cap",
                        meter.remaining_usd().max(0.0),
                        meter.cap_usd()
                    ));
                }
                // A prior unmetered turn also closes the phase, even when its token count is zero.
                if index == 1
                    && rounds.iter().any(|r: &Round| {
                        r.turns
                            .iter()
                            .any(|t| t.member == "fable" && t.status == Status::Ok)
                    })
                    && !meter.observed()
                {
                    seat.cut("no usage was reported, so its cost cannot be bounded");
                }
                let mut schema_for = schema2.clone();
                if index == 1 && phase == "A" {
                    schema_for["required"] = json!(["stances", "approved"]);
                }
                let prompt = format!(
                    "{}\nPlan phase agreement: {}. Open plan disputes: {}.",
                    cross_prompt(
                        &request.brief,
                        &claims,
                        seat.name,
                        &key,
                        &schema_for,
                        phase,
                        n,
                        &objections
                    ),
                    phases.first().map(|p: &Phase| p.agreement).unwrap_or(false),
                    phase_a_open(&claims).join(", ")
                );
                let timeout = if phase == "A" && n == 1 {
                    setup.round1_timeout
                } else {
                    setup.round2_timeout
                };
                let mut answer = if seat.status == Status::BudgetCut {
                    Answer {
                        status: Status::BudgetCut,
                        output: None,
                        tokens: 0,
                        detail: seat.detail.clone(),
                    }
                } else if index == 0 {
                    let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        astra(&CrossRequest {
                            prompt,
                            schema: schema_for.clone(),
                            timeout_s: timeout.as_secs().max(10),
                            token_cap: setup.astra_token_cap.saturating_sub(seat.tokens),
                        })
                    }))
                    .unwrap_or_else(|_| CrossReply {
                        status: Status::Failed,
                        output: None,
                        tokens: 0,
                        model: None,
                        detail: Some("member panicked".into()),
                    });
                    if let Some(model) = reply.model {
                        seat.model = model;
                    }
                    Answer {
                        status: reply.status,
                        output: reply.output,
                        tokens: reply.tokens,
                        detail: reply.detail,
                    }
                } else {
                    let ask = Ask {
                        prompt,
                        cwd: &request.cwd,
                        schema: &schema_for,
                        timeout,
                        stop,
                    };
                    if index == 1 {
                        ask_fable(
                            &setup.fable,
                            ask,
                            if phase == "A" && n == 1 {
                                ROUND1_TOOLS
                            } else {
                                ""
                            },
                            &mut meter,
                        )
                    } else {
                        ask_argon(&setup.argon, ask, &schema2_path)
                    }
                };
                if index == 1
                    && answer.status == Status::Ok
                    && (meter.exceeded() || !meter.observed())
                {
                    answer.status = Status::BudgetCut;
                    answer.output = None;
                    answer.detail = Some(if !meter.observed() {
                        "no usage was reported, so its cost cannot be bounded".into()
                    } else {
                        "Fable budget spent".into()
                    });
                }
                seat.tokens = seat.tokens.saturating_add(answer.tokens);
                let cap = if index == 0 {
                    setup.astra_token_cap
                } else {
                    setup.argon_token_cap
                };
                if index != 1 && seat.tokens >= cap && answer.status == Status::Ok {
                    answer.status = Status::BudgetCut;
                    answer.output = None;
                    answer.detail = Some(format!("{} tokens reached the {} cap", seat.tokens, cap));
                }
                if answer.status == Status::Ok {
                    if let Some(value) = &answer.output {
                        let complete = if index == 1 || index == 2 {
                            claims.iter().filter(|c| c.author != seat.name).all(|c| {
                                value["stances"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .any(|v| v["claim"] == c.id)
                            })
                        } else {
                            true
                        };
                        let valid = base::schema::validate(&schema_for, value).and_then(|_| {
                            if complete {
                                Ok(())
                            } else {
                                Err("missing per-claim stances".into())
                            }
                        });
                        if let Err(e) = valid {
                            answer.status = Status::Malformed;
                            answer.output = None;
                            answer.detail = Some(e);
                        }
                    } else {
                        answer.status = Status::Malformed;
                        answer.detail = Some("no position was returned".into());
                    }
                }
                seat.status = answer.status;
                seat.detail = answer.detail;
                round.turns.push(Turn {
                    member: seat.name.into(),
                    status: seat.status,
                    detail: seat.detail.clone(),
                });
                if seat.status == Status::BudgetCut {
                    cut.push(format!("{}:{phase}:round{n}", seat.name));
                }
                if seat.status != Status::Ok {
                    reason = format!(
                        "{}: {}{}",
                        seat.name,
                        seat.status.word(),
                        seat.detail
                            .as_ref()
                            .map(|d| format!(" ({d})"))
                            .unwrap_or_default()
                    );
                    failed = true;
                    break;
                }
                if let Some(value) = answer.output {
                    round
                        .positions
                        .push(write_position(&work, seat.name, phase, n, &value)?);
                    apply_stances(&mut claims, seat.name, phase, n, &value);
                    let before = claims.len();
                    intake(&mut claims, seat.name, phase, n, &value, &mut dropped);
                    if index == 1 {
                        approved = value["approved"] == true;
                    }
                    if index == 2 {
                        objections = claims
                            .iter()
                            .filter(|c| {
                                matches!(
                                    c.stances.get("argon"),
                                    Some(Stance::Disagree | Stance::Unsupported)
                                )
                            })
                            .map(|c| c.id.clone())
                            .chain(claims[before..].iter().map(|c| c.id.clone()))
                            .collect();
                    }
                }
            }
            if !round.turns.is_empty() {
                rounds.push(round);
            }
            if failed {
                break;
            }
            agreement = if phase == "B" {
                objections.is_empty()
            } else if phase == "A" {
                approved && phase_a_open(&claims).is_empty()
            } else {
                phase_c_open(&claims, &objections).is_empty()
            };
            if agreement {
                reason = if phase == "B" {
                    "no objections".into()
                } else {
                    "agreement reached".into()
                };
                break;
            }
        }
        let open = if phase == "A" {
            phase_a_open(&claims)
        } else if phase == "B" {
            objections.clone()
        } else {
            phase_c_open(&claims, &objections)
        };
        phases.push(Phase {
            phase: phase.into(),
            agreement,
            reason,
            open,
        });
    }
    for claim in &mut claims {
        converge::settle(claim);
    }
    seats[1].tokens = meter.tokens();
    seats[1].usd = Some(meter.spent_usd());
    let degraded = seats.iter().any(|s| s.status != Status::Ok);
    let council = Council {
        key,
        question: request.brief.clone(),
        members: seats
            .into_iter()
            .map(|s| Member {
                name: s.name.into(),
                family: family(&s.model).into(),
                model: s.model,
                status: s.status,
                tokens: s.tokens,
                usd: s.usd,
                detail: s.detail,
            })
            .collect(),
        rounds,
        phases,
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

fn affirmative(s: Option<&Stance>) -> bool {
    matches!(s, Some(Stance::Author | Stance::Agree | Stance::Concede))
}
fn phase_a_open(claims: &[Claim]) -> Vec<String> {
    claims
        .iter()
        .filter(|c| {
            c.phase == "A"
                && (!affirmative(c.stances.get("astra")) || !affirmative(c.stances.get("fable")))
        })
        .map(|c| c.id.clone())
        .collect()
}
fn phase_c_open(claims: &[Claim], objections: &[String]) -> Vec<String> {
    objections
        .iter()
        .filter(|id| {
            let Some(claim) = claims.iter().find(|c| &c.id == *id) else {
                return true;
            };
            let resolved = |member| {
                claim
                    .history
                    .iter()
                    .rev()
                    .find(|h| h.phase == "C" && h.member == member && h.decision.is_some())
            };
            match (resolved("astra"), resolved("fable")) {
                (Some(a), Some(f)) => {
                    a.decision != f.decision
                        || (a.decision.as_deref() == Some("accepted") && a.change != f.change)
                }
                _ => true,
            }
        })
        .cloned()
        .collect()
}
fn intake(
    claims: &mut Vec<Claim>,
    member: &str,
    phase: &str,
    n: u64,
    value: &Value,
    dropped: &mut usize,
) {
    let mut pending = claims_of(value);
    // IDs carry no model identity; a neutral keyed order is used within each intake.
    pending.sort_by_key(|(text, _)| sha256(text.as_bytes()));
    for (text, evidence) in pending {
        let text = converge::cut(&text, CLAIM_TEXT_CHARS);
        let evidence = converge::cut(&evidence, CLAIM_EVIDENCE_CHARS);
        if claims
            .iter()
            .any(|c| c.author == member && c.text == text && c.evidence == evidence)
        {
            continue;
        }
        if claims.iter().filter(|c| c.author == member).count() >= CLAIMS_PER_MEMBER {
            *dropped += 1;
            continue;
        }
        claims.push(Claim {
            id: format!("C{:02}", claims.len() + 1),
            text,
            evidence,
            author: member.into(),
            phase: phase.into(),
            round: n,
            stances: MEMBERS
                .iter()
                .map(|m| {
                    (
                        (*m).into(),
                        if *m == member {
                            Stance::Author
                        } else {
                            Stance::Absent
                        },
                    )
                })
                .collect(),
            history: vec![ClaimTurn {
                phase: phase.into(),
                round: n,
                member: member.into(),
                stance: Stance::Author,
                evidence: String::new(),
                decision: None,
                change: None,
            }],
            dissent: vec![],
            status: ClaimStatus::Disputed,
            ruling: None,
        });
    }
}
fn cross_prompt(
    brief: &str,
    claims: &[Claim],
    reader: &str,
    key: &str,
    schema: &Value,
    phase: &str,
    n: u64,
    objections: &[String],
) -> String {
    let mut others: Vec<&str> = MEMBERS.iter().copied().filter(|m| *m != reader).collect();
    others.sort_by_key(|m| sha256(format!("{key}{reader}{m}").as_bytes()));
    let label = |author: &str| {
        if author == reader {
            "YOU"
        } else if others.first().copied() == Some(author) {
            "P"
        } else {
            "Q"
        }
    };
    let mut listing = String::new();
    for claim in claims {
        listing.push_str(&format!(
            "{} [Position {}]\n  claim: {}\n  evidence: {}\n",
            claim.id,
            label(&claim.author),
            anonymised(&claim.text),
            anonymised(&claim.evidence)
        ));
        for member in MEMBERS {
            if let Some(h) = claim.history.iter().rev().find(|h| h.member == member) {
                listing.push_str(&format!(
                    "  [Position {}] phase {} round {}: {:?}; {} {} {}\n",
                    label(member),
                    h.phase,
                    h.round,
                    h.stance,
                    anonymised(&h.evidence),
                    h.decision.as_deref().unwrap_or(""),
                    anonymised(h.change.as_deref().unwrap_or(""))
                ));
            }
        }
    }
    let instruction=match phase {
        "B"=>"You are the sole independent reviewer. Review the final plan or both final positions and their open disputes. Agree or object per claim, with checkable evidence. You may add claims. This is your only review turn.",
        "C"=>"Settle every review objection below. For each resolution give accepted with a concrete change, or rejected with a reason. Both participants must settle each objection. The original review stays in the record for the core's ruling.",
        _=>"Agree or rebut per claim with evidence. Revise or defend the plan against the earlier replies. State approved=true only if you approve the whole plan and every rebuttal is closed. To revise one of your own claims, supply an accepted resolution with its complete revised text in change and the reason; otherwise defend it in stance evidence.",
    };
    format!("COUNCIL PHASE {phase}\nCOUNCIL ROUND {n}\nCROSS-EXAMINATION\nBrief: {}\n{instruction}\nThe turns below are in order; your partner sees your latest reply. YOUR CLAIMS and the other positions are shown under neutral labels. EARLIER REPLIES include the latest stance of each participant. Open review objections: {}. Work read-only; change no file. Answer one JSON object matching this schema: {schema}\nCLAIMS\n{listing}",anonymised(&converge::cut(brief,BRIEF_ECHO_CHARS)),objections.join(", "))
}
fn apply_stances(claims: &mut [Claim], member: &str, phase: &str, n: u64, answer: &Value) {
    for item in answer["stances"].as_array().into_iter().flatten() {
        let Some(claim) = claims
            .iter_mut()
            .find(|c| c.id == item["claim"].as_str().unwrap_or(""))
        else {
            continue;
        };
        let evidence = item["evidence"].as_str().unwrap_or("");
        if let Some(reply_stance) =
            converge::stance_of(item["stance"].as_str().unwrap_or(""), evidence)
        {
            let stance = if claim.author == member {
                Stance::Author
            } else {
                reply_stance
            };
            claim.stances.insert(member.into(), stance);
            claim.dissent.retain(|d| d.member != member);
            if stance == Stance::Disagree {
                claim.dissent.push(Dissent {
                    member: member.into(),
                    evidence: converge::cut(evidence, CLAIM_EVIDENCE_CHARS),
                });
            }
            claim.history.push(ClaimTurn {
                phase: phase.into(),
                round: n,
                member: member.into(),
                stance,
                evidence: converge::cut(evidence, CLAIM_EVIDENCE_CHARS),
                decision: None,
                change: None,
            });
        }
    }
    if phase == "B" {
        return;
    }
    for item in answer["resolutions"].as_array().into_iter().flatten() {
        let Some(claim) = claims
            .iter_mut()
            .find(|c| c.id == item["claim"].as_str().unwrap_or(""))
        else {
            continue;
        };
        let decision = item["decision"].as_str().unwrap_or("");
        let reason = item["reason"].as_str().unwrap_or("").trim();
        let change = item["change"].as_str().unwrap_or("").trim();
        if reason.is_empty()
            || (decision == "accepted" && change.is_empty())
            || !["accepted", "rejected"].contains(&decision)
        {
            continue;
        }
        if phase == "A" {
            // An author revises its own claim. The other participant must answer the
            // new text; approval of the previous text cannot carry across a revision.
            if claim.author != member || decision != "accepted" {
                continue;
            }
            let revised = converge::cut(change, CLAIM_TEXT_CHARS);
            if claim.text != revised {
                claim.text = revised;
                claim.evidence = converge::cut(reason, CLAIM_EVIDENCE_CHARS);
                for other in MEMBERS.into_iter().filter(|m| *m != member) {
                    claim.stances.insert(other.into(), Stance::Absent);
                }
                claim.dissent.clear();
            }
        }
        // Rejecting a new review claim disputes that claim; rejecting an objection
        // to a plan claim defends the plan. The reviewer's own stance never changes.
        if claim.author != member {
            let stance = if decision == "accepted" {
                Stance::Concede
            } else if claim.author == "argon" {
                Stance::Disagree
            } else {
                Stance::Agree
            };
            claim.stances.insert(member.into(), stance);
            claim.dissent.retain(|d| d.member != member);
            if stance == Stance::Disagree {
                claim.dissent.push(Dissent {
                    member: member.into(),
                    evidence: converge::cut(reason, CLAIM_EVIDENCE_CHARS),
                });
            }
        }
        claim.history.push(ClaimTurn {
            phase: phase.into(),
            round: n,
            member: member.into(),
            stance: *claim.stances.get(member).unwrap_or(&Stance::Absent),
            evidence: converge::cut(reason, CLAIM_EVIDENCE_CHARS),
            decision: Some(decision.into()),
            change: Some(converge::cut(change, CLAIM_TEXT_CHARS)),
        });
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
    let (next, total) = converge::unruled_batch(&council);
    let ids: Vec<&str> = next.iter().filter_map(|e| e["id"].as_str()).collect();
    Ok(json!({
        "status": "completed",
        "unruled": ids,
        "unruled_total": total,
        "more_open": total > next.len(),
        "next": next,
    }))
}

/// The one-line state for a report or the app; None when no council ran.
pub fn stage_note_at(dir: &Path) -> Option<String> {
    Some(stage_note(&load_council(&dir.join("council.json"))?))
}
