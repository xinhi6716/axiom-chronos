mod common;

use axiom::{
    base,
    codex::AppServer,
    engine::policy,
    orch::{self, Evidence},
    review::roles,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

struct Fixture {
    _temp: tempfile::TempDir,
    store: PathBuf,
    repo: PathBuf,
    log: PathBuf,
    server: AppServer,
}
impl Fixture {
    fn new(worker: &str, reply: Value) -> Self {
        let temp = common::temp();
        let store = temp.path().join("orchestrate/store");
        let repo = temp.path().join("repo");
        fs::create_dir_all(store.join("reports")).unwrap();
        fs::create_dir_all(&repo).unwrap();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        fs::write(repo.join("a.txt"), "one\n").unwrap();
        let brief = store.join("u.brief");
        fs::write(&brief, "GOAL x\nSCOPE a.txt\nVERIFY echo ok\n").unwrap();
        fs::write(
            store.join("units.tsv"),
            format!(
                "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tpending\t\t\t\t{}\n",
                brief.display()
            ),
        )
        .unwrap();
        orch::append_evidence(
            &store,
            "u",
            &Evidence {
                kind: "run".into(),
                command: worker.into(),
                exit_code: 0,
                commit: String::new(),
                ts: base::now_iso(),
            },
        )
        .unwrap();
        let script = temp.path().join("script.json");
        let log = temp.path().join("codex.jsonl");
        fs::write(
            &script,
            json!([{"match":"AXIOM-ENGINE reviewer review","reply":reply}]).to_string(),
        )
        .unwrap();
        let mut cmd = common::fake("fake_codex");
        cmd.push(format!("--fake-script={}", script.display()));
        cmd.push(format!("--fake-log={}", log.display()));
        let server = AppServer::new(cmd).with_slots_home(temp.path().join("slots"));
        Self {
            _temp: temp,
            store,
            repo,
            log,
            server,
        }
    }
    fn verify(&self) {
        orch::record_verify(&self.store, "u", &self.repo).unwrap();
    }
    fn rows(&self) -> Vec<Evidence> {
        orch::evidence(&self.store, "u").unwrap()
    }
    fn tree(&self) -> String {
        self.rows()
            .iter()
            .rev()
            .find(|r| r.kind == "verify")
            .unwrap()
            .commit
            .clone()
    }
    fn pack(&self) {
        orch::review_pack(&self.store, "u", &self.repo).unwrap();
    }
    fn state(&self) -> String {
        orch::derive_state(&self.rows(), false)
    }
    fn verdict(&self, reviewer: &str, findings: Value) {
        fs::write(
            orch::review_verdict_path(&self.store, "u", reviewer),
            json!({"unit":"u","tree":self.tree(),"verdict":"pass","findings":findings}).to_string(),
        )
        .unwrap();
    }
    fn run(&self) -> Option<Value> {
        roles::engine_review(&self.server, &self.store, "u", &self.repo).unwrap()
    }
}
fn lines(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}

#[test]
fn selfcheck_missing_blocks_review() {
    assert_selfcheck_blocks_review(orch::SelfCheckState::Missing);
}

#[test]
fn selfcheck_failed_blocks_review() {
    assert_selfcheck_blocks_review(orch::SelfCheckState::Failed);
}

#[test]
fn selfcheck_stale_blocks_review() {
    assert_selfcheck_blocks_review(orch::SelfCheckState::Stale);
}

fn selfcheck_fixture() -> Fixture {
    let f = Fixture::new("gpt-6-terra", json!({"verdict":"pass","findings":[]}));
    policy::set(&f.store, "reviewer", "astra-max").unwrap();
    fs::write(f.store.join("u.brief"), "GOAL x\nSCOPE a.txt\nVERIFY echo ok\nCONTRACT behavior\nTEST_MATRIX\ns1 | behaviour | a.txt::test_case | checks behavior | echo ok\n").unwrap();
    f.verify();
    f.pack();
    f
}

fn selfcheck_row(f: &Fixture, exit_code: i32, tree: &str) {
    orch::append_evidence(&f.store, "u", &Evidence {
        kind: "selfcheck".into(), command: "a1|self-check".into(),
        exit_code, commit: tree.into(), ts: base::now_iso(),
    }).unwrap();
}

fn assert_selfcheck_blocks_review(state: orch::SelfCheckState) {
    // Also prove self-check refusal takes precedence over budget checks.
    for exhausted_budget in [false, true] {
        let f = selfcheck_fixture();
        match state {
            orch::SelfCheckState::Failed => selfcheck_row(&f, 1, &f.tree()),
            orch::SelfCheckState::Stale => selfcheck_row(&f, 0, "old-tree"),
            _ => {}
        }
        if exhausted_budget { policy::set(&f.store, "budget.reviewer", "0").unwrap(); }
        assert_eq!(orch::selfcheck_state(&f.store, "u").unwrap(), state);
        let before = orch::events::read_events(&f.store, 0).unwrap().len();
        let err = roles::engine_review(&f.server, &f.store, "u", &f.repo).unwrap_err();
        assert_eq!(err.kind, "selfcheck_required");
        assert_eq!(lines(&f.log).iter().filter(|v| v["method"] == "turn/start").count(), 0);
        assert!(!orch::review_verdict_path(&f.store, "u", "engine-astra:x").exists());
        assert!(!f.rows().iter().any(|r| r.kind == "review"));
        assert_eq!(orch::events::read_events(&f.store, 0).unwrap().len(), before);
    }
}

#[test]
fn selfcheck_passed_allows_review() {
    let f = selfcheck_fixture();
    selfcheck_row(&f, 0, &f.tree());
    assert_eq!(orch::selfcheck_state(&f.store, "u").unwrap(), orch::SelfCheckState::Passed);
    assert!(f.run().is_some());
    assert_eq!(lines(&f.log).iter().filter(|v| v["method"] == "turn/start").count(), 1);
    assert!(orch::review_verdict_path(&f.store, "u", "engine-astra:x").exists());
    assert!(f.rows().iter().any(|r| r.kind == "review"));
}

#[test]
fn review_uses_policy_stage() {
    for stage in [None, Some("sol/xhigh"), Some("sol/high")] {
        let f = Fixture::new("gpt-6-terra", json!({"verdict":"pass","findings":[]}));
        policy::set(&f.store, "reviewer", "astra-max").unwrap();
        if let Some(stage) = stage { policy::set(&f.store, "models.reviewer", stage).unwrap(); }
        f.verify();
        f.pack();
        f.run();
        let log = lines(&f.log);
        let turn = log.iter().find(|v| v["method"] == "turn/start").unwrap();
        let tier = if stage.is_some() { "sol" } else { "astra" };
        assert_eq!(turn["params"]["effort"], stage.map_or("max", |s| s.split_once('/').unwrap().1));
        let start = log.iter().find(|v| v["method"] == "thread/start").unwrap();
        assert_eq!(start["params"]["model"], f.server.resolve_model(tier).unwrap());
        assert!(f.rows().iter().any(|r| r.kind == "review" && r.command == format!("engine-astra:{}", f.server.resolve_model(tier).unwrap())));
    }
}

#[test]
fn review_thread_is_fresh_and_distinct() {
    let f = Fixture::new("gpt-6-terra", json!({"verdict":"pass","findings":[]}));
    policy::set(&f.store, "reviewer", "astra-max").unwrap();
    let req = axiom::codex::RunRequest {
        brief: "prior worker".into(), cwd: f.repo.to_string_lossy().into_owned(),
        model: f.server.resolve_model("sol").unwrap(), sandbox: "read-only".into(),
        effort: Some("high".into()), output_schema: None, developer_instructions: None,
        timeout_s: 30, lean: true, keep_plugins: vec![], web_search: false,
    };
    let worker = f.server.run(&req).unwrap();
    let self_check = f.server.run(&axiom::codex::RunRequest { brief: "prior self-check".into(), ..req }).unwrap();
    f.verify(); f.pack();
    for n in 1..=2 {
        f.run();
        let log = lines(&f.log);
        let starts: Vec<_> = log.iter().filter(|v| v["method"] == "thread/start").collect();
        let turns: Vec<_> = log.iter().filter(|v| v["method"] == "turn/start").collect();
        assert_eq!(starts.len(), 2 + n);
        assert_eq!(starts[1+n]["params"]["sandbox"], "read-only");
        let id = &turns[1+n]["params"]["threadId"];
        assert_ne!(id, &worker["thread_id"]);
        assert_ne!(id, &self_check["thread_id"]);
        assert_ne!(id, &turns[n]["params"]["threadId"]);
        let verdict: Value = serde_json::from_str(&fs::read_to_string(orch::review_verdict_path(&f.store, "u", "engine-astra:x")).unwrap()).unwrap();
        assert_eq!(&verdict["thread_id"], id);
    }
}

#[test]
fn stale_pack_gives_no_review_pack() {
    for stale_tree in [false, true] {
        let f = Fixture::new("gpt-6-terra", json!({"verdict":"pass","findings":[]}));
        policy::set(&f.store, "reviewer", "astra-max").unwrap();
        f.verify(); f.pack();
        if stale_tree { fs::write(f.repo.join("a.txt"), "changed").unwrap(); }
        else { fs::write(orch::pack_path(&f.store, "u"), "stale pack").unwrap(); }
        let err = roles::engine_review(&f.server, &f.store, "u", &f.repo).unwrap_err();
        assert_eq!(err.kind, "no_review_pack");
        assert!(!orch::review_verdict_path(&f.store, "u", "engine-astra:x").exists());
        assert!(lines(&f.log).iter().all(|v| v["method"] != "turn/start"));
    }
}

#[test]
fn claude_and_both_modes_unchanged() {
    for (mode, expected) in [("claude", 0), ("both", 1)] {
        let f = Fixture::new("gpt-6-terra", json!({"verdict":"pass","findings":[]}));
        policy::set(&f.store, "reviewer", mode).unwrap();
        f.verify(); f.pack();
        let names: Vec<_> = roles::review_items(&f.store, "u").unwrap().into_iter().map(|i| i.reviewer).collect();
        assert_eq!(names, vec!["chronos:reviewer"]);
        f.run();
        let turns = lines(&f.log).iter().filter(|v| v["method"] == "turn/start").count();
        assert_eq!(turns, expected);
    }
}

#[test]
fn matrix_and_engine_prompt() {
    for (mode, names, astra) in [
        ("claude", vec!["chronos:reviewer"], false),
        ("astra-max", vec![], true),
        ("both", vec!["chronos:reviewer"], true),
    ] {
        let f = Fixture::new("gpt-6-terra", json!({"verdict":"pass","findings":[]}));
        policy::set(&f.store, "reviewer", mode).unwrap();
        f.verify();
        f.pack();
        let items = roles::review_items(&f.store, "u").unwrap();
        assert_eq!(
            items
                .iter()
                .map(|i| i.reviewer.as_str())
                .collect::<Vec<_>>(),
            names
        );
        assert_eq!(f.run().is_some(), astra);
        let log = lines(&f.log);
        let turns: Vec<_> = log.iter().filter(|v| v["method"] == "turn/start").collect();
        assert_eq!(turns.len(), usize::from(astra));
        if astra {
            let start = log.iter().find(|v| v["method"] == "thread/start").unwrap();
            assert_eq!(start["params"]["sandbox"], "read-only");
            assert_eq!(turns[0]["params"]["effort"], "max");
            let prompt = turns[0]["params"]["input"][0]["text"].as_str().unwrap();
            assert_eq!(
                prompt,
                format!(
                    "AXIOM-ENGINE reviewer review\n{}\n\n{}",
                    axiom::engine::asset("roles/astra-reviewer.md").unwrap(),
                    fs::read_to_string(orch::pack_path(&f.store, "u")).unwrap()
                )
            );
            assert_eq!(
                log.iter().filter(|v| v["method"] == "thread/start").count(),
                1
            );
            assert!(!prompt.contains("AXIOM-ENGINE builder"));
            assert!(!prompt.contains("AXIOM-ENGINE dispatcher"));
        }
    }
}

#[test]
fn requirements_and_events() {
    let f = Fixture::new("gpt-6-terra", json!({"verdict":"pass","findings":[]}));
    policy::set(&f.store, "reviewer", "both").unwrap();
    policy::set(&f.store, "pm_qa", "on").unwrap();
    f.verify();
    f.pack();
    let rows = f.rows();
    assert_eq!(
        rows.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(),
        vec!["run", "verify", "require", "require", "require"]
    );
    assert_eq!(
        rows.iter()
            .filter(|r| r.kind == "require")
            .map(|r| r.command.as_str())
            .collect::<Vec<_>>(),
        vec!["chronos:reviewer", "engine-astra:", "chronos:pm-qa"]
    );
    assert!(rows
        .iter()
        .filter(|r| r.kind == "require")
        .all(|r| r.commit == f.tree()));
    assert_eq!(roles::review_items(&f.store, "u").unwrap().len(), 2);
    f.verdict("chronos:reviewer", json!([]));
    orch::record_review(&f.store, &["u".into()], "chronos:reviewer", true, &f.repo).unwrap();
    assert_eq!(f.state(), "built");
    f.run();
    assert_eq!(f.state(), "built");
    f.verdict("chronos:pm-qa", json!([]));
    orch::record_review(&f.store, &["u".into()], "chronos:pm-qa", true, &f.repo).unwrap();
    assert_eq!(f.state(), "verified");
    let events: Vec<_> = orch::events::read_events(&f.store, 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == "review_recorded")
        .collect();
    assert_eq!(events.len(), 3);
    assert_eq!(events[1].data["review_id"], "u#2");
    assert_eq!(events[1].data["role"], "astra-max");
    assert_eq!(events[1].data["reviewer"], "engine-astra:gpt-6-astra");
    assert_eq!(events[1].data["family"], "openai");
    assert_eq!(events[1].data["model"], "gpt-6-astra");
    assert_eq!(events[1].data["tree"], f.tree());
    assert_eq!(events[1].data["unit"], "u");
    assert_eq!(events[1].data["pass"], true);
    assert!(events[1].data["paths"].is_array());
    assert!(events[1].data["findings"].is_array());
}

#[test]
fn same_family_default_and_failure_gate() {
    let f = Fixture::new("claude-sonnet", json!({"verdict":"pass","findings":[]}));
    f.verify();
    f.pack();
    assert!(!f.rows().iter().any(|r| r.kind == "require"));
    assert_eq!(
        orch::record_review(&f.store, &["u".into()], "chronos:reviewer", true, &f.repo)
            .unwrap_err()
            .kind,
        "same_family"
    );
    let f = Fixture::new("gpt-6-astra", json!({"verdict":"pass","findings":[]}));
    policy::set(&f.store, "reviewer", "astra-max").unwrap();
    f.verify();
    f.pack();
    f.run();
    assert_eq!(f.state(), "verified");
    let f = Fixture::new("gpt-6-terra", json!({"verdict":"pass","findings":[]}));
    policy::set(&f.store, "reviewer", "both").unwrap();
    f.verify();
    f.pack();
    f.verdict(
        "chronos:reviewer",
        json!([{"severity":"major","location":"a.txt:1"}]),
    );
    orch::record_review(&f.store, &["u".into()], "chronos:reviewer", false, &f.repo).unwrap();
    f.run();
    assert_eq!(f.state(), "review-failed");
}

#[test]
fn astra_finding_is_checked_once_and_both_verdicts_can_block() {
    let finding = json!({"severity":"major","location":"a.txt:1","check":"contract","evidence":"bad","fix":"repair"});
    let f = Fixture::new(
        "gpt-6-terra",
        json!({"verdict":"fail","findings":[finding]}),
    );
    let member_log = f._temp.path().join("member.jsonl");
    let member_script = f._temp.path().join("member.json");
    fs::write(&member_script, json!([{"match":"AXIOM-ENGINE verifier finding","reply":{"findings":[{"finding":1,"status":"confirmed","reason":"confirmed"}]}}]).to_string()).unwrap();
    let mut member = common::fake("fake_member");
    member.push(format!("--fake-script={}", member_script.display()));
    member.push(format!("--fake-log={}", member_log.display()));
    let old = std::env::var_os("AXIOM_CLAUDE_CMD");
    std::env::set_var("AXIOM_CLAUDE_CMD", serde_json::to_string(&member).unwrap());
    policy::set(&f.store, "reviewer", "astra-max").unwrap();
    f.verify();
    f.pack();
    f.run();
    let other = Fixture::new(
        "gpt-6-terra",
        json!({"verdict":"fail","findings":[{"severity":"major","location":"a.txt:1","check":"contract","evidence":"bad","fix":"repair"}]}),
    );
    policy::set(&other.store, "reviewer", "both").unwrap();
    other.verify();
    other.pack();
    other.verdict("chronos:reviewer", json!([]));
    orch::record_review(
        &other.store,
        &["u".into()],
        "chronos:reviewer",
        true,
        &other.repo,
    )
    .unwrap();
    other.run();
    assert_eq!(other.state(), "review-failed");
    match old {
        Some(v) => std::env::set_var("AXIOM_CLAUDE_CMD", v),
        None => std::env::remove_var("AXIOM_CLAUDE_CMD"),
    }
    let calls = lines(&member_log);
    assert_eq!(calls.len(), 2);
    assert!(calls[0]["prompt"]
        .as_str()
        .unwrap()
        .starts_with("AXIOM-ENGINE verifier finding\n"));
    assert!(calls[0]["argv"].to_string().contains("sonnet"));
    let checks = axiom::engine::finding_verification::checks(&f.store, "u").unwrap();
    assert_eq!(
        checks[0].status,
        axiom::engine::finding_verification::Status::Confirmed
    );
    assert_eq!(f.state(), "review-failed");
    let e = orch::events::read_events(&f.store, 0)
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "review_recorded" && e.data["role"] == "astra-max")
        .unwrap();
    assert_eq!(
        e.data["findings"],
        json!([{"number":1,"severity":"major","location":"a.txt:1"}])
    );
    assert_eq!(e.data["paths"], json!(["a.txt"]));
}

#[test]
fn requirements_are_bound_to_current_tree() {
    let f = Fixture::new("gpt-6-terra", json!({"verdict":"pass","findings":[]}));
    policy::set(&f.store, "reviewer", "both").unwrap();
    f.verify();
    f.pack();
    f.verdict("chronos:reviewer", json!([]));
    orch::record_review(&f.store, &["u".into()], "chronos:reviewer", true, &f.repo).unwrap();
    f.run();
    assert_eq!(f.state(), "verified");
    let old_tree = f.tree();
    fs::write(f.repo.join("a.txt"), "changed\n").unwrap();
    f.verify();
    assert_ne!(old_tree, f.tree());
    assert_eq!(f.state(), "built");
}

#[test]
fn waiver_for_one_review_does_not_close_another() {
    let row = |kind: &str, command: &str, exit_code| Evidence {
        kind: kind.into(),
        command: command.into(),
        exit_code,
        commit: "tree:a".into(),
        ts: base::now_iso(),
    };
    let rows = vec![
        row("run", "gpt-6-terra", 0),
        row("verify", "tests", 0),
        row("require", "chronos:reviewer", 0),
        row("require", "engine-astra:", 0),
        row("review", "chronos:reviewer", 1),
        row("review", "engine-astra:gpt-6-astra", 1),
        row("waive", "1 waived", 0),
    ];
    assert_eq!(orch::derive_state(&rows, false), "review-failed");
}

#[test]
fn failed_review_is_not_hidden_by_an_earlier_pass() {
    let f = Fixture::new("gpt-6-terra", json!({"verdict":"pass","findings":[]}));
    policy::set(&f.store, "reviewer", "both").unwrap();
    f.verify();
    f.pack();
    f.verdict("chronos:reviewer", json!([]));
    orch::record_review(&f.store, &["u".into()], "chronos:reviewer", true, &f.repo).unwrap();
    orch::record_review(&f.store, &["u".into()], "chronos:reviewer", false, &f.repo).unwrap();
    assert_eq!(f.state(), "review-failed");
    assert_eq!(
        roles::review_items(&f.store, "u")
            .unwrap()
            .iter()
            .map(|i| i.reviewer.as_str())
            .collect::<Vec<_>>(),
        vec!["chronos:reviewer"]
    );
}

#[test]
fn new_worker_run_requires_new_review_even_on_the_same_tree() {
    let f = Fixture::new("gpt-6-terra", json!({"verdict":"pass","findings":[]}));
    f.verify();
    f.pack();
    f.verdict("chronos:reviewer", json!([]));
    orch::record_review(&f.store, &["u".into()], "chronos:reviewer", true, &f.repo).unwrap();
    orch::append_evidence(
        &f.store,
        "u",
        &Evidence {
            kind: "run".into(),
            command: "gpt-6-terra".into(),
            exit_code: 0,
            commit: String::new(),
            ts: base::now_iso(),
        },
    )
    .unwrap();
    f.verify();
    assert_eq!(f.state(), "built");
    assert_eq!(
        roles::review_items(&f.store, "u")
            .unwrap()
            .iter()
            .map(|i| i.reviewer.as_str())
            .collect::<Vec<_>>(),
        vec!["chronos:reviewer"]
    );
}

#[test]
fn verify_never_emits_verified_before_requirements_are_recorded() {
    let f = Fixture::new("gpt-6-terra", json!({"verdict":"pass","findings":[]}));
    f.verify();
    orch::append_evidence(
        &f.store,
        "u",
        &Evidence {
            kind: "review".into(),
            command: "chronos:reviewer".into(),
            exit_code: 0,
            commit: f.tree(),
            ts: base::now_iso(),
        },
    )
    .unwrap();
    policy::set(&f.store, "reviewer", "both").unwrap();
    f.verify();
    assert_eq!(f.state(), "built");
    assert!(!orch::events::read_events(&f.store, 0)
        .unwrap()
        .iter()
        .any(|e| e.kind == "unit_state" && e.data["state"] == "verified"));
}

#[test]
fn failed_latest_verify_never_starts_astra() {
    let f = Fixture::new("gpt-6-terra", json!({"verdict":"pass","findings":[]}));
    policy::set(&f.store, "reviewer", "astra-max").unwrap();
    f.verify();
    f.pack();
    orch::append_evidence(
        &f.store,
        "u",
        &Evidence {
            kind: "verify".into(),
            command: "tests".into(),
            exit_code: 1,
            commit: f.tree(),
            ts: base::now_iso(),
        },
    )
    .unwrap();
    assert_eq!(
        roles::engine_review(&f.server, &f.store, "u", &f.repo)
            .unwrap_err()
            .kind,
        "not_built"
    );
    assert!(lines(&f.log).iter().all(|v| v["method"] != "turn/start"));
}

#[test]
fn pm_qa_hook_uses_its_own_verdict_and_binding() {
    let f = Fixture::new("gpt-6-terra", json!({"verdict":"pass","findings":[]}));
    policy::set(&f.store, "pm_qa", "on").unwrap();
    f.verify();
    f.pack();
    let home = f._temp.path().join("home");
    let goal = axiom::goal::open(&home, "s", "wish", Some(f.repo.to_str().unwrap())).unwrap();
    let target = orch::store_for(&home, goal.project.as_deref().unwrap()).unwrap();
    fs::create_dir_all(&target).unwrap();
    fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let e = entry.unwrap();
            if e.path().is_dir() {
                copy_dir(&e.path(), &to.join(e.file_name()));
            } else {
                fs::copy(e.path(), to.join(e.file_name())).unwrap();
            }
        }
    }
    copy_dir(&f.store, &target);
    let start = json!({"hook_event_name":"PreToolUse","session_id":"s","cwd":f.repo,"agent_id":"dispatcher","agent_type":"chronos:dispatcher","tool_name":"Agent","tool_input":{"subagent_type":"chronos:pm-qa"}});
    let allowed = axiom::hook::handle(&start, Some(&home), false);
    assert!(allowed
        .as_ref()
        .is_none_or(|v| v["hookSpecificOutput"]["permissionDecision"] != "deny"));
    let read = json!({"hook_event_name":"PreToolUse","session_id":"s","cwd":f.repo,"agent_id":"r1","agent_type":"chronos:pm-qa","tool_name":"Read","tool_input":{"file_path":orch::pack_path(&target,"u")}});
    let allowed = axiom::hook::handle(&read, Some(&home), false);
    assert!(allowed
        .as_ref()
        .is_none_or(|v| v["hookSpecificOutput"]["permissionDecision"] != "deny"));
    assert_eq!(
        fs::read_to_string(target.join("agents/r1.reviewer")).unwrap(),
        "u"
    );
    let write = json!({"hook_event_name":"PreToolUse","session_id":"s","cwd":f.repo,"agent_id":"r1","agent_type":"chronos:pm-qa","tool_name":"Write","tool_input":{"file_path":orch::review_verdict_path(&target,"u","chronos:pm-qa")}});
    let allowed = axiom::hook::handle(&write, Some(&home), false);
    assert!(allowed
        .as_ref()
        .is_none_or(|v| v["hookSpecificOutput"]["permissionDecision"] != "deny"));
    fs::write(
        orch::review_verdict_path(&target, "u", "chronos:pm-qa"),
        json!({"unit":"u","tree":f.tree(),"verdict":"pass","findings":[]}).to_string(),
    )
    .unwrap();
    let event = json!({"hook_event_name":"PostToolUse","session_id":"s","tool_name":"Agent","tool_input":{"subagent_type":"chronos:pm-qa","prompt":"review"},"tool_response":{"agentId":"r1"}});
    let response = axiom::hook::handle(&event, Some(&home), false)
        .unwrap()
        .to_string();
    assert!(response.contains("recorded the review"), "{response}");
    assert!(orch::evidence(&target, "u")
        .unwrap()
        .iter()
        .any(|r| r.kind == "review" && r.command == "chronos:pm-qa"));
}
