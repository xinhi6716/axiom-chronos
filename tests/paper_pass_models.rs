use axiom::{
    council::{Status, FABLE_MODEL},
    paper::{
        budget,
        evidence::{self, BundleInputs},
        members::{PaperMember, PaperMembers},
        runner::{run_passes, RunConfig},
        state, *,
    },
    research_program::{
        self, dossier,
        investigation::{MemberCall, MemberReply, ResearchMember},
    },
    usage::pools::HYDROGEN_XHIGH,
};
use serde_json::Value;
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

type Trace = Arc<Mutex<Vec<MemberCall>>>;
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
        self.trace.lock().unwrap().push(call.clone());
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
impl PaperMember for Fake {
    fn call_model(&self, call: &MemberCall, _model: &str, stop: &AtomicBool) -> MemberReply {
        self.call(call, stop)
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
                3 => HYDROGEN_XHIGH,
                _ => "gpt-6-astra",
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
            trace,
        }
    }
    fn calls(&self) -> Vec<MemberCall> {
        self.trace.lock().unwrap().clone()
    }
    fn replace(&self, member: &str, index: usize, r: MemberReply) {
        let f = match member {
            "astra" => &self.astra,
            _ => panic!(),
        };
        f.replies.lock().unwrap()[index] = r;
    }
}
struct Case {
    _dir: TempDir,
    cfg: RunConfig,
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
            journal,
            stop: AtomicBool::new(false),
        }
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

mod common;
use axiom::council::paper_member::{PaperFable, PaperGemini};

fn process_members(c: &Case, scenario: &str) -> (PaperFable, PaperGemini) {
    let mut command = common::fake("fake_paper_member");
    command.push(format!(
        "--fake-log={}",
        c._dir.path().join("calls.jsonl").display()
    ));
    command.push("--fake-paper-output=1".into());
    command.push(format!("--fake-scenario={scenario}"));
    (
        PaperFable::new(Ok(command.clone()), c._dir.path().join("meter.json"), 5.0),
        PaperGemini::new(Ok(command)),
    )
}
fn process_run(
    c: &Case,
    astra: &Fake,
    fable: &PaperFable,
    gemini: &PaperGemini,
) -> axiom::base::Res<Vec<PassRecord>> {
    run_passes(
        &c.cfg,
        &PaperMembers {
            astra,
            fable,
            gemini,
            reviewer: astra,
        },
        &c.journal,
        RULES,
        &c.stop,
        &|_| {},
    )
}
fn process_logs(c: &Case) -> Vec<Value> {
    fs::read_to_string(c._dir.path().join("calls.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}
fn argv_model(log: &Value) -> &str {
    let argv = log["argv"].as_array().unwrap();
    argv.windows(2).find(|w| w[0] == "--model").unwrap()[1]
        .as_str()
        .unwrap()
}
fn has_argon(logs: &[Value]) -> bool {
    logs.iter()
        .any(|l| argv_model(l).starts_with("gemini-4-argon"))
}
fn pools(c: &Case) -> Vec<Value> {
    fs::read_to_string(c.cfg.home.join("usage/pools.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}
#[test]
fn default_process_uses_hydrogen_and_never_argon() {
    let c = Case::new();
    let a = Members::new();
    let (f, g) = process_members(&c, "ok");
    process_run(&c, &a.astra, &f, &g).unwrap();
    let logs = process_logs(&c);
    assert_eq!(logs.len(), 3);
    assert!(!has_argon(&logs));
    assert_eq!(argv_model(&logs[1]), HYDROGEN_XHIGH);
    let charges = pools(&c);
    assert!(charges
        .iter()
        .any(|x| x["pool"] == "hydrogen_flash" && x["model"] == HYDROGEN_XHIGH));
    assert!(!charges.iter().any(|x| x["pool"] == "argon"));
}
#[test]
fn overrides_reach_process_records_disclosure_and_estimate() {
    let c = Case::new();
    let a = Members::new();
    let (f, g) = process_members(&c, "ok");
    let mut m = models::PassModels::default();
    m.fable_revise = "claude-fable-5-1".into();
    m.gemini_revise = "gemini-3.8-flash".into();
    m.fable_style = "claude-opus-4-6".into();
    models::save(&c.cfg.home, &m).unwrap();
    let records = process_run(&c, &a.astra, &f, &g).unwrap();
    let logs = process_logs(&c);
    for (i, pass) in [
        PassId::FableRevise,
        PassId::GeminiRevise,
        PassId::FableStyle,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(argv_model(&logs[i]), m.get(pass));
        assert_eq!(records[i + 1].model.as_deref(), Some(m.get(pass)));
    }
    let mut journal = c.journal.clone();
    let source = SourceTag {
        kind: SourceKind::Person,
        url: None,
        fetched_at: "today".into(),
        data_year: None,
        raw: "test".into(),
    };
    journal.publisher = Known::Known(Sourced {
        value: "Example".into(),
        source,
    });
    let policy = AiPolicy {
        publisher: "Example".into(),
        url: "https://example.org".into(),
        fetched_at: "today".into(),
        sha256: "test".into(),
        policy: PolicyKind::Allowed,
        placement: "body".into(),
        required_items: vec![],
    };
    let disclosure =
        disclosure::generate_with_policies(&[policy], &records, None, &journal).unwrap();
    assert_eq!(disclosure.uses.len(), 4);
    assert_eq!(
        disclosure
            .uses
            .iter()
            .find(|u| u.pass == Some(PassId::GeminiRevise))
            .unwrap()
            .model,
        m.gemini_revise
    );
    for model in [&m.fable_revise, &m.gemini_revise, &m.fable_style] {
        assert!(disclosure.uses.iter().any(|u| &u.model == model));
    }
    for (record, usage) in records.iter().zip(disclosure.uses.iter()) {
        assert_eq!(record.model.as_deref(), Some(usage.model.as_str()));
    }
    let estimate = budget::estimate(&c.cfg.home, &models::estimate_models(), &m).unwrap();
    assert!(estimate.line.contains(&m.gemini_revise));
    assert_ne!(
        estimate.digest,
        budget::estimate(
            &c.cfg.home,
            &models::estimate_models(),
            &models::PassModels::default()
        )
        .unwrap()
        .digest
    );
}
#[test]
fn process_reply_wrong_model_is_rejected() {
    let c = Case::new();
    let a = Members::new();
    let (f, g) = process_members(&c, "reply-wrong-model");
    assert_eq!(
        process_run(&c, &a.astra, &f, &g).unwrap_err().kind,
        "rejected"
    );
    let record: PassRecord = read_json(c.pass_dir(2).join("pass-record.json"));
    assert_eq!(record.status, Status::Malformed);
    assert!(record.validation.iter().any(|s| s.contains("wrong_model")));
}
#[test]
fn gemini_process_reply_wrong_model_is_rejected() {
    let c = Case::new();
    let a = Members::new();
    let (f, _) = process_members(&c, "ok");
    let (_, g) = process_members(&c, "reply-wrong-model");
    assert_eq!(
        process_run(&c, &a.astra, &f, &g).unwrap_err().kind,
        "rejected"
    );
    let logs = process_logs(&c);
    assert_eq!(logs.len(), 2);
    assert_eq!(argv_model(&logs[1]), HYDROGEN_XHIGH);
    let record: PassRecord = read_json(c.pass_dir(3).join("pass-record.json"));
    assert_eq!(record.status, Status::Malformed);
    assert!(record.validation.iter().any(|s| s == "wrong_model"));
    assert!(!c.pass_dir(4).join("pass-record.json").exists());
}
#[test]
fn astra_reported_wrong_model_is_rejected_and_recorded() {
    let c = Case::new();
    let a = Members::new();
    let mut r = reply(1);
    r.model = Some("other-astra".into());
    a.replace("astra", 0, r);
    let (f, g) = process_members(&c, "ok");
    assert_eq!(
        process_run(&c, &a.astra, &f, &g).unwrap_err().kind,
        "rejected"
    );
    let record: PassRecord = read_json(c.pass_dir(1).join("pass-record.json"));
    assert_eq!(record.model.as_deref(), Some("other-astra"));
    assert!(record.validation.iter().any(|s| s == "wrong_model"));
    assert!(process_logs(&c).is_empty());
}
#[test]
fn astra_override_without_reported_model_is_rejected() {
    let c = Case::new();
    let a = Members::new();
    let mut r = reply(1);
    r.model = None;
    a.replace("astra", 0, r);
    let mut m = models::PassModels::default();
    m.astra_draft = "gpt-6-astra-override".into();
    models::save(&c.cfg.home, &m).unwrap();
    let (f, g) = process_members(&c, "ok");
    assert_eq!(
        process_run(&c, &a.astra, &f, &g).unwrap_err().kind,
        "rejected"
    );
    let record: PassRecord = read_json(c.pass_dir(1).join("pass-record.json"));
    assert_eq!(record.model, None);
    assert!(record.validation.iter().any(|s| s == "wrong_model"));
    assert!(process_logs(&c).is_empty());
}
#[test]
fn astra_default_without_reported_model_records_default() {
    let c = Case::new();
    let a = Members::new();
    let mut r = reply(1);
    r.model = None;
    a.replace("astra", 0, r);
    let (f, g) = process_members(&c, "ok");
    let records = process_run(&c, &a.astra, &f, &g).unwrap();
    assert_eq!(
        records[0].model.as_deref(),
        Some(models::DEFAULT_ASTRA_MODEL)
    );
}
#[test]
fn changed_model_invalidates_only_that_pass_and_later() {
    for changed in [
        PassId::AstraDraft,
        PassId::FableRevise,
        PassId::GeminiRevise,
        PassId::FableStyle,
    ] {
        let c = Case::new();
        let a = Members::new();
        let (f, g) = process_members(&c, "ok");
        process_run(&c, &a.astra, &f, &g).unwrap();
        let mut m = models::PassModels::default();
        match changed {
            PassId::AstraDraft => m.astra_draft = "gpt-6-astra-override".into(),
            PassId::FableRevise => m.fable_revise = "claude-fable-5-1".into(),
            PassId::GeminiRevise => m.gemini_revise = "gemini-3.8-flash".into(),
            PassId::FableStyle => m.fable_style = "claude-opus-4-6".into(),
        };
        models::save(&c.cfg.home, &m).unwrap();
        if changed == PassId::AstraDraft {
            let mut r = reply(1);
            r.model = Some(m.astra_draft.clone());
            a.astra.replies.lock().unwrap().push_back(r);
        }
        process_run(&c, &a.astra, &f, &g).unwrap();
        let logs = process_logs(&c);
        let expected: Vec<_> = (changed.number().max(2)..=4)
            .map(|n| format!("{n}-"))
            .collect();
        assert_eq!(logs.len(), 3 + expected.len());
        for (entry, prefix) in logs[3..].iter().zip(expected) {
            assert!(Path::new(entry["cwd"].as_str().unwrap())
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(&prefix));
        }
        assert_eq!(
            a.calls().len(),
            if changed == PassId::AstraDraft { 2 } else { 1 }
        );
    }
}
#[test]
fn argon_detector_negative_control() {
    let c = Case::new();
    let a = Members::new();
    let (f, g) = process_members(&c, "ok");
    let mut m = models::PassModels::default();
    m.gemini_revise = "gemini-4-argon-high".into();
    models::save(&c.cfg.home, &m).unwrap();
    process_run(&c, &a.astra, &f, &g).unwrap();
    assert!(has_argon(&process_logs(&c)));
}

#[test]
fn exhausted_hydrogen_pool_blocks_before_pass_three() {
    let c = Case::new();
    let a = Members::new();
    let (f, g) = process_members(&c, "ok");
    axiom::usage::pools::mark_exhausted(
        &c.cfg.home,
        axiom::usage::pools::Pool::HydrogenFlash,
        i64::MAX,
        "test",
    )
    .unwrap();
    assert_eq!(
        process_run(&c, &a.astra, &f, &g).unwrap_err().kind,
        "budget_exceeded"
    );
    assert_eq!(process_logs(&c).len(), 1);
    let record: PassRecord = read_json(c.pass_dir(3).join("pass-record.json"));
    assert_eq!(record.status, Status::BudgetCut);
    assert_eq!(record.model, None);
}
#[test]
fn model_defaults_and_invalid_ids() {
    let home = common::temp();
    let m = models::load(home.path()).unwrap();
    assert_eq!(m, models::PassModels::default());
    assert_eq!(m.gemini_revise, HYDROGEN_XHIGH);
    let mut invalid = m;
    invalid.gemini_revise.clear();
    assert!(invalid.validate().is_err());
    invalid.gemini_revise = "invalid model".into();
    assert!(invalid.validate().is_err());
}
