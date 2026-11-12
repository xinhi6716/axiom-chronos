//! Independent paper review.
use super::{budget::PaperBudget, readiness, runner::RunConfig, state, Step};
use crate::{
    base::{atomic_write, now_iso, schema, Failure, Res},
    council::Status,
    research_program::investigation::{MemberCall, ResearchMember},
    seams::sha256_hex,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ReviewRecord {
    pub verdict: String,
    pub findings: Vec<serde_json::Value>,
    pub input_hash: String,
    pub model: Option<String>,
    pub usd: Option<f64>,
    pub tokens: u64,
    pub at: String,
}
fn io(e: std::io::Error) -> Failure {
    Failure::new("io", e.to_string())
}
fn remove_result(path: &Path) -> Res<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io(e)),
    }
}
pub fn input_files(store: &Path) -> Res<Vec<(String, PathBuf)>> {
    let files = vec![
        ("manuscript.json", readiness::final_manuscript_path(store)),
        (
            "evidence.json",
            super::paper_dir(store).join("evidence.json"),
        ),
        ("journal-rules.md", readiness::rules_path(store)),
        (
            "citation-report.json",
            readiness::citation_report_path(store),
        ),
        ("numbers-report.json", readiness::numbers_report_path(store)),
        ("compile-report.json", readiness::compile_report_path(store)),
        ("manuscript.pdf", readiness::pdf_path(store)),
    ];
    for (_, path) in &files {
        if !path.is_file() {
            return Err(Failure::new(
                "not_ready",
                format!("missing {}", path.display()),
            ));
        }
    }
    Ok(files.into_iter().map(|(n, p)| (n.to_owned(), p)).collect())
}
pub fn current_input_hash(store: &Path) -> Res<String> {
    let mut material = String::new();
    for (name, path) in input_files(store)? {
        let bytes = fs::read(path).map_err(io)?;
        material.push_str(&format!("{name}\0{}\n", sha256_hex(&bytes)));
    }
    Ok(sha256_hex(material.as_bytes()))
}
pub fn review(
    cfg: &RunConfig,
    reviewer: &dyn ResearchMember,
    budget: &mut PaperBudget,
    stop: &AtomicBool,
) -> Res<ReviewRecord> {
    let dir = super::paper_dir(&cfg.store).join("review");
    let result_path = dir.join("review.json");
    let st = state::load(&cfg.store)?;
    let reject = |error: Failure| -> Res<ReviewRecord> {
        remove_result(&result_path)?;
        if st
            .as_ref()
            .is_some_and(|s| s.steps.contains_key(&Step::Review))
        {
            state::invalidate(&cfg.store, Step::Review)?;
        }
        Err(error)
    };
    let Some(st) = st.as_ref() else {
        return reject(Failure::new("not_ready", "paper state missing"));
    };
    for &step in state::upstream(Step::Review) {
        if !state::current(st, step) {
            return reject(Failure::new(
                "not_ready",
                format!("{step:?} is not current"),
            ));
        }
    }
    if stop.load(Ordering::SeqCst) {
        return reject(Failure::new("cancelled", "review cancelled"));
    }
    remove_result(&result_path)?;
    state::invalidate(&cfg.store, Step::Review)?;
    let input = dir.join("input");
    match fs::remove_dir_all(&input) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(io(e)),
    }
    fs::create_dir_all(&input).map_err(io)?;
    let mut prompt = format!(
        "{}\n\n",
        crate::engine::asset("paper/prompts/review.md")
            .ok_or_else(|| Failure::new("not_ready", "review instructions missing"))?
    );
    let mut material = String::new();
    for (name, path) in input_files(&cfg.store)? {
        let bytes = fs::read(path).map_err(io)?;
        atomic_write(&input.join(&name), &bytes).map_err(io)?;
        let hash = sha256_hex(&bytes);
        material.push_str(&format!("{name}\0{hash}\n"));
        if name == "manuscript.pdf" {
            prompt.push_str(&format!(
                "=== manuscript.pdf === sha256 {hash}, file in the input directory"
            ));
        } else {
            let content = std::str::from_utf8(&bytes)
                .map_err(|e| Failure::new("malformed", e.to_string()))?;
            prompt.push_str(&format!("=== {name} ===\n{content}\n"));
        }
    }
    budget.before_call(&cfg.store, "reviewer")?;
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../../engine/paper/reviewer.schema.json"))
            .expect("valid reviewer schema");
    let call = MemberCall {
        member: "reviewer".into(),
        stage: "paper-review".into(),
        prompt,
        readable: vec![input],
        out_dir: dir,
        schema: schema.clone(),
        timeout_s: cfg.timeout_s,
    };
    let reply = reviewer.call(&call, stop);
    let charged = budget.charge(
        &cfg.store,
        "reviewer",
        reply.model.as_deref().unwrap_or("reviewer"),
        reply.usd,
        reply.tokens,
    );
    remove_result(&result_path)?;
    if stop.load(Ordering::SeqCst) {
        return Err(Failure::new("cancelled", "review cancelled"));
    }
    if reply.status != Status::Ok {
        return Err(Failure::new(
            "member_failed",
            reply
                .detail
                .unwrap_or_else(|| reply.status.word().to_owned()),
        ));
    }
    let output = reply
        .output
        .ok_or_else(|| Failure::new("malformed", "missing output"))?;
    schema::validate(&schema, &output).map_err(|e| Failure::new("malformed", e))?;
    charged?;
    let input_hash = current_input_hash(&cfg.store)?;
    if input_hash != sha256_hex(material.as_bytes()) {
        return Err(Failure::new(
            "not_ready",
            "review inputs changed during call",
        ));
    }
    let record = ReviewRecord {
        verdict: output["verdict"].as_str().unwrap().to_owned(),
        findings: output["findings"].as_array().unwrap().clone(),
        input_hash,
        model: reply.model,
        usd: reply.usd,
        tokens: reply.tokens,
        at: now_iso(),
    };
    let bytes =
        serde_json::to_vec_pretty(&record).map_err(|e| Failure::new("json", e.to_string()))?;
    atomic_write(&result_path, &bytes).map_err(io)?;
    if let Err(e) = readiness::record_gate(
        &cfg.store,
        Step::Review,
        &record.input_hash,
        true,
        Some(record.verdict.clone()),
    ) {
        let _ = fs::remove_file(&result_path);
        return Err(e);
    }
    Ok(record)
}
