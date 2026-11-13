use axiom::codex::routing::*;
use axiom::orch::receipts::*;
mod common;
struct RunFixture {
    _temp: tempfile::TempDir,
    repo: std::path::PathBuf,
    store: std::path::PathBuf,
    log: std::path::PathBuf,
    bridge: axiom::bridge::Bridge,
}
impl RunFixture {
    fn new(route: &str, settings: &str) -> Self {
        use std::{fs, process::Command};
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(repo.join("src/mod.py"), "fixture").unwrap();
        fs::create_dir_all(repo.join("tests")).unwrap();
        fs::write(repo.join("tests/routing.py"), "import pathlib, unittest\nclass Routing(unittest.TestCase):\n    def test_routing_fixture(self):\n        self.assertEqual(pathlib.Path('src/mod.py').read_text(), 'implemented')\nunittest.main(verbosity=2)\n").unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "src/mod.py", "tests/routing.py"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "fixture",
            ],
        ] {
            assert!(Command::new("git")
                .args(args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success());
        }
        let store = axiom::orch::store_for(temp.path(), "demo").unwrap();
        fs::create_dir_all(store.join("inbox")).unwrap();
        fs::write(store.join("modules.md"), format!("## m\nPURPOSE x\nPATHS src/mod.py tests/routing.py\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n{route}\n")).unwrap();
        let path = store.join("u1.brief");
        let matrix = if route.is_empty() {
            String::new()
        } else {
            format!("CONTRACT routing fixture completes with the assigned model and effort\nTEST_MATRIX routing | behaviour | tests/routing.py::test_routing_fixture | fixture implementation completes | \"{}\" tests/routing.py\nCONTEXT\nFAKE_SELFCHECK {{\"status\":\"pass\",\"changed\":false,\"fixed\":0,\"findings\":[]}}\n", common::python())
        };
        fs::write(
            &path,
            format!("GOAL build\nSCOPE src/mod.py tests/routing.py\nVERIFY echo ok\n{settings}\n{matrix}"),
        )
        .unwrap();
        fs::write(
            store.join("units.tsv"),
            format!(
                "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu1\tm\tpending\t\t\t\t{}\n",
                path.display()
            ),
        )
        .unwrap();
        let log = temp.path().join("codex.log");
        let wrapper = temp.path().join("fake.py");
        fs::write(&wrapper, "import os,runpy,sys\nos.environ['FAKE_CODEX_LOG']=sys.argv[1]\nos.environ['FAKE_CODEX_MODELS']='gpt-6.1-sol,gpt-6-astra,gpt-6-terra,gpt-6-luna'\nsys.argv=sys.argv[2:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n").unwrap();
        if !route.is_empty() {
            let hook = format!(r#"
import json,pathlib
repo = pathlib.Path({repo})
original_stdin = sys.stdin
class Input:
    def __getattr__(self, key): return getattr(original_stdin, key)
    def __iter__(self): return self
    def __next__(self):
        line = next(original_stdin)
        msg = json.loads(line)
        if msg.get('method') == 'turn/start':
            params = msg['params']
            if not params.get('outputSchema'):
                text = params['input'][0]['text']
                (repo/'src/mod.py').write_text('fixture' if 'TESTS-FIRST' in text else 'implemented')
        return line
sys.stdin = Input()
"#, repo = serde_json::to_string(&repo.to_string_lossy()).unwrap());
            let original = fs::read_to_string(&wrapper).unwrap();
            fs::write(&wrapper, original.replace("runpy.run_path", &(hook + "\nrunpy.run_path"))).unwrap();
        }
        let mut config = axiom::bridge::Config::from_env();
        config.data_home = Some(temp.path().into());
        config.codex = Ok(vec![
            common::python(),
            wrapper.to_string_lossy().into_owned(),
            log.to_string_lossy().into_owned(),
            common::repo_root()
                .join("tests/fakes/fake_codex.py")
                .to_string_lossy()
                .into_owned(),
        ]);
        config.wsl = None;
        config.on_windows = false;
        let bridge = axiom::bridge::Bridge::new(config);
        Self {
            _temp: temp,
            repo,
            store,
            log,
            bridge,
        }
    }
    fn run(&self, role: &str) -> serde_json::Value {
        axiom::bridge::call(
            &self.bridge,
            "codex_run",
            &serde_json::json!({"project":"demo","unit":"u1","role":role,"cwd":self.repo}),
        )["structuredContent"]
            .clone()
    }
    fn params(&self, method: &str) -> serde_json::Value {
        std::fs::read_to_string(&self.log)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
            .find(|v| v["method"] == method)
            .unwrap()["params"]
            .clone()
    }
}
fn check_run(route: &str, settings: &str, model: &str, effort: Option<&str>, rule: Rule) {
    let f = RunFixture::new(route, settings);
    let result = f.run("terra");
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(f.params("thread/start")["model"], model);
    assert_eq!(
        f.params("turn/start")
            .get("effort")
            .and_then(|v| v.as_str()),
        effort
    );
    assert_eq!(read_snapshot(&f.store, "u1").unwrap().unwrap().rule, rule);
}
#[test]
fn map_unit_line_applied() {
    check_run(
        "ROUTING\n  default sol/high standard reason\n  u1 sol/xhigh parser reason",
        "",
        "gpt-6.1-sol",
        Some("xhigh"),
        Rule::MapUnit,
    );
}
#[test]
fn map_default_line_applied() {
    check_run(
        "ROUTING\n  default sol/high standard reason",
        "",
        "gpt-6.1-sol",
        Some("high"),
        Rule::MapDefault,
    );
}
#[test]
fn brief_tier_effort_applied() {
    check_run(
        "",
        "TIER sol\nEFFORT xhigh",
        "gpt-6.1-sol",
        Some("xhigh"),
        Rule::Brief,
    );
}
#[test]
fn legacy_role_unchanged() {
    check_run("", "", "gpt-6-terra", None, Rule::LegacyRole);
}
#[test]
fn astra_tier_applied() {
    check_run(
        "ROUTING\n  default sol/high standard reason\n  u1 astra/xhigh novel reason",
        "",
        "gpt-6-astra",
        Some("xhigh"),
        Rule::MapUnit,
    );
}
#[test]
fn fix_record_wins() {
    let f = RunFixture::new("ROUTING\n  default sol/high standard reason", "");
    let mut record = fix();
    record.unit = "u1".into();
    write_fix(&f.store, &record).unwrap();
    let result = f.run("terra");
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(f.params("turn/start")["effort"], "xhigh");
    assert_eq!(
        read_snapshot(&f.store, "u1").unwrap().unwrap().rule,
        Rule::FixEscalation
    );
}
#[test]
fn routing_receipt_written() {
    let f = RunFixture::new("ROUTING\n  default sol/xhigh parser reason", "");
    let axiom::orch::Begin::Run { attempt_id, .. } =
        axiom::orch::begin_unit(&f.store, "u1", "terra", &f.repo, false).unwrap()
    else { panic!("expected claim") };
    let checkpoint_path = f.store.join("checkpoints/u1.json");
    let checkpoint = std::fs::read(&checkpoint_path).unwrap();
    axiom::orch::release_claim(&f.store, "u1", &attempt_id).unwrap();
    // Replay the claimed checkpoint with an ownerless legacy claim so begin
    // resumes this exact attempt, even after a completed run releases its claim.
    let resume = || {
        std::fs::write(&checkpoint_path, &checkpoint).unwrap();
        std::fs::write(
            f.store.join("claims.tsv"),
            format!("unit\tattempt\tscope\tstarted\nu1\t{attempt_id}\tsrc/mod.py tests/routing.py\t\n"),
        )
        .unwrap();
    };
    resume();
    let result = f.run("terra");
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(result["attempt_id"], attempt_id);
    let rows = read_routing(&f.store).unwrap();
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(r.unit, "u1");
    assert_eq!(r.attempt, result["attempt_id"].as_str().unwrap());
    assert_eq!(r.tier.as_deref(), Some("sol"));
    assert_eq!(r.effort.as_deref(), Some("xhigh"));
    assert_eq!(r.risk.as_deref(), Some("parser"));
    assert_eq!(r.model_assigned, "gpt-6.1-sol");
    assert_eq!(r.model_used.as_deref(), Some("gpt-6.1-sol"));
    assert_eq!(r.rule, "map_default");
    resume();
    let next = f.run("terra");
    assert_eq!(next["status"], "completed", "{next}");
    assert_eq!(next["attempt_id"], attempt_id);
    let rows = read_routing(&f.store).unwrap();
    assert_eq!(rows.len(), 1);
}
#[test]
fn sol_role_still_gated() {
    let f = RunFixture::new("", "");
    let result = f.run("sol");
    assert_eq!(result["error"]["kind"], "sol_not_allowed", "{result}");
    assert!(!f.log.exists());
}
#[test]
fn luna_brief_legacy() {
    check_run("", "TIER luna", "gpt-6-luna", None, Rule::Brief);
    let f = RunFixture::new("", "TIER luna");
    let result = f.run("sol");
    assert!(result["error"]["kind"].is_string());
    assert!(!f.log.exists());
}
fn attempt_store(line: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(
        d.path().join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu1\tm\tpending\t\t\t\t\n",
    )
    .unwrap();
    std::fs::write(d.path().join("modules.md"), module("m", &routed(line))).unwrap();
    d
}
#[test]
fn explicit_model_with_routing() {
    let d = attempt_store("default sol/high standard reason");
    let server = axiom::codex::rpc::AppServer::new(vec![]);
    let b = parse_brief(&brief("")).unwrap();
    assert_eq!(
        for_attempt(
            &server,
            None,
            d.path(),
            "u1",
            "a",
            "terra",
            &b,
            Some("gpt-6-terra")
        )
        .unwrap_err()
        .kind,
        "invalid_argument"
    );
    assert!(!snapshot_path(d.path(), "u1").exists());
}
#[test]
fn fix_brief_mismatch() {
    let d = attempt_store("default sol/high standard reason");
    let mut f = fix();
    f.unit = "u1".into();
    f.tier = Tier::Astra;
    write_fix(d.path(), &f).unwrap();
    let server = axiom::codex::rpc::AppServer::new(vec![]);
    let b = parse_brief(&brief("TIER sol")).unwrap();
    assert_eq!(
        for_attempt(&server, None, d.path(), "u1", "a", "terra", &b, None)
            .unwrap_err()
            .kind,
        "routing_mismatch"
    );
}
#[test]
fn no_model_no_fallback() {
    let d = attempt_store("default sol/high standard reason");
    // Keep a fallback role available, but explicitly exclude every sol model.
    let wrapper = d.path().join("server.py");
    std::fs::write(&wrapper, "import os,runpy,sys\nos.environ['FAKE_CODEX_MODELS']='gpt-6-terra'\nsys.argv=sys.argv[1:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n").unwrap();
    let server = axiom::codex::rpc::AppServer::new(vec![
        common::python(),
        wrapper.to_string_lossy().into_owned(),
        common::repo_root()
            .join("tests/fakes/fake_codex.py")
            .to_string_lossy()
            .into_owned(),
    ]);
    assert_eq!(server.list_models().unwrap(), vec!["gpt-6-terra"]);
    let b = parse_brief(&brief("")).unwrap();
    assert_eq!(
        for_attempt(&server, None, d.path(), "u1", "a", "terra", &b, None)
            .unwrap_err()
            .kind,
        "model_not_found"
    );
    assert!(!snapshot_path(d.path(), "u1").exists());
}
#[test]
fn snapshot_reused_for_same_attempt() {
    let d = attempt_store("default sol/high standard reason");
    let models = d.path().join("models");
    let wrapper = d.path().join("server.py");
    std::fs::write(&models, "gpt-6.1-sol").unwrap();
    std::fs::write(&wrapper, "import os,runpy,sys\nos.environ['FAKE_CODEX_MODELS']=open(sys.argv[1]).read()\nsys.argv=sys.argv[2:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n").unwrap();
    let server = axiom::codex::rpc::AppServer::new(vec![
        common::python(),
        wrapper.to_string_lossy().into_owned(),
        models.to_string_lossy().into_owned(),
        common::repo_root()
            .join("tests/fakes/fake_codex.py")
            .to_string_lossy()
            .into_owned(),
    ]);
    let b = parse_brief(&brief("")).unwrap();
    let s = for_attempt(&server, None, d.path(), "u1", "a", "terra", &b, None).unwrap();
    assert_eq!(s.model, "gpt-6.1-sol");
    let before = std::fs::read(snapshot_path(d.path(), "u1")).unwrap();
    server.close();
    std::fs::write(&models, "gpt-6.2-sol").unwrap();
    assert_eq!(
        for_attempt(&server, None, d.path(), "u1", "a", "terra", &b, None).unwrap(),
        s
    );
    assert_eq!(
        std::fs::read(snapshot_path(d.path(), "u1")).unwrap(),
        before
    );
    assert_eq!(
        for_attempt(&server, None, d.path(), "u1", "b", "terra", &b, None)
            .unwrap()
            .model,
        "gpt-6.2-sol"
    );
}
#[test]
fn brief_map_mismatch() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(
        d.path().join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu1\tm\tpending\t\t\t\t\n",
    )
    .unwrap();
    std::fs::write(
        d.path().join("modules.md"),
        module("m", &routed("default sol/xhigh parser reason")),
    )
    .unwrap();
    let b = axiom::orch::parse::parse_brief(&brief("EFFORT high")).unwrap();
    let server = axiom::codex::rpc::AppServer::new(vec![]);
    assert_eq!(
        for_attempt(&server, None, d.path(), "u1", "a", "terra", &b, None)
            .unwrap_err()
            .kind,
        "routing_mismatch"
    );
}
fn route(tier: Tier, effort: Effort, risk: Option<Risk>) -> Routing {
    Routing {
        tier,
        effort,
        risk,
        reason: "test".into(),
    }
}
fn fix() -> FixRecord {
    FixRecord {
        unit: "u".into(),
        parent: "p".into(),
        parent_tier: Tier::Sol,
        parent_effort: Some(Effort::High),
        tier: Tier::Sol,
        effort: Effort::Xhigh,
        rule: Rule::FixEscalation,
        reason: "step".into(),
    }
}
fn snap() -> RoutingSnapshot {
    RoutingSnapshot {
        unit: "u".into(),
        attempt_id: "a".into(),
        role: "map".into(),
        tier: Some(Tier::Sol),
        effort: Some(Effort::High),
        risk: Some(Risk::Standard),
        model: "m".into(),
        rule: Rule::MapUnit,
        reason: "r".into(),
        settings_version: None,
    }
}
fn receipt() -> RoutingReceipt {
    RoutingReceipt {
        unit: "u".into(),
        attempt: "a".into(),
        tier: Some("sol".into()),
        effort: None,
        risk: None,
        model_assigned: "m".into(),
        model_used: None,
        rule: "map_unit".into(),
        reason: "r".into(),
        settings_version: None,
    }
}
#[test]
fn parse_effort_accepts_five() {
    let e = [
        Effort::Low,
        Effort::Medium,
        Effort::High,
        Effort::Xhigh,
        Effort::Max,
    ];
    for (s, v) in ["low", "medium", "high", "xhigh", "max"].iter().zip(e) {
        assert_eq!(parse_effort(s), Ok(v));
    }
    assert!(e.windows(2).all(|w| w[0] < w[1]));
}
#[test]
fn parse_effort_rejects_ultra() {
    for s in ["ultra", "", "HIGH"] {
        assert!(parse_effort(s).is_err());
    }
}
#[test]
fn parse_tier_values() {
    for (s, t) in [
        ("terra", Tier::Terra),
        ("sol", Tier::Sol),
        ("astra", Tier::Astra),
        ("luna", Tier::Luna),
    ] {
        assert_eq!(parse_tier(s), Ok(t));
    }
    assert!(parse_tier("gpt-6.1-sol").is_err());
}
#[test]
fn rubric_astra_effort() {
    for e in [Effort::Low, Effort::Medium] {
        assert!(check_rubric(&route(Tier::Astra, e, Some(Risk::Novel))).is_err());
    }
    for e in [Effort::High, Effort::Xhigh, Effort::Max] {
        assert!(check_rubric(&route(Tier::Astra, e, Some(Risk::Novel))).is_ok());
    }
}
#[test]
fn rubric_terra_only_trivial() {
    assert!(check_rubric(&route(Tier::Terra, Effort::Medium, Some(Risk::Trivial))).is_ok());
    for r in [Risk::Standard, Risk::Parser] {
        assert!(check_rubric(&route(Tier::Terra, Effort::Medium, Some(r))).is_err());
    }
}
#[test]
fn rubric_risky_needs_xhigh() {
    for r in [
        Risk::StateMachine,
        Risk::Concurrency,
        Risk::Migration,
        Risk::Parser,
        Risk::Security,
        Risk::Privacy,
        Risk::CrossModule,
        Risk::PriorFailure,
    ] {
        assert!(check_rubric(&route(Tier::Sol, Effort::High, Some(r))).is_err());
        for e in [Effort::Xhigh, Effort::Max] {
            assert!(check_rubric(&route(Tier::Sol, e, Some(r))).is_ok());
        }
        assert!(check_rubric(&route(Tier::Astra, Effort::High, Some(r))).is_ok());
    }
}
#[test]
fn rubric_standard_sol_high() {
    assert!(check_rubric(&route(Tier::Sol, Effort::High, Some(Risk::Standard))).is_ok());
    assert!(check_rubric(&route(Tier::Luna, Effort::High, Some(Risk::Standard))).is_err());
    assert!(check_rubric(&route(Tier::Sol, Effort::High, None)).is_err());
    assert!(check_rubric(&route(Tier::Sol, Effort::High, Some(Risk::Novel))).is_err());
}
fn step(t: Tier, e: Option<Effort>, expected: (Tier, Effort)) {
    match escalate(t, e) {
        Escalation::Next(r) => {
            assert_eq!((r.tier, r.effort), expected);
            assert_eq!(r.risk, None);
            assert!(!r.reason.is_empty());
        }
        _ => panic!("expected next"),
    }
}
#[test]
fn ladder_every_step() {
    for e in [
        Effort::Low,
        Effort::Medium,
        Effort::High,
        Effort::Xhigh,
        Effort::Max,
    ] {
        step(Tier::Terra, Some(e), (Tier::Sol, Effort::High));
        step(Tier::Luna, Some(e), (Tier::Sol, Effort::High));
    }
    for (t, e, nt, ne) in [
        (Tier::Sol, Effort::Low, Tier::Sol, Effort::High),
        (Tier::Sol, Effort::Medium, Tier::Sol, Effort::High),
        (Tier::Sol, Effort::High, Tier::Sol, Effort::Xhigh),
        (Tier::Sol, Effort::Xhigh, Tier::Astra, Effort::Xhigh),
        (Tier::Sol, Effort::Max, Tier::Astra, Effort::Max),
        (Tier::Astra, Effort::High, Tier::Astra, Effort::Xhigh),
        (Tier::Astra, Effort::Xhigh, Tier::Astra, Effort::Max),
    ] {
        step(t, Some(e), (nt, ne));
    }
}
#[test]
fn ladder_saturated_and_legacy() {
    assert_eq!(
        escalate(Tier::Astra, Some(Effort::Max)),
        Escalation::Saturated
    );
    for t in [Tier::Terra, Tier::Luna, Tier::Sol] {
        step(t, None, (Tier::Sol, Effort::High));
    }
}
#[test]
fn serde_names() {
    assert_eq!(
        serde_json::to_string(&Risk::CrossModule).unwrap(),
        "\"cross_module\""
    );
    assert_eq!(serde_json::to_string(&Tier::Astra).unwrap(), "\"astra\"");
    assert_eq!(serde_json::to_string(&Effort::Xhigh).unwrap(), "\"xhigh\"");
    assert_eq!(
        serde_json::to_string(&Rule::PlanException).unwrap(),
        "\"plan_exception\""
    );
}
#[test]
fn fix_record_roundtrip() {
    let d = tempfile::tempdir().unwrap();
    let f = fix();
    write_fix(d.path(), &f).unwrap();
    assert_eq!(
        fix_record_path(d.path(), "u"),
        d.path().join("routing/u.fix.json")
    );
    assert_eq!(read_fix(d.path(), "u").unwrap(), Some(f));
    assert_eq!(read_fix(d.path(), "missing").unwrap(), None);
}
#[test]
fn fix_record_atomic() {
    let d = tempfile::tempdir().unwrap();
    let mut f = fix();
    write_fix(d.path(), &f).unwrap();
    f.reason = "new".into();
    write_fix(d.path(), &f).unwrap();
    assert_eq!(read_fix(d.path(), "u").unwrap(), Some(f));
    assert!(std::fs::read_dir(d.path().join("routing"))
        .unwrap()
        .all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
}
#[test]
fn snapshot_roundtrip() {
    let d = tempfile::tempdir().unwrap();
    let s = snap();
    write_snapshot(d.path(), &s).unwrap();
    assert_eq!(
        snapshot_path(d.path(), "u"),
        d.path().join("routing/u.attempt.json")
    );
    assert_eq!(read_snapshot(d.path(), "u").unwrap(), Some(s));
    assert_eq!(read_snapshot(d.path(), "missing").unwrap(), None);
    std::fs::write(snapshot_path(d.path(), "u"), "{").unwrap();
    assert!(read_snapshot(d.path(), "u").is_err());
}
#[test]
fn routing_receipts_append() {
    let d = tempfile::tempdir().unwrap();
    let a = receipt();
    let mut b = a.clone();
    b.attempt = "b".into();
    record_routing(d.path(), &a).unwrap();
    record_routing(d.path(), &b).unwrap();
    assert_eq!(read_routing(d.path()).unwrap(), vec![a, b]);
    assert!(!d.path().join("receipts/runs.jsonl").exists());
}
#[test]
fn old_store_no_routing_jsonl() {
    let d = tempfile::tempdir().unwrap();
    assert!(read_routing(d.path()).unwrap().is_empty());
}

use axiom::orch::parse::{lint_map, parse_brief, parse_map};
fn brief(setting: &str) -> String {
    format!("GOAL test\nSCOPE src/a\nVERIFY test\n{setting}\n")
}
fn module(id: &str, routing: &str) -> String {
    format!("## {id}\nPURPOSE test\nPATHS {id}\nDEPENDS none\nCONTRACT test\nACCEPT file `test`\n{routing}\n")
}
fn routed(line: &str) -> String {
    format!("ROUTING\n  {line}\n")
}
fn has(text: &str, kind: &str) -> bool {
    lint_map(text, None, None).iter().any(|p| p.kind == kind)
}
#[test]
fn brief_accepts_xhigh_and_astra() {
    for (k, v) in [
        ("EFFORT", "xhigh"),
        ("EFFORT", "max"),
        ("TIER", "astra"),
        ("TIER", "sol"),
    ] {
        let b = parse_brief(&brief(&format!("{k} {v}"))).unwrap();
        assert_eq!(
            if k == "TIER" { b.tier } else { b.effort }.as_deref(),
            Some(v)
        );
    }
}
#[test]
fn brief_rejects_ultra_and_duplicates() {
    for s in [
        "EFFORT ultra",
        "EFFORT high\nEFFORT low",
        "TIER luna\nTIER luna",
    ] {
        assert!(parse_brief(&brief(s)).is_err());
    }
}
#[test]
fn map_routing_parsed() {
    let m = module(
        "a",
        &routed("default sol/xhigh parser because risk\n  unit-id astra/high cross_module reason"),
    ) + &module("b", &routed("default terra/low trivial simple"));
    let json = serde_json::to_value(parse_map(&m).unwrap()).unwrap();
    assert_eq!(
        json["modules"][0]["routing"],
        serde_json::json!([{ "key":"default", "tier":"sol", "effort":"xhigh", "risk":"parser", "reason":"because risk" }, { "key":"unit-id", "tier":"astra", "effort":"high", "risk":"cross_module", "reason":"reason" }])
    );
}
#[test]
fn map_rubric_violation() {
    let m = module("a", &routed("default terra/medium parser x"));
    assert!(has(&m, "routing_rubric"));
    assert!(parse_map(&m).is_err());
}
#[test]
fn map_luna_rejected() {
    assert!(has(
        &module("a", &routed("default luna/high trivial x")),
        "routing_luna"
    ));
}
#[test]
fn map_partial_routing() {
    assert!(has(
        &(module("a", &routed("default sol/high standard x")) + &module("b", "")),
        "routing_partial"
    ));
}
#[test]
fn map_bad_value_and_empty_reason() {
    assert!(has(
        &module("a", &routed("default sol/ultra standard x")),
        "routing_value"
    ));
    assert!(has(
        &module("a", &routed("default sol/high standard")),
        "routing_reason"
    ));
}
#[test]
fn map_without_routing_unchanged() {
    let m = include_str!("fixtures/orch/modules.md");
    let j = serde_json::to_value(parse_map(m).unwrap()).unwrap();
    assert!(j["modules"]
        .as_array()
        .unwrap()
        .iter()
        .all(|m| m["routing"] == serde_json::json!([])));
    assert!(!lint_map(m, None, None)
        .iter()
        .any(|p| p.kind.starts_with("routing_")));
}
#[test]
fn routed_store_true_false() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("modules.md");
    assert!(!routed_store(d.path()).unwrap());
    for (s, expected) in [
        (module("a", &routed("default sol/high standard x")), true),
        (module("a", ""), false),
        (
            module("a", &routed("default sol/high standard x")) + &module("b", ""),
            false,
        ),
        ("garbage".into(), false),
    ] {
        std::fs::write(&p, s).unwrap();
        assert_eq!(routed_store(d.path()).unwrap(), expected);
    }
}
#[test]
fn plan_schema_changes_required() {
    let plan: serde_json::Value =
        serde_json::from_str(include_str!("../engine/research/plan.schema.json")).unwrap();
    assert!(plan["required"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("changes")));
    assert_eq!(
        plan["properties"]["changes"]["items"]["required"],
        serde_json::json!(["tier", "effort", "risk", "reason"])
    );
}

fn schema_required_matches_properties(node: &serde_json::Value) -> bool {
    match node {
        serde_json::Value::Object(map) => {
            if map.get("type") == Some(&serde_json::json!("object")) {
                let Some(properties) = map.get("properties").and_then(|v| v.as_object()) else {
                    return false;
                };
                let Some(required) = map.get("required").and_then(|v| v.as_array()) else {
                    return false;
                };
                let required: Option<std::collections::BTreeSet<_>> =
                    required.iter().map(|v| v.as_str()).collect();
                if required != Some(properties.keys().map(String::as_str).collect()) {
                    return false;
                }
            }
            map.values().all(schema_required_matches_properties)
        }
        serde_json::Value::Array(items) => items.iter().all(schema_required_matches_properties),
        _ => true,
    }
}

#[test]
fn plan_schema_required_negative_control() {
    let plan: serde_json::Value =
        serde_json::from_str(include_str!("../engine/research/plan.schema.json")).unwrap();
    assert!(schema_required_matches_properties(&plan));
    let mut missing_changes = plan.clone();
    missing_changes["required"]
        .as_array_mut()
        .unwrap()
        .retain(|field| field != "changes");
    assert!(!schema_required_matches_properties(&missing_changes));
}

#[test]
fn schemas_carry_changes() {
    for s in [
        include_str!("../engine/research/plan.schema.json"),
        include_str!("../engine/research/audit.schema.json"),
    ] {
        let j: serde_json::Value = serde_json::from_str(s).unwrap();
        let p = &j["properties"]["changes"]["items"]["properties"];
        for (k, vals) in [
            ("tier", vec!["terra", "sol", "astra"]),
            ("effort", vec!["low", "medium", "high", "xhigh", "max"]),
            (
                "risk",
                vec![
                    "trivial",
                    "standard",
                    "state_machine",
                    "concurrency",
                    "migration",
                    "parser",
                    "security",
                    "privacy",
                    "cross_module",
                    "prior_failure",
                    "novel",
                ],
            ),
        ] {
            assert_eq!(p[k]["enum"], serde_json::json!(vals));
        }
        assert_eq!(p["reason"]["type"], "string");
        assert_eq!(
            j["properties"]["changes"]["items"]["additionalProperties"],
            false
        );
    }
    let plan: serde_json::Value =
        serde_json::from_str(include_str!("../engine/research/plan.schema.json")).unwrap();
    assert!(plan["required"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("changes")));
}
#[test]
fn audit_changes_keeps_existing() {
    let audit: serde_json::Value =
        serde_json::from_str(include_str!("../engine/research/audit.schema.json")).unwrap();
    let items = &audit["properties"]["changes"]["items"];
    assert_eq!(
        items["required"],
        serde_json::json!(["id", "area", "location", "evidence", "problem", "change", "verify", "tier", "effort", "risk", "reason"])
    );
    assert!(items["properties"]["id"]["description"]
        .as_str()
        .unwrap()
        .starts_with("One word without spaces"));
    assert!(audit["required"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("changes")));
}
#[test]
fn parse_rs_has_no_format_only_hunks() {
    assert!(include_str!("../src/orch/parse.rs").contains("return Err(format!(\n                        \"has SCOPE twice: line {number} repeats line {m}\"\n                    ));"));
}
#[test]
fn unit_line_overrides_default() {
    let m = module(
        "a",
        &routed("default sol/high standard x\n  unit-id sol/high standard y"),
    );
    let j = serde_json::to_value(parse_map(&m).unwrap()).unwrap();
    assert_eq!(j["modules"][0]["routing"][1]["key"], "unit-id");
}
#[test]
fn map_missing_default_and_invalid_values() {
    assert!(has(
        &module("a", &routed("unit sol/high standard x")),
        "routing_default"
    ));
    for line in [
        "default",
        "default bogus/high standard x",
        "default sol/high unknown x",
    ] {
        assert!(has(&module("a", &routed(line)), "routing_value"));
    }
}
