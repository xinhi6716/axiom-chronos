use axiom::{
    council::{Status, AGY_PROMPT_LIMIT, ARGON_MODEL, FABLE_MODEL},
    paper::{
        budget,
        evidence::{self, BundleInputs},
        members::{expected_model, PaperMembers},
        runner::{run_passes, RunConfig},
        state, style, *,
    },
    research_program::{
        self, dossier,
        investigation::{MemberCall, MemberReply, ResearchMember},
    },
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tempfile::{tempdir, TempDir};

const DOSSIER: &str = include_str!("fixtures/paper/evidence/dossier.json");
const RESULTS: &str = include_str!("fixtures/paper/evidence/results.json");
const DATA: &[u8] = include_bytes!("fixtures/paper/evidence/data.csv");
const MODEL: &[u8] = include_bytes!("fixtures/paper/evidence/model.txt");
const PASSES: [&str; 4] = [
    include_str!("fixtures/paper/runner/pass1.json"),
    include_str!("fixtures/paper/runner/pass2.json"),
    include_str!("fixtures/paper/runner/pass3.json"),
    include_str!("fixtures/paper/runner/pass4.json"),
];
const RULES: &str = "Template rules: use the journal format.";

type Trace = Arc<Mutex<Vec<TraceItem>>>;
#[derive(Clone, Debug)]
enum TraceItem {
    Call(MemberCall),
    Event(PaperEvent),
}
struct Fake {
    name: &'static str,
    replies: Mutex<VecDeque<MemberReply>>,
    trace: Trace,
    cancel: bool,
}
impl ResearchMember for Fake {
    fn name(&self) -> &str {
        self.name
    }
    fn call(&self, call: &MemberCall, stop: &AtomicBool) -> MemberReply {
        self.trace
            .lock()
            .unwrap()
            .push(TraceItem::Call(call.clone()));
        if self.cancel {
            stop.store(true, Ordering::SeqCst);
        }
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected call / retry")
    }
}
fn reply(pass: usize) -> MemberReply {
    MemberReply {
        status: Status::Ok,
        output: Some(serde_json::from_str(PASSES[pass - 1]).unwrap()),
        raw: Some(format!("raw-{pass}")),
        model: Some(
            match pass {
                2 | 4 => FABLE_MODEL,
                3 => ARGON_MODEL,
                _ => "codex-model",
            }
            .into(),
        ),
        provider: format!("provider-{pass}"),
        usd: Some(0.1),
        tokens: 10,
        detail: None,
    }
}
struct Members {
    astra: Fake,
    fable: Fake,
    argon: Fake,
    reviewer: Fake,
    trace: Trace,
}
impl Members {
    fn new() -> Self {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let fake = |name, replies: Vec<MemberReply>| Fake {
            name,
            replies: Mutex::new(replies.into()),
            trace: trace.clone(),
            cancel: false,
        };
        Self {
            astra: fake("astra", vec![reply(1)]),
            fable: fake("fable", vec![reply(2), reply(4)]),
            argon: fake("argon", vec![reply(3)]),
            reviewer: fake("reviewer", vec![]),
            trace,
        }
    }
    fn refs(&self) -> PaperMembers<'_> {
        PaperMembers {
            astra: &self.astra,
            fable: &self.fable,
            argon: &self.argon,
            reviewer: &self.reviewer,
        }
    }
    fn calls(&self) -> Vec<MemberCall> {
        self.trace
            .lock()
            .unwrap()
            .iter()
            .filter_map(|x| {
                if let TraceItem::Call(c) = x {
                    Some(c.clone())
                } else {
                    None
                }
            })
            .collect()
    }
    fn events(&self) -> Vec<PaperEvent> {
        self.trace
            .lock()
            .unwrap()
            .iter()
            .filter_map(|x| {
                if let TraceItem::Event(e) = x {
                    Some(e.clone())
                } else {
                    None
                }
            })
            .collect()
    }
    fn replace(&self, member: &str, index: usize, r: MemberReply) {
        let f = match member {
            "astra" => &self.astra,
            "fable" => &self.fable,
            "argon" => &self.argon,
            _ => panic!(),
        };
        f.replies.lock().unwrap()[index] = r;
    }
}
struct Case {
    _dir: TempDir,
    cfg: RunConfig,
    inputs: BundleInputs,
    bundle: EvidenceBundle,
    journal: JournalRecord,
    stop: AtomicBool,
}
impl Case {
    fn new() -> Self {
        let dir = tempdir().unwrap();
        let root = dir.path().join("project");
        let store = root.join("store");
        fs::create_dir_all(research_program::program_dir(&store)).unwrap();
        let dossier: dossier::Dossier = serde_json::from_str(DOSSIER).unwrap();
        dossier::write(
            &research_program::program_dir(&store).join("02-dossier.json"),
            &dossier,
        )
        .unwrap();
        fs::write(root.join("results.json"), RESULTS).unwrap();
        fs::write(root.join("data.csv"), DATA).unwrap();
        fs::write(root.join("model.txt"), MODEL).unwrap();
        let inputs = BundleInputs {
            store: store.clone(),
            project_root: root.clone(),
            results: vec![root.join("results.json")],
            datasets: vec![root.join("data.csv")],
            models: vec![root.join("model.txt")],
        };
        let bundle = evidence::build(&inputs).unwrap();
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
            "shortlist-1",
            true,
            None,
        )
        .unwrap();
        state::record(
            &store,
            Step::Journal,
            BTreeMap::from([("shortlist".into(), "shortlist-1".into())]),
            "journal-1",
            true,
            None,
        )
        .unwrap();
        let unknown = || Known::Unknown {
            reason: "not supplied".into(),
        };
        let journal = JournalRecord {
            id: "journal-1".into(),
            title: "Journal".into(),
            issns: vec![],
            issn_l: None,
            publisher: unknown(),
            homepage: unknown(),
            quartiles: vec![],
            sjr: Known::Unknown {
                reason: "not supplied".into(),
            },
            access: Known::Unknown {
                reason: "not supplied".into(),
            },
            apc: Known::Unknown {
                reason: "not supplied".into(),
            },
            review_weeks: Known::Unknown {
                reason: "not supplied".into(),
            },
            topics: vec![],
            works_count: Known::Unknown {
                reason: "not supplied".into(),
            },
        };
        let cfg = RunConfig {
            store,
            home: dir.path().join("home"),
            project_root: root,
            timeout_s: 71,
            fable_cap_usd: 5.0,
        };
        Self {
            _dir: dir,
            cfg,
            inputs,
            bundle,
            journal,
            stop: AtomicBool::new(false),
        }
    }
    fn run(&self, members: &Members) -> axiom::base::Res<Vec<PassRecord>> {
        run_passes(
            &self.cfg,
            &members.refs(),
            &self.journal,
            RULES,
            &self.stop,
            &|e| {
                members
                    .trace
                    .lock()
                    .unwrap()
                    .push(TraceItem::Event(e.clone()))
            },
        )
    }
    fn pass_dir(&self, n: usize) -> PathBuf {
        paper_dir(&self.cfg.store).join("passes").join(format!(
            "{}-{}",
            n,
            PassId::ALL[n - 1].member()
        ))
    }
}
fn read_json<T: serde::de::DeserializeOwned>(path: impl AsRef<Path>) -> T {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn assert_error(c: &Case, m: &Members, kind: &str) {
    assert_eq!(c.run(m).unwrap_err().kind, kind);
    assert!(
        m.events()
            .iter()
            .any(|e| matches!(e, PaperEvent::Blocked { kind: k, .. } if k == kind)),
        "missing Blocked {kind}: {:?}",
        m.events()
    );
}
fn assert_failed_pass(c: &Case, m: &Members, n: usize, status: Status) {
    let calls = m.calls();
    assert_eq!(calls.len(), n, "later pass or retry was called");
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.stage == format!("paper-pass-{n}"))
            .count(),
        1
    );
    let dir = c.pass_dir(n);
    let record: PassRecord = read_json(dir.join("pass-record.json"));
    assert_eq!(record.pass, PassId::ALL[n - 1]);
    assert_eq!(record.status, status);
    assert!(!record.validation.is_empty());
    let validation: Value = read_json(dir.join("validation.json"));
    assert!(
        validation.to_string().len() > 2,
        "validation must describe failure"
    );
}

#[test]
fn happy_path_records_prompts_events_files_and_state() {
    let c = Case::new();
    let m = Members::new();
    let expected_estimate = budget::estimate(
        &c.cfg.home,
        &["astra".into(), "fable".into(), "argon".into()],
    )
    .unwrap();
    let records = c.run(&m).unwrap();
    assert_eq!(records.len(), 4);
    let calls = m.calls();
    assert_eq!(
        calls.iter().map(|x| x.member.as_str()).collect::<Vec<_>>(),
        ["astra", "fable", "argon", "fable"]
    );
    let schema: Value =
        serde_json::from_str(include_str!("../engine/paper/pass.schema.json")).unwrap();
    let bundle_json = serde_json::to_value(&c.bundle).unwrap();
    let instructions = [
        axiom::engine::asset("paper/prompts/draft.md")
            .unwrap()
            .to_owned(),
        style::revise_instructions(),
        axiom::engine::asset("paper/prompts/critique.md")
            .unwrap()
            .to_owned(),
        style::style_instructions(),
    ];
    for n in 1..=4 {
        let call = &calls[n - 1];
        let dir = c.pass_dir(n);
        let record = &records[n - 1];
        assert_eq!(record.pass, PassId::ALL[n - 1]);
        assert_eq!(record.evidence_hash, c.bundle.hash);
        assert_eq!(call.stage, format!("paper-pass-{n}"));
        assert_eq!(call.readable, vec![dir.clone()]);
        assert_eq!(call.out_dir, dir);
        assert_eq!(call.schema, schema);
        assert_eq!(call.timeout_s, c.cfg.timeout_s);
        for file in [
            "prompt.txt",
            "input.json",
            "raw.txt",
            "reply.json",
            "manuscript.json",
            "changelog.json",
            "validation.json",
            "pass-record.json",
        ] {
            assert!(dir.join(file).is_file(), "missing {n}/{file}");
        }
        assert_eq!(
            call.prompt.as_bytes(),
            fs::read(dir.join("prompt.txt")).unwrap()
        );
        assert!(call.prompt.contains(RULES));
        assert!(call.prompt.contains(&instructions[n - 1]));
        assert!(call.prompt.contains(&c.bundle.hash));
        // Accept compact or pretty JSON while requiring the complete bundle.
        assert!([
            serde_json::to_string(&bundle_json).unwrap(),
            serde_json::to_string_pretty(&bundle_json).unwrap(),
            serde_json::to_string(&c.bundle).unwrap(),
            serde_json::to_string_pretty(&c.bundle).unwrap()
        ]
        .iter()
        .any(|json| call.prompt.contains(json)));
        assert!(
            call.prompt
                .contains(&serde_json::to_string(&schema).unwrap())
                || call
                    .prompt
                    .contains(&serde_json::to_string_pretty(&schema).unwrap())
        );
        assert!(fs::read_to_string(dir.join("input.json"))
            .unwrap()
            .contains(&c.bundle.hash));
        let prior = serde_json::from_str::<PassOutput>(PASSES[n.saturating_sub(2)])
            .unwrap()
            .manuscript;
        let compact = serde_json::to_string(&prior).unwrap();
        let pretty = serde_json::to_string_pretty(&prior).unwrap();
        if n == 1 {
            assert!(!call.prompt.contains(&compact) && !call.prompt.contains(&pretty));
        } else {
            assert!(call.prompt.contains(&compact) || call.prompt.contains(&pretty));
        }
        let output: PassOutput = serde_json::from_str(PASSES[n - 1]).unwrap();
        assert_eq!(record.change_log, output.change_log);
        assert_eq!(
            read_json::<Vec<Change>>(dir.join("changelog.json")),
            output.change_log
        );
        assert_eq!(
            read_json::<PassRecord>(dir.join("pass-record.json")),
            *record
        );
    }
    let final_manuscript: Manuscript = read_json(c.pass_dir(4).join("manuscript.json"));
    for (_, p) in final_manuscript.paragraphs() {
        for link in &p.claims {
            assert!(c.bundle.claims.iter().any(|x| x.id == link.claim));
            for id in &link.evidence {
                assert!(c.bundle.evidence.iter().any(|x| &x.id == id));
            }
        }
    }
    let trace = m.trace.lock().unwrap();
    let first_call = trace
        .iter()
        .position(|x| matches!(x, TraceItem::Call(_)))
        .unwrap();
    let estimate = trace
        .iter()
        .position(|x| matches!(x, TraceItem::Event(PaperEvent::Estimate(_))))
        .unwrap();
    assert!(estimate < first_call);
    assert!(
        matches!(&trace[estimate], TraceItem::Event(PaperEvent::Estimate(e)) if *e == expected_estimate)
    );
    let pass_events: Vec<_> = trace
        .iter()
        .filter_map(|x| match x {
            TraceItem::Event(PaperEvent::PassStarted { pass }) => Some(("start", *pass)),
            TraceItem::Event(PaperEvent::PassFinished {
                pass,
                status,
                changes,
            }) => {
                assert_eq!(*status, Status::Ok);
                assert_eq!(*changes, 1);
                Some(("finish", *pass))
            }
            _ => None,
        })
        .collect();
    let expected_events: Vec<_> = PassId::ALL
        .into_iter()
        .flat_map(|id| [("start", id), ("finish", id)])
        .collect();
    assert_eq!(pass_events, expected_events);
    let state = state::load(&c.cfg.store).unwrap().unwrap();
    for (n, step) in [Step::Pass1, Step::Pass2, Step::Pass3, Step::Pass4]
        .into_iter()
        .enumerate()
    {
        assert!(state::current(&state, step));
        let r = &state.steps[&step];
        assert_eq!(r.inputs.get("evidence"), Some(&c.bundle.hash));
        assert_eq!(
            r.inputs.get("journal").map(String::as_str),
            Some("journal-1")
        );
        if n > 0 {
            assert_eq!(
                r.inputs.get(&format!("pass{n}")),
                Some(&state.steps[&[Step::Pass1, Step::Pass2, Step::Pass3][n - 1]].output)
            );
        }
        assert_eq!(r.output, records[n].output_hash.as_ref().unwrap().as_str());
    }
}

#[test]
fn precondition_evidence() {
    let c = Case::new();
    state::invalidate(&c.cfg.store, Step::Evidence).unwrap();
    let m = Members::new();
    assert_eq!(c.run(&m).unwrap_err().kind, "not_ready");
    assert!(m.calls().is_empty());
}
#[test]
fn precondition_journal() {
    let c = Case::new();
    state::invalidate(&c.cfg.store, Step::Journal).unwrap();
    let m = Members::new();
    assert_eq!(c.run(&m).unwrap_err().kind, "no_journal");
    assert!(m.calls().is_empty());
}

fn status_failure(status: Status) {
    let c = Case::new();
    let m = Members::new();
    let mut r = reply(1);
    r.status = status;
    m.replace("astra", 0, r);
    assert_error(&c, &m, "member_failed");
    assert_failed_pass(&c, &m, 1, status);
}
#[test]
fn missing_member_blocks() {
    status_failure(Status::Missing);
}
#[test]
fn timed_out_member_blocks() {
    status_failure(Status::Timeout);
}
#[test]
fn failed_member_blocks() {
    status_failure(Status::Failed);
}
fn bad_output(n: usize, fixture: &str) {
    let c = Case::new();
    let m = Members::new();
    let mut r = reply(n);
    r.output = Some(serde_json::from_str(fixture).unwrap());
    m.replace(PassId::ALL[n - 1].member(), if n == 4 { 1 } else { 0 }, r);
    assert_error(&c, &m, "rejected");
    assert_failed_pass(&c, &m, n, Status::Ok);
}
#[test]
fn malformed_output_rejected() {
    bad_output(1, include_str!("fixtures/paper/runner/malformed.json"));
}
#[test]
fn unknown_claim_rejected() {
    bad_output(1, include_str!("fixtures/paper/runner/bad_claim.json"));
}
#[test]
fn unknown_evidence_rejected() {
    bad_output(1, include_str!("fixtures/paper/runner/bad_evidence.json"));
}
#[test]
fn unknown_quantity_rejected() {
    bad_output(1, include_str!("fixtures/paper/runner/bad_quantity.json"));
}
#[test]
fn revision_literal_number_rejected() {
    bad_output(2, include_str!("fixtures/paper/runner/bad_numeral.json"));
}
#[test]
fn style_claim_change_rejected() {
    bad_output(
        4,
        include_str!("fixtures/paper/runner/bad_style_claim.json"),
    );
}
#[test]
fn style_citation_change_rejected() {
    bad_output(
        4,
        include_str!("fixtures/paper/runner/bad_style_citation.json"),
    );
}
fn wrong_model(n: usize) {
    let c = Case::new();
    let m = Members::new();
    let mut r = reply(n);
    r.model = Some("other-model".into());
    m.replace(PassId::ALL[n - 1].member(), 0, r);
    assert_error(&c, &m, "rejected");
    assert_failed_pass(&c, &m, n, Status::Ok);
    let record: PassRecord = read_json(c.pass_dir(n).join("pass-record.json"));
    assert!(record.validation.iter().any(|s| s.contains("wrong_model")));
    assert!(fs::read_to_string(c.pass_dir(n).join("validation.json"))
        .unwrap()
        .contains("wrong_model"));
}
#[test]
fn fable_wrong_model_rejected() {
    wrong_model(2);
}
#[test]
fn argon_wrong_model_rejected() {
    wrong_model(3);
}
#[test]
fn astra_any_model_and_argon_expected_accepted() {
    let c = Case::new();
    let m = Members::new();
    let mut r = reply(1);
    r.model = Some("any-codex".into());
    m.replace("astra", 0, r);
    let records = c.run(&m).unwrap();
    assert_eq!(records[0].model.as_deref(), Some("any-codex"));
    assert_eq!(records[2].model.as_deref(), Some(ARGON_MODEL));
}

#[test]
fn cancelled_before_run() {
    let c = Case::new();
    let m = Members::new();
    c.stop.store(true, Ordering::SeqCst);
    assert_eq!(c.run(&m).unwrap_err().kind, "cancelled");
    assert!(m.calls().is_empty());
    assert!(m
        .events()
        .iter()
        .any(|e| matches!(e, PaperEvent::Cancelled)));
}
#[test]
fn cancelled_during_pass_two() {
    let c = Case::new();
    let mut m = Members::new();
    m.fable.cancel = true;
    assert_eq!(c.run(&m).unwrap_err().kind, "cancelled");
    assert_eq!(
        m.calls()
            .iter()
            .map(|x| x.stage.as_str())
            .collect::<Vec<_>>(),
        ["paper-pass-1", "paper-pass-2"]
    );
    assert!(m
        .events()
        .iter()
        .any(|e| matches!(e, PaperEvent::Cancelled)));
}
#[test]
fn zero_fable_budget_refuses_pass_two() {
    let mut c = Case::new();
    c.cfg.fable_cap_usd = 0.0;
    let m = Members::new();
    assert_error(&c, &m, "budget_exceeded");
    assert_eq!(
        m.calls()
            .iter()
            .map(|x| x.member.as_str())
            .collect::<Vec<_>>(),
        ["astra"]
    );
}
#[test]
fn fable_cap_is_shared_between_passes() {
    let mut c = Case::new();
    c.cfg.fable_cap_usd = 1.0;
    let m = Members::new();
    let mut r = reply(2);
    r.usd = Some(1.0);
    m.replace("fable", 0, r);
    assert_error(&c, &m, "budget_exceeded");
    assert_eq!(
        m.calls()
            .iter()
            .map(|x| x.stage.as_str())
            .collect::<Vec<_>>(),
        ["paper-pass-1", "paper-pass-2", "paper-pass-3"]
    );
    assert_eq!(m.calls().iter().filter(|x| x.member == "fable").count(), 1);
}

#[test]
fn resume_current_passes_and_invalidate_on_changed_evidence() {
    let c = Case::new();
    let m = Members::new();
    let mut bad = reply(3);
    bad.output = Some(json!({"bad": true}));
    m.replace("argon", 0, bad);
    assert_error(&c, &m, "rejected");
    assert_failed_pass(&c, &m, 3, Status::Ok);
    m.argon.replies.lock().unwrap().push_back(reply(3));
    let records = c.run(&m).unwrap();
    assert_eq!(records.len(), 4);
    assert_eq!(
        m.calls()
            .iter()
            .map(|x| x.stage.as_str())
            .collect::<Vec<_>>(),
        [
            "paper-pass-1",
            "paper-pass-2",
            "paper-pass-3",
            "paper-pass-3",
            "paper-pass-4"
        ]
    );
    assert_eq!(c.run(&m).unwrap(), records);
    assert_eq!(m.calls().len(), 5);
    let mut results: Value = serde_json::from_str(RESULTS).unwrap();
    results["quantities"][0]["value"] = json!(3.5);
    fs::write(&c.inputs.results[0], serde_json::to_vec(&results).unwrap()).unwrap();
    let b = evidence::build(&c.inputs).unwrap();
    assert_ne!(b.hash, c.bundle.hash);
    evidence::save(&c.cfg.store, &b).unwrap();
    state::record(
        &c.cfg.store,
        Step::Evidence,
        BTreeMap::new(),
        &b.hash,
        true,
        None,
    )
    .unwrap();
    m.astra.replies.lock().unwrap().push_back(reply(1));
    m.fable.replies.lock().unwrap().extend([reply(2), reply(4)]);
    m.argon.replies.lock().unwrap().push_back(reply(3));
    let refreshed = c.run(&m).unwrap();
    assert_eq!(refreshed.len(), 4);
    assert!(refreshed.iter().all(|r| r.evidence_hash == b.hash));
    assert_eq!(
        m.calls()
            .iter()
            .skip(5)
            .map(|x| x.stage.as_str())
            .collect::<Vec<_>>(),
        [
            "paper-pass-1",
            "paper-pass-2",
            "paper-pass-3",
            "paper-pass-4"
        ]
    );
}
#[test]
fn long_cjk_prompt_reaches_argon_intact() {
    let c = Case::new();
    let m = Members::new();
    let mut out: PassOutput = serde_json::from_str(PASSES[1]).unwrap();
    out.manuscript
        .abstract_text
        .text
        .push_str(&"漢".repeat(AGY_PROMPT_LIMIT + 1));
    let mut r = reply(2);
    r.output = Some(serde_json::to_value(&out).unwrap());
    m.replace("fable", 0, r);
    let mut r3 = reply(3);
    r3.output = Some(serde_json::to_value(&out).unwrap());
    m.replace("argon", 0, r3);
    let mut r4 = reply(4);
    r4.output = Some(serde_json::to_value(&out).unwrap());
    m.replace("fable", 1, r4);
    c.run(&m).unwrap();
    let calls = m.calls();
    let call = &calls[2];
    assert!(call.prompt.len() > AGY_PROMPT_LIMIT);
    assert_eq!(
        call.prompt.as_bytes(),
        fs::read(c.pass_dir(3).join("prompt.txt")).unwrap()
    );
    assert!(call.prompt.contains(&"漢".repeat(AGY_PROMPT_LIMIT + 1)));
}
#[test]
fn model_expectations() {
    assert_eq!(expected_model("fable"), Some(FABLE_MODEL));
    assert_eq!(expected_model("argon"), Some(ARGON_MODEL));
    for name in ["astra", "reviewer", "unknown", ""] {
        assert_eq!(expected_model(name), None);
    }
}
#[test]
fn prompt_assets_have_allowed_wording() {
    for path in ["paper/prompts/draft.md", "paper/prompts/critique.md"] {
        let text = axiom::engine::asset(path).expect(path);
        assert!(!text.trim().is_empty(), "{path}");
        let lower = text.to_lowercase();
        for forbidden in [
            "detector",
            "detect",
            "evade",
            "evasion",
            "undetectable",
            "bypass",
        ] {
            assert!(!lower.contains(forbidden), "{path}: {forbidden}");
        }
    }
}

#[test]
fn fixtures_use_bundle_ids_and_exercise_style_checks() {
    let c = Case::new();
    let outputs: Vec<PassOutput> = PASSES
        .iter()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    for output in &outputs {
        for (_, p) in output.manuscript.paragraphs() {
            for link in &p.claims {
                assert!(c.bundle.claims.iter().any(|x| x.id == link.claim));
                assert!(link.evidence.iter().all(|id| c
                    .bundle
                    .evidence
                    .iter()
                    .any(|x| &x.id == id)));
            }
            for (_, token) in tokens(&p.text) {
                if let Token::Quantity(id) = token {
                    assert!(c.bundle.quantities.iter().any(|x| x.id == id));
                }
            }
        }
    }
    for n in 1..=2 {
        assert!(style::check_revision(
            &outputs[n - 1].manuscript,
            &outputs[n].manuscript,
            &c.bundle,
            &outputs[n].change_log
        )
        .is_ok());
    }
    assert!(style::check_style_only(&outputs[2].manuscript, &outputs[3].manuscript).is_ok());
    let bad: PassOutput =
        serde_json::from_str(include_str!("fixtures/paper/runner/bad_numeral.json")).unwrap();
    assert!(style::check_revision(
        &outputs[0].manuscript,
        &bad.manuscript,
        &c.bundle,
        &bad.change_log
    )
    .is_err());
    for fixture in [
        include_str!("fixtures/paper/runner/bad_style_claim.json"),
        include_str!("fixtures/paper/runner/bad_style_citation.json"),
    ] {
        let bad: PassOutput = serde_json::from_str(fixture).unwrap();
        assert!(style::check_style_only(&outputs[2].manuscript, &bad.manuscript).is_err());
    }
}
