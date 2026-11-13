use axiom::{
    base::Res,
    council::{Status, DIGEST_MAX_CHARS},
    research_program::{
        self,
        adjudication::{Adjudication, Adjudicator, Finding, Ruling, RulingDecision},
        investigation::{MemberCall, MemberReply, ResearchMember},
        runner::{run, CouncilStage, Members, ProgramConfig},
        state::{self, StageStatus},
        Stage,
    },
};
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
};

struct Fake {
    name: &'static str,
    calls: AtomicUsize,
    mode: &'static str,
    paths: Mutex<Vec<Vec<std::path::PathBuf>>>,
    cost: f64,
    stop_on_call: bool,
}
impl Fake {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            calls: AtomicUsize::new(0),
            mode: "ok",
            paths: Mutex::new(vec![]),
            cost: 0.01,
            stop_on_call: false,
        }
    }
}
impl ResearchMember for Fake {
    fn name(&self) -> &str {
        self.name
    }
    fn call(&self, call: &MemberCall, stop: &AtomicBool) -> MemberReply {
        if self.stop_on_call {
            stop.store(true, Ordering::SeqCst);
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.paths.lock().unwrap().push(call.readable.clone());
        let status = match self.mode {
            "missing" => Status::Missing,
            "malformed" => Status::Malformed,
            _ => Status::Ok,
        };
        let mut output = match call.stage.as_str() {
            "investigate" => {
                json!({"items":[{"claim":"claim", "source":"src/a", "confidence":"high", "evidence":"proof"}]})
            }
            "red_team" => {
                json!({"verdict":"revise", "findings":[{"check":"complexity", "item":"C1", "reason":"risk"}]})
            }
            "pre_mortem" => {
                json!({"failures":[{"scenario":"fails", "item":"C1", "likelihood":"medium", "mitigation":"test"}]})
            }
            "synthesis" => {
                json!({"summary":"s", "conclusions":[{"id":"E1", "claim":"claim", "confidence":"high", "evidence":["E1"]}], "open_questions":[], "priority_checks":[], "plan":{"summary":"s", "coverage":{"read":["src/a"],"not_read":[]}, "changes":[{"id":"C1", "area":"tests", "location":"src/a", "evidence":"proof", "problem":"risk", "change":"add test", "verify":"run test", "tier":"sol", "effort":"high", "risk":"standard", "reason":"Bounded test change"}], "out_of_scope":[]}})
            }
            _ => panic!("unknown stage"),
        };
        if self.mode == "missing-tier" && call.stage == "synthesis" {
            output["plan"]["changes"][0].as_object_mut().unwrap().remove("tier");
        }
        MemberReply {
            status,
            output: Some(if self.mode == "invalid" {
                json!({})
            } else {
                output
            }),
            raw: None,
            model: Some(self.name.into()),
            provider: self.name.into(),
            usd: Some(self.cost),
            tokens: 5,
            detail: None,
        }
    }
}
struct Debate(AtomicUsize);
impl CouncilStage for Debate {
    fn debate(&self, plan: &Path, schema: &Path, _: &str, _: f64, _: &AtomicBool) -> Res<Value> {
        assert!(plan.is_file() && schema.is_file());
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"cost":{"usd":0.01}}))
    }
}
struct Judge {
    missing: bool,
    calls: AtomicUsize,
}
impl Adjudicator for Judge {
    fn adjudicate(&self, _: &Value, findings: &[Finding], _: &AtomicBool) -> Res<Adjudication> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Adjudication {
            model: "gpt-6-astra".into(),
            rulings: findings
                .iter()
                .take(if self.missing { 1 } else { findings.len() })
                .map(|f| Ruling {
                    finding: f.id.clone(),
                    decision: RulingDecision::Upheld,
                    reason: "yes".into(),
                })
                .collect(),
            dropped_changes: vec![],
        })
    }
}
struct Case {
    dir: tempfile::TempDir,
    a: Fake,
    b: Fake,
    c: Fake,
    red: Fake,
    pre: Fake,
    synth: Fake,
    debate: Debate,
    judge: Judge,
}
impl Case {
    fn new() -> Self {
        Self {
            // Keep ancestor names in the system temp path out of readable-path assertions.
            dir: tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap(),
            a: Fake::new("a"),
            b: Fake::new("b"),
            c: Fake::new("c"),
            red: Fake::new("red"),
            pre: Fake::new("pre"),
            synth: Fake::new("synth"),
            debate: Debate(AtomicUsize::new(0)),
            judge: Judge {
                missing: false,
                calls: AtomicUsize::new(0),
            },
        }
    }
    fn cfg(&self, cap: f64) -> ProgramConfig {
        ProgramConfig {
            store: self.dir.path().join("store"),
            cwd: self.dir.path().join("project"),
            question: "question".into(),
            sources: vec!["src/a".into()],
            home: self.dir.path().join("home"),
            initial_cap_usd: cap,
        }
    }
    fn run(&self, cap: f64, stop: &AtomicBool) -> Res<Value> {
        let cfg = self.cfg(cap);
        fs::create_dir_all(&cfg.cwd).unwrap();
        run(
            &cfg,
            &Members {
                investigators: vec![&self.a, &self.b, &self.c],
                red_team: &self.red,
                pre_mortem: &self.pre,
                synthesizer: &self.synth,
            },
            &self.debate,
            &self.judge,
            stop,
        )
    }
}
#[test]
fn synthesis_rejects_a_plan_change_without_tier() {
    let mut c = Case::new();
    c.synth.mode = "missing-tier";
    let error = c.run(10., &AtomicBool::new(false)).unwrap_err();
    assert_eq!(error.kind, "malformed");
    assert_eq!(error.message, "$.plan.changes[0].tier: required");
    assert_eq!(c.debate.0.load(Ordering::SeqCst), 1);
    assert_eq!(c.judge.calls.load(Ordering::SeqCst), 0);
    assert!(!c.cfg(10.).store.join("astra/plan.json").exists());
}

#[test]
fn complete_gate_resume_and_tamper() {
    let c = Case::new();
    let cfg = c.cfg(10.);
    let stop = AtomicBool::new(false);
    assert!(research_program::gate(&cfg.store).is_ok());
    let digest = c.run(10., &stop).unwrap();
    assert_eq!(digest["complete"], true);
    assert!(digest.to_string().chars().count() < DIGEST_MAX_CHARS);
    let state = state::load(&cfg.store).unwrap().unwrap();
    assert!(state.complete);
    assert_eq!(
        state.stages.iter().map(|e| e.stage).collect::<Vec<_>>(),
        Stage::ALL
    );
    assert!(research_program::gate(&cfg.store).is_ok());
    for stage in Stage::ALL {
        let p = research_program::program_dir(&cfg.store).join(stage.artifact());
        assert!(p.exists());
        let mut s = state.clone();
        s.stages.retain(|e| e.stage != stage);
        state::save(&cfg.store, &s).unwrap();
        assert_eq!(
            research_program::gate(&cfg.store).unwrap_err().kind,
            "research_program_gate"
        );
    }
    state::save(&cfg.store, &state).unwrap();
    for name in ["plan.json", "review.json", "ruling.md", "plan-final.md"] {
        assert!(cfg.store.join("astra").join(name).is_file());
    }
    assert!(fs::read_to_string(cfg.store.join("astra/plan-final.md"))
        .unwrap()
        .contains("### C1 add test"));
    c.run(10., &stop).unwrap();
    assert_eq!(c.red.calls.load(Ordering::SeqCst), 1);
    assert_eq!(c.debate.0.load(Ordering::SeqCst), 1);
    fs::write(
        research_program::program_dir(&cfg.store).join(Stage::RedTeam.artifact()),
        "tampered",
    )
    .unwrap();
    c.run(10., &stop).unwrap();
    assert_eq!(c.red.calls.load(Ordering::SeqCst), 2);
    assert_eq!(c.pre.calls.load(Ordering::SeqCst), 1);
    assert!(research_program::gate(&cfg.store).is_ok());
    for fake in [&c.a, &c.b, &c.c] {
        for paths in fake.paths.lock().unwrap().iter() {
            assert_eq!(paths, &vec![cfg.cwd.clone(), cfg.cwd.join("src/a")]);
            assert!(!paths.iter().any(|p| p.to_string_lossy().contains("plan")));
        }
    }
}
#[test]
fn cancel_then_resume() {
    let mut c = Case::new();
    c.red.stop_on_call = true;
    let cfg = c.cfg(10.);
    let stop = AtomicBool::new(false);
    assert_eq!(c.run(10., &stop).unwrap_err().kind, "interrupted");
    let state = state::load(&cfg.store).unwrap().unwrap();
    assert_eq!(state.stages.last().unwrap().status, StageStatus::Cancelled);
    assert_eq!(state.stages.last().unwrap().stage, Stage::RedTeam);
    assert!(!state.complete);
    c.red.stop_on_call = false;
    stop.store(false, Ordering::SeqCst);
    assert_eq!(c.run(10., &stop).unwrap()["complete"], true);
    assert_eq!(c.a.calls.load(Ordering::SeqCst), 1);
    assert_eq!(c.debate.0.load(Ordering::SeqCst), 1);
    assert_eq!(c.red.calls.load(Ordering::SeqCst), 2);
}
#[test]
fn degraded_members_are_recorded_and_two_valid_are_required() {
    let mut c = Case::new();
    c.c.mode = "missing";
    let stop = AtomicBool::new(false);
    c.run(10., &stop).unwrap();
    let s = state::load(&c.cfg(10.).store).unwrap().unwrap();
    assert_eq!(s.stages[0].members["c"], "Missing");
    let mut c = Case::new();
    c.b.mode = "invalid";
    c.c.mode = "missing";
    assert_eq!(c.run(10., &stop).unwrap_err().kind, "too_few_members");
    assert!(!c.cfg(10.).store.join("astra/plan.json").exists());
}
#[test]
fn budget_cut_and_missing_ruling() {
    let c = Case::new();
    let stop = AtomicBool::new(false);
    let err = c.run(0., &stop).unwrap_err();
    assert!(err.kind == "too_few_members" || err.kind == "budget_cut");
    assert_eq!(
        c.a.calls.load(Ordering::SeqCst)
            + c.b.calls.load(Ordering::SeqCst)
            + c.c.calls.load(Ordering::SeqCst),
        0
    );
    let partial = Case::new();
    assert_eq!(partial.run(0.015, &stop).unwrap_err().kind, "budget_cut");
    assert_eq!(
        partial.a.calls.load(Ordering::SeqCst)
            + partial.b.calls.load(Ordering::SeqCst)
            + partial.c.calls.load(Ordering::SeqCst),
        2
    );
    let state = state::load(&partial.cfg(0.015).store).unwrap().unwrap();
    assert!(state.stages[0].members.values().any(|s| s == "BudgetCut"));
    let mut c = Case::new();
    c.judge.missing = true;
    assert_eq!(c.run(10., &stop).unwrap_err().kind, "malformed");
    let cfg = c.cfg(10.);
    assert!(!state::load(&cfg.store).unwrap().unwrap().complete);
    assert!(!cfg.store.join("astra").exists());
}

#[test]
fn fable_once_reports_missing_command() {
    let missing = || Err(axiom::base::Failure::new("not_installed", "not available"));
    let setup = axiom::council::Setup::new(missing(), missing());
    let temp = tempfile::tempdir().unwrap();
    let shape = json!({"type":"object"});
    let reply = axiom::council::ask_fable_once(
        &setup,
        "prompt",
        temp.path(),
        &shape,
        &temp.path().join("schema.json"),
        std::time::Duration::from_secs(1),
        &AtomicBool::new(false),
    );
    assert_eq!(reply.status, Status::Missing);
    assert!(reply.detail.unwrap().contains("not available"));
}
