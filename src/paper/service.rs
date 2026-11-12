//! service; owned by module paper-service.
use super::{
    bibliography, budget, citations, crossref, disclosure, evidence,
    figures::{audit, fonts::FontMeasure, pages, plot, presets, tikz},
    http::RealPause,
    integrity,
    journals::{
        doaj, jcr,
        matching::{self, JournalSources},
        openalex::OpenAlex,
        requirements as req, sjr,
    },
    latex,
    readiness::{self, Condition, *},
    review,
    runner::{self, RunConfig},
    state, tables, tectonic, templates, Chooser, Manuscript, PassId, PolicyKind,
};
use super::{
    cache::FetchMode, evidence::BundleInputs, http::Transport, members::PaperMembers,
    readiness::ReadyStatus, EvidenceBundle, JournalChoice, PaperEvent, PassRecord, Requirements,
    Shortlist, SpendEstimate, Step,
};
use super::{citations::CitationReport, tectonic::CompileReport, Disclosure};
use crate::{
    base::{self, Failure, Res},
    seams::{sha256_hex, Clock},
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::{cell::Cell, fs, time::Duration};
use std::{
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
};

#[derive(Debug, Clone)]
pub struct PaperStatus {
    pub steps: Vec<(Step, bool, Option<String>)>,
    pub evidence: Option<String>,
    pub requirements: Option<Requirements>,
    pub shortlist: Option<Shortlist>,
    pub choice: Option<JournalChoice>,
    pub estimate: Option<SpendEstimate>,
    pub passes: Vec<PassRecord>,
    pub ready: Option<ReadyStatus>,
    pub pdf: Option<PathBuf>,
    pub running: bool,
    pub last_error: Option<(String, String)>,
}

pub struct PaperSession {
    pub store: PathBuf,
    pub home: PathBuf,
    pub project_root: PathBuf,
    pub transport: Arc<dyn Transport>,
    pub clock: Arc<dyn Clock>,
    pub timeout_s: u64,
    pub fable_cap_usd: f64,
}

impl PaperSession {
    pub fn open(
        store: &Path,
        data_home: &Path,
        project_root: &Path,
        transport: Arc<dyn Transport>,
        clock: Arc<dyn Clock>,
    ) -> Res<PaperSession> {
        Ok(Self {
            store: store.into(),
            home: data_home.into(),
            project_root: project_root.into(),
            transport,
            clock,
            timeout_s: 1800,
            fable_cap_usd: crate::council::DEFAULT_FABLE_CAP_USD,
        })
    }
    pub fn status(&self) -> Res<PaperStatus> {
        let dir = super::paper_dir(&self.store);
        let st = state::load(&self.store)?;
        let steps = st
            .as_ref()
            .map(|s| {
                s.steps
                    .values()
                    .map(|r| (r.step, state::current(s, r.step), r.detail.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let mut passes = Vec::new();
        for pass in PassId::ALL {
            let path = dir
                .join("passes")
                .join(format!("{}-{}", pass.number(), pass.member()))
                .join("pass-record.json");
            if let Some(record) = read_optional(&path)? {
                passes.push(record);
            }
        }
        let status: Option<Value> = read_optional(&dir.join("status.json"))?;
        let last_error = status
            .as_ref()
            .and_then(|s| s.get("last_error"))
            .and_then(|v| {
                Some((
                    v.get("kind")?.as_str()?.to_owned(),
                    v.get("reason")?.as_str()?.to_owned(),
                ))
            });
        let ready = if st.as_ref().is_some_and(|s| {
            s.steps.contains_key(&Step::Review) || s.steps.contains_key(&Step::Ready)
        }) {
            Some(evaluate_readonly(&self.store, &self.project_root)?)
        } else {
            None
        };
        let pdf_path = readiness::pdf_path(&self.store);
        Ok(PaperStatus {
            steps,
            evidence: evidence::load(&self.store)?.map(|e| e.hash),
            requirements: read_optional(&dir.join("requirements.json"))?,
            shortlist: read_optional(&dir.join("shortlist.json"))?,
            choice: read_optional(&readiness::journal_path(&self.store))?,
            estimate: read_optional(&dir.join("estimate.json"))?,
            passes,
            ready,
            pdf: pdf_path.is_file().then_some(pdf_path),
            running: status
                .as_ref()
                .and_then(|s| s.get("running"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            last_error,
        })
    }
    pub fn build_evidence(&self, inputs: &BundleInputs) -> Res<EvidenceBundle> {
        let bundle = evidence::build(inputs)?;
        evidence::save(&self.store, &bundle)?;
        readiness::record_gate(&self.store, Step::Evidence, &bundle.hash, true, None)?;
        Ok(bundle)
    }
    pub fn shortlist(
        &self,
        requirements: &Requirements,
        jcr: Option<&Path>,
        mode: FetchMode,
    ) -> Res<Shortlist> {
        req::validate(requirements)?;
        let cache = self.cache()?;
        let api = OpenAlex::new(&cache, self.transport.as_ref(), mode, &self.home)?;
        let now = timestamp(self.clock.now_ms());
        let year = now[..4]
            .parse::<i32>()
            .map_err(|e| Failure::new("time", e.to_string()))?;
        let table = sjr::load_year(&cache, self.transport.as_ref(), mode, year)?;
        let jcr_data = match jcr {
            Some(path) => {
                let data = jcr::import(path)?;
                jcr::save(&self.home, &data)?;
                Some(data)
            }
            None => jcr::load_latest(&self.home)?,
        };
        let lookup = |id: &str| doaj::journal(&cache, self.transport.as_ref(), mode, id);
        let sources = JournalSources {
            openalex: &api,
            sjr: &table,
            doaj: &lookup,
            jcr: jcr_data.as_ref(),
        };
        let (journals, works) = matching::gather(requirements, &sources, 100)?;
        let list = matching::rank(requirements, &journals, &works, &now);
        let dir = super::paper_dir(&self.store);
        write_json(&dir.join("requirements.json"), requirements)?;
        write_json(&dir.join("shortlist.json"), &list)?;
        readiness::record_gate(&self.store, Step::Shortlist, &list.hash, true, None)?;
        Ok(list)
    }
    pub fn choose_journal(&self, journal_id: &str, shortlist_hash: &str) -> Res<JournalChoice> {
        let list: Shortlist = read_optional(&super::paper_dir(&self.store).join("shortlist.json"))?
            .ok_or_else(|| Failure::new("stale_shortlist", "shortlist is missing"))?;
        if list.hash != shortlist_hash {
            return Err(Failure::new("stale_shortlist", "shortlist hash changed"));
        }
        if !list.ranked.iter().any(|j| j.journal.id == journal_id) {
            return Err(Failure::new(
                "not_in_shortlist",
                "journal is not in the ranked shortlist",
            ));
        }
        let choice = JournalChoice {
            journal_id: journal_id.into(),
            shortlist_hash: shortlist_hash.into(),
            chosen_by: Chooser::Person,
            at: timestamp(self.clock.now_ms()),
        };
        readiness::record_journal(&self.store, &choice)?;
        Ok(choice)
    }
    pub fn estimate(&self) -> Res<SpendEstimate> {
        let estimate = budget::estimate(&self.home, &models())?;
        write_json(
            &super::paper_dir(&self.store).join("estimate.json"),
            &estimate,
        )?;
        Ok(estimate)
    }
    pub fn write(
        &self,
        m: &PaperMembers,
        estimate_digest: &str,
        stop: &AtomicBool,
        on_event: &dyn Fn(&PaperEvent),
    ) -> Res<ReadyStatus> {
        let mut step = Step::Journal;
        let blocked = Cell::new(false);
        let checked = Cell::new(None);
        let emit = |event: &PaperEvent| {
            match event {
                PaperEvent::Blocked { .. } => blocked.set(true),
                PaperEvent::Check { step, .. } => checked.set(Some(*step)),
                _ => {}
            }
            on_event(event);
        };
        write_status(&self.store, true, None)?;
        let result = (|| -> Res<ReadyStatus> {
            let st = state::load(&self.store)?;
            let choice: JournalChoice = read_optional(&readiness::journal_path(&self.store))?
                .filter(|c: &JournalChoice| {
                    st.as_ref().is_some_and(|s| {
                        state::current(s, Step::Journal)
                            && s.steps
                                .get(&Step::Shortlist)
                                .is_some_and(|r| r.output == c.shortlist_hash)
                    }) && fs::read(readiness::journal_path(&self.store))
                        .ok()
                        .is_some_and(|b| {
                            st.as_ref().unwrap().steps[&Step::Journal].output == sha256_hex(&b)
                        })
                })
                .ok_or_else(|| Failure::new("no_journal", "current journal choice is missing"))?;
            let list: Shortlist =
                read_optional(&super::paper_dir(&self.store).join("shortlist.json"))?
                    .ok_or_else(|| Failure::new("no_journal", "shortlist missing"))?;
            let journal = list
                .ranked
                .iter()
                .find(|j| j.journal.id == choice.journal_id && list.hash == choice.shortlist_hash)
                .map(|j| &j.journal)
                .ok_or_else(|| {
                    Failure::new("no_journal", "chosen journal not in current shortlist")
                })?;
            if budget::estimate(&self.home, &models())?.digest != estimate_digest {
                return Err(Failure::new("estimate_changed", "estimate has changed"));
            }
            let template = templates::for_journal(journal)?;
            let rules = format!(
                "Journal:
{}

Template:
{}
",
                serde_json::to_string_pretty(journal).map_err(json_error)?,
                serde_json::to_string_pretty(&template).map_err(json_error)?
            );
            write_bytes(&readiness::rules_path(&self.store), rules.as_bytes())?;
            let cfg = RunConfig {
                store: self.store.clone(),
                home: self.home.clone(),
                project_root: self.project_root.clone(),
                timeout_s: self.timeout_s,
                fable_cap_usd: self.fable_cap_usd,
            };
            step = Step::Pass1;
            let passes = runner::run_passes(&cfg, m, journal, &rules, stop, &emit)?;
            let manuscript: Manuscript =
                read_optional(&readiness::final_manuscript_path(&self.store))?
                    .ok_or_else(|| Failure::new("not_ready", "manuscript missing"))?;
            let bundle = evidence::load(&self.store)?
                .ok_or_else(|| Failure::new("not_ready", "evidence missing"))?;
            let build = super::paper_dir(&self.store).join("build");
            fs::create_dir_all(&build).map_err(io)?;
            let duration = Duration::from_secs(self.timeout_s);
            step = Step::Figures;
            let mut rendered_figures = Vec::new();
            let mut rendered_tables = Vec::new();
            let mut issues = Vec::new();
            for (index, spec) in manuscript.figures.iter().enumerate() {
                let preset = presets::preset(&spec.preset)?;
                let measure = FontMeasure::new(&preset);
                let data = evidence::load_table(&bundle, &self.project_root, &spec.artifact)?;
                let layout = plot::layout(spec, &data, &preset, &measure)?;
                issues.extend(audit::audit_figure(&layout, &preset, &measure));
                if issues.is_empty() {
                    let tex = build.join(format!("figure-{index}.tex"));
                    write_bytes(&tex, tikz::standalone(&layout, &preset).as_bytes())?;
                    let t = tectonic::find(&self.home)?;
                    let pdf = tectonic::compile_figure(&t, &tex, true, duration)?;
                    rendered_figures.push((spec.id.clone(), pdf));
                }
            }
            for spec in &manuscript.tables {
                let preset = presets::presets()?
                    .into_iter()
                    .next()
                    .ok_or_else(|| Failure::new("preset_asset", "no figure preset"))?;
                let measure = FontMeasure::new(&preset);
                let data = evidence::load_table(&bundle, &self.project_root, &spec.artifact)?;
                let table =
                    tables::booktabs(spec, &data, preset.width_mm, &measure, &preset.font_family)?;
                issues.extend(audit::audit_table(&table, preset.width_mm));
                rendered_tables.push(table);
            }
            let detail = format!("{} figure and table audit issues", issues.len());
            readiness::record_gate(
                &self.store,
                step,
                &sha256_hex(format!("{}:{issues:?}", manuscript.hash()).as_bytes()),
                issues.is_empty(),
                Some(detail.clone()),
            )?;
            check(step, issues.is_empty(), &detail, &emit)?;
            step = Step::Citations;
            let cache = self.cache()?;
            let api = OpenAlex::new(
                &cache,
                self.transport.as_ref(),
                FetchMode::Online,
                &self.home,
            )?;
            let cr =
                |id: &str| crossref::work(&cache, self.transport.as_ref(), FetchMode::Online, id);
            let oa = |id: &str| api.work(id);
            let bib = bibliography::from_seeds(&bundle.references, &|seed| match &seed.doi {
                Some(doi) => cr(doi),
                None => match &seed.openalex {
                    Some(id) => oa(id),
                    None => Ok(None),
                },
            });
            let report = citations::verify_manuscript(&manuscript, &bib, &cr, &oa);
            readiness::record_citations(&self.store, &report, &bibliography::to_bibtex(&bib))?;
            check(
                step,
                report.blocking.is_empty(),
                &report.blocking.join("; "),
                &emit,
            )?;
            step = Step::Numbers;
            let numbers =
                integrity::check(&manuscript, &bundle, &rendered_tables, &self.project_root)?;
            readiness::record_numbers(&self.store, &numbers)?;
            check(
                step,
                numbers.passed,
                &format!("{:?}", numbers.issues),
                &emit,
            )?;
            step = Step::Compile;
            let staging = build.join(format!(".assemble-{}", base::unique()));
            let assembled = (|| -> Res<()> {
                let project = latex::assemble(
                    &staging,
                    &manuscript,
                    &bundle,
                    &template,
                    &rendered_figures,
                    &rendered_tables,
                    &bib,
                    None,
                )?;
                for file in &project.files {
                    let name = file
                        .file_name()
                        .ok_or_else(|| Failure::new("io", "missing assembled file name"))?;
                    write_bytes(&build.join(name), &fs::read(file).map_err(io)?)?;
                }
                Ok(())
            })();
            if staging.exists() {
                fs::remove_dir_all(&staging).map_err(io)?;
            }
            assembled?;
            let t = tectonic::find(&self.home)?;
            let compiled = tectonic::compile(&t, &build.join("main.tex"), true, duration)?;
            let pdf = compiled
                .pdf
                .as_ref()
                .and_then(|p| fs::read(p).ok())
                .unwrap_or_default();
            readiness::record_compile(&self.store, &compiled, &pdf)?;
            let compile_ok = compiled.ok
                && compiled.pdf.is_some()
                && compiled.unresolved_citations.is_empty()
                && compiled.undefined_refs.is_empty()
                && compiled.missing_files.is_empty()
                && compiled.missing_chars.is_empty();
            check(step, compile_ok, &format!("{:?}", compiled), &emit)?;
            step = Step::Render;
            let page_report =
                pages::check_pdf(&readiness::pdf_path(&self.store), &compiled.log, None)?;
            readiness::record_render(&self.store, &page_report)?;
            check(
                step,
                page_report.issues.is_empty(),
                &format!("{:?}", page_report.issues),
                &emit,
            )?;
            step = Step::Disclosure;
            let disclosure = disclosure::generate(&passes, None, journal)?;
            readiness::record_disclosure(&self.store, &disclosure)?;
            check(
                step,
                disclosure.policy.policy != PolicyKind::Prohibited,
                "disclosure policy",
                &emit,
            )?;
            step = Step::Review;
            let run_id = state::load(&self.store)?
                .ok_or_else(|| Failure::new("not_ready", "state missing"))?
                .run_id;
            let mut review_budget =
                budget::PaperBudget::open(&self.store, &run_id, self.fable_cap_usd)?;
            let record = review::review(&cfg, m.reviewer, &mut review_budget, stop)?;
            check(step, true, &record.verdict, &emit)?;
            let ready = readiness::mark_ready(&self.store, &self.project_root)?;
            if ready.ready {
                on_event(&PaperEvent::Ready);
            }
            Ok(ready)
        })();
        if let Err(ref e) = result {
            if step >= Step::Figures && step <= Step::Review && checked.get() != Some(step) {
                emit(&PaperEvent::Check {
                    step,
                    ok: false,
                    detail: e.message.clone(),
                });
                if step != Step::Review {
                    let _ = readiness::record_gate(
                        &self.store,
                        step,
                        "",
                        false,
                        Some(e.message.clone()),
                    );
                }
            }
            if !blocked.get() {
                emit(&PaperEvent::Blocked {
                    step,
                    kind: e.kind.clone(),
                    reason: e.message.clone(),
                });
            }
        }
        write_status(&self.store, false, result.as_ref().err())?;
        result
    }
    fn cache(&self) -> Res<super::cache::Cache> {
        super::cache::Cache::open(
            &self.home.join("paper/cache"),
            self.clock.clone(),
            Arc::new(RealPause),
        )
    }
}

fn evaluate_readonly(store: &Path, project_root: &Path) -> Res<ReadyStatus> {
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
    Ok(ReadyStatus { ready, conditions })
}

fn compile_ok(report: &tectonic::CompileReport) -> bool {
    report.ok
        && report.pdf.is_some()
        && report.unresolved_citations.is_empty()
        && report.undefined_refs.is_empty()
        && report.missing_files.is_empty()
        && report.missing_chars.is_empty()
}
fn io(e: std::io::Error) -> Failure {
    Failure::new("io", e.to_string())
}
fn json_error(e: serde_json::Error) -> Failure {
    Failure::new("json", e.to_string())
}
fn write_bytes(path: &Path, bytes: &[u8]) -> Res<()> {
    fs::create_dir_all(path.parent().unwrap()).map_err(io)?;
    base::atomic_write(path, bytes).map_err(io)
}
fn write_json(path: &Path, value: &impl Serialize) -> Res<()> {
    write_bytes(path, &serde_json::to_vec_pretty(value).map_err(json_error)?)
}
fn read_optional<T: DeserializeOwned>(path: &Path) -> Res<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(json_error),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io(e)),
    }
}
fn models() -> Vec<String> {
    ["astra", "fable", "argon"]
        .iter()
        .map(|s| (*s).into())
        .collect()
}
fn write_status(store: &Path, running: bool, error: Option<&Failure>) -> Res<()> {
    write_json(
        &super::paper_dir(store).join("status.json"),
        &json!({"running": running, "last_error": error.map(|e| json!({"kind": e.kind, "reason": e.message}))}),
    )
}
fn check(step: Step, ok: bool, detail: &str, event: &dyn Fn(&PaperEvent)) -> Res<()> {
    event(&PaperEvent::Check {
        step,
        ok,
        detail: detail.into(),
    });
    if ok {
        Ok(())
    } else {
        let kind = match step {
            Step::Figures => "figures_blocked",
            Step::Citations => "citations_blocked",
            Step::Numbers => "numbers_blocked",
            Step::Compile => "compile_blocked",
            Step::Render => "render_blocked",
            Step::Disclosure => "disclosure_blocked",
            Step::Review => "review_blocked",
            _ => "blocked",
        };
        Err(Failure::new(kind, detail))
    }
}

fn timestamp(ms: i64) -> String {
    let seconds = ms.div_euclid(1000);
    let days = seconds.div_euclid(86400);
    let sec = seconds.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    y += if m <= 2 { 1 } else { 0 };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        sec / 3600,
        (sec % 3600) / 60,
        sec % 60
    )
}
