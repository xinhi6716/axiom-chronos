mod common;
use axiom::{codex::AppServer, engine::self_check, orch};
use serde_json::{json, Value};
use std::{fs, path::PathBuf, process::Command};
struct Fixture {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    store: PathBuf,
    server: AppServer,
    log: PathBuf,
}
impl Fixture {
    fn new(answer: Value, edit: &str) -> Self {
        std::env::set_var("FAKE_CODEX_MODELS", "gpt-6-sol,gpt-6-astra");
        let temp = common::temp();
        let repo = temp.path().join("repo");
        let store = orch::store_for(temp.path(), "demo").unwrap();
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::create_dir_all(store.join("inbox")).unwrap();
        fs::write(
            repo.join("src/test.py"),
            include_str!("fixtures/self_check_dev/seed.py"),
        )
        .unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "seed",
            ],
        ] {
            assert!(Command::new("git")
                .args(args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success());
        }
        let brief = store.join("one.brief");
        fs::write(&brief,format!("GOAL test\nSCOPE src/\nVERIFY echo ok\nCONTRACT behavior\nTEST_MATRIX\ns1 | behaviour | src/test.py::test_case | checks behavior | echo ok\nCONTEXT\nFAKE_SELFCHECK {}\nFAKE_SELFCHECK_WRITE {}\n",answer,edit)).unwrap();
        fs::write(
            store.join("units.tsv"),
            format!(
                "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\none\tm\tpending\t\t\t\t{}\n",
                brief.display()
            ),
        )
        .unwrap();
        let log = temp.path().join("log");
        let mut args = common::fake("fake_codex");
        args.push(format!("--fake-log={}", log.display()));
        let server = AppServer::new(args).with_slots_home(temp.path().join("slots"));
        Self {
            _temp: temp,
            repo,
            store,
            server,
            log,
        }
    }
    fn run(&self) -> self_check::SelfCheck {
        self_check::run(
            &self.server,
            Some(self._temp.path()),
            &self.store,
            "one",
            "a1",
            &self.repo,
        )
        .unwrap()
    }
    fn report(&self) -> Value {
        serde_json::from_slice(&fs::read(self.store.join("reports/one.selfcheck.json")).unwrap())
            .unwrap()
    }
}
fn answer(kind: &str) -> Value {
    json!({"status":"fail","changed":false,"fixed":0,"findings":[{"kind":kind,"reference":"src/test.py::test_case","detail":"problem","fixed":false}]})
}
#[test]
fn self_check_asset_is_registered() {
    let a = axiom::engine::asset("roles/self-check.md").unwrap();
    for k in ["untested", "vacuous_check", "contract_mismatch"] {
        assert!(a.contains(k));
    }
}
#[test]
fn finds_vacuous_test() {
    let f = Fixture::new(answer("vacuous_check"), "");
    let x = f.run();
    assert_eq!(x.findings[0].kind, "vacuous_check");
    assert!(!x.findings[0].reference.is_empty());
    assert_eq!(f.report()["status"], "fail");
}
#[test]
fn finds_untested_row() {
    let f = Fixture::new(answer("untested"), "");
    assert_eq!(f.run().findings[0].kind, "untested");
}
#[test]
fn finds_contract_mismatch() {
    let f = Fixture::new(answer("contract_mismatch"), "");
    assert_eq!(f.run().findings[0].kind, "contract_mismatch");
}
#[test]
fn fix_inside_scope_reverifies() {
    let f = Fixture::new(
        json!({"status":"pass","changed":true,"fixed":1,"findings":[]}),
        "src/test.py",
    );
    let x = f.run();
    assert!(x.changed);
    assert_eq!(x.fixed, 1);
    assert_eq!(x.status, "pass");
    assert_eq!(f.report()["changed"], true);
    assert_eq!(
        orch::evidence(&f.store, "one")
            .unwrap()
            .last()
            .unwrap()
            .exit_code,
        0
    );
}
#[test]
fn edit_outside_scope_fails() {
    let f = Fixture::new(
        json!({"status":"pass","changed":true,"fixed":1,"findings":[]}),
        "outside.txt",
    );
    assert_eq!(f.run().status, "fail");
    assert_eq!(
        orch::evidence(&f.store, "one")
            .unwrap()
            .last()
            .unwrap()
            .exit_code,
        1
    );
}
#[test]
fn model_and_effort_applied() {
    let f = Fixture::new(
        json!({"status":"pass","changed":false,"fixed":0,"findings":[]}),
        "",
    );
    f.run();
    let log = fs::read_to_string(&f.log).unwrap();
    let rows: Vec<Value> = log
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(rows.iter().any(|r| r["method"] == "thread/start"
        && r["params"]["model"].as_str().unwrap_or("").contains("sol")));
    assert!(rows
        .iter()
        .any(|r| r["method"] == "turn/start" && r["params"]["effort"] == "xhigh"));
    let mut policy = axiom::engine::policy::load(&f.store).unwrap();
    let mut models = axiom::engine::policy::Models::default();
    models.self_check.tier = "astra".into();
    models.self_check.effort = "high".into();
    policy.models = Some(models);
    axiom::engine::policy::save(&f.store, &policy).unwrap();
    f.run();
    let log = fs::read_to_string(&f.log).unwrap();
    let rows: Vec<Value> = log
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(rows
        .iter()
        .any(|r| r["method"] == "thread/start" && r["params"]["model"] == "gpt-6-astra"));
    assert!(rows
        .iter()
        .any(|r| r["method"] == "turn/start" && r["params"]["effort"] == "high"));
}
#[test]
fn report_schema_and_asset() {
    let f = Fixture::new(
        json!({"status":"pass","changed":false,"fixed":0,"findings":[]}),
        "",
    );
    f.run();
    let r = f.report();
    for k in [
        "unit",
        "attempt_id",
        "tree",
        "model",
        "effort",
        "thread_id",
        "status",
        "changed",
        "fixed",
        "findings",
    ] {
        assert!(r.get(k).is_some(), "{k}");
    }
    self_check_asset_is_registered();
}

#[test]
fn state_machine_states() {
    let f = Fixture::new(
        json!({"status":"pass","changed":false,"fixed":0,"findings":[]}),
        "",
    );
    assert_eq!(
        orch::selfcheck_state(&f.store, "one").unwrap(),
        orch::SelfCheckState::Missing
    );
    let add = |kind: &str, exit_code: i32, commit: &str| orch::append_evidence(&f.store, "one", &orch::Evidence {kind: kind.into(), command: "a1|engine-selfcheck:test".into(), exit_code, commit: commit.into(), ts: "1".into()}).unwrap();
    add("verify", 0, "tree:a");
    add("selfcheck", 0, "tree:a");
    assert_eq!(orch::selfcheck_state(&f.store, "one").unwrap(), orch::SelfCheckState::Passed);
    add("selfcheck", 1, "tree:a");
    assert_eq!(orch::selfcheck_state(&f.store, "one").unwrap(), orch::SelfCheckState::Failed);
    add("verify", 0, "tree:b");
    assert_eq!(orch::selfcheck_state(&f.store, "one").unwrap(), orch::SelfCheckState::Stale);
    fs::write(f.store.join("one.brief"), "GOAL test\nSCOPE src/\nVERIFY true\n").unwrap();
    assert_eq!(orch::selfcheck_state(&f.store, "one").unwrap(), orch::SelfCheckState::NotRequired);

}
#[test]
fn gate_through_each_entry() {
    let f = Fixture::new(
        json!({"status":"pass","changed":false,"fixed":0,"findings":[]}),
        "",
    );
    let tree = orch::snapshot(&f.repo, &["src/".into()]);
    let home = f._temp.path();
    // Bind the hook session to the same project store (without a wish-local store).
    fs::create_dir_all(home.join("goals")).unwrap();
    fs::write(home.join("goals/gate.json"), json!({
        "wish":"test", "status":"active", "cwd":f.repo, "project":"demo"
    }).to_string()).unwrap();
    fs::create_dir_all(f.store.join("agents")).unwrap();
    fs::write(f.store.join("agents/reviewer.reviewer"), "one").unwrap();
    let event = json!({"hook_event_name":"PostToolUse", "session_id":"gate",
        "tool_name":"Agent", "tool_input":{"subagent_type":"chronos:reviewer"},
        "tool_response":{"agentId":"reviewer", "status":"completed"}});
    let mut policy = axiom::engine::policy::load(&f.store).unwrap();
    policy.reviewer = axiom::engine::policy::ReviewerMode::Both;
    axiom::engine::policy::save(&f.store, &policy).unwrap();
    let add = |kind: &str, exit_code: i32, commit: &str| orch::append_evidence(&f.store, "one", &orch::Evidence {kind: kind.into(), command: "a1|engine-selfcheck:test".into(), exit_code, commit: commit.into(), ts: "1".into()}).unwrap();
    add("verify", 0, &tree);
    orch::review_pack(&f.store, "one", &f.repo).unwrap();
    let pack = orch::pack_path(&f.store, "one");
    fs::write(&pack, format!("{}\nFAKE_REVIEW {{\"verdict\":\"pass\",\"findings\":[]}}\n",
        fs::read_to_string(&pack).unwrap())).unwrap();
    fs::write(orch::verdict_path(&f.store, "one"), json!({
        "unit":"one", "tree":tree, "verdict":"pass", "findings":[]
    }).to_string()).unwrap();
    let check_entries = |state: orch::SelfCheckState, allowed: bool| {
        for entry in ["record_review", "cli", "hook", "engine"] {
            assert_eq!(orch::selfcheck_state(&f.store, "one").unwrap(), state);
            let reviews = || orch::evidence(&f.store, "one").unwrap().iter()
                .filter(|r| r.kind == "review").count();
            let before = reviews();
            match entry {
                "record_review" => {
                    let result = orch::record_review(&f.store, &["one".into()], "engine-astra:test", true, &f.repo);
                    if allowed { result.unwrap(); }
                    else { assert_eq!(result.unwrap_err().kind, "selfcheck_required", "{state:?}: {entry}"); }
                }
                "cli" => {
                    let out = Command::new(env!("CARGO_BIN_EXE_axiom"))
                        .args(["orch", "--store"]).arg(&f.store)
                        .args(["review", "one", "--reviewer", "chronos:reviewer", "--repo"])
                        .arg(&f.repo)
                        .env("AXIOM_HOME", home).env("AXIOM_DATA_HOME", home)
                        .env_remove("ORCH_STORE").output().unwrap();
                    let error = String::from_utf8_lossy(&out.stderr);
                    assert_eq!(out.status.success(), allowed, "{state:?}: {entry}: {error}");
                    // The CLI renders Failure.message, not Failure.kind.
                    if !allowed { assert_eq!(error.trim(), "error: self-check required for one"); }
                }
                "hook" => {
                    let out = axiom::hook::handle(&event, Some(home), false).unwrap();
                    let text = out["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
                    if allowed { assert!(text.starts_with("Chronos recorded the review."), "{state:?}: {text}"); }
                    else { assert!(text.contains("did not record the review of one: self-check required for one."), "{state:?}: {text}"); }
                }
                "engine" => {
                    let result = axiom::review::roles::engine_review(&f.server, &f.store, "one", &f.repo);
                    if allowed {
                        let verdict = result.unwrap().expect("engine reviewer must run");
                        assert_eq!(verdict["verdict"], "pass");
                    } else { assert_eq!(result.unwrap_err().kind, "selfcheck_required", "{state:?}: {entry}"); }
                }
                _ => unreachable!(),
            }
            assert_eq!(reviews(), before + usize::from(allowed), "{state:?}: {entry}");
        }
    };
    check_entries(orch::SelfCheckState::Missing, false);
    add("selfcheck", 1, &tree);
    check_entries(orch::SelfCheckState::Failed, false);
    add("selfcheck", 0, "tree:stale");
    check_entries(orch::SelfCheckState::Stale, false);
    add("selfcheck", 0, &tree);
    check_entries(orch::SelfCheckState::Passed, true);
    fs::write(f.store.join("one.brief"), "GOAL test\nSCOPE src/\nVERIFY echo ok\n").unwrap();
    check_entries(orch::SelfCheckState::NotRequired, true);
}
#[test]
fn next_for_never_review_without_selfcheck() {
    let f = Fixture::new(
        json!({"status":"pass","changed":false,"fixed":0,"findings":[]}),
        "",
    );
    for kind in ["run", "verify"] {
        orch::append_evidence(
            &f.store,
            "one",
            &orch::Evidence {
                kind: kind.into(),
                command: "worker".into(),
                exit_code: 0,
                commit: "tree:test".into(),
                ts: "1".into(),
            },
        )
        .unwrap();
    }
    let add = |kind: &str, exit_code: i32, commit: &str| orch::append_evidence(&f.store, "one", &orch::Evidence {kind: kind.into(), command: "a1|engine-selfcheck:test".into(), exit_code, commit: commit.into(), ts: "1".into()}).unwrap();
    assert_ne!(orch::summary::next_for(&f.store, "one").unwrap(), orch::summary::Next::Review);
    add("selfcheck", 1, "tree:test");
    assert_eq!(orch::selfcheck_state(&f.store,"one").unwrap(), orch::SelfCheckState::Failed);
    assert_ne!(orch::summary::next_for(&f.store, "one").unwrap(), orch::summary::Next::Review);
    add("verify", 0, "tree:new");
    assert_eq!(orch::selfcheck_state(&f.store,"one").unwrap(), orch::SelfCheckState::Stale);
    assert_ne!(orch::summary::next_for(&f.store, "one").unwrap(), orch::summary::Next::Review);
    add("selfcheck", 0, "tree:new");
    assert_eq!(orch::summary::next_for(&f.store, "one").unwrap(), orch::summary::Next::Review);
}
#[test]
fn built_unit_resumes_only_selfcheck() {
    use axiom::bridge::{call, Bridge, Config};
    let f = Fixture::new(json!({"status":"pass","changed":false,"fixed":0,"findings":[]}), "");
    // This bridge test also passes through the engine's tests-first protocol.
    let matrix = f.repo.join("src/matrix.py");
    fs::write(&matrix, "import pathlib, unittest\nclass Matrix(unittest.TestCase):\n    def test_case(self):\n        self.assertTrue(pathlib.Path('src/implemented').exists())\nunittest.main(verbosity=2)\n").unwrap();
    let command = format!("\"{}\" src/matrix.py", common::python());
    let brief = fs::read_to_string(f.store.join("one.brief")).unwrap()
        .replace("src/test.py::test_case | checks behavior | echo ok", &format!("src/matrix.py::test_case | checks behavior | {command}"));
    fs::write(f.store.join("one.brief"), brief).unwrap();
    let wrapper = f._temp.path().join("wrapper.py");
    fs::write(&wrapper, "import os,runpy,sys\nos.environ['FAKE_CODEX_LOG']=sys.argv[1]\nos.environ['FAKE_CODEX_MODELS']='gpt-6-sol,gpt-6-astra,gpt-6-terra'\nsys.argv=sys.argv[2:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n").unwrap();
    let hook = format!(r#"
import json,pathlib
original_stdin = sys.stdin
class Input:
    def __getattr__(self, key): return getattr(original_stdin, key)
    def __iter__(self): return self
    def __next__(self):
        line = next(original_stdin)
        msg = json.loads(line)
        if msg.get('method') == 'turn/start':
            params = msg['params']
            text = params['input'][0]['text']
            if not params.get('outputSchema') and 'TESTS-FIRST' not in text:
                pathlib.Path({marker}).write_text('implemented')
        return line
sys.stdin = Input()
"#, marker=serde_json::to_string(&f.repo.join("src/implemented").to_string_lossy()).unwrap());
    let original = fs::read_to_string(&wrapper).unwrap();
    fs::write(&wrapper, original.replace("runpy.run_path", &(hook + "\nrunpy.run_path"))).unwrap();
    let bridge = Bridge::new(Config { data_home: Some(f._temp.path().into()), codex: Ok(vec![common::python(), wrapper.to_string_lossy().into_owned(), f.log.to_string_lossy().into_owned(), common::repo_root().join("tests/fakes/fake_codex.py").to_string_lossy().into_owned()]), wsl: None, on_windows: false });
    let args = json!({"project":"demo","unit":"one","role":"terra","cwd":f.repo,"cycle":false});
    let first = call(&bridge, "codex_run", &args)["structuredContent"].clone();
    assert_eq!(first["state"], "built", "{first}");
    let path = f.store.join("reports/one.evidence.tsv");
    let data = fs::read_to_string(&path).unwrap();
    fs::write(&path, data.lines().filter(|l| !l.starts_with("selfcheck\t")).collect::<Vec<_>>().join("\n") + "\n").unwrap();
    assert_eq!(orch::selfcheck_state(&f.store,"one").unwrap(), orch::SelfCheckState::Missing);
    fs::write(&f.log, "").unwrap();
    let resumed = call(&bridge, "codex_run", &args)["structuredContent"].clone();
    assert_eq!(resumed["state"], "built", "{resumed}");
    assert_eq!(orch::selfcheck_state(&f.store,"one").unwrap(), orch::SelfCheckState::Passed);
    let log = fs::read_to_string(&f.log).unwrap();
    let rows: Vec<Value> = log.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let turns: Vec<_> = rows.iter().filter(|r| r["method"] == "turn/start").collect();
    assert_eq!(turns.len(), 1, "{log}");
    assert_eq!(turns[0]["params"]["outputSchema"]["title"], "axiom.self_check.v1", "{log}");
}
