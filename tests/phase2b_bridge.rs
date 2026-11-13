mod common;

use axiom::{
    engine::{
        approvals::{self, Action, Origin},
        kinds,
    },
    orch::{self, events},
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const WISH: &str = "w-1234567890123-abcdef";

struct Fixture {
    temp: tempfile::TempDir,
    store: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = common::temp();
        let store = orch::wish_store(temp.path(), "demo", Some(WISH)).unwrap();
        fs::create_dir_all(&store).unwrap();
        Self { temp, store }
    }
    fn cli(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_axiom"))
            .env("AXIOM_HOME", self.temp.path())
            .env("AXIOM_DATA_HOME", self.temp.path())
            .env_remove("ORCH_STORE")
            .args(args)
            .output()
            .unwrap()
    }
    fn request(&self, marker: &Path) -> approvals::Requested {
        approvals::request(
            &self.store,
            Action {
                command: common::python(),
                args: vec![
                    "-c".into(),
                    "import sys; open(sys.argv[1], 'a').write('run\\n')".into(),
                    marker.to_string_lossy().into_owned(),
                ],
                cwd: self.temp.path().to_string_lossy().into_owned(),
            },
            "run",
            Origin::Model,
        )
        .unwrap()
    }
}
fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn success(out: &Output) {
    assert!(
        out.status.success(),
        "stdout: {} stderr: {}",
        stdout(out),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn cli_policy_set_show_and_invalid_key() {
    let f = Fixture::new();
    let store = f.store.to_str().unwrap();
    let set = f.cli(&[
        "orch", "--store", store, "--json", "policy", "set", "reviewer", "both",
    ]);
    success(&set);
    let show = f.cli(&["orch", "--store", store, "--json", "policy", "show"]);
    success(&show);
    let policy: Value = serde_json::from_str(&stdout(&show)).unwrap();
    assert_eq!(policy["reviewer"], "both");
    let invalid = f.cli(&["orch", "--store", store, "policy", "set", "bad", "on"]);
    assert!(!invalid.status.success());
    assert_eq!(
        axiom::engine::policy::load(&f.store).unwrap().reviewer,
        axiom::engine::policy::ReviewerMode::Both
    );
}

#[test]
fn cli_stats_forwards_and_prints() {
    let f = Fixture::new();
    events::append_event(&f.store, kinds::REVIEW_RECORDED, None, Some("u1"), json!({"review_id":"r1", "unit":"u1", "role":"reviewer", "reviewer":"alice", "model":"m", "paths":[], "pass":true, "findings":[]})).unwrap();
    let out = f.cli(&["stats", "--project", "demo", "--wish", WISH, "--json"]);
    success(&out);
    let result: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(result["rows"][0]["reviewer"], "alice");
}

#[test]
fn cli_approval_approve_executes_once_and_deny_records_reason() {
    let f = Fixture::new();
    let marker = f.temp.path().join("marker");
    let req = f.request(&marker);
    assert!(!f
        .cli(&[
            "approval",
            "approve",
            &req.id,
            "--project",
            "demo",
            "--digest",
            "wrong",
            "--wish",
            WISH
        ])
        .status
        .success());
    assert!(!marker.exists());
    let args = [
        "approval",
        "approve",
        &req.id,
        "--project",
        "demo",
        "--digest",
        &req.digest,
        "--wish",
        WISH,
    ];
    success(&f.cli(&args));
    assert_eq!(
        fs::read_to_string(&marker)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["run"]
    );
    assert!(!f.cli(&args).status.success());
    assert_eq!(
        fs::read_to_string(&marker)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["run"]
    );
    let denied = f.request(&marker);
    success(&f.cli(&[
        "approval",
        "deny",
        &denied.id,
        "--project",
        "demo",
        "--digest",
        &denied.digest,
        "--wish",
        WISH,
        "--reason",
        "not needed",
    ]));
    assert_eq!(
        fs::read_to_string(&marker)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["run"]
    );
    let records = events::read_events(&f.store, 0).unwrap();
    assert!(records.iter().any(|e| e.kind == kinds::APPROVAL_DENIED
        && e.data["id"] == denied.id
        && e.data["reason"] == "not needed"));
}

#[test]
fn cli_approval_without_wish_uses_active_store() {
    let f = Fixture::new();
    let mut goal = axiom::goal::open(
        f.temp.path(),
        "s",
        "test",
        Some(f.temp.path().to_str().unwrap()),
    )
    .unwrap();
    goal.project = Some("demo".into());
    goal.wish_id = Some(WISH.into());
    fs::write(
        f.temp.path().join("goals/s.json"),
        serde_json::to_vec(&goal).unwrap(),
    )
    .unwrap();
    let marker = f.temp.path().join("active-marker");
    let req = f.request(&marker);
    success(&f.cli(&[
        "approval",
        "approve",
        &req.id,
        "--project",
        "demo",
        "--digest",
        &req.digest,
    ]));
    assert_eq!(
        fs::read_to_string(marker)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["run"]
    );
}

#[test]
fn cli_adjudication_overturn_appends_event() {
    let f = Fixture::new();
    let store = orch::store_for(f.temp.path(), "demo").unwrap();
    events::append_event(&store, kinds::ADJUDICATION, None, None, json!({"id":"r-1"})).unwrap();
    success(&f.cli(&[
        "adjudication",
        "overturn",
        "r-1",
        "--project",
        "demo",
        "--reason",
        "new evidence",
    ]));
    let records = events::read_events(&store, 0).unwrap();
    assert!(records
        .iter()
        .any(|e| e.kind == kinds::ADJUDICATION_OVERTURNED
            && e.data["id"] == "r-1"
            && e.data["reason"] == "new evidence"));
}

#[test]
fn cli_orch_review_records_supplied_verdict() {
    let f = Fixture::new();
    let repo = f.temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("file.txt"), "fixture").unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    let brief = f.store.join("u1.brief");
    fs::write(&brief, "GOAL x\nSCOPE file.txt\nVERIFY test\n").unwrap();
    fs::write(
        f.store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu1\tm\tpending\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    let tree = orch::snapshot(&repo, &["file.txt".into()]);
    orch::append_evidence(
        &f.store,
        "u1",
        &orch::Evidence {
            kind: "verify".into(),
            command: "test".into(),
            exit_code: 0,
            commit: tree.clone(),
            ts: "2026-01-01".into(),
        },
    )
    .unwrap();
    fs::create_dir_all(f.store.join("reports")).unwrap();
    fs::write(orch::pack_path(&f.store, "u1"), &tree).unwrap();
    let verdict = f.temp.path().join("verdict.json");
    fs::write(&verdict, json!({"unit":"u1", "tree":tree, "verdict":"pass", "findings":[{"severity":"minor", "location":"file.txt:1", "check":"check"}]}).to_string()).unwrap();
    let out = f.cli(&[
        "orch",
        "--store",
        f.store.to_str().unwrap(),
        "review",
        "u1",
        "--repo",
        repo.to_str().unwrap(),
        "--reviewer",
        "chronos:reviewer",
        "--verdict-file",
        verdict.to_str().unwrap(),
    ]);
    success(&out);
    let records = events::read_events(&f.store, 0).unwrap();
    let review = records
        .iter()
        .find(|e| e.kind == kinds::REVIEW_RECORDED)
        .unwrap();
    assert_eq!(review.data["findings"][0]["location"], "file.txt:1");
}

#[test]
fn tool_approval_request_status_person_only_and_execute_once() {
    use axiom::bridge::server::{call, Bridge, Config};
    let f = Fixture::new();
    let mut cfg = Config::from_env();
    cfg.data_home = Some(f.temp.path().into());
    let bridge = Bridge::new(cfg);
    let invoke = |args: Value| call(&bridge, "approval", &args)["structuredContent"].clone();
    let marker = f.temp.path().join("tool-marker");
    let requested = invoke(
        json!({"project":"demo", "op":"request", "action":{"command":[common::python(),"-c","import sys; open(sys.argv[1], 'a').write('run\\n')",marker],"cwd":f.temp.path()},"reason":"test"}),
    );
    assert_eq!(requested["status"], "requested", "{requested}");
    let id = requested["id"].as_str().unwrap();
    let digest = requested["digest"].as_str().unwrap();
    assert_eq!(
        invoke(json!({"project":"demo","op":"status","id":id}))["status"],
        "requested"
    );
    let status = call(
        &bridge,
        "approval",
        &json!({"project":"demo","op":"status","id":id}),
    );
    assert_eq!(status["isError"], false);
    assert!(status["structuredContent"].get("error").is_none());
    for op in ["approve", "deny"] {
        assert_eq!(
            invoke(json!({"project":"demo","op":op,"id":id}))["error"]["kind"],
            "person_only"
        );
    }
    let store = orch::store_for(f.temp.path(), "demo").unwrap();
    approvals::reply(
        f.temp.path(),
        &store,
        id,
        true,
        digest,
        "demo",
        None,
        Origin::Person,
    )
    .unwrap();
    assert_eq!(
        invoke(json!({"project":"demo","op":"execute","id":id}))["status"],
        "completed"
    );
    assert_eq!(
        invoke(json!({"project":"demo","op":"execute","id":id}))["error"]["kind"],
        "already_executed"
    );
    assert_eq!(fs::read_to_string(marker).unwrap().lines().count(), 1);
}

#[test]
fn tool_interjection_classify_apply_decline_and_exclusive_keys() {
    use axiom::{
        bridge::server::{call, Bridge, Config},
        engine::interjections,
        goal,
    };
    let f = Fixture::new();
    let mut cfg = Config::from_env();
    cfg.data_home = Some(f.temp.path().into());
    let bridge = Bridge::new(cfg);
    let mut g = goal::open(
        f.temp.path(),
        "s",
        "test",
        Some(f.temp.path().to_str().unwrap()),
    )
    .unwrap();
    g.project = Some("demo".into());
    fs::write(
        f.temp.path().join("goals/s.json"),
        serde_json::to_vec(&g).unwrap(),
    )
    .unwrap();
    let invoke = |args: Value| call(&bridge, "goal", &args)["structuredContent"].clone();
    assert_eq!(
        invoke(json!({"session":"s","mode":"change","classify":"replan"}))["error"]["kind"],
        "invalid_argument"
    );
    interjections::submit(f.temp.path(), "s", "revise the plan").unwrap();
    assert_eq!(
        invoke(json!({"session":"s","classify":"replan"}))["status"],
        "completed"
    );
    assert_eq!(
        invoke(
            json!({"session":"s","interjection":{"id":"ij-1","applied":{"plan_revision":"p2"}}})
        )["status"],
        "completed"
    );
    interjections::submit(f.temp.path(), "s", "pause batch").unwrap();
    assert_eq!(
        invoke(json!({"session":"s","classify":"pause_batch"}))["status"],
        "completed"
    );
    assert_eq!(
        invoke(json!({"session":"s","interjection":{"id":"ij-2","declined":"not needed"}}))
            ["status"],
        "completed"
    );
    let store = orch::store_for(f.temp.path(), "demo").unwrap();
    let receipts = interjections::receipts(&store).unwrap();
    assert!(receipts[0]
        .states
        .iter()
        .any(|s| s.0 == "interjection_applied"));
    assert!(receipts[1]
        .states
        .iter()
        .any(|s| s.0 == "interjection_declined"));
}

struct ToolFixture {
    _temp: tempfile::TempDir,
    store: PathBuf,
    bridge: axiom::bridge::server::Bridge,
}
impl ToolFixture {
    fn new(finding: Value, state: &str) -> Self {
        use axiom::{
            bridge::server::{Bridge, Config},
            goal,
        };
        let temp = common::temp();
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("evidence.txt"), "evidence").unwrap();
        let store = orch::store_for(temp.path(), "demo").unwrap();
        fs::create_dir_all(store.join("reports")).unwrap();
        let brief = store.join("u1.brief");
        fs::write(&brief, "GOAL build\nRISK trivial\nSCOPE evidence.txt\nVERIFY test\n").unwrap();
        fs::write(
            store.join("units.tsv"),
            format!(
                "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu1\tm\t{state}\t\t\t\t{}\n",
                brief.display()
            ),
        )
        .unwrap();
        fs::write(
            store.join("reports/u1.verdict.json"),
            json!({"unit":"u1","tree":"tree","verdict":"pass","findings":[finding]}).to_string(),
        )
        .unwrap();
        orch::append_evidence(
            &store,
            "u1",
            &orch::Evidence {
                kind: "review".into(),
                command: "chronos:reviewer".into(),
                exit_code: 0,
                commit: "tree".into(),
                ts: "2025-01-01".into(),
            },
        )
        .unwrap();
        // Unit events also supply evidence when the temporary path is excluded.
        events::append_event(
            &store,
            kinds::REVIEW_RECORDED,
            None,
            Some("u1"),
            json!({"review_id":"r1", "unit":"u1", "role":"reviewer", "reviewer":"alice", "model":"m", "paths":["evidence.txt"], "pass":true, "findings":[finding]}),
        )
        .unwrap();
        orch::append_evidence(
            &store,
            "_full",
            &orch::Evidence {
                kind: "full".into(),
                command: "test".into(),
                exit_code: 0,
                commit: "tree".into(),
                ts: "9999-01-01".into(),
            },
        )
        .unwrap();
        let mut g = goal::open(temp.path(), "s", "test", Some(repo.to_str().unwrap())).unwrap();
        g.project = Some("demo".into());
        g.wish_id = None;
        fs::write(
            temp.path().join("goals/s.json"),
            serde_json::to_vec(&g).unwrap(),
        )
        .unwrap();
        let script = temp.path().join("codex.json");
        fs::write(&script, json!([
            {"match":"AXIOM-ENGINE adjudicator needs_check","reply":{"decision":"pass","reason":"evidence checked","evidence":["E1"]}},
            {"match":"AXIOM-ENGINE verifier","reply":{"findings":[{"finding":1,"status":"rejected","reason":"already satisfied"}]}},
            {"match":"AXIOM-ENGINE adjudicator close_ask","reply":{"decision":"answer","reason":"known","evidence":["E1"],"answer":"use the existing fixture"}},
            {"match":"AXIOM-ENGINE co_core goal_close","reply":{"choice":"done","reason":"agreed"}},
            {"match":"AXIOM-ENGINE co_core module_map","reply":{"choice":"map","reason":"agreed"}}
        ]).to_string()).unwrap();
        let mut config = Config::from_env();
        config.data_home = Some(temp.path().into());
        let mut codex = common::fake("fake_codex");
        codex.push(format!("--fake-script={}", script.display()));
        config.codex = Ok(codex);
        Self {
            _temp: temp,
            store,
            bridge: Bridge::new(config),
        }
    }
    fn invoke(&self, name: &str, args: Value) -> Value {
        axiom::bridge::server::call(&self.bridge, name, &args)["structuredContent"].clone()
    }
}

#[test]
fn tool_done_adjudicates_manual_finding() {
    let f = ToolFixture::new(
        json!({"severity":"major","check":"untested","needs":"manual","evidence":"evidence.txt"}),
        "verified",
    );
    axiom::engine::policy::set(&f.store, "adjudicator", "on").unwrap();
    let result = f.invoke("goal", json!({"session":"s","status":"done"}));
    assert_eq!(result["goal"]["status"], "done", "{result}");
    assert_eq!(result["adjudication"]["decision"], "pass", "{result}");
    assert!(fs::read_to_string(f.store.join("decisions.md"))
        .unwrap()
        .contains("needs_check"));
}

#[test]
fn tool_done_block_ruling_closes_blocked_with_finding() {
    let f = ToolFixture::new(
        json!({"severity":"major","check":"untested","needs":"manual","evidence":"evidence.txt"}),
        "verified",
    );
    fs::write(f._temp.path().join("codex.json"), json!([
        {"match":"AXIOM-ENGINE adjudicator needs_check","reply":{"decision":"block","reason":"evidence insufficient","evidence":["E1"]}}
    ]).to_string()).unwrap();
    axiom::engine::policy::set(&f.store, "adjudicator", "on").unwrap();
    let result = f.invoke("goal", json!({"session":"s","status":"done"}));
    assert_eq!(result["goal"]["status"], "blocked", "{result}");
    assert_eq!(result["adjudication"]["decision"], "block", "{result}");
    assert_eq!(
        result["person_requests"],
        json!(["u1#1: evidence insufficient"])
    );
    let stored = axiom::goal::load(f._temp.path(), "s").unwrap().unwrap();
    assert_eq!(stored.status, "blocked");
    assert!(stored.note.unwrap().contains("u1#1: evidence insufficient"));
}

#[test]
fn tool_done_person_outcome_closes_blocked_with_finding() {
    let f = ToolFixture::new(
        json!({"severity":"major","check":"untested","needs":"manual","evidence":"evidence.txt"}),
        "verified",
    );
    fs::write(f._temp.path().join("codex.json"), json!([
        {"match":"AXIOM-ENGINE adjudicator needs_check","reply":{"decision":"unknown","reason":"cannot decide","evidence":["E1"]}}
    ]).to_string()).unwrap();
    axiom::engine::policy::set(&f.store, "adjudicator", "on").unwrap();
    let result = f.invoke("goal", json!({"session":"s","status":"done"}));
    assert_eq!(result["goal"]["status"], "blocked", "{result}");
    assert_eq!(result["person_requests"], json!(["u1#1: failed"]));
    let stored = axiom::goal::load(f._temp.path(), "s").unwrap().unwrap();
    assert_eq!(stored.status, "blocked");
    assert!(stored.note.unwrap().contains("u1#1: failed"));
    assert!(events::read_events(&f.store, 0)
        .unwrap()
        .iter()
        .any(|e| e.kind == kinds::PERSON_REQUEST
            && e.data["case"] == "needs_check"
            && e.data["reason"] == "failed"));
}

#[test]
fn tool_blocked_reserved_close_ask_closes_as_asked() {
    let f = ToolFixture::new(json!({"severity":"minor","check":"style"}), "verified");
    axiom::engine::policy::set(&f.store, "adjudicator", "on").unwrap();
    let note = "May I publish this?";
    let result = f.invoke(
        "goal",
        json!({"session":"s","status":"blocked","note":note}),
    );
    assert_eq!(result["goal"]["status"], "blocked", "{result}");
    assert_eq!(result["person_requests"], json!(["reserved"]));
    assert!(result.get("adjudication").is_none(), "{result}");
    let stored = axiom::goal::load(f._temp.path(), "s").unwrap().unwrap();
    assert_eq!(stored.status, "blocked");
    assert_eq!(stored.note.as_deref(), Some(note));
}

#[test]
fn tool_waive_runs_scope_reduction_dual_core() {
    let f = ToolFixture::new(json!({"severity":"major","check":"contract"}), "built");
    fs::write(f._temp.path().join("codex.json"), json!([
        {"match":"AXIOM-ENGINE co_core scope_reduction","reply":{"choice":"waive u1 1 --reason accepted risk","reason":"agreed"}}
    ]).to_string()).unwrap();
    axiom::engine::policy::set(&f.store, "dual_core", "on").unwrap();
    let result = f.invoke(
        "orch",
        json!({"project":"demo","args":["waive","u1","1","--reason","accepted risk"]}),
    );
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(result["dual_core"], "Agree", "{result}");
    assert!(orch::waivers(&orch::evidence(&f.store, "u1").unwrap())
        .iter()
        .any(|w| w.number == 1 && w.reason == "accepted risk"));
    assert!(events::read_events(&f.store, 0)
        .unwrap()
        .iter()
        .any(|e| e.kind == kinds::DUAL_CORE
            && e.data["kind"] == "scope_reduction"
            && e.data["agree"] == true));
}

#[test]
fn tool_fix_gate_rejected_finding_preserves_units() {
    let f = ToolFixture::new(
        json!({"severity":"major","check":"contract","evidence":"evidence.txt"}),
        "built",
    );
    let before = fs::read(f.store.join("units.tsv")).unwrap();
    let result = f.invoke("orch", json!({"project":"demo","args":["unit","add","u2","--track","m","--brief",f.store.join("u1.brief"),"--fixes","u1"]}));
    assert_eq!(result["error"]["kind"], "findings_rejected", "{result}");
    assert_eq!(fs::read(f.store.join("units.tsv")).unwrap(), before);
}

#[test]
fn tool_dual_core_off_preserves_reply() {
    let f = Fixture::new();
    let mut g = axiom::goal::open(
        f.temp.path(),
        "s",
        "test",
        Some(f.temp.path().to_str().unwrap()),
    )
    .unwrap();
    g.project = Some("demo".into());
    let goal_file = f.temp.path().join("goals/s.json");
    fs::write(&goal_file, serde_json::to_vec(&g).unwrap()).unwrap();
    let mut cfg = axiom::bridge::server::Config::from_env();
    cfg.data_home = Some(f.temp.path().into());
    let bridge = axiom::bridge::server::Bridge::new(cfg);
    let run = || {
        axiom::bridge::server::call(&bridge, "goal", &json!({"session":"s","status":"done"}))
            ["structuredContent"]
            .clone()
    };
    let before = run();
    let mut restored = axiom::goal::load(f.temp.path(), "s").unwrap().unwrap();
    restored.status = "active".into();
    fs::write(goal_file, serde_json::to_vec(&restored).unwrap()).unwrap();
    let store = orch::store_for(f.temp.path(), "demo").unwrap();
    axiom::engine::policy::set(&store, "dual_core", "off").unwrap();
    assert_eq!(run(), before);
}

#[test]
fn tool_blocked_question_answer_keeps_goal_active() {
    let f = ToolFixture::new(json!({"severity":"minor","check":"style"}), "verified");
    axiom::engine::policy::set(&f.store, "adjudicator", "on").unwrap();
    let result = f.invoke(
        "goal",
        json!({"session":"s","status":"blocked","note":"Which fixture should be used?"}),
    );
    assert_eq!(result["goal"]["status"], "active", "{result}");
    assert_eq!(result["adjudication"]["answer"], "use the existing fixture");
}

#[test]
fn tool_dual_core_goal_and_map_hooks() {
    let f = ToolFixture::new(json!({"severity":"minor","check":"style"}), "verified");
    axiom::engine::policy::set(&f.store, "dual_core", "on").unwrap();
    let map = f.invoke("orch", json!({"project":"demo","args":["map"]}));
    assert_eq!(map["dual_core"], "Agree", "{map}");
    let done = f.invoke("goal", json!({"session":"s","status":"done"}));
    assert_eq!(done["goal"]["status"], "done", "{done}");
    assert_eq!(done["dual_core"], "Agree");
    let kinds: Vec<_> = events::read_events(&f.store, 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == kinds::DUAL_CORE)
        .map(|e| e.data["kind"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(kinds, ["module_map", "goal_close"]);
}

#[test]
fn tool_fix_rounds_failure_is_adjudicated_and_applied() {
    let f = ToolFixture::new(
        json!({"severity":"major","check":"contract","evidence":"evidence.txt"}),
        "built",
    );
    let brief = f.store.join("u1.brief");
    fs::write(f.store.join("units.tsv"), format!("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu0\tm\tsuperseded:u1\t\t\t\t{0}\nu1\tm\tsuperseded:u2\t\t\t\t{0}\nu2\tm\tbuilt\t\t\t\t{0}\n", brief.display())).unwrap();
    for (unit, old) in [("u1", "u0"), ("u2", "u1")] {
        orch::append_evidence(
            &f.store,
            unit,
            &orch::Evidence {
                kind: "fixes".into(),
                command: old.into(),
                exit_code: 0,
                commit: String::new(),
                ts: "2025-01-01".into(),
            },
        )
        .unwrap();
    }
    orch::append_evidence(
        &f.store,
        "u2",
        &orch::Evidence {
            kind: "review".into(),
            command: "chronos:reviewer".into(),
            exit_code: 1,
            commit: "tree".into(),
            ts: "2025-01-01".into(),
        },
    )
    .unwrap();
    fs::write(f.store.join("reports/u2.verdict.json"), json!({"unit":"u2","tree":"tree","verdict":"fail","findings":[{"severity":"major","check":"contract","evidence":"evidence.txt"}]}).to_string()).unwrap();
    fs::write(f._temp.path().join("codex.json"), json!([
        {"match":"AXIOM-ENGINE verifier","reply":{"findings":[{"finding":1,"status":"confirmed","reason":"still present"}]}},
        {"match":"AXIOM-ENGINE adjudicator fix_rounds","reply":{"decision":"waive","reason":"bounded risk","evidence":["E1"]}}
    ]).to_string()).unwrap();
    axiom::engine::policy::set(&f.store, "adjudicator", "on").unwrap();
    let result = f.invoke("orch", json!({"project":"demo","args":["unit","add","u3","--track","m","--brief",brief,"--fixes","u2"]}));
    assert_eq!(result["status"], "failed", "{result}");
    assert_eq!(result["adjudication"]["decision"], "waive", "{result}");
    assert!(orch::waivers(&orch::evidence(&f.store, "u2").unwrap())
        .iter()
        .any(|w| w.number == 1));
}

#[test]
fn tool_fix_gate_unavailable_codex_returns_findings_rejected() {
    let f = ToolFixture::new(
        json!({"severity":"major","check":"contract","evidence":"evidence.txt"}),
        "built",
    );
    let before = fs::read(f.store.join("units.tsv")).unwrap();
    let mut config = axiom::bridge::server::Config::from_env();
    config.data_home = Some(f._temp.path().into());
    config.codex = Err(axiom::base::Failure::new(
        "unavailable",
        "codex unavailable",
    ));
    let bridge = axiom::bridge::server::Bridge::new(config);
    let result = axiom::bridge::server::call(&bridge, "orch", &json!({"project":"demo","args":["unit","add","u2","--track","m","--brief",f.store.join("u1.brief"),"--fixes","u1"]}))["structuredContent"].clone();
    assert_eq!(result["status"], "failed", "{result}");
    assert_eq!(result["error"]["kind"], "findings_rejected", "{result}");
    assert_eq!(fs::read(f.store.join("units.tsv")).unwrap(), before);
}

#[test]
fn tool_fix_gate_does_not_override_active_goal_repository() {
    let f = ToolFixture::new(
        json!({"severity":"major","check":"contract","evidence":"evidence.txt"}),
        "built",
    );
    let before = fs::read(f.store.join("units.tsv")).unwrap();
    let goal_file = f._temp.path().join("goals/s.json");
    let mut goal: Value = serde_json::from_slice(&fs::read(&goal_file).unwrap()).unwrap();
    goal["cwd"] = Value::Null;
    fs::write(goal_file, serde_json::to_vec(&goal).unwrap()).unwrap();
    f.bridge
        .note_project_cwd("demo", f._temp.path().join("repo").to_str().unwrap());
    let result = f.invoke("orch", json!({"project":"demo","args":["unit","add","u2","--track","m","--brief",f.store.join("u1.brief"),"--fixes","u1"]}));
    assert_eq!(result["error"]["kind"], "findings_rejected", "{result}");
    assert!(
        result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("repository is unavailable"),
        "{result}"
    );
    assert_eq!(fs::read(f.store.join("units.tsv")).unwrap(), before);
}

struct WireFixture {
    temp: tempfile::TempDir,
    store: PathBuf,
    repo: PathBuf,
    log: PathBuf,
    bridge: axiom::bridge::server::Bridge,
}
impl WireFixture {
    fn new(script: Value) -> Self {
        use axiom::{
            bridge::server::{Bridge, Config},
            council,
        };
        let temp = common::temp();
        let repo = temp.path().join("repo");
        let store = orch::store_for(temp.path(), "demo").unwrap();
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(store.join("inbox")).unwrap();
        fs::create_dir_all(store.join("reports")).unwrap();
        let script_path = temp.path().join("script.json");
        let log = temp.path().join("fake.jsonl");
        fs::write(&script_path, script.to_string()).unwrap();
        let mut cmd = common::fake("fake_codex");
        cmd.push(format!("--fake-script={}", script_path.display()));
        cmd.push(format!("--fake-log={}", log.display()));
        let mut config = Config::from_env();
        config.data_home = Some(temp.path().into());
        config.codex = Ok(cmd);
        config.wsl = None;
        config.on_windows = false;
        let mut member = common::fake("fake_member");
        let member_script = temp.path().join("member-script.json");
        fs::write(&member_script, "[]").unwrap();
        member.push(format!("--fake-script={}", member_script.display()));
        member.push(format!(
            "--fake-log={}",
            temp.path().join("member.jsonl").display()
        ));
        let bridge =
            Bridge::new(config).with_council(council::Setup::new(Ok(member.clone()), Ok(member)));
        bridge.note_project_cwd("demo", repo.to_str().unwrap());
        Self {
            temp,
            store,
            repo,
            log,
            bridge,
        }
    }
    fn call(&self, tool: &str, args: Value) -> Value {
        axiom::bridge::server::call(&self.bridge, tool, &args)["structuredContent"].clone()
    }
    fn turns(&self) -> Vec<Value> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    }
    fn cycle_setup(&self) {
        fs::create_dir_all(self.repo.join("src")).unwrap();
        fs::write(self.repo.join("src/mod.py"), "fixture\n").unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "src/mod.py"],
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
                .current_dir(&self.repo)
                .status()
                .unwrap()
                .success());
        }
        fs::write(
            self.store.join("modules.md"),
            "## m\nPURPOSE x\nPATHS src/mod.py\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
        )
        .unwrap();
        let brief = self.store.join("u1.brief");
        fs::write(&brief, "GOAL build\nRISK trivial\nSCOPE src/mod.py\nVERIFY echo ok\n").unwrap();
        fs::write(
            self.store.join("units.tsv"),
            format!(
                "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu1\tm\tpending\t\t\t\t{}\n",
                brief.display()
            ),
        )
        .unwrap();
    }
}

#[test]
fn codex_budget_refuses_before_calls_and_records_spend() {
    use axiom::engine::{
        budgets::{self, Role},
        policy,
    };
    let f = WireFixture::new(json!([]));
    policy::set(&f.store, "budget.core", "0").unwrap();
    let args =
        json!({"project":"demo","role":"astra","brief":"test","cwd":f.repo,"sandbox":"read-only"});
    let refused = f.call("codex_run", args.clone());
    assert_eq!(refused["status"], "refused", "{refused}");
    assert_eq!(refused["error"]["kind"], "budget_exceeded");
    assert!(f.turns().is_empty());
    policy::set(&f.store, "budget.core", "none").unwrap();
    let result = f.call("codex_run", args);
    assert_eq!(result["status"], "completed", "{result}");
    assert!(events::read_events(&f.store, 0)
        .unwrap()
        .iter()
        .any(|e| e.kind == kinds::ROLE_SPEND && e.data["role"] == "core"));
    assert!(!budgets::spent(&f.store, Role::Core).unwrap().unknown);
}

#[test]
fn codex_cycle_review_modes_route_and_name_reviewers() {
    use axiom::engine::policy;
    for mode in ["claude", "astra-max", "both"] {
        let f = WireFixture::new(
            json!([{"match":"AXIOM-ENGINE reviewer review","reply":{"verdict":"pass","findings":[]}}]),
        );
        f.cycle_setup();
        policy::set(&f.store, "reviewer", mode).unwrap();
        let result = f.call(
            "codex_run",
            json!({"project":"demo","unit":"u1","role":"terra","cwd":f.repo,"cycle":true}),
        );
        assert_eq!(result["status"], "completed", "{mode}: {result}");
        let review = result["review"].as_array().unwrap();
        if mode == "astra-max" {
            assert!(review.is_empty(), "{result}");
            assert_eq!(result["next"], "done", "{result}");
            assert_eq!(result["state"], "verified", "{result}");
        } else {
            assert_eq!(review[0]["reviewer"], "chronos:reviewer", "{result}");
            assert_eq!(result["next"], "review", "{result}");
            if mode == "both" {
                assert_eq!(result["state"], "built", "{result}");
                assert_eq!(
                    orch::derive_state(&orch::evidence(&f.store, "u1").unwrap(), true),
                    "built"
                );
            }
        }
        let calls = f
            .turns()
            .iter()
            .filter(|v| {
                v["method"] == "turn/start"
                    && v["params"]["input"]
                        .to_string()
                        .contains("AXIOM-ENGINE reviewer review")
            })
            .count();
        assert_eq!(calls, usize::from(mode != "claude"), "{mode}: {result}");
    }
}

#[test]
fn council_budget_refuses_before_calls() {
    let f = WireFixture::new(json!([]));
    axiom::engine::policy::set(&f.store, "budget.council_member", "0").unwrap();
    let result = f.call("council", json!({"project":"demo","plan_path":f.store.join("plan.json"),"schema_path":f.temp.path().join("schema.json"),"cwd":f.repo,"brief":"test"}));
    assert_eq!(result["status"], "refused", "{result}");
    assert_eq!(result["error"]["kind"], "budget_exceeded");
    assert!(f.turns().is_empty());
    assert!(fs::read_to_string(f.temp.path().join("member.jsonl"))
        .unwrap_or_default()
        .is_empty());
}

#[test]
fn council_open_dispute_is_adjudicated_and_dual_core_judges_ruling() {
    let f = WireFixture::new(json!([
        {"match":"AXIOM-ENGINE adjudicator council_dispute","reply":{"decision":"answer","reason":"evidence supports it","evidence":["E1"],"answer":"keep it"}},
        {"match":"AXIOM-ENGINE co_core council_ruling","reply":{"choice":"upheld","reason":"agreed"}}
    ]));
    axiom::engine::policy::set(&f.store, "adjudicator", "on").unwrap();
    axiom::engine::policy::set(&f.store, "dual_core", "on").unwrap();
    fs::write(f.store.join("council.json"), json!({"key":"k","question":"test","members":[],"rounds":[],"claims":[
        {"id":"C1","text":"first claim","evidence":"first evidence","stances":{},"status":"disputed","ruling":null},
        {"id":"C2","text":"second claim","evidence":"second evidence","stances":{},"status":"disputed","ruling":null}],
        "cost":{"usd":0.0,"cap_usd":5.0,"cut":[]},"degraded":false}).to_string()).unwrap();
    let result = f.call("council", json!({"project":"demo","plan_path":f.store.join("plan.json"),"action":"rule","rulings":[{"id":"C1","decision":"upheld","reason":"supported"}]}));
    assert_eq!(result["status"], "completed", "{result}");
    let ev = events::read_events(&f.store, 0).unwrap();
    assert!(
        ev.iter().any(|e| e.kind == kinds::ADJUDICATION
            && e.data["case"] == "council_dispute"
            && e.data["subject"] == "second claim"),
        "{result}"
    );
    assert!(
        ev.iter()
            .any(|e| e.kind == kinds::DUAL_CORE && e.data["kind"] == "council_ruling"),
        "{result}"
    );
    let co_core = f
        .turns()
        .into_iter()
        .find(|v| {
            v["method"] == "turn/start"
                && v["params"]["input"]
                    .to_string()
                    .contains("AXIOM-ENGINE co_core council_ruling")
        })
        .unwrap();
    let prompt = co_core["params"]["input"].to_string();
    assert!(
        prompt.contains("first claim") && prompt.contains("supported"),
        "{prompt}"
    );
    assert!(fs::read_to_string(f.store.join("decisions.md"))
        .unwrap()
        .contains("council_dispute"));
}
