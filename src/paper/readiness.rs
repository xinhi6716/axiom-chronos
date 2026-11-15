//! Readiness gates backed by current state and artifacts.
use super::{
    citations::CitationReport, evidence, figures::pages::PageReport, integrity::IntegrityReport,
    review, state, tectonic::CompileReport, Disclosure, JournalChoice, Manuscript, PaperState,
    PolicyKind, Step,
};
use crate::{
    base::{atomic_write, Failure, Res},
    seams::sha256_hex,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    FourPasses,
    Citations,
    Numbers,
    Compile,
    Render,
    Disclosure,
    Review,
}
#[derive(Debug, Clone)]
pub struct ReadyStatus {
    pub ready: bool,
    pub conditions: Vec<(Condition, bool, String)>,
}
pub fn final_manuscript_path(store: &Path) -> PathBuf {
    super::paper_dir(store).join("passes/4-fable/manuscript.json")
}
pub fn journal_path(store: &Path) -> PathBuf {
    super::paper_dir(store).join("journal-choice.json")
}
pub fn bibliography_path(store: &Path) -> PathBuf {
    super::paper_dir(store).join("build/refs.bib")
}
pub fn pdf_path(store: &Path) -> PathBuf {
    super::paper_dir(store).join("build/main.pdf")
}
pub fn compile_report_path(store: &Path) -> PathBuf {
    super::paper_dir(store).join("build/compile-report.json")
}
pub fn citation_report_path(store: &Path) -> PathBuf {
    super::paper_dir(store).join("citation-report.json")
}
pub fn numbers_report_path(store: &Path) -> PathBuf {
    super::paper_dir(store).join("numbers-report.json")
}
pub fn render_report_path(store: &Path) -> PathBuf {
    super::paper_dir(store).join("render-report.json")
}
pub fn disclosure_path(store: &Path) -> PathBuf {
    super::paper_dir(store).join("disclosure.json")
}
pub fn rules_path(store: &Path) -> PathBuf {
    super::paper_dir(store).join("journal-rules.md")
}
fn io(e: std::io::Error) -> Failure {
    Failure::new("io", e.to_string())
}
fn write(path: &Path, bytes: &[u8]) -> Res<()> {
    fs::create_dir_all(path.parent().unwrap()).map_err(io)?;
    atomic_write(path, bytes).map_err(io)
}
fn write_json(path: &Path, value: &impl Serialize) -> Res<Vec<u8>> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|e| Failure::new("json", e.to_string()))?;
    write(path, &bytes)?;
    Ok(bytes)
}
fn read_json<T: DeserializeOwned>(path: &Path) -> Res<T> {
    let bytes = fs::read(path).map_err(io)?;
    serde_json::from_slice(&bytes).map_err(|e| Failure::new("json", e.to_string()))
}
fn key(step: Step) -> String {
    serde_json::to_value(step)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}
fn inputs(store: &Path, step: Step) -> Res<BTreeMap<String, String>> {
    let st = state::load(store)?;
    state::upstream(step)
        .iter()
        .map(|&parent| {
            let record = st
                .as_ref()
                .and_then(|s| s.steps.get(&parent))
                .ok_or_else(|| Failure::new("not_ready", format!("{parent:?} is missing")))?;
            Ok((key(parent), record.output.clone()))
        })
        .collect()
}
pub fn record_gate(
    store: &Path,
    step: Step,
    output: &str,
    ok: bool,
    detail: Option<String>,
) -> Res<PaperState> {
    state::record(store, step, inputs(store, step)?, output, ok, detail)
}
pub fn record_journal(store: &Path, choice: &JournalChoice) -> Res<()> {
    let bytes = write_json(&journal_path(store), choice)?;
    record_gate(store, Step::Journal, &sha256_hex(&bytes), true, None)?;
    Ok(())
}
pub fn record_citations(store: &Path, report: &CitationReport, bib_text: &str) -> Res<()> {
    write_json(&citation_report_path(store), report)?;
    write(&bibliography_path(store), bib_text.as_bytes())?;
    let ms: Manuscript = read_json(&final_manuscript_path(store))?;
    let hash = ms.hash();
    let output = sha256_hex(format!("{hash}\0{}", sha256_hex(bib_text.as_bytes())).as_bytes());
    record_gate(
        store,
        Step::Citations,
        &output,
        report.blocking.is_empty() && report.manuscript_hash == hash,
        None,
    )?;
    Ok(())
}
pub fn record_numbers(store: &Path, report: &IntegrityReport) -> Res<()> {
    let issues: Vec<_> = report.issues.iter().map(|i| json!({"location": i.location, "kind": format!("{:?}", i.kind), "detail": i.detail})).collect();
    write_json(
        &numbers_report_path(store),
        &json!({"passed": report.passed, "manuscript_hash": report.manuscript_hash, "evidence_hash": report.evidence_hash, "issues": issues}),
    )?;
    let ms: Manuscript = read_json(&final_manuscript_path(store))?;
    let ev = evidence::load(store)?.ok_or_else(|| Failure::new("not_ready", "evidence missing"))?;
    let output =
        sha256_hex(format!("{}\0{}", report.manuscript_hash, report.evidence_hash).as_bytes());
    let ok = report.passed
        && report.manuscript_hash == ms.hash()
        && report.evidence_hash == ev.hash
        && ev.hash == ev.compute_hash();
    record_gate(store, Step::Numbers, &output, ok, None)?;
    Ok(())
}
fn compile_ok(report: &CompileReport) -> bool {
    report.ok
        && report.pdf.is_some()
        && report.unresolved_citations.is_empty()
        && report.undefined_refs.is_empty()
        && report.missing_files.is_empty()
        && report.missing_chars.is_empty()
}
pub fn record_compile(store: &Path, report: &CompileReport, pdf_bytes: &[u8]) -> Res<()> {
    write_json(&compile_report_path(store), report)?;
    write(&pdf_path(store), pdf_bytes)?;
    record_gate(
        store,
        Step::Compile,
        &sha256_hex(pdf_bytes),
        compile_ok(report),
        None,
    )?;
    Ok(())
}
pub fn record_render(store: &Path, report: &PageReport) -> Res<()> {
    let issues: Vec<_> = report
        .issues
        .iter()
        .map(|i| format!("{}: {:?}: {}", i.target, i.kind, i.detail))
        .collect();
    write_json(
        &render_report_path(store),
        &json!({"pages": report.pages, "vector_figures": report.vector_figures, "issues": issues}),
    )?;
    let _ = inputs(store, Step::Render)?;
    let pdf = fs::read(pdf_path(store)).map_err(io)?;
    record_gate(
        store,
        Step::Render,
        &sha256_hex(&pdf),
        report.issues.is_empty(),
        None,
    )?;
    Ok(())
}
pub fn record_disclosure(store: &Path, disclosure: &Disclosure) -> Res<()> {
    let bytes = write_json(&disclosure_path(store), disclosure)?;
    record_gate(
        store,
        Step::Disclosure,
        &sha256_hex(&bytes),
        disclosure.policy.policy != PolicyKind::Prohibited,
        None,
    )?;
    Ok(())
}

pub fn evaluate(store: &Path, project_root: &Path) -> Res<ReadyStatus> {
    let st = state::load(store)?;
    let check = |step: Step| -> Result<&str, String> {
        let s = st
            .as_ref()
            .ok_or_else(|| "paper state missing".to_owned())?;
        if !state::current(s, step) {
            return Err(format!("{step:?} is not current"));
        }
        Ok(&s.steps[&step].output)
    };
    let bytes = |path: PathBuf| fs::read(&path).map_err(|e| format!("{}: {e}", path.display()));
    let json_file = |path: PathBuf| -> Result<Value, String> {
        serde_json::from_slice(&bytes(path)?).map_err(|e| e.to_string())
    };
    let manuscript = || -> Result<String, String> {
        let ms: Manuscript = serde_json::from_slice(&bytes(final_manuscript_path(store))?)
            .map_err(|e| format!("manuscript: {e}"))?;
        Ok(ms.hash())
    };
    let evidence_hash = || -> Result<String, String> {
        let ev = evidence::load(store)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "evidence missing".to_owned())?;
        if ev.hash != ev.compute_hash() {
            return Err("evidence hash mismatch".into());
        }
        Ok(ev.hash)
    };
    let conditions_checks: Vec<(Condition, Result<(), String>)> = vec![
        (
            Condition::FourPasses,
            (|| {
                for step in [
                    Step::Evidence,
                    Step::Shortlist,
                    Step::Journal,
                    Step::Pass1,
                    Step::Pass2,
                    Step::Pass3,
                    Step::Pass4,
                ] {
                    check(step)?;
                }
                let ev = evidence::load(store)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "evidence missing".to_owned())?;
                if ev.hash != ev.compute_hash() || ev.hash != check(Step::Evidence)? {
                    return Err("evidence hash mismatch".into());
                }
                evidence::verify(&ev, project_root).map_err(|e| e.to_string())?;
                if sha256_hex(&bytes(journal_path(store))?) != check(Step::Journal)? {
                    return Err("journal hash mismatch".into());
                }
                if manuscript()? != check(Step::Pass4)? {
                    return Err("manuscript hash mismatch".into());
                }
                Ok(())
            })(),
        ),
        (
            Condition::Citations,
            (|| {
                let output = check(Step::Citations)?;
                let hash = manuscript()?;
                let bib_hash = sha256_hex(&bytes(bibliography_path(store))?);
                if output != sha256_hex(format!("{hash}\0{bib_hash}").as_bytes()) {
                    return Err("citation hash mismatch".into());
                }
                let report: CitationReport =
                    serde_json::from_slice(&bytes(citation_report_path(store))?)
                        .map_err(|e| format!("citation report: {e}"))?;
                if !report.blocking.is_empty() || report.manuscript_hash != hash {
                    return Err("citation report failed or is stale".into());
                }
                Ok(())
            })(),
        ),
        (
            Condition::Numbers,
            (|| {
                let output = check(Step::Numbers)?;
                let hash = manuscript()?;
                let ev = evidence_hash()?;
                let report = json_file(numbers_report_path(store))?;
                if report.get("passed").and_then(Value::as_bool) != Some(true)
                    || report.get("manuscript_hash").and_then(Value::as_str) != Some(&hash)
                    || report.get("evidence_hash").and_then(Value::as_str) != Some(&ev)
                    || output != sha256_hex(format!("{hash}\0{ev}").as_bytes())
                {
                    return Err("numbers report failed or is stale".into());
                }
                Ok(())
            })(),
        ),
        (
            Condition::Compile,
            (|| {
                let output = check(Step::Compile)?;
                if sha256_hex(&bytes(pdf_path(store))?) != output {
                    return Err("PDF hash mismatch".into());
                }
                let report: CompileReport =
                    serde_json::from_slice(&bytes(compile_report_path(store))?)
                        .map_err(|e| format!("compile report: {e}"))?;
                if !compile_ok(&report) {
                    return Err("compile report failed".into());
                }
                Ok(())
            })(),
        ),
        (
            Condition::Render,
            (|| {
                check(Step::Figures)?;
                let output = check(Step::Render)?;
                if sha256_hex(&bytes(pdf_path(store))?) != output {
                    return Err("render PDF hash mismatch".into());
                }
                let report = json_file(render_report_path(store))?;
                if !report
                    .get("issues")
                    .and_then(Value::as_array)
                    .is_some_and(|v| v.is_empty())
                {
                    return Err("render issues or invalid report".into());
                }
                Ok(())
            })(),
        ),
        (
            Condition::Disclosure,
            (|| {
                let output = check(Step::Disclosure)?;
                let data = bytes(disclosure_path(store))?;
                if sha256_hex(&data) != output {
                    return Err("disclosure hash mismatch".into());
                }
                let report: Disclosure =
                    serde_json::from_slice(&data).map_err(|e| format!("disclosure: {e}"))?;
                if report.policy.policy == PolicyKind::Prohibited {
                    return Err("disclosure policy prohibits use".into());
                }
                Ok(())
            })(),
        ),
        (
            Condition::Review,
            (|| {
                let output = check(Step::Review)?;
                let report: review::ReviewRecord = serde_json::from_slice(&bytes(
                    super::paper_dir(store).join("review/review.json"),
                )?)
                .map_err(|e| format!("review: {e}"))?;
                if report.verdict != "ready"
                    || report
                        .findings
                        .iter()
                        .any(|f| f.get("severity").and_then(Value::as_str) == Some("blocking"))
                {
                    return Err("review is not ready".into());
                }
                if report.input_hash != output
                    || report.input_hash
                        != review::current_input_hash(store).map_err(|e| e.to_string())?
                {
                    return Err("review input hash mismatch".into());
                }
                Ok(())
            })(),
        ),
    ];
    let conditions: Vec<_> = conditions_checks
        .into_iter()
        .map(|(c, result)| match result {
            Ok(()) => (c, true, "ok".into()),
            Err(reason) => (c, false, reason),
        })
        .collect();
    let ready = conditions.iter().all(|c| c.1);
    if !ready
        && st
            .as_ref()
            .is_some_and(|s| s.steps.contains_key(&Step::Ready))
    {
        state::invalidate(store, Step::Ready)?;
    }
    Ok(ReadyStatus { ready, conditions })
}
pub fn mark_ready(store: &Path, project_root: &Path) -> Res<ReadyStatus> {
    let status = evaluate(store, project_root)?;
    if status.ready {
        let map = inputs(store, Step::Ready)?;
        let outputs = state::upstream(Step::Ready)
            .iter()
            .map(|&step| map[&key(step)].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        state::record(
            store,
            Step::Ready,
            map,
            &outputs,
            true,
            None,
        )?;
    }
    Ok(status)
}
pub fn is_ready(store: &Path, project_root: &Path) -> Res<bool> {
    if !evaluate(store, project_root)?.ready {
        return Ok(false);
    }
    Ok(state::load(store)?
        .as_ref()
        .is_some_and(|s| state::current(s, Step::Ready)))
}
