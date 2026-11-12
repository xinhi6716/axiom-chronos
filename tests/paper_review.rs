use axiom::{
    base::Res,
    council::Status,
    engine::policy,
    paper::{
        bibliography::{self, BibEntry, Bibliography},
        budget::PaperBudget,
        cache::{Cache, FetchMode},
        citations::{self, CitationReport},
        crossref,
        evidence::{self, BundleInputs},
        figures::{
            audit::{AuditIssue, IssueKind},
            pages::PageReport,
        },
        http::{CannedTransport, ClockPause},
        integrity::{self, IntegrityIssue, IntegrityKind, IntegrityReport},
        journals::openalex::OpenAlex,
        readiness::{self, Condition, ReadyStatus},
        review,
        runner::RunConfig,
        state,
        tectonic::CompileReport,
        *,
    },
    research_program::{
        self,
        budget::BudgetState,
        dossier,
        investigation::{MemberCall, MemberReply, ResearchMember},
    },
    seams::FakeClock,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tempfile::{tempdir, TempDir};

const PASSES: [&str; 4] = [
    include_str!("fixtures/paper/runner/pass1.json"),
    include_str!("fixtures/paper/runner/pass2.json"),
    include_str!("fixtures/paper/runner/pass3.json"),
    include_str!("fixtures/paper/runner/pass4.json"),
];
const PDF: &[u8] = b"%PDF-1.5 fake";

struct Fake {
    reply: MemberReply,
    calls: Mutex<Vec<MemberCall>>,
    cancel: bool,
}
impl Fake {
    fn new(verdict: &str, findings: Value) -> Self {
        Self {
            reply: MemberReply {
                status: Status::Ok,
                output: Some(json!({"verdict":verdict,"findings":findings,"checked":[]})),
                raw: Some("reviewer raw".into()),
                model: Some("review-model".into()),
                provider: "fake".into(),
                usd: Some(0.1),
                tokens: 10,
                detail: None,
            },
            calls: Mutex::new(vec![]),
            cancel: false,
        }
    }
}
impl ResearchMember for Fake {
    fn name(&self) -> &str {
        "reviewer"
    }
    fn call(&self, call: &MemberCall, stop: &AtomicBool) -> MemberReply {
        self.calls.lock().unwrap().push(call.clone());
        if self.cancel {
            stop.store(true, Ordering::SeqCst);
        }
        self.reply.clone()
    }
}
struct Case {
    _dir: TempDir,
    cfg: RunConfig,
    bundle: EvidenceBundle,
    ms: Manuscript,
    stop: AtomicBool,
}
impl Case {
    fn new() -> Self {
        let dir = tempdir().unwrap();
        let root = dir.path().join("project");
        let store = root.join("store");
        fs::create_dir_all(research_program::program_dir(&store)).unwrap();
        let dossier: dossier::Dossier =
            serde_json::from_str(include_str!("fixtures/paper/evidence/dossier.json")).unwrap();
        dossier::write(
            &research_program::program_dir(&store).join("02-dossier.json"),
            &dossier,
        )
        .unwrap();
        fs::write(
            root.join("results.json"),
            include_bytes!("fixtures/paper/evidence/results.json"),
        )
        .unwrap();
        fs::write(
            root.join("data.csv"),
            include_bytes!("fixtures/paper/evidence/data.csv"),
        )
        .unwrap();
        fs::write(
            root.join("model.txt"),
            include_bytes!("fixtures/paper/evidence/model.txt"),
        )
        .unwrap();
        let bundle = evidence::build(&BundleInputs {
            store: store.clone(),
            project_root: root.clone(),
            results: vec![root.join("results.json")],
            datasets: vec![root.join("data.csv")],
            models: vec![root.join("model.txt")],
        })
        .unwrap();
        evidence::save(&store, &bundle).unwrap();
        state::record(
            &store,
            Step::Evidence,
            BTreeMap::new(),
            &bundle.hash,
            true,
            None,
        )
        .unwrap();
        state::record(
            &store,
            Step::Shortlist,
            BTreeMap::new(),
            "shortlist",
            true,
            None,
        )
        .unwrap();
        readiness::record_journal(
            &store,
            &JournalChoice {
                journal_id: "journal".into(),
                shortlist_hash: "shortlist".into(),
                chosen_by: Chooser::Person,
                at: "now".into(),
            },
        )
        .unwrap();
        let ms = serde_json::from_str::<PassOutput>(PASSES[3])
            .unwrap()
            .manuscript;
        for (i, step) in [Step::Pass1, Step::Pass2, Step::Pass3, Step::Pass4]
            .into_iter()
            .enumerate()
        {
            let manuscript = serde_json::from_str::<PassOutput>(PASSES[i])
                .unwrap()
                .manuscript;
            let st = state::load(&store).unwrap().unwrap();
            let inputs = state::upstream(step)
                .iter()
                .map(|p| {
                    (
                        serde_json::to_value(p)
                            .unwrap()
                            .as_str()
                            .unwrap()
                            .to_owned(),
                        st.steps[p].output.clone(),
                    )
                })
                .collect();
            state::record(&store, step, inputs, &manuscript.hash(), true, None).unwrap();
            let pass_dir = paper_dir(&store).join("passes").join(format!(
                "{}-{}",
                i + 1,
                PassId::ALL[i].member()
            ));
            fs::create_dir_all(&pass_dir).unwrap();
            fs::write(
                pass_dir.join("manuscript.json"),
                serde_json::to_vec_pretty(&manuscript).unwrap(),
            )
            .unwrap();
            for (name, text) in [
                ("prompt.txt", format!("secret-pass-prompt-{i}")),
                ("raw.txt", format!("secret-pass-raw-{i}")),
                ("pass-record.json", format!("secret-pass-record-{i}")),
            ] {
                fs::write(pass_dir.join(name), text).unwrap();
            }
        }
        let cfg = RunConfig {
            store,
            home: dir.path().join("home"),
            project_root: root,
            timeout_s: 71,
            fable_cap_usd: 5.0,
        };
        let c = Self {
            _dir: dir,
            cfg,
            bundle,
            ms,
            stop: AtomicBool::new(false),
        };
        readiness::record_gate(c.store(), Step::Figures, "figures", true, None).unwrap();
        readiness::record_citations(
            c.store(),
            &c.citation(true),
            "@article{example,title={Example}}\n",
        )
        .unwrap();
        readiness::record_numbers(c.store(), &c.numbers(true)).unwrap();
        readiness::record_compile(c.store(), &compile(true), PDF).unwrap();
        readiness::record_render(c.store(), &pages(false)).unwrap();
        readiness::record_disclosure(c.store(), &disclosure(PolicyKind::Allowed)).unwrap();
        fs::write(
            paper_dir(c.store()).join("journal-rules.md"),
            "Test journal rules",
        )
        .unwrap();
        c
    }
    fn store(&self) -> &Path {
        &self.cfg.store
    }
    fn budget(&self) -> PaperBudget {
        PaperBudget::open(self.store(), "review-test", 5.0).unwrap()
    }
    fn run_review(&self, f: &Fake) -> Res<review::ReviewRecord> {
        review::review(&self.cfg, f, &mut self.budget(), &self.stop)
    }
    fn ready_review(&self) {
        self.run_review(&Fake::new("ready", json!([]))).unwrap();
    }
    fn evaluate(&self) -> ReadyStatus {
        readiness::evaluate(self.store(), &self.cfg.project_root).unwrap()
    }
    fn citation(&self, ok: bool) -> CitationReport {
        CitationReport {
            checks: vec![],
            unknown_keys: vec![],
            blocking: if ok {
                vec![]
            } else {
                vec!["bad citation".into()]
            },
            manuscript_hash: self.ms.hash(),
        }
    }
    fn numbers(&self, ok: bool) -> IntegrityReport {
        IntegrityReport {
            issues: if ok {
                vec![]
            } else {
                vec![IntegrityIssue {
                    location: "abstract".into(),
                    kind: IntegrityKind::UnsourcedNumber,
                    detail: "37%".into(),
                }]
            },
            passed: ok,
            manuscript_hash: self.ms.hash(),
            evidence_hash: self.bundle.hash.clone(),
        }
    }
    fn no_ready(&self, condition: Condition) {
        let status = self.evaluate();
        assert!(!status.ready, "{status:?}");
        assert!(
            !status
                .conditions
                .iter()
                .find(|x| x.0 == condition)
                .unwrap()
                .1,
            "{status:?}"
        );
        assert!(
            !readiness::mark_ready(self.store(), &self.cfg.project_root)
                .unwrap()
                .ready
        );
        assert!(!state::load(self.store())
            .unwrap()
            .unwrap()
            .steps
            .contains_key(&Step::Ready));
    }
    fn assert_charge(&self) {
        let budget: BudgetState =
            serde_json::from_slice(&fs::read(paper_dir(self.store()).join("budget.json")).unwrap())
                .unwrap();
        assert_eq!(budget.spent["reviewer"].priced_tokens, 10);
    }
    fn no_review(&self) {
        assert!(!paper_dir(self.store()).join("review/review.json").exists());
        assert!(!state::load(self.store())
            .unwrap()
            .unwrap()
            .steps
            .contains_key(&Step::Review));
    }
}
fn compile(ok: bool) -> CompileReport {
    CompileReport {
        pdf: Some("main.pdf".into()),
        log: "main.log".into(),
        ok,
        unresolved_citations: vec![],
        undefined_refs: vec![],
        missing_files: vec![],
        missing_chars: vec![],
    }
}
fn pages(bad: bool) -> PageReport {
    PageReport {
        pages: 1,
        vector_figures: true,
        issues: if bad {
            vec![AuditIssue {
                target: "page".into(),
                kind: IssueKind::PageRaster,
                detail: "raster".into(),
            }]
        } else {
            vec![]
        },
    }
}
fn disclosure(policy: PolicyKind) -> Disclosure {
    Disclosure {
        text: "Disclosure".into(),
        placement: "end".into(),
        uses: vec![],
        policy: AiPolicy {
            publisher: "publisher".into(),
            url: "url".into(),
            fetched_at: "now".into(),
            sha256: "hash".into(),
            policy,
            placement: "end".into(),
            required_items: vec![],
        },
    }
}
fn assert_ok(status: &ReadyStatus) {
    assert!(status.ready, "{status:?}");
    assert_eq!(
        status.conditions.iter().map(|x| x.0).collect::<Vec<_>>(),
        vec![
            Condition::FourPasses,
            Condition::Citations,
            Condition::Numbers,
            Condition::Compile,
            Condition::Render,
            Condition::Disclosure,
            Condition::Review
        ]
    );
    assert!(status.conditions.iter().all(|x| x.1 && x.2 == "ok"));
}
#[test]
fn happy_path() {
    let c = Case::new();
    let record = c.run_review(&Fake::new("ready", json!([]))).unwrap();
    assert_eq!(
        record.input_hash,
        review::current_input_hash(c.store()).unwrap()
    );
    assert_eq!(record.model.as_deref(), Some("review-model"));
    assert_eq!(record.usd, Some(0.1));
    assert_eq!(record.tokens, 10);
    let saved: review::ReviewRecord =
        serde_json::from_slice(&fs::read(paper_dir(c.store()).join("review/review.json")).unwrap())
            .unwrap();
    assert_eq!(saved, record);
    let state = state::load(c.store()).unwrap().unwrap();
    let review_step = &state.steps[&Step::Review];
    assert_eq!(review_step.output, record.input_hash);
    assert_eq!(review_step.detail.as_deref(), Some("ready"));
    for step in state::upstream(Step::Review) {
        let key = serde_json::to_value(step)
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(review_step.inputs[&key], state.steps[step].output);
    }
    assert_ok(&c.evaluate());
    assert_ok(&readiness::mark_ready(c.store(), &c.cfg.project_root).unwrap());
    assert!(state::load(c.store())
        .unwrap()
        .unwrap()
        .steps
        .contains_key(&Step::Ready));
    assert!(readiness::is_ready(c.store(), &c.cfg.project_root).unwrap());
}
#[test]
fn failing_citations_block() {
    let c = Case::new();
    c.ready_review();
    readiness::record_citations(c.store(), &c.citation(false), "bad").unwrap();
    c.no_ready(Condition::Citations);
}
#[test]
fn unsourced_numbers_block() {
    let c = Case::new();
    c.ready_review();
    readiness::record_numbers(c.store(), &c.numbers(false)).unwrap();
    c.no_ready(Condition::Numbers);
}
#[test]
fn failing_compile_blocks() {
    let c = Case::new();
    c.ready_review();
    readiness::record_compile(c.store(), &compile(false), PDF).unwrap();
    c.no_ready(Condition::Compile);
}
#[test]
fn render_issue_blocks() {
    let c = Case::new();
    c.ready_review();
    readiness::record_render(c.store(), &pages(true)).unwrap();
    c.no_ready(Condition::Render);
}
#[test]
fn prohibited_disclosure_blocks() {
    let c = Case::new();
    c.ready_review();
    readiness::record_disclosure(c.store(), &disclosure(PolicyKind::Prohibited)).unwrap();
    c.no_ready(Condition::Disclosure);
}
#[test]
fn missing_pass_blocks() {
    let c = Case::new();
    c.ready_review();
    let mut st = state::load(c.store()).unwrap().unwrap();
    st.steps.remove(&Step::Pass2);
    state::save(c.store(), &st).unwrap();
    c.no_ready(Condition::FourPasses);
}
#[test]
fn revise_verdict_blocks() {
    let c = Case::new();
    c.run_review(&Fake::new("revise", json!([]))).unwrap();
    c.no_ready(Condition::Review);
}
#[test]
fn blocking_finding_blocks() {
    let c = Case::new();
    c.run_review(&Fake::new("ready", json!([{ "location":"abstract", "severity":"blocking", "category":"clarity", "issue":"issue", "evidence":"evidence" }]))).unwrap();
    c.no_ready(Condition::Review);
}
fn invalidate_file(
    path: fn(&Path) -> PathBuf,
    bytes: &[u8],
    condition: Condition,
    review_false: bool,
) {
    let c = Case::new();
    c.ready_review();
    readiness::mark_ready(c.store(), &c.cfg.project_root).unwrap();
    fs::write(path(c.store()), bytes).unwrap();
    c.no_ready(condition);
    if review_false {
        assert!(
            !c.evaluate()
                .conditions
                .iter()
                .find(|x| x.0 == Condition::Review)
                .unwrap()
                .1
        );
    }
}
#[test]
fn manuscript_edit_invalidates_ready_and_review() {
    let c = Case::new();
    c.ready_review();
    readiness::mark_ready(c.store(), &c.cfg.project_root).unwrap();
    let mut ms = c.ms.clone();
    ms.abstract_text.text.push_str(" Changed.");
    fs::write(
        readiness::final_manuscript_path(c.store()),
        serde_json::to_vec(&ms).unwrap(),
    )
    .unwrap();
    c.no_ready(Condition::FourPasses);
    assert!(
        !c.evaluate()
            .conditions
            .iter()
            .find(|x| x.0 == Condition::Review)
            .unwrap()
            .1
    );
}
#[test]
fn evidence_file_edit_invalidates_ready() {
    let c = Case::new();
    c.ready_review();
    readiness::mark_ready(c.store(), &c.cfg.project_root).unwrap();
    fs::write(c.cfg.project_root.join("data.csv"), b"changed").unwrap();
    c.no_ready(Condition::FourPasses);
}
#[test]
fn bibliography_edit_invalidates_ready() {
    invalidate_file(
        readiness::bibliography_path,
        b"changed",
        Condition::Citations,
        false,
    );
}
#[test]
fn journal_edit_invalidates_ready() {
    invalidate_file(
        readiness::journal_path,
        b"changed",
        Condition::FourPasses,
        false,
    );
}
#[test]
fn pdf_edit_invalidates_ready() {
    invalidate_file(readiness::pdf_path, b"changed", Condition::Compile, false);
}
#[test]
fn reviewer_is_isolated() {
    let c = Case::new();
    let f = Fake::new("ready", json!([]));
    c.run_review(&f).unwrap();
    let calls = f.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    let input = paper_dir(c.store()).join("review/input");
    assert_eq!(call.member, "reviewer");
    assert_eq!(call.stage, "paper-review");
    assert_eq!(call.readable, vec![input.clone()]);
    assert_eq!(call.out_dir, paper_dir(c.store()).join("review"));
    assert_eq!(call.timeout_s, c.cfg.timeout_s);
    assert_eq!(
        call.schema,
        serde_json::from_str::<Value>(include_str!("../engine/paper/reviewer.schema.json"))
            .unwrap()
    );
    let files = review::input_files(c.store()).unwrap();
    assert_eq!(
        files.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(),
        vec![
            "manuscript.json",
            "evidence.json",
            "journal-rules.md",
            "citation-report.json",
            "numbers-report.json",
            "compile-report.json",
            "manuscript.pdf"
        ]
    );
    let mut names = fs::read_dir(&input)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    let mut expected = files.iter().map(|x| x.0.clone()).collect::<Vec<_>>();
    expected.sort();
    assert_eq!(names, expected);
    for (name, source) in &files {
        assert_eq!(
            fs::read(input.join(name)).unwrap(),
            fs::read(source).unwrap()
        );
    }
    let mut expected_prompt = format!(
        "{}\n\n",
        axiom::engine::asset("paper/prompts/review.md").unwrap()
    );
    for (name, source) in &files {
        let bytes = fs::read(source).unwrap();
        if name == "manuscript.pdf" {
            expected_prompt.push_str(&format!(
                "=== manuscript.pdf === sha256 {}, file in the input directory",
                axiom::seams::sha256_hex(&bytes)
            ));
        } else {
            expected_prompt.push_str(&format!(
                "=== {name} ===\n{}\n",
                String::from_utf8(bytes).unwrap()
            ));
        }
    }
    assert_eq!(
        call.prompt.trim_end_matches('\n'),
        expected_prompt.trim_end_matches('\n')
    );
    assert!(call
        .prompt
        .contains(axiom::engine::asset("paper/prompts/review.md").unwrap()));
    assert!(call
        .prompt
        .contains(&fs::read_to_string(readiness::final_manuscript_path(c.store())).unwrap()));
    for i in 0..4 {
        for part in ["prompt", "raw", "record"] {
            assert!(!call.prompt.contains(&format!("secret-pass-{part}-{i}")));
        }
    }
}
#[test]
fn reviewer_charge_recorded() {
    let c = Case::new();
    c.ready_review();
    c.assert_charge();
}
#[test]
fn budget_refusal_never_calls() {
    let c = Case::new();
    policy::set(c.store(), "budget.reviewer", "0").unwrap();
    let f = Fake::new("ready", json!([]));
    assert_eq!(c.run_review(&f).unwrap_err().kind, "budget_exceeded");
    assert!(f.calls.lock().unwrap().is_empty());
    c.no_review();
}
fn failed_review(mut f: Fake, kind: &str, cancel: bool) {
    let c = Case::new();
    f.cancel = cancel;
    assert_eq!(c.run_review(&f).unwrap_err().kind, kind);
    assert_eq!(f.calls.lock().unwrap().len(), 1);
    c.no_review();
    c.assert_charge();
}
#[test]
fn cancellation_during_call_charges_without_record() {
    failed_review(Fake::new("ready", json!([])), "cancelled", true);
}
#[test]
fn member_failure_charges_without_record() {
    let mut f = Fake::new("ready", json!([]));
    f.reply.status = Status::Failed;
    failed_review(f, "member_failed", false);
}
#[test]
fn malformed_output_charges_without_record() {
    let mut f = Fake::new("ready", json!([]));
    f.reply.output = Some(json!({"verdict":"ready"}));
    failed_review(f, "malformed", false);
}
#[test]
fn missing_compile_prevents_review_call() {
    let c = Case::new();
    state::invalidate(c.store(), Step::Compile).unwrap();
    let f = Fake::new("ready", json!([]));
    assert_eq!(c.run_review(&f).unwrap_err().kind, "not_ready");
    assert!(f.calls.lock().unwrap().is_empty());
    c.no_review();
}
fn seeded(bad_citation: bool, bad_number: bool) {
    let mut c = Case::new();
    c.ms.abstract_text.text = "Prior work [@10-1234-example].".into();
    c.ms.sections[0].paragraphs[0].text = if bad_citation {
        "A fake source [@w123456789]."
    } else if bad_number {
        "A 37% gain."
    } else {
        "The result improved."
    }
    .into();
    fs::write(
        readiness::final_manuscript_path(c.store()),
        serde_json::to_vec_pretty(&c.ms).unwrap(),
    )
    .unwrap();
    let st = state::load(c.store()).unwrap().unwrap();
    let inputs = state::upstream(Step::Pass4)
        .iter()
        .map(|p| {
            (
                serde_json::to_value(p)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned(),
                st.steps[p].output.clone(),
            )
        })
        .collect();
    state::record(c.store(), Step::Pass4, inputs, &c.ms.hash(), true, None).unwrap();
    readiness::record_gate(c.store(), Step::Figures, "figures", true, None).unwrap();
    let article = BibEntry {
        key: "10-1234-example".into(),
        kind: "article".into(),
        doi: Some("10.1038/nature14539".into()),
        openalex: None,
        title: "Deep learning".into(),
        authors: vec![
            "LeCun, Yann".into(),
            "Yoshua Bengio".into(),
            "Geoffrey Hinton".into(),
        ],
        year: Some(2015),
        venue: Some("Nature".into()),
        volume: None,
        issue: None,
        pages: Some("436-444".into()),
        article_number: None,
    };
    let mut fake = article.clone();
    fake.key = "w123456789".into();
    fake.doi = Some("10.9999/fake".into());
    let bib = Bibliography {
        entries: vec![article, fake],
    };
    let clock = Arc::new(FakeClock::new(1_700_000_000_000));
    let cache = Cache::open(
        &c.cfg.home.join("cache"),
        clock.clone(),
        Arc::new(ClockPause(clock)),
    )
    .unwrap();
    let transport = CannedTransport::from_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/paper/citations"),
    )
    .unwrap();
    let cross = |id: &str| crossref::work(&cache, &transport, FetchMode::Online, id);
    let open = |id: &str| {
        OpenAlex::new(&cache, &transport, FetchMode::Online, &c.cfg.home)
            .unwrap()
            .work(id)
    };
    let citations = citations::verify_manuscript(&c.ms, &bib, &cross, &open);
    assert_eq!(
        !citations.blocking.is_empty(),
        bad_citation,
        "{citations:?}"
    );
    readiness::record_citations(c.store(), &citations, &bibliography::to_bibtex(&bib)).unwrap();
    assert_eq!(
        state::load(c.store()).unwrap().unwrap().steps[&Step::Citations].ok,
        !bad_citation
    );
    let numbers = integrity::check(&c.ms, &c.bundle, &[], &c.cfg.project_root).unwrap();
    assert_eq!(
        numbers
            .issues
            .iter()
            .any(|x| x.kind == IntegrityKind::UnsourcedNumber),
        bad_number,
        "{numbers:?}"
    );
    assert_eq!(numbers.passed, !bad_number, "{numbers:?}");
    readiness::record_numbers(c.store(), &numbers).unwrap();
    assert_eq!(
        state::load(c.store()).unwrap().unwrap().steps[&Step::Numbers].ok,
        !bad_number
    );
    readiness::record_compile(c.store(), &compile(true), PDF).unwrap();
    readiness::record_render(c.store(), &pages(false)).unwrap();
    readiness::record_disclosure(c.store(), &disclosure(PolicyKind::Allowed)).unwrap();
    if bad_citation {
        c.no_ready(Condition::Citations);
    } else if bad_number {
        c.no_ready(Condition::Numbers);
    } else {
        c.ready_review();
        assert_ok(&c.evaluate());
    }
}
#[test]
fn seeded_fake_doi_blocks_readiness() {
    seeded(true, false);
}
#[test]
fn seeded_unsourced_number_blocks_readiness() {
    seeded(false, true);
}
#[test]
fn seeded_clean_citations_and_numbers_are_ready() {
    seeded(false, false);
}

#[test]
fn review_input_paths_hash_and_missing_source() {
    let c = Case::new();
    let p = paper_dir(c.store());
    let expected = vec![
        ("manuscript.json", p.join("passes/4-fable/manuscript.json")),
        ("evidence.json", p.join("evidence.json")),
        ("journal-rules.md", p.join("journal-rules.md")),
        ("citation-report.json", p.join("citation-report.json")),
        ("numbers-report.json", p.join("numbers-report.json")),
        ("compile-report.json", p.join("build/compile-report.json")),
        ("manuscript.pdf", p.join("build/main.pdf")),
    ];
    let files = review::input_files(c.store()).unwrap();
    assert_eq!(
        files,
        expected
            .iter()
            .map(|(n, p)| (n.to_string(), p.clone()))
            .collect::<Vec<_>>()
    );
    let material = expected
        .iter()
        .map(|(n, p)| format!("{n}\0{}\n", axiom::seams::sha256_hex(&fs::read(p).unwrap())))
        .collect::<String>();
    assert_eq!(
        review::current_input_hash(c.store()).unwrap(),
        axiom::seams::sha256_hex(material.as_bytes())
    );
    fs::remove_file(&expected[0].1).unwrap();
    let error = review::input_files(c.store()).unwrap_err();
    assert_eq!(error.kind, "not_ready");
    assert!(format!("{error:?}").contains("manuscript"));
}
