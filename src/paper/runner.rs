use crate::{
    base::{atomic_write, now_iso, schema, Failure, Res},
    council::Status,
    paper::{
        budget::{self, PaperBudget},
        evidence,
        members::{expected_model, PaperMembers},
        state, style, *,
    },
    research_program::investigation::MemberCall,
    seams::sha256_hex,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub struct RunConfig {
    pub store: PathBuf,
    pub home: PathBuf,
    pub project_root: PathBuf,
    pub timeout_s: u64,
    pub fable_cap_usd: f64,
}

fn write(path: &Path, bytes: &[u8]) -> Res<()> {
    atomic_write(path, bytes).map_err(|e| Failure::new("io", e.to_string()))
}
fn write_json(path: &Path, value: &impl Serialize) -> Res<()> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|e| Failure::new("json", e.to_string()))?;
    write(path, &bytes)
}
fn read_json<T: DeserializeOwned>(path: &Path) -> Res<T> {
    let bytes = fs::read(path).map_err(|e| Failure::new("io", e.to_string()))?;
    serde_json::from_slice(&bytes).map_err(|e| Failure::new("json", e.to_string()))
}
fn blocked(
    step: Step,
    kind: &str,
    reason: impl Into<String>,
    events: &dyn Fn(&PaperEvent),
) -> Failure {
    let reason = reason.into();
    if kind == "cancelled" {
        events(&PaperEvent::Cancelled);
    }
    events(&PaperEvent::Blocked {
        step,
        kind: kind.into(),
        reason: reason.clone(),
    });
    Failure::new(kind, reason)
}

fn record_unstarted(
    dir: &Path,
    pass: PassId,
    evidence_hash: &str,
    status: Status,
    reason: &str,
) -> Res<()> {
    fs::create_dir_all(dir).map_err(|e| Failure::new("io", e.to_string()))?;
    let validation = vec![reason.to_owned()];
    let record = PassRecord {
        pass,
        member: pass.member().into(),
        model: None,
        provider: String::new(),
        status,
        evidence_hash: evidence_hash.into(),
        input_hash: String::new(),
        output_hash: None,
        usd: None,
        tokens: 0,
        raw_path: dir.join("raw.txt").to_string_lossy().into_owned(),
        change_log: Vec::new(),
        validation: validation.clone(),
        at: now_iso(),
    };
    write_json(&dir.join("validation.json"), &validation)?;
    write_json(&dir.join("pass-record.json"), &record)
}

fn validate_bundle_ids(manuscript: &Manuscript, bundle: &EvidenceBundle) -> Vec<String> {
    let mut errors = Vec::new();
    let mut texts = vec![manuscript.title.as_str()];
    texts.extend(manuscript.keywords.iter().map(String::as_str));
    texts.extend(manuscript.sections.iter().map(|s| s.heading.as_str()));
    for (location, paragraph) in manuscript.paragraphs() {
        texts.push(&paragraph.text);
        for link in &paragraph.claims {
            if !bundle.claims.iter().any(|c| c.id == link.claim) {
                errors.push(format!("{location}: unknown claim {}", link.claim));
            }
            for id in &link.evidence {
                if !bundle.evidence.iter().any(|e| e.id == *id) {
                    errors.push(format!("{location}: unknown evidence {id}"));
                }
            }
        }
    }
    for text in texts {
        for (_, token) in tokens(text) {
            match token {
                Token::Quantity(id) if !bundle.quantities.iter().any(|q| q.id == id) => {
                    errors.push(format!("unknown quantity {id}"))
                }
                Token::Cite(keys) => {
                    for key in keys {
                        if !bundle.references.iter().any(|r| r.key == key) {
                            errors.push(format!("unknown citation {key}"));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    errors
}

pub fn run_passes(
    cfg: &RunConfig,
    m: &PaperMembers,
    _journal: &JournalRecord,
    template_rules: &str,
    stop: &AtomicBool,
    on_event: &dyn Fn(&PaperEvent),
) -> Res<Vec<PassRecord>> {
    let mut state = state::load(&cfg.store)?.ok_or_else(|| {
        blocked(
            Step::Evidence,
            "not_ready",
            "evidence is not current",
            on_event,
        )
    })?;
    if !state::current(&state, Step::Evidence) {
        return Err(blocked(
            Step::Evidence,
            "not_ready",
            "evidence is not current",
            on_event,
        ));
    }
    let bundle = evidence::load(&cfg.store)?.ok_or_else(|| {
        blocked(
            Step::Evidence,
            "not_ready",
            "evidence bundle is missing",
            on_event,
        )
    })?;
    let evidence_hash = state.steps[&Step::Evidence].output.clone();
    if bundle.hash != evidence_hash || bundle.compute_hash() != evidence_hash {
        return Err(blocked(
            Step::Evidence,
            "not_ready",
            "evidence bundle hash differs from state",
            on_event,
        ));
    }
    if !state::current(&state, Step::Journal) {
        return Err(blocked(
            Step::Journal,
            "no_journal",
            "journal is not current",
            on_event,
        ));
    }
    let journal_hash = state.steps[&Step::Journal].output.clone();
    let pass_models = models::load(&cfg.home)?;
    let models = models::estimate_models();
    let mut budget = PaperBudget::open(&cfg.store, &state.run_id, cfg.fable_cap_usd)?;
    let result = (|| -> Res<Vec<PassRecord>> {
        let schema: Value =
            serde_json::from_str(include_str!("../../engine/paper/pass.schema.json"))
                .expect("valid pass schema");
        let bundle_json = serde_json::to_string_pretty(&bundle)
            .map_err(|e| Failure::new("json", e.to_string()))?;
        let schema_json = serde_json::to_string_pretty(&schema)
            .map_err(|e| Failure::new("json", e.to_string()))?;
        let steps = [Step::Pass1, Step::Pass2, Step::Pass3, Step::Pass4];
        let mut records = Vec::new();
        let mut prior: Option<Manuscript> = None;
        let mut estimated = false;
        for (index, pass) in PassId::ALL.into_iter().enumerate() {
            let step = steps[index];
            let member_name = pass.member();
            let model = pass_models.get(pass);
            let dir = paper_dir(&cfg.store)
                .join("passes")
                .join(format!("{}-{member_name}", pass.number()));
            if stop.load(Ordering::SeqCst) {
                if !state::current(&state, step) {
                    record_unstarted(&dir, pass, &evidence_hash, Status::Failed, "cancelled")?;
                }
                return Err(blocked(step, "cancelled", "paper run cancelled", on_event));
            }
            if state::current(&state, step)
                && state.steps[&step]
                    .inputs
                    .get("model")
                    .is_some_and(|v| v == model)
                && dir.join("pass-record.json").is_file()
                && dir.join("manuscript.json").is_file()
            {
                records.push(read_json(&dir.join("pass-record.json"))?);
                prior = Some(read_json(&dir.join("manuscript.json"))?);
                continue;
            }
            state = state::invalidate(&cfg.store, step)?;
            if let Err(e) = budget.before_call(&cfg.store, &cfg.home, member_name, model) {
                record_unstarted(
                    &dir,
                    pass,
                    &evidence_hash,
                    Status::BudgetCut,
                    &format!("budget_exceeded: {}", e.message),
                )?;
                return Err(blocked(step, "budget_exceeded", e.message, on_event));
            }
            if !estimated {
                on_event(&PaperEvent::Estimate(budget::estimate(
                    &cfg.home,
                    &models,
                    &pass_models,
                )?));
                estimated = true;
            }
            fs::create_dir_all(&dir).map_err(|e| Failure::new("io", e.to_string()))?;
            let instructions = match pass {
                PassId::AstraDraft => crate::engine::asset("paper/prompts/draft.md")
                    .unwrap_or_default()
                    .to_owned(),
                PassId::FableRevise => style::revise_instructions(),
                PassId::GeminiRevise => crate::engine::asset("paper/prompts/critique.md")
                    .unwrap_or_default()
                    .to_owned(),
                PassId::FableStyle => style::style_instructions(),
            };
            let mut prompt = format!("{instructions}\n\nJournal template rules:\n{template_rules}\n\nEvidence bundle:\n{bundle_json}");
            if let Some(manuscript) = &prior {
                prompt.push_str(&format!(
                    "\n\nPrior manuscript:\n{}",
                    serde_json::to_string_pretty(manuscript)
                        .map_err(|e| Failure::new("json", e.to_string()))?
                ));
            }
            prompt.push_str(&format!("\n\nOutput schema:\n{schema_json}"));
            write(&dir.join("prompt.txt"), prompt.as_bytes())?;
            write_json(
                &dir.join("input.json"),
                &json!({"evidence_hash": evidence_hash, "journal_hash": journal_hash, "prior_hash": prior.as_ref().map(Manuscript::hash)}),
            )?;
            let call = MemberCall {
                member: member_name.into(),
                stage: format!("paper-pass-{}", pass.number()),
                prompt,
                readable: vec![dir.clone()],
                out_dir: dir.clone(),
                schema: schema.clone(),
                timeout_s: cfg.timeout_s,
            };
            let member = match pass {
                PassId::AstraDraft => m.astra,
                PassId::FableRevise | PassId::FableStyle => m.fable,
                PassId::GeminiRevise => m.gemini,
            };
            on_event(&PaperEvent::PassStarted { pass });
            let reply = member.call_model(&call, model, stop);
            write(
                &dir.join("raw.txt"),
                reply.raw.as_deref().unwrap_or("").as_bytes(),
            )?;
            write_json(&dir.join("reply.json"), &reply)?;
            let mut validation = Vec::new();
            let mut failure = None;
            let mut output = None;
            if stop.load(Ordering::SeqCst) {
                validation.push("cancelled".into());
                failure = Some("cancelled");
            } else if reply.detail.as_deref() == Some("wrong_model") {
                validation.push("wrong_model".into());
                failure = Some("rejected");
            } else if reply.status != Status::Ok {
                validation.push(format!(
                    "member_failed: {}",
                    reply.detail.as_deref().unwrap_or(reply.status.word())
                ));
                failure = Some("member_failed");
            } else if expected_model(&pass_models, pass)
                .is_some_and(|expected| reply.model.as_deref() != Some(expected.as_str()))
                || (pass == PassId::AstraDraft
                    && match reply.model.as_deref() {
                        Some(actual) => actual != model,
                        None => model != models::DEFAULT_ASTRA_MODEL,
                    })
            {
                validation.push("wrong_model".into());
                failure = Some("rejected");
            } else {
                let parsed = reply
                    .output
                    .as_ref()
                    .ok_or_else(|| "missing output".to_string())
                    .and_then(|v| {
                        schema::validate(&schema, v)?;
                        serde_json::from_value::<PassOutput>(v.clone()).map_err(|e| e.to_string())
                    });
                match parsed {
                    Err(e) => {
                        validation.push(format!("malformed: {e}"));
                        failure = Some("rejected");
                    }
                    Ok(value) => {
                        validation.extend(validate_bundle_ids(&value.manuscript, &bundle));
                        if let Some(previous) = &prior {
                            let check = if pass == PassId::FableStyle {
                                style::check_style_only(previous, &value.manuscript)
                            } else {
                                style::check_revision(
                                    previous,
                                    &value.manuscript,
                                    &bundle,
                                    &value.change_log,
                                )
                            };
                            if let Err(errors) = check {
                                validation.extend(errors.iter().map(|e| {
                                    format!("{}: {:?}: {}", e.location, e.kind, e.detail)
                                }));
                            }
                        }
                        if !validation.is_empty() {
                            failure = Some("rejected");
                        }
                        output = Some(value);
                    }
                }
            }
            let output_hash = output
                .as_ref()
                .filter(|_| failure.is_none())
                .map(|o| o.manuscript.hash());
            if let Some(value) = &output {
                write_json(&dir.join("manuscript.json"), &value.manuscript)?;
                write_json(&dir.join("changelog.json"), &value.change_log)?;
            }
            let mut record = PassRecord {
                pass,
                member: member_name.into(),
                model: if pass == PassId::AstraDraft {
                    reply.model.clone().or_else(|| {
                        (model == models::DEFAULT_ASTRA_MODEL)
                            .then(|| models::DEFAULT_ASTRA_MODEL.to_owned())
                    })
                } else {
                    reply.model.clone()
                },
                provider: reply.provider.clone(),
                status: reply.status,
                evidence_hash: evidence_hash.clone(),
                input_hash: sha256_hex(call.prompt.as_bytes()),
                output_hash: output_hash.clone(),
                usd: reply.usd,
                tokens: reply.tokens,
                raw_path: dir.join("raw.txt").to_string_lossy().into_owned(),
                change_log: output
                    .as_ref()
                    .map(|o| o.change_log.clone())
                    .unwrap_or_default(),
                validation: validation.clone(),
                at: now_iso(),
            };
            write_json(&dir.join("validation.json"), &validation)?;
            write_json(&dir.join("pass-record.json"), &record)?;
            let charged = budget.charge(
                &cfg.store,
                &cfg.home,
                member_name,
                model,
                reply.usd,
                reply.tokens,
            );
            if let Some(kind) = failure {
                return Err(blocked(step, kind, validation.join("; "), on_event));
            }
            if let Err(e) = charged {
                record.output_hash = None;
                record
                    .validation
                    .push(format!("budget_exceeded: {}", e.message));
                write_json(&dir.join("validation.json"), &record.validation)?;
                write_json(&dir.join("pass-record.json"), &record)?;
                return Err(blocked(step, "budget_exceeded", e.message, on_event));
            }
            let mut inputs = BTreeMap::from([
                ("evidence".into(), evidence_hash.clone()),
                ("journal".into(), journal_hash.clone()),
                ("model".into(), model.into()),
            ]);
            if index > 0 {
                inputs.insert(
                    format!("pass{index}"),
                    state.steps[&steps[index - 1]].output.clone(),
                );
            }
            state = state::record(
                &cfg.store,
                step,
                inputs,
                output_hash.as_deref().unwrap(),
                true,
                None,
            )?;
            on_event(&PaperEvent::PassFinished {
                pass,
                status: reply.status,
                changes: record.change_log.len(),
            });
            prior = output.map(|o| o.manuscript);
            records.push(record);
        }
        Ok(records)
    })();
    let finished = budget.finish(
        &cfg.home,
        &models,
        result.is_ok()
            && [Step::Pass1, Step::Pass2, Step::Pass3, Step::Pass4]
                .iter()
                .all(|s| state::current(&state, *s)),
    );
    match result {
        Err(e) => Err(e),
        Ok(records) => {
            finished?;
            Ok(records)
        }
    }
}
