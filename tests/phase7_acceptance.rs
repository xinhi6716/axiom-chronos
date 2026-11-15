mod common;
#[path = "phase7/mod.rs"]
mod phase7;

use axiom::{bridge::{call, Bridge, Config}, orch, usage::review_stats};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::{Path, PathBuf}, process::{Command, Stdio}};

// A1 criterion; A2 model/role table; A3 seed findings before review;
// A4 planning-home isolation; A5 wish items (separate tests); A6 frozen inputs.
fn seeds() -> PathBuf {
    common::repo_root().join("tests/fixtures/phase7/acceptance")
}
fn workload() -> phase7::Workload { phase7::run_workload(&seeds()) }
fn before() -> phase7::Metrics {
    serde_json::from_slice(&fs::read(seeds().parent().unwrap().join("before.json")).unwrap()).unwrap()
}
fn metrics(w: &phase7::Workload) -> phase7::Metrics {
    phase7::collect(w.home.path(), &w.project)
}
fn bridge(w: &phase7::Workload, name: &str) -> Bridge {
    let mut fake = common::fake("fake_codex");
    fake.push(format!("--fake-log={}", w.home.path().join(name).display()));
    Bridge::new(Config { data_home: Some(w.home.path().into()), codex: Ok(fake), wsl: None, on_windows: false })
}
fn invoke(b: &Bridge, tool: &str, args: Value) -> Value {
    call(b, tool, &args)["structuredContent"].clone()
}
fn turns(path: &Path) -> Vec<Value> {
    fs::read_to_string(path).unwrap_or_default().lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|v| v["method"] == "turn/start").collect()
}

#[test]
fn pass_criterion_holds() {
    let before = before();
    let w = workload();
    let after = metrics(&w);
    for (label, m) in [("before", &before), ("after", &after)] {
        println!("{label}: Claude calls/verified unit={:?}, Claude tokens/verified unit={:?}, rework rate={:?}",
            m.claude_calls_per_unit, m.claude_tokens_per_unit, m.rework_rate);
    }
    phase7::passes(&before, &after).unwrap();
}

#[test]
fn per_model_role_table_printed() {
    let w = workload();
    let after = metrics(&w);
    assert!(!after.by_model_role.is_empty());
    println!("model\trole\tcalls\ttokens");
    for (key, row) in &after.by_model_role {
        let (model, role) = key.split_once('|').unwrap();
        println!("{model}\t{role}\t{}\t{}", row.calls,
            row.tokens.map(|n| n.to_string()).unwrap_or_else(|| "unknown".into()));
    }
    let label = phase7::fixture_cost_label(&after);
    assert!(label.contains("fixture-only"));
    println!("{label}");
}

#[test]
fn selfcheck_catches_both_seeds_before_review() {
    let w = workload();
    for (unit, finding) in [("vacuous", "vacuous_check"), ("missing", "untested")] {
        let report: Value = serde_json::from_slice(&fs::read(w.store.join(format!("reports/{unit}.selfcheck.json"))).unwrap()).unwrap();
        assert!(report["findings"].as_array().unwrap().iter().any(|f| f["kind"] == finding), "{unit}: {report}");
        let rows = orch::evidence(&w.store, unit).unwrap();
        let check = rows.iter().position(|r| r.kind == "selfcheck").expect("selfcheck evidence");
        let review = rows.iter().position(|r| r.kind == "review").expect("review evidence");
        assert!(check < review, "{unit}: selfcheck must precede review");
    }
}

#[test]
fn astra_summary_only_from_own_home() {
    let w = workload();
    let unrelated = common::temp();
    let other = orch::wish_store(unrelated.path(), &w.project, Some("w-1234567890123-abcdef")).unwrap();
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("units.tsv"), "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nmarker\tm\tverified\t\t\t\tmissing\n").unwrap();
    let marker = "UNRELATED_HOME_MARKER_7ad908";
    orch::events::append_event(&other, "review_recorded", None, Some("marker"), json!({
        "review_id":"marker-review", "unit":"marker", "role":"reviewer", "reviewer":"astra-max",
        "model":"review-model", "worker_model":marker, "worker_effort":"high", "risk":"standard", "pass":true, "findings":[]
    })).unwrap();
    assert!(review_stats::astra_summary(unrelated.path(), &w.project).contains(marker));
    let summary = review_stats::astra_summary(w.home.path(), &w.project);
    assert!(!summary.contains(marker));
    let b = bridge(&w, "planning.log");
    let r = invoke(&b, "codex_run", json!({"project":w.project,"cwd":w.repo,"role":"astra",
        "sandbox":"read-only","brief":"PHASE7 PLANNING BRIEF","output_schema":{"type":"object"},
        "save_to":w.store.join("astra/acceptance-plan.json")}));
    assert_eq!(r["status"], "completed", "{r}");
    let log = turns(&w.home.path().join("planning.log"));
    let prompt = log[0]["params"]["input"][0]["text"].as_str().unwrap();
    assert!(summary.contains("FIRST-PASS STATS"), "workload home has no summary; planning prompt: {prompt}");
    assert!(prompt.contains(&summary), "{prompt}");
    assert!(prompt.find("FIRST-PASS STATS").unwrap() < prompt.find("PHASE7 PLANNING BRIEF").unwrap());
    assert!(!prompt.contains(marker));
}

#[test]
fn wish_acceptance_first_pass_stats_recorded() {
    let w = workload();
    let rows = review_stats::collect_first_pass(&[w.store.clone()]).unwrap();
    assert!(!rows.is_empty());
    assert_eq!(rows.iter().map(|r| r.units).sum::<u64>(), 2);
    assert_eq!(rows.iter().map(|r| r.first_pass.den).sum::<u64>(), 2);
    assert_eq!(rows.iter().map(|r| r.first_pass.num).sum::<u64>(), 2);
    assert_eq!(rows.iter().map(|r| r.verified).sum::<u64>(), 2);
}

#[test]
fn seeds_and_before_untouched() {
    let dir = seeds();
    let hashes = fs::read_to_string(dir.parent().unwrap().join("acceptance.sha256")).unwrap();
    let mut files: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    assert_eq!(files.len(), 2);
    assert_eq!(hashes.lines().count(), 2);
    for path in files {
        let line = format!("{:x}  {}", Sha256::digest(fs::read(&path).unwrap()), path.file_name().unwrap().to_str().unwrap());
        assert!(hashes.lines().any(|l| l == line), "seed changed: {}", path.display());
    }
    let m = before();
    assert_eq!(m.claude_calls_per_unit, Some(3.0));
    assert_eq!(m.claude_tokens_per_unit, Some(45.0));
    assert_eq!(m.rework_rate, Some(0.0));
}

fn orch_ok(w: &phase7::Workload, args: &[&str]) -> orch::Output {
    let out = orch::run(&w.store, &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(), true, false);
    assert_eq!(out.exit_code, 0, "{args:?}: {}", out.stderr);
    out
}

#[test]
fn wish_acceptance_items_present() {
    // Exercise brief routing, real matrix RED/GREEN and the routed default reviewer
    // through the bridge, inspecting the fake's actual turn/start messages.
    for (tier, effort, model) in [("sol", "xhigh", "gpt-6-sol"), ("astra", "high", "gpt-6-astra")] {
        let w = workload();
        let modules = fs::read_to_string(w.store.join("modules.md")).unwrap();
        let modules = modules.replace("ACCEPT test `echo ok`", "ACCEPT test `echo ok`\nROUTING\n  default sol/xhigh standard acceptance");
        fs::write(w.store.join("modules.md"), format!("{modules}\n## matrix\nPURPOSE acceptance\nPATHS acceptance/ src/\nDEPENDS none\nCONTRACT result\nACCEPT test `echo ok`\nROUTING\n  default {tier}/{effort} standard acceptance\n")).unwrap();
        fs::create_dir_all(w.repo.join("acceptance")).unwrap();
        fs::write(w.repo.join("acceptance/value.txt"), "stub").unwrap();
        let template = "import pathlib,unittest\nclass Matrix(unittest.TestCase):\n def test_value(self):\n  self.assertEqual(pathlib.Path('acceptance/value.txt').read_text(), 'implemented')\nunittest.main(verbosity=2)\n";
        let template_path = w.home.path().join("matrix-template.py");
        fs::write(&template_path, template).unwrap();
        let brief = w.store.join("matrix.md");
        let command = format!("\"{}\" acceptance/matrix.py", common::python());
        fs::write(&brief, format!("GOAL implement result\nSCOPE acceptance/ src/\nVERIFY {command}\nTIER {tier}\nEFFORT {effort}\nRISK standard\nCONTRACT Return implemented\nTEST_MATRIX\nA | behaviour | acceptance/matrix.py::test_value | checks result | {command}\nCONTEXT\nFAKE_SELFCHECK {{\"status\":\"pass\",\"changed\":false,\"fixed\":0,\"findings\":[]}}\nFAKE_REVIEW {{\"verdict\":\"pass\",\"findings\":[]}}\n")).unwrap();
        orch_ok(&w, &["unit", "add", "matrix", "--track", "matrix", "--brief", brief.to_str().unwrap()]);
        assert_eq!(axiom::engine::policy::load(&w.store).unwrap().reviewer, axiom::engine::policy::ReviewerMode::AstraMax);
        assert_eq!(orch::unit_brief(&w.store, "matrix").unwrap().test_matrix.len(), 1);
        let wrapper = w.home.path().join("matrix-fake.py");
        // Same stdin hook as test_first.rs: only the fake worker writes tests/code.
        fs::write(&wrapper, format!(r#"import json,pathlib,runpy,sys
repo=pathlib.Path({repo})
template=pathlib.Path({template})
original=sys.stdin
class Input:
 def __getattr__(self,k): return getattr(original,k)
 def __iter__(self): return self
 def __next__(self):
  line=next(original)
  msg=json.loads(line)
  if msg.get('method')=='turn/start':
   p=msg['params']; text=p['input'][0]['text']
   if 'TESTS-FIRST' in text: (repo/'acceptance/matrix.py').write_text(template.read_text())
   elif not p.get('outputSchema'): (repo/'acceptance/value.txt').write_text('implemented')
  return line
sys.stdin=Input()
sys.argv=sys.argv[1:]
runpy.run_path(sys.argv[0],run_name='__main__')
"#, repo=json!(w.repo.to_string_lossy()), template=json!(template_path.to_string_lossy()))).unwrap();
        let log = w.home.path().join("matrix.log");
        let b = Bridge::new(Config { data_home:Some(w.home.path().into()), codex:Ok(vec![common::python(),
            "-I".into(), wrapper.to_string_lossy().into_owned(), common::repo_root().join("tests/fakes/fake_codex.py").to_string_lossy().into_owned(),
            format!("--fake-log={}", log.display())]), wsl:None, on_windows:false });
        let r = invoke(&b, "codex_run", json!({"project":w.project,"unit":"matrix","role":"terra","cwd":w.repo,"cycle":true}));
        assert_eq!(r["status"], "completed", "{r}");
        let logged = turns(&log);
        assert!(logged.len() >= 3, "worker, selfcheck and default reviewer must run: {logged:?}");
        assert!(logged[0]["params"]["input"][0]["text"].as_str().unwrap().contains("TESTS-FIRST"));
        let requests: Vec<Value> = fs::read_to_string(&log).unwrap().lines().map(|s| serde_json::from_str(s).unwrap()).collect();
        let threads: Vec<_> = requests.iter().filter(|r| r["method"] == "thread/start").collect();
        // The app-server model is assigned on thread/start; turn/start carries effort.
        for turn in &logged[..2] {
            let thread = turn["params"]["threadId"].as_str().unwrap().strip_prefix("thread-").unwrap().parse::<usize>().unwrap();
            assert_eq!(threads[thread - 1]["params"]["model"], model, "{turn}");
            assert_eq!(turn["params"]["effort"], effort, "{turn}");
        }
        assert!(threads.iter().any(|t| t["params"]["model"] == "gpt-6-astra"));
        let rows = orch::evidence(&w.store, "matrix").unwrap();
        assert!(rows.iter().any(|r| r.kind == "red" && r.exit_code == 1));
        assert!(rows.iter().any(|r| r.kind == "verify" && r.exit_code == 0 && r.command == command));
        assert!(rows.iter().any(|r| r.kind == "review" && r.exit_code == 0));
        assert_eq!(orch::read_units(&w.store).unwrap().iter().find(|u| u.id == "matrix").unwrap().state, "verified");
    }
}

#[test]
fn wish_acceptance_fix_unit_escalation() {
    let w = workload();
    let brief = w.store.join("fix-origin.md");
    let modules = fs::read_to_string(w.store.join("modules.md")).unwrap();
    fs::write(w.store.join("modules.md"), modules.replace("ACCEPT test `echo ok`", "ACCEPT test `echo ok`\nROUTING\n  default sol/xhigh standard acceptance")).unwrap();
    fs::write(&brief, "GOAL fix\nSCOPE missing.txt\nVERIFY echo ok\n").unwrap();
    orch_ok(&w, &["unit","add","origin","--track","missing","--brief",brief.to_str().unwrap()]);
    let out = orch_ok(&w, &["unit","add","fix1","--track","missing","--brief",brief.to_str().unwrap(),"--fixes","origin"]);
    let value: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(value["routing"]["tier"], "astra");
    assert_eq!(value["routing"]["effort"], "xhigh");
    assert_eq!(orch::fixed_unit(&w.store,"fix1").unwrap().as_deref(), Some("origin"));
    assert!(orch::evidence(&w.store,"fix1").unwrap().iter().any(|e| e.kind == "fixes" && e.command == "origin"));
    orch_ok(&w, &["unit","add","fix2","--track","missing","--brief",brief.to_str().unwrap(),"--fixes","fix1"]);
    let out = orch::run(&w.store, &vec!["unit","add","fix3","--track","missing","--brief",brief.to_str().unwrap(),"--fixes","fix2"].into_iter().map(str::to_owned).collect::<Vec<_>>(), true, false);
    assert_ne!(out.exit_code, 0);
    assert!(out.stderr.contains("fix_rounds:"), "{}", out.stderr);
}

#[test]
fn wish_acceptance_core_moves_to_sonnet() {
    use axiom::{goal, session};
    use axiom::session::Clock;
    let w = workload();
    let home = w.home.path();
    let id = "12345678-1234-1234-1234-123456789abc";
    let g = goal::open(home, id, "phase7 stages", w.repo.to_str()).unwrap();
    let store = orch::store_for_goal(home, &g).unwrap();
    fs::create_dir_all(store.join("astra")).unwrap();
    fs::create_dir_all(store.join("reports")).unwrap();
    fs::write(store.join("reports/_full.evidence.tsv"), "kind\tcommand\texit_code\tcommit\tts\nfull\tfixture\t0\t\t9999\n").unwrap();
    let record = home.join("stages.json");
    let wrapper = home.join("stage-fake.py");
    fs::write(&wrapper, format!("import os,runpy,sys\nos.environ['FAKE_CLAUDE_SCENARIO']='stages'\nos.environ['FAKE_CLAUDE_SET_MODEL']='supported'\nos.environ['FAKE_CLAUDE_SESSION']={id}\nos.environ['FAKE_CLAUDE_RECORD']={record}\nsys.argv=sys.argv[1:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n", id=json!(id), record=json!(record.to_string_lossy()))).unwrap();
    let config = session::Config {home:home.into(), bin:home.join("axiom"), clock:Clock::system(), claude:Ok(vec![common::python(),"-I".into(),wrapper.to_string_lossy().into_owned(), common::repo_root().join("tests/fakes/fake_claude.py").to_string_lossy().into_owned()])};
    let spawned = session::spawn(&config,"wish","phase7 stages",&w.repo,None,None).unwrap();
    let mut n = 0;
    let code = session::drive(spawned,&config,&mut Vec::new(),&std::sync::atomic::AtomicBool::new(false),&mut |event| {
        if event["type"] != "result" {return;}
        n += 1;
        if n == 1 {
            fs::write(store.join("astra/plan-final.md"), "plan").unwrap();
            fs::write(store.join("modules.md"), "modules").unwrap();
        } else { goal::set_status(home,id,"done",None).unwrap(); }
    }).unwrap();
    assert_eq!(code,0);
    let rows = session::stage::read_turns(home,id).unwrap();
    assert_eq!(rows.len(),2);
    assert_eq!(rows[0].model,"opus");
    assert_eq!(rows[1].stage,"build");
    assert_eq!(rows[1].model,"sonnet");
    assert!(rows[1].ok);
    assert!(matches!(rows[1].via, session::stage::Via::Control | session::stage::Via::Resume));
    let actual: Value = serde_json::from_slice(&fs::read(record).unwrap()).unwrap();
    assert_eq!(actual["turn_models"], json!(["opus","sonnet"]));
}

// A killed owner must not cause either previously verified seed to be dispatched.
#[test]
fn wish_acceptance_kill_child() {
    let Ok(home) = std::env::var("PHASE7_KILL_HOME") else {return};
    let home = PathBuf::from(home);
    let store = orch::store_for(&home,"phase7").unwrap();
    orch::begin_unit(&store,"pending","terra",&home.join("repo"),false).unwrap();
    fs::write(home.join("claimed"),"ready").unwrap();
    loop { std::thread::park(); }
}

#[test]
fn wish_acceptance_resume_after_kill_skips_verified() {
    let w = workload();
    let brief = w.store.join("pending.md");
    let mut modules = fs::read_to_string(w.store.join("modules.md")).unwrap();
    modules.push_str("\n## pending\nPURPOSE pending\nPATHS pending.txt\nDEPENDS none\nCONTRACT pending\nACCEPT test `echo ok`\nROUTING\n  default sol/xhigh standard acceptance\n");
    fs::write(w.store.join("modules.md"), modules).unwrap();
    fs::write(&brief,"GOAL pending\nSCOPE pending.txt\nVERIFY echo ok\nCONTRACT pending\nTEST_MATRIX\nP | behaviour | pending.txt::test_pending | checks pending | echo ok\n").unwrap();
    orch_ok(&w,&["unit","add","pending","--track","pending","--brief",brief.to_str().unwrap()]);
    let before = metrics(&w);
    let evidence: Vec<_> = ["missing","vacuous"].iter().map(|u| json!(orch::evidence(&w.store,u).unwrap())).collect();
    let mut child = Command::new(std::env::current_exe().unwrap()).args(["--exact","wish_acceptance_kill_child","--nocapture"])
        .env("PHASE7_KILL_HOME",w.home.path()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !w.home.path().join("claimed").exists() && std::time::Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {break;}
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let claimed = w.home.path().join("claimed").exists();
    let _ = child.kill();
    child.wait().unwrap();
    assert!(claimed,"child never claimed pending unit");
    w.resume();
    let b = bridge(&w,"resume.log");
    for (i, unit) in ["missing","vacuous"].iter().enumerate() {
        let r = invoke(&b,"codex_run",json!({"project":w.project,"unit":unit,"role":"terra","cwd":w.repo,"cycle":true}));
        assert_eq!(r["error"]["kind"],"already_verified","{r}");
        assert_eq!(json!(orch::evidence(&w.store,unit).unwrap()),evidence[i]);
    }
    assert!(turns(&w.home.path().join("resume.log")).is_empty());
    assert_eq!(metrics(&w),before);
}
