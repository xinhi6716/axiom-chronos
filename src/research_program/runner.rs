//! Persistent, resumable research program.
use super::{
    adjudication::{Adjudication, Adjudicator, Finding},
    budget::Budget,
    dossier, history,
    investigation::{self, InvestigationRequest, MemberCall, MemberReply, ResearchMember},
    program_dir,
    state::{self, ProgramState, StageEntry, StageStatus},
    Stage, PROGRAM_TYPE,
};
use crate::{
    base::{self, schema, Failure, Res},
    council::{self, Status},
    orch,
    seams::sha256_hex,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

pub trait CouncilStage: Sync {
    fn debate(
        &self,
        plan_path: &Path,
        schema_path: &Path,
        brief: &str,
        cap_usd: f64,
        stop: &AtomicBool,
    ) -> Res<Value>;
}
pub struct CouncilRunner<'a> {
    pub setup: &'a council::Setup,
    pub astra: &'a (dyn Fn(&council::CrossRequest) -> council::CrossReply + Sync),
}
impl CouncilStage for CouncilRunner<'_> {
    fn debate(
        &self,
        plan_path: &Path,
        schema_path: &Path,
        brief: &str,
        cap_usd: f64,
        stop: &AtomicBool,
    ) -> Res<Value> {
        let setup = council::Setup {
            fable: self.setup.fable.clone(),
            argon: self.setup.argon.clone(),
            fable_cap_usd: cap_usd,
            astra_token_cap: self.setup.astra_token_cap,
            argon_token_cap: self.setup.argon_token_cap,
            rounds: self.setup.rounds,
            round1_timeout: self.setup.round1_timeout,
            round2_timeout: self.setup.round2_timeout,
        };
        council::run_with_events(
            &setup,
            &council::Request {
                brief: brief.into(),
                cwd: plan_path.parent().unwrap_or(Path::new(".")).into(),
                plan_path: plan_path.into(),
                schema_path: schema_path.into(),
            },
            self.astra,
            stop,
            &|_| {},
        )
    }
}
pub struct Members<'a> {
    pub investigators: Vec<&'a dyn ResearchMember>,
    pub red_team: &'a dyn ResearchMember,
    pub pre_mortem: &'a dyn ResearchMember,
    pub synthesizer: &'a dyn ResearchMember,
}
pub struct ProgramConfig {
    pub store: PathBuf,
    pub cwd: PathBuf,
    pub question: String,
    pub sources: Vec<String>,
    pub home: PathBuf,
    pub initial_cap_usd: f64,
}
fn io(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn write_json(path: &Path, value: &impl serde::Serialize) -> Res<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(io)?;
    base::atomic_write(path, &bytes).map_err(io)
}
fn read_json(path: &Path) -> Res<Value> {
    serde_json::from_slice(&fs::read(path).map_err(io)?)
        .map_err(|e| Failure::new("json", e.to_string()))
}
// Hash paths and contents, with sorted names and length delimiters, including empty directories.
fn artifact_hash(path: &Path) -> Res<String> {
    fn walk(root: &Path, p: &Path, data: &mut Vec<u8>) -> Res<()> {
        let meta = fs::symlink_metadata(p).map_err(io)?;
        if meta.file_type().is_symlink() {
            return Err(Failure::new("io", "artifact contains symlink"));
        }
        let name = p.strip_prefix(root).map_err(io)?.to_string_lossy();
        data.extend_from_slice(&(name.len() as u64).to_be_bytes());
        data.extend_from_slice(name.as_bytes());
        if meta.is_dir() {
            data.push(b'd');
            let mut entries = fs::read_dir(p)
                .map_err(io)?
                .map(|e| e.map(|e| e.path()).map_err(io))
                .collect::<Res<Vec<_>>>()?;
            entries.sort();
            for entry in entries {
                walk(root, &entry, data)?;
            }
        } else if meta.is_file() {
            data.push(b'f');
            let bytes = fs::read(p).map_err(io)?;
            data.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
            data.extend_from_slice(&bytes);
        } else {
            return Err(Failure::new("io", "unsupported artifact"));
        }
        Ok(())
    }
    if fs::symlink_metadata(path).map_err(io)?.is_file() {
        return Ok(sha256_hex(&fs::read(path).map_err(io)?));
    }
    let mut bytes = Vec::new();
    walk(path, path, &mut bytes)?;
    Ok(sha256_hex(&bytes))
}
struct Metered<'a> {
    member: &'a dyn ResearchMember,
    budget: &'a Mutex<Budget>,
    failure: &'a Mutex<Option<Failure>>,
    statuses: Option<&'a Mutex<BTreeMap<String, String>>>,
}
impl ResearchMember for Metered<'_> {
    fn name(&self) -> &str {
        self.member.name()
    }
    fn call(&self, call: &MemberCall, stop: &AtomicBool) -> MemberReply {
        let mut budget = self.budget.lock().unwrap();
        if budget.remaining_usd() <= 0.0 || !budget.allows(0.0) {
            if let Some(statuses) = self.statuses {
                statuses
                    .lock()
                    .unwrap()
                    .insert(self.name().into(), "BudgetCut".into());
            }
            return MemberReply {
                status: Status::BudgetCut,
                output: None,
                raw: None,
                model: None,
                provider: self.name().into(),
                usd: None,
                tokens: 0,
                detail: Some("research budget exhausted".into()),
            };
        }
        let mut reply = self.member.call(call, stop);
        if reply.status == Status::Ok {
            let valid = reply
                .output
                .as_ref()
                .ok_or_else(|| "missing output".to_string())
                .and_then(|value| schema::validate(&call.schema, value));
            if let Err(error) = valid {
                reply.status = Status::Malformed;
                reply.detail = Some(error);
            }
        }
        if let Some(statuses) = self.statuses {
            statuses
                .lock()
                .unwrap()
                .insert(self.name().into(), format!("{:?}", reply.status));
        }
        if let Err(e) = budget.charge(&reply.provider, reply.usd, reply.tokens) {
            *self.failure.lock().unwrap() = Some(e);
        }
        reply
    }
}
fn call_member(
    member: &dyn ResearchMember,
    stage: &str,
    prompt: String,
    shape: Value,
    dir: &Path,
    budget: &Mutex<Budget>,
    failure: &Mutex<Option<Failure>>,
    stop: &AtomicBool,
) -> Res<Value> {
    fs::create_dir_all(dir).map_err(io)?;
    let call = MemberCall {
        member: member.name().into(),
        stage: stage.into(),
        prompt,
        readable: vec![],
        out_dir: dir.into(),
        schema: shape.clone(),
        timeout_s: 900,
    };
    let reply = Metered {
        member,
        budget,
        failure,
        statuses: None,
    }
    .call(&call, stop);
    if let Some(e) = failure.lock().unwrap().take() {
        return Err(e);
    }
    if stop.load(Ordering::Relaxed) {
        return Err(Failure::new("interrupted", "research interrupted"));
    }
    if reply.status != Status::Ok {
        return Err(Failure::new(
            if reply.status == Status::BudgetCut {
                "budget_cut"
            } else if reply.status == Status::Malformed {
                "malformed"
            } else {
                "member_failed"
            },
            reply.detail.unwrap_or_else(|| reply.status.word().into()),
        ));
    }
    let value = reply
        .output
        .ok_or_else(|| Failure::new("malformed", "missing member output"))?;
    schema::validate(&shape, &value).map_err(|e| Failure::new("malformed", e))?;
    Ok(value)
}
fn findings(red: &Value, pre: &Value) -> Vec<Finding> {
    let mut out = Vec::new();
    for (prefix, stage, array, item, reason) in [
        ("R", Stage::RedTeam, &red["findings"], "item", "reason"),
        ("P", Stage::PreMortem, &pre["failures"], "item", "scenario"),
    ] {
        if let Some(items) = array.as_array() {
            for (i, v) in items.iter().enumerate() {
                out.push(Finding {
                    id: format!("{}{}", prefix, i + 1),
                    stage,
                    item: v[item].as_str().unwrap_or_default().into(),
                    reason: v[reason].as_str().unwrap_or_default().into(),
                });
            }
        }
    }
    out
}
fn validate_rulings(a: &Adjudication, f: &[Finding]) -> Res<()> {
    let expected: HashSet<_> = f.iter().map(|f| f.id.as_str()).collect();
    let mut seen = HashSet::new();
    for r in &a.rulings {
        if !expected.contains(r.finding.as_str()) || !seen.insert(r.finding.as_str()) {
            return Err(Failure::new("malformed", "unknown or duplicate ruling"));
        }
    }
    if seen.len() != expected.len() {
        return Err(Failure::new("malformed", "missing ruling for finding"));
    }
    Ok(())
}
fn entry(
    state: &mut ProgramState,
    stage: Stage,
    status: StageStatus,
    sha: Option<String>,
    detail: Option<String>,
    members: BTreeMap<String, String>,
) {
    state.stages.retain(|e| e.stage != stage);
    state.stages.push(StageEntry {
        stage,
        status,
        artifact: stage.artifact().into(),
        sha256: sha,
        at: base::now_iso(),
        detail,
        members,
    });
    state
        .stages
        .sort_by_key(|e| Stage::ALL.iter().position(|s| *s == e.stage).unwrap());
}
fn remove_published(store: &Path) -> Res<()> {
    for name in ["plan.json", "review.json", "ruling.md", "plan-final.md"] {
        match fs::remove_file(store.join("astra").join(name)) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(io(e)),
        }
    }
    Ok(())
}
fn publish(store: &Path, plan: &Value, red: &Value, adjudication: &Adjudication) -> Res<()> {
    let shape: Value =
        serde_json::from_str(include_str!("../../engine/research/audit.schema.json"))
            .map_err(io)?;
    schema::validate(&shape, plan).map_err(|e| Failure::new("malformed", e))?;
    let dir = store.join("astra");
    fs::create_dir_all(&dir).map_err(io)?;
    let mut ruling = String::new();
    for r in &adjudication.rulings {
        ruling.push_str(&format!(
            "## {} {:?}\n\n{}\n\n",
            r.finding, r.decision, r.reason
        ));
    }
    let mut final_plan = String::new();
    for change in plan["changes"].as_array().unwrap() {
        let id = change["id"].as_str().unwrap_or_default();
        if !adjudication.dropped_changes.iter().any(|d| d == id) {
            final_plan.push_str(&format!(
                "### {} {}\n\n{}\n\n",
                id,
                change["change"]
                    .as_str()
                    .unwrap_or_default()
                    .replace('\n', " "),
                change
            ));
        }
    }
    final_plan.push_str("## Coverage\n\n");
    for key in ["read", "not_read"] {
        final_plan.push_str(&format!("{}:\n", key));
        for item in plan["coverage"][key].as_array().unwrap() {
            final_plan.push_str(&format!("- {}\n", item.as_str().unwrap_or_default()));
        }
    }
    write_json(&dir.join("plan.json"), plan)?;
    write_json(&dir.join("review.json"), red)?;
    base::atomic_write(&dir.join("ruling.md"), ruling.as_bytes()).map_err(io)?;
    base::atomic_write(&dir.join("plan-final.md"), final_plan.as_bytes()).map_err(io)?;
    orch::record_stage(store, "astra", &adjudication.model, true)
}

pub fn run(
    cfg: &ProgramConfig,
    members: &Members,
    council: &dyn CouncilStage,
    adjudicator: &dyn Adjudicator,
    stop: &AtomicBool,
) -> Res<Value> {
    let dir = program_dir(&cfg.store);
    fs::create_dir_all(&dir).map_err(io)?;
    let run_id = cfg
        .store
        .file_name()
        .ok_or_else(|| Failure::new("invalid_argument", "store has no directory name"))?
        .to_string_lossy()
        .into_owned();
    let brief_sha = sha256_hex(cfg.question.as_bytes());
    let mut state = state::load(&cfg.store)?.unwrap_or_else(|| ProgramState {
        run_id: run_id.clone(),
        question: cfg.question.clone(),
        brief_sha256: brief_sha.clone(),
        stages: vec![],
        complete: false,
    });
    if state.run_id != run_id || state.brief_sha256 != brief_sha || state.question != cfg.question {
        return Err(Failure::new(
            "state_mismatch",
            "run or question differs from saved program",
        ));
    }
    let budget = Mutex::new(Budget::open(
        &dir.join("budget.json"),
        &run_id,
        cfg.initial_cap_usd,
    )?);
    let failure = Mutex::new(None);
    let mut models: Vec<String> = members
        .investigators
        .iter()
        .map(|m| m.name().to_owned())
        .collect();
    models.extend([
        members.red_team.name().to_owned(),
        members.pre_mortem.name().to_owned(),
        members.synthesizer.name().to_owned(),
    ]);
    let estimate_line = history::estimate(&cfg.home, PROGRAM_TYPE, &models)?.line();
    let was_complete = state.complete;
    let all_valid = Stage::ALL.iter().all(|stage| {
        state.stages.iter().any(|e| {
            e.stage == *stage
                && e.status == StageStatus::Done
                && e.sha256.as_ref().is_some_and(|h| {
                    artifact_hash(&dir.join(stage.artifact())).ok().as_ref() == Some(h)
                })
        })
    });
    if was_complete && !all_valid {
        remove_published(&cfg.store)?;
    }
    let published = ["plan.json", "review.json", "ruling.md", "plan-final.md"]
        .iter()
        .all(|name| cfg.store.join("astra").join(name).is_file());
    state.complete = was_complete && all_valid && published;
    state::save(&cfg.store, &state)?;
    let mut changed = false;
    for stage in Stage::ALL {
        if stop.load(Ordering::Relaxed) {
            if state.complete {
                remove_published(&cfg.store)?;
            }
            state.complete = false;
            entry(
                &mut state,
                stage,
                StageStatus::Cancelled,
                None,
                Some("interrupted".into()),
                BTreeMap::new(),
            );
            state::save(&cfg.store, &state)?;
            return Err(Failure::new("interrupted", "research interrupted"));
        }
        let path = dir.join(stage.artifact());
        if state.stages.iter().any(|e| {
            e.stage == stage
                && e.status == StageStatus::Done
                && e.sha256
                    .as_ref()
                    .is_some_and(|h| artifact_hash(&path).ok().as_ref() == Some(h))
        }) {
            continue;
        }
        changed = true;
        state.complete = false;
        state.stages.retain(|e| e.stage != stage);
        state::save(&cfg.store, &state)?;
        let mut member_status = BTreeMap::new();
        let result: Res<()> = (|| {
            match stage {
                Stage::Investigation => {
                    fs::create_dir_all(&path).map_err(io)?;
                    let statuses = Mutex::new(BTreeMap::new());
                    let proxies: Vec<_> = members
                        .investigators
                        .iter()
                        .map(|m| Metered {
                            member: *m,
                            budget: &budget,
                            failure: &failure,
                            statuses: Some(&statuses),
                        })
                        .collect();
                    let refs: Vec<&dyn ResearchMember> =
                        proxies.iter().map(|m| m as &dyn ResearchMember).collect();
                    let outcomes = investigation::investigate(
                        &path,
                        &InvestigationRequest {
                            question: cfg.question.clone(),
                            sources: cfg.sources.clone(),
                            cwd: cfg.cwd.clone(),
                        },
                        &refs,
                        stop,
                    );
                    member_status = statuses.into_inner().unwrap();
                    if let Some(e) = failure.lock().unwrap().take() {
                        return Err(e);
                    }
                    if stop.load(Ordering::Relaxed) {
                        return Err(Failure::new("interrupted", "research interrupted"));
                    }
                    let outcomes = match outcomes {
                        Err(e) if member_status.values().any(|s| s == "BudgetCut") => {
                            return Err(Failure::new("budget_cut", e.message))
                        }
                        other => other?,
                    };
                    for o in outcomes {
                        member_status.insert(o.member, format!("{:?}", o.status));
                    }
                }
                Stage::Dossier => {
                    let mut inputs = vec![];
                    for member in &members.investigators {
                        let p = dir
                            .join(Stage::Investigation.artifact())
                            .join(member.name())
                            .join("findings.json");
                        if p.is_file() {
                            inputs.push((member.name().to_owned(), read_json(&p)?));
                        }
                    }
                    dossier::write(&path, &dossier::build(&cfg.question, &inputs))?;
                }
                Stage::Council => {
                    fs::create_dir_all(&path).map_err(io)?;
                    let d = dossier::read(&dir.join(Stage::Dossier.artifact()))?;
                    let claims: Vec<Value> = d.items.iter().map(|i| json!({"claim":i.claim,"evidence":format!("{}: {}", i.id, i.sources.iter().map(|s| s.evidence.as_deref().unwrap_or(&s.source)).collect::<Vec<_>>().join("; "))})).collect();
                    let shape = json!({"type":"object","required":["claims"],"properties":{"claims":{"type":"array","items":{"type":"object","required":["claim","evidence"],"properties":{"claim":{"type":"string"},"evidence":{"type":"string"}}}}}});
                    let plan_path = path.join("plan.json");
                    let schema_path = path.join("claims.schema.json");
                    write_json(&plan_path, &json!({"claims":claims}))?;
                    write_json(&schema_path, &shape)?;
                    let remaining = budget.lock().unwrap().remaining_usd();
                    if remaining <= 0.0 {
                        return Err(Failure::new("budget_cut", "research budget exhausted"));
                    }
                    let digest =
                        council.debate(&plan_path, &schema_path, &cfg.question, remaining, stop)?;
                    {
                        let mut budget = budget.lock().unwrap();
                        budget.charge(
                            "council",
                            digest.pointer("/cost/usd").and_then(Value::as_f64),
                            0,
                        )?;
                        let unpriced = digest
                            .pointer("/cost/unpriced_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        if unpriced > 0 {
                            budget.charge("council", None, unpriced)?;
                        }
                    }
                    write_json(&path.join("digest.json"), &digest)?;
                }
                Stage::RedTeam | Stage::PreMortem | Stage::Synthesis => {
                    let (member, shape) = match stage {
                        Stage::RedTeam => (
                            members.red_team,
                            serde_json::from_str(include_str!(
                                "../../engine/research/review.schema.json"
                            ))
                            .map_err(io)?,
                        ),
                        Stage::PreMortem => (
                            members.pre_mortem,
                            json!({"type":"object","required":["failures"],"properties":{"failures":{"type":"array","items":{"type":"object","required":["scenario","item","likelihood","mitigation"],"properties":{"scenario":{"type":"string"},"item":{"type":"string"},"likelihood":{"type":"string","enum":["low","medium","high"]},"mitigation":{"type":"string"}}}}}}),
                        ),
                        _ => (
                            members.synthesizer,
                            json!({"type":"object","required":["summary","conclusions","open_questions","priority_checks","plan"],"properties":{"summary":{"type":"string"},"conclusions":{"type":"array","items":{"type":"object","required":["id","claim","confidence","evidence"],"properties":{"id":{"type":"string"},"claim":{"type":"string"},"confidence":{"type":"string","enum":["low","medium","high"]},"evidence":{"type":"array","items":{"type":"string"}}}}},"open_questions":{"type":"array","items":{"type":"string"}},"priority_checks":{"type":"array","items":{"type":"string"}},"plan":serde_json::from_str::<Value>(include_str!("../../engine/research/audit.schema.json")).map_err(io)?}}),
                        ),
                    };
                    let context = format!("Question: {}\nDossier: {}\nCouncil: {}\nRed team: {}\nPre-mortem: {}\nReturn JSON matching the provided schema.", cfg.question, read_json(&dir.join(Stage::Dossier.artifact()))?, read_json(&dir.join(Stage::Council.artifact()).join("digest.json"))?, if stage == Stage::Synthesis { read_json(&dir.join(Stage::RedTeam.artifact()))? } else { Value::Null }, if stage == Stage::Synthesis { read_json(&dir.join(Stage::PreMortem.artifact()))? } else { Value::Null });
                    let value = call_member(
                        member,
                        stage.as_str(),
                        context,
                        shape,
                        &dir.join("scratch").join(stage.as_str()),
                        &budget,
                        &failure,
                        stop,
                    )?;
                    member_status.insert(member.name().into(), "ok".into());
                    write_json(&path, &value)?;
                }
                Stage::Adjudication => {
                    if budget.lock().unwrap().remaining_usd() <= 0.0 {
                        return Err(Failure::new("budget_cut", "research budget exhausted"));
                    }
                    let synth = read_json(&dir.join(Stage::Synthesis.artifact()))?;
                    let red = read_json(&dir.join(Stage::RedTeam.artifact()))?;
                    let pre = read_json(&dir.join(Stage::PreMortem.artifact()))?;
                    let f = findings(&red, &pre);
                    let a = adjudicator.adjudicate(&synth["plan"], &f, stop)?;
                    validate_rulings(&a, &f)?;
                    write_json(&path, &a)?;
                }
            }
            if stop.load(Ordering::Relaxed) {
                return Err(Failure::new("interrupted", "research interrupted"));
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                entry(
                    &mut state,
                    stage,
                    StageStatus::Done,
                    Some(artifact_hash(&path)?),
                    None,
                    member_status,
                );
                state::save(&cfg.store, &state)?;
            }
            Err(e) => {
                let cancelled = e.kind == "interrupted" || stop.load(Ordering::Relaxed);
                entry(
                    &mut state,
                    stage,
                    if cancelled {
                        StageStatus::Cancelled
                    } else {
                        StageStatus::Failed
                    },
                    None,
                    Some(e.message.clone()),
                    member_status,
                );
                state::save(&cfg.store, &state)?;
                return Err(if cancelled {
                    Failure::new("interrupted", e.message)
                } else {
                    e
                });
            }
        }
    }
    if changed || !state.complete {
        let synth = read_json(&dir.join(Stage::Synthesis.artifact()))?;
        let red = read_json(&dir.join(Stage::RedTeam.artifact()))?;
        let pre = read_json(&dir.join(Stage::PreMortem.artifact()))?;
        let a: Adjudication =
            serde_json::from_value(read_json(&dir.join(Stage::Adjudication.artifact()))?)
                .map_err(|e| Failure::new("malformed", e.to_string()))?;
        validate_rulings(&a, &findings(&red, &pre))?;
        let completed = (|| {
            publish(&cfg.store, &synth["plan"], &red, &a)?;
            history::append(
                &cfg.home,
                &budget.lock().unwrap().summary(PROGRAM_TYPE, &models, true),
            )?;
            state.complete = true;
            state::save(&cfg.store, &state)
        })();
        if let Err(error) = completed {
            state.complete = false;
            let _ = remove_published(&cfg.store);
            let _ = state::save(&cfg.store, &state);
            return Err(error);
        }
    }
    let budget = budget.lock().unwrap();
    let summary = budget.summary(PROGRAM_TYPE, &models, state.complete);
    let unpriced: u64 = summary
        .per_provider
        .values()
        .map(|s| s.unpriced_tokens)
        .sum();
    let bounded = |s: &str, n: usize| s.chars().take(n).collect::<String>();
    let mut digest = json!({"complete":state.complete,"run_id":bounded(&run_id, 300),"stages":state.stages.iter().map(|e| json!({"stage":e.stage,"status":e.status})).collect::<Vec<_>>(),"estimate_line":bounded(&estimate_line, 1000),"budget":{"cap_usd":budget.cap_usd(),"priced_spent":budget.priced_spent(),"unpriced_tokens":unpriced},"plan_final":bounded(&cfg.store.join("astra/plan-final.md").to_string_lossy(), 2000),"note":"Research program complete"});
    if digest.to_string().chars().count() > council::DIGEST_MAX_CHARS {
        digest["estimate_line"] = json!("estimate omitted; see history");
    }
    if digest.to_string().chars().count() > council::DIGEST_MAX_CHARS {
        digest["plan_final"] = json!("astra/plan-final.md");
    }
    Ok(digest)
}
