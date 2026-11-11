mod common;

use axiom::{
    base,
    bridge::{call, tools, Bridge, Config},
    council::{self, CrossReply, CrossRequest, Setup, Status, DIGEST_MAX_CHARS},
    orch,
    usage::price,
};
use regex::Regex;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
    time::Duration,
};

const BRIEF: &str = "GOAL the changes that carry out the wish\nWISH make it faster";

#[test]
fn phased_protocol_calls_argon_once_after_phase_a() {
    let case = Case::new();
    let reply = case.run(&case.setup("ok", "ok"), &Astra::ok()).unwrap();
    let calls = case.calls();
    assert_eq!(calls.iter().filter(|c| c["role"] == "argon").count(), 1);
    assert_eq!(reply["phases"][0]["phase"], "A");
    assert_eq!(reply["phases"][0]["agreement"], true);
    assert_eq!(reply["phases"].as_array().unwrap().len(), 2);
}

#[test]
fn invalid_budget_refuses_start_before_any_child() {
    for name in [
        "AXIOM_FABLE_CAP_USD",
        "AXIOM_ASTRA_TOKEN_CAP",
        "AXIOM_ARGON_TOKEN_CAP",
        "AXIOM_COUNCIL_ROUNDS",
    ] {
        for value in ["text", "0", "-1", "NaN", "inf", "", "18446744073709551616"] {
            let temp = common::temp();
            let record = temp.path().join("record.json");
            let output = Command::new(env!("CARGO_BIN_EXE_axiom"))
                .env("AXIOM_HOME", temp.path())
                .env("AXIOM_DATA_HOME", temp.path())
                .env(
                    "AXIOM_CLAUDE_CMD",
                    serde_json::to_string(&common::fake("fake_claude")).unwrap(),
                )
                .env("FAKE_CLAUDE_RECORD", &record)
                .env(name, value)
                .args(["wish", "--advanced-research", "fix"])
                .stdin(Stdio::null())
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(2), "{name}={value}");
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(
                error.contains(name) && error.contains(&format!("{value:?}")),
                "{error}"
            );
            assert!(!record.exists());
        }
    }
}

fn absent() -> axiom::base::Res<Vec<String>> {
    Err(base::Failure::new("not_installed", "not on PATH"))
}

struct Case {
    temp: tempfile::TempDir,
    plan: PathBuf,
    schema: PathBuf,
    log: PathBuf,
}
impl Case {
    fn new() -> Self {
        let temp = common::temp();
        let dir = temp.path().join("store/astra");
        fs::create_dir_all(&dir).unwrap();
        let plan = dir.join("plan.json");
        fs::write(
            &plan,
            serde_json::to_vec(&json!({
                "summary":"astra", "coverage":{"read":[],"not_read":[]}, "out_of_scope":[],
                "changes":[
                    {"id":"a1","area":"design","location":"src/a.py","evidence":"astra evidence one","problem":"slow","change":"cache it","verify":"time it"},
                    {"id":"a2","area":"tests","location":"src/b.py","evidence":"astra evidence two","problem":"flaky","change":"pin it","verify":"run it"}
                ]
            }))
            .unwrap(),
        )
        .unwrap();
        let schema = common::repo_root().join("engine/research/audit.schema.json");
        let log = temp.path().join("calls.jsonl");
        Self {
            temp,
            plan,
            schema,
            log,
        }
    }
    fn member(&self, scenario: &str) -> axiom::base::Res<Vec<String>> {
        let mut argv = common::fake("fake_member");
        argv.push(format!("--fake-scenario={scenario}"));
        argv.push(format!("--fake-log={}", self.log.display()));
        Ok(argv)
    }
    fn setup(&self, fable: &str, argon: &str) -> Setup {
        Setup::new(self.member(fable), self.member(argon))
    }
    fn request(&self) -> council::Request {
        council::Request {
            brief: BRIEF.into(),
            cwd: self.temp.path().to_path_buf(),
            plan_path: self.plan.clone(),
            schema_path: self.schema.clone(),
        }
    }
    fn calls(&self) -> Vec<Value> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
    fn calls_of(&self, role: &str, round: u64) -> Vec<Value> {
        self.calls()
            .into_iter()
            .filter(|c| c["role"] == role && c["round"] == round)
            .collect()
    }
    fn stored(&self) -> Value {
        let text = fs::read_to_string(self.plan.with_file_name("council.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }
    fn member_status(&self, name: &str) -> String {
        let stored = self.stored();
        let member = stored["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["name"] == name)
            .unwrap();
        member["status"].as_str().unwrap().to_owned()
    }
    fn run(&self, setup: &Setup, astra: &Astra) -> axiom::base::Res<Value> {
        let cross = |r: &CrossRequest| astra.answer(r);
        council::run(setup, &self.request(), &cross, &AtomicBool::new(false))
    }
}

struct Astra {
    status: Status,
    calls: AtomicUsize,
    prompt: Mutex<String>,
}
impl Astra {
    fn with(status: Status) -> Self {
        Self {
            status,
            calls: AtomicUsize::new(0),
            prompt: Mutex::new(String::new()),
        }
    }
    fn ok() -> Self {
        Self::with(Status::Ok)
    }
    fn answer(&self, request: &CrossRequest) -> CrossReply {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.prompt.lock().unwrap() = request.prompt.clone();
        let ids = Regex::new(r"(?m)^(C\d+) \[Position").unwrap();
        let stances: Vec<Value> = ids
            .captures_iter(&request.prompt)
            .map(|c| json!({"claim":c[1].to_string(),"stance":"agree","evidence":""}))
            .collect();
        CrossReply {
            status: self.status,
            output: (self.status == Status::Ok).then(|| json!({ "stances": stances })),
            tokens: 321,
            model: Some("gpt-6-astra".into()),
            detail: (self.status != Status::Ok).then(|| "not this time".to_string()),
        }
    }
}

fn valid_council(case: &Case) {
    let schema: Value = serde_json::from_str(
        &fs::read_to_string(common::repo_root().join("engine/research/council.schema.json"))
            .unwrap(),
    )
    .unwrap();
    base::schema::validate(&schema, &case.stored()).unwrap();
}

fn claims(case: &Case) -> Vec<Value> {
    case.stored()["claims"].as_array().unwrap().clone()
}

fn flag_value(argv: &[Value], flag: &str) -> String {
    let at = argv.iter().position(|a| a == flag).unwrap();
    argv[at + 1].as_str().unwrap().to_owned()
}

#[test]
fn fable_5_2_is_priced_from_the_published_table() {
    let usage = price::CallUsage {
        input: 1_000_000,
        cache_write: 1_000_000,
        cache_write_1h: 0,
        cache_read: 1_000_000,
        output: 1_000_000,
        ..Default::default()
    };
    let usd = price::usd("claude-fable-5-2", &usage).unwrap();
    assert!((usd - (8.0 + 10.0 + 0.20 + 40.0)).abs() < 1e-6, "{usd}");
}

#[test]
fn phased_order_agreement_and_single_review() {
    let case = Case::new();
    let astra = |r: &CrossRequest| {
        assert_eq!(case.calls_of("fable", 1).len(), 1);
        assert!(case.calls_of("fable", 2).is_empty());
        assert!(case.calls_of("argon", 1).is_empty());
        Astra::ok().answer(r)
    };
    let reply = council::run(
        &case.setup("ok", "ok"),
        &case.request(),
        &astra,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply["rounds"][0]["speakers"], json!(["astra", "fable"]));
    assert_eq!(reply["rounds"][1]["speakers"], json!(["astra", "fable"]));
    assert_eq!(reply["rounds"][2]["speakers"], json!(["argon"]));
    assert_eq!(
        case.calls()
            .iter()
            .map(|c| c["role"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>(),
        vec!["fable", "fable", "argon"]
    );
    assert_eq!(reply["phases"][0]["agreement"], true);
    assert_eq!(reply["phases"].as_array().unwrap().len(), 2);
    valid_council(&case);
    let stored = case.stored();
    for round in stored["rounds"].as_array().unwrap() {
        for p in round["positions"].as_array().unwrap() {
            let bytes = fs::read(p["path"].as_str().unwrap()).unwrap();
            assert_eq!(
                p["sha256"],
                format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&bytes))
            );
            assert!(p["path"].as_str().unwrap().contains(&format!(
                "{}-r{}-{}",
                round["phase"].as_str().unwrap(),
                round["n"],
                p["member"].as_str().unwrap()
            )));
        }
    }
}
#[test]
fn phase_a_stops_on_agreement_or_the_configured_cap() {
    for (scenario, cap, want, agreement) in [
        ("no-claims", 3, 1, true),
        ("disagree", 3, 3, false),
        ("disagree", 1, 1, false),
        ("disagree,resolve-a", 5, 2, true),
    ] {
        let case = Case::new();
        let mut setup = case.setup(scenario, "ok");
        setup.rounds = cap;
        let result = case.run(&setup, &Astra::ok()).unwrap();
        assert_eq!(
            case.calls().iter().filter(|c| c["role"] == "fable").count(),
            want
        );
        assert_eq!(result["phases"][0]["agreement"], agreement);
        if !agreement {
            assert!(!result["phases"][0]["open"].as_array().unwrap().is_empty());
        }
    }
    let case = Case::new();
    let result = case
        .run(
            &case.setup("disagree,approve-with-rebuttal", "ok"),
            &Astra::ok(),
        )
        .unwrap();
    assert_eq!(
        result["phases"][0]["agreement"], false,
        "approval alone must not close rebuttals"
    );
}
#[test]
fn phase_c_requires_objections_and_orders_replies_without_recalling_argon() {
    let case = Case::new();
    let astra = |r: &CrossRequest| {
        let mut reply = Astra::ok().answer(r);
        if r.prompt.contains("COUNCIL PHASE C") {
            assert_eq!(
                case.calls().iter().filter(|c| c["role"] == "argon").count(),
                1
            );
            assert_eq!(
                case.calls()
                    .iter()
                    .filter(|c| c["role"] == "fable" && c["phase"] == "C")
                    .count(),
                0
            );
            let ids = Regex::new(r"(?m)^(C\d+) \[Position").unwrap();
            reply.output.as_mut().unwrap()["resolutions"]=json!(ids.captures_iter(&r.prompt).map(|c|json!({"claim":c[1].to_string(),"decision":"rejected","reason":"src/mod.py:12 already covers this","change":""})).collect::<Vec<_>>());
        }
        reply
    };
    let result = council::run(
        &case.setup("ok,settle", "disagree"),
        &case.request(),
        &astra,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result["phases"][2]["agreement"], true);
    assert_eq!(result["rounds"][3]["speakers"], json!(["astra", "fable"]));
    assert_eq!(
        case.calls().iter().filter(|c| c["role"] == "argon").count(),
        1
    );
    for claim in claims(&case) {
        assert_eq!(claim["stances"]["argon"], "disagree");
        assert!(claim["history"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h["phase"] == "C" && h["decision"] == "rejected"));
        assert_ne!(claim["status"], "consensus", "review remains for the core");
    }
}
#[test]
fn unresolved_review_objections_use_the_phase_c_cap() {
    let case = Case::new();
    let astra = |r: &CrossRequest| {
        if r.prompt.contains("COUNCIL PHASE C") {
            let n = Regex::new(r"COUNCIL ROUND (\d+)")
                .unwrap()
                .captures(&r.prompt)
                .unwrap()[1]
                .parse::<usize>()
                .unwrap();
            assert_eq!(
                case.calls()
                    .iter()
                    .filter(|c| c["role"] == "fable" && c["phase"] == "C")
                    .count(),
                n - 1
            );
            assert_eq!(
                case.calls().iter().filter(|c| c["role"] == "argon").count(),
                1
            );
        }
        Astra::ok().answer(r)
    };
    let result = council::run(
        &case.setup("ok", "disagree"),
        &case.request(),
        &astra,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result["phases"][2]["agreement"], false);
    assert_eq!(
        case.calls()
            .iter()
            .filter(|c| c["role"] == "fable" && c["phase"] == "C")
            .count(),
        3
    );
    assert_eq!(
        case.calls().iter().filter(|c| c["role"] == "argon").count(),
        1
    );
}
#[test]
fn argon_failure_or_cap_ends_after_phase_a() {
    for scenario in ["garbage", "wrong-shape", "quota", "crash", "argon-tokens"] {
        let case = Case::new();
        let result = case.run(&case.setup("ok", scenario), &Astra::ok()).unwrap();
        assert_eq!(result["degraded"], true, "{scenario}");
        assert!(result["note"]
            .as_str()
            .unwrap()
            .contains("Argon did not review"));
        assert_eq!(result["phases"].as_array().unwrap().len(), 2);
        assert_eq!(
            case.calls().iter().filter(|c| c["role"] == "argon").count(),
            1
        );
        assert!(case.stored()["rounds"][2]["positions"]
            .as_array()
            .unwrap()
            .is_empty());
    }
}
#[test]
fn fable_failure_mid_phase_preserves_astra_and_the_reason() {
    for scenario in ["disagree,fail-third", "disagree,r2-garbage"] {
        let case = Case::new();
        let result = case.run(&case.setup(scenario, "ok"), &Astra::ok()).unwrap();
        assert_eq!(case.member_status("fable"), "malformed");
        assert!(result["phases"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("fable"));
        assert!(case.plan.exists());
        assert!(!case.stored()["rounds"][0]["positions"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(
            case.calls().iter().filter(|c| c["role"] == "argon").count(),
            1
        );
        valid_council(&case);
    }
}
#[test]
fn missing_members_and_missing_plan_degrade_without_losing_the_plan() {
    for (fable, argon, outcome) in [
        (false, false, "ordinary"),
        (false, true, "degraded"),
        (true, false, "degraded"),
    ] {
        let case = Case::new();
        let setup = Setup::new(
            if fable { case.member("ok") } else { absent() },
            if argon { case.member("ok") } else { absent() },
        );
        let result = case.run(&setup, &Astra::ok()).unwrap();
        assert_eq!(result["outcome"], outcome);
        assert!(case.plan.exists());
        assert!(claims(&case).iter().any(|c| c["author"] == "astra"));
        valid_council(&case);
    }
    let case = Case::new();
    fs::remove_file(&case.plan).unwrap();
    assert_eq!(
        case.run(&case.setup("ok", "ok"), &Astra::ok())
            .unwrap_err()
            .kind,
        "astra_missing"
    );
    assert!(case.calls().is_empty());
}
#[test]
fn readonly_flags_and_neutral_latest_replies_are_preserved() {
    let case = Case::new();
    let astra = Astra::ok();
    case.run(&case.setup("ok", "ok"), &astra).unwrap();
    for (role, round) in [("fable", 1), ("fable", 2), ("argon", 1)] {
        let calls = case.calls_of(role, round);
        let call = &calls[0];
        let argv = call["argv"].as_array().unwrap();
        assert_eq!(
            flag_value(argv, "--model"),
            if role == "fable" {
                "claude-fable-5-2"
            } else {
                "gemini-4-argon-high"
            }
        );
        if role == "fable" {
            assert_eq!(
                flag_value(argv, "--tools"),
                if round == 1 { "Read,Glob,Grep" } else { "" }
            );
        } else {
            assert!(argv.iter().any(|a| a == "--sandbox"));
            assert_eq!(flag_value(argv, "--mode"), "plan");
        }
        let prompt = call["prompt"].as_str().unwrap();
        assert!(prompt.contains("[Position"));
        for name in ["astra", "fable", "argon", "claude", "gemini"] {
            assert!(!prompt.to_lowercase().contains(name), "{prompt}");
        }
    }
    assert!(case.calls_of("fable", 1)[0]["prompt"]
        .as_str()
        .unwrap()
        .contains("evidence one"));
    assert!(case.calls_of("argon", 1)[0]["prompt"]
        .as_str()
        .unwrap()
        .contains("SECRET-RAW"));
    let stored = case.stored();
    let usd = stored["cost"]["usd"].as_f64().unwrap();
    let expected = 2.0 * (1000.0 * 8.0 + 2000.0 * 0.20 + 4000.0 * 10.0 + 500.0 * 40.0) / 1e6;
    assert!((usd - expected).abs() < 1e-9, "{usd}");
}
#[test]
fn streaming_and_unreported_usage_stop_later_turns_and_retries() {
    for (scenario, cap) in [
        ("r1-over-cap", 5.0),
        ("unpriced", 0.05),
        ("no-usage", 5.0),
        ("r1-near-cap", 5.0),
        ("result-only", 5.0),
        ("r2-over-cap", 5.0),
    ] {
        let case = Case::new();
        let mut setup = case.setup(scenario, "ok");
        setup.fable_cap_usd = cap;
        let started = std::time::Instant::now();
        let result = case.run(&setup, &Astra::ok()).unwrap();
        if ["r1-over-cap", "r2-over-cap"].contains(&scenario) {
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "stopped by the cap, not the timeout"
            );
        }
        let fable_calls = case
            .calls()
            .into_iter()
            .filter(|c| c["role"] == "fable")
            .collect::<Vec<_>>();
        assert_eq!(
            fable_calls.len(),
            if ["result-only", "r2-over-cap"].contains(&scenario) {
                2
            } else {
                1
            }
        );
        for call in fable_calls {
            assert!(!base::proc::pid_alive(call["pid"].as_u64().unwrap() as u32));
        }
        assert_eq!(
            case.member_status("fable"),
            "budget_cut",
            "{scenario}: {result}"
        );
        assert!(!case.stored()["cost"]["cut"].as_array().unwrap().is_empty());
        if ["r1-over-cap", "unpriced", "result-only"].contains(&scenario) {
            fs::remove_file(case.plan.with_file_name("council.json")).unwrap();
            let before = case.calls().len();
            case.run(&setup, &Astra::ok()).unwrap();
            assert!(case.calls()[before..].iter().all(|c| c["role"] != "fable"));
        }
    }
}
#[test]
fn astra_budget_is_shared_between_phases_and_failures_do_not_erase_the_plan() {
    for status in [Status::Ok, Status::Quota] {
        let case = Case::new();
        let mut setup = case.setup("ok", "disagree");
        setup.astra_token_cap = 100;
        let result = case.run(&setup, &Astra::with(status)).unwrap();
        assert_eq!(result["degraded"], true);
        assert!(case.plan.exists());
        assert!(claims(&case).iter().any(|c| c["author"] == "astra"));
    }
}
#[test]
fn digest_is_bounded_and_rulings_paginate_without_losing_provenance() {
    let case = Case::new();
    let result = case
        .run(
            &case.setup("huge,disagree", "huge,disagree,add-claims"),
            &Astra::ok(),
        )
        .unwrap();
    assert!(
        result.to_string().chars().count() <= DIGEST_MAX_CHARS + 512,
        "{}",
        result.to_string().chars().count()
    );
    assert!(result["counts"]["dropped_at_intake"].as_u64().unwrap() >= 8);
    assert!(!result.to_string().contains(&"x".repeat(250)));
    assert!(
        fs::read_to_string(case.plan.with_file_name("council").join("A-r1-fable.json"))
            .unwrap()
            .contains(&"x".repeat(4000))
    );
    let mut batch = result["open"].as_array().unwrap().clone();
    let mut total = 0;
    while !batch.is_empty() {
        total += batch.len();
        let rulings = batch
            .iter()
            .map(|c| {
                (
                    c["id"].as_str().unwrap().into(),
                    "upheld".into(),
                    "checked".into(),
                )
            })
            .collect::<Vec<_>>();
        let reply = council::rule(&case.plan, &rulings).unwrap();
        batch = reply["next"].as_array().unwrap().clone();
    }
    assert_eq!(
        total,
        claims(&case)
            .iter()
            .filter(|c| c["status"] != "consensus")
            .count()
    );
    valid_council(&case);
    assert!(council::rule(
        &case.plan,
        &[("C99".into(), "upheld".into(), "reason".into())]
    )
    .is_err());
}
#[test]
fn completed_council_is_reused_without_another_review_or_spend() {
    let case = Case::new();
    let setup = case.setup("ok", "ok");
    let astra = Astra::ok();
    case.run(&setup, &astra).unwrap();
    let count = case.calls().len();
    assert_eq!(case.run(&setup, &astra).unwrap()["reused"], true);
    assert_eq!(case.calls().len(), count);
}

// ---- bridge and launch ----

fn bridge_config(home: &Path) -> Config {
    Config {
        data_home: Some(home.into()),
        codex: Ok(common::fake("fake_codex")),
        wsl: None,
        on_windows: false,
    }
}

fn bridge_args(store: &Path, repo: &Path) -> Value {
    json!({
        "project": "demo",
        "cwd": repo,
        "plan_path": store.join("astra/plan.json"),
        "schema_path": common::repo_root().join("engine/research/audit.schema.json"),
        "brief": BRIEF,
    })
}

fn bridge_fixture(temp: &tempfile::TempDir) -> (PathBuf, PathBuf) {
    let store = orch::store_for(temp.path(), "demo").unwrap();
    let repo = temp.path().join("repo");
    fs::create_dir_all(store.join("astra")).unwrap();
    fs::create_dir_all(&repo).unwrap();
    fs::copy(
        {
            let case = Case::new();
            let plan = fs::read(&case.plan).unwrap();
            let path = temp.path().join("plan-source.json");
            fs::write(&path, plan).unwrap();
            path
        },
        store.join("astra/plan.json"),
    )
    .unwrap();
    (store, repo)
}

#[test]
fn the_tool_does_not_exist_without_the_switch() {
    let temp = common::temp();
    let (store, repo) = bridge_fixture(&temp);
    let bridge = Bridge::new(bridge_config(temp.path()));
    assert_eq!(bridge.tool_list(), tools());
    assert!(!tools()
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "council"));
    let reply = call(&bridge, "council", &bridge_args(&store, &repo));
    assert_eq!(
        reply["structuredContent"]["error"]["kind"], "unknown_tool",
        "{reply}"
    );
    assert!(!store.join("astra/council.json").exists());
}

#[test]
fn the_tool_runs_with_the_real_astra_path_and_caps_its_tokens() {
    let temp = common::temp();
    let (store, repo) = bridge_fixture(&temp);
    let case = Case::new();
    let bridge = Bridge::new(bridge_config(temp.path())).with_council(case.setup("ok", "ok"));
    assert!(bridge
        .tool_list()
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "council"));
    let reply = call(&bridge, "council", &bridge_args(&store, &repo));
    let digest = &reply["structuredContent"];
    assert_eq!(digest["outcome"], "full", "{reply}");
    assert!(
        digest["rounds"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r.get("positions").is_none()),
        "the core gets a digest only"
    );
    let stored: Value =
        serde_json::from_slice(&fs::read(store.join("astra/council.json")).unwrap()).unwrap();
    assert_eq!(stored["members"][0]["status"], "ok", "{stored}");
    assert_eq!(stored["members"][0]["tokens"], 1240);
    assert!(stored["members"][0]["model"]
        .as_str()
        .unwrap()
        .contains("astra"));

    let temp = common::temp();
    let (store, repo) = bridge_fixture(&temp);
    let mut setup = case.setup("ok", "ok");
    setup.astra_token_cap = 100;
    let bridge = Bridge::new(bridge_config(temp.path())).with_council(setup);
    let reply = call(&bridge, "council", &bridge_args(&store, &repo));
    assert_eq!(reply["structuredContent"]["outcome"], "degraded", "{reply}");
    let stored: Value =
        serde_json::from_slice(&fs::read(store.join("astra/council.json")).unwrap()).unwrap();
    assert_eq!(stored["members"][0]["status"], "budget_cut", "{stored}");
    assert_eq!(stored["cost"]["cut"], json!(["astra:A:round2"]));

    let temp = common::temp();
    let (store, repo) = bridge_fixture(&temp);
    let bridge = Bridge::new(bridge_config(temp.path())).with_council(case.setup("ok", "ok"));
    let mut args = bridge_args(&store, &repo);
    args["plan_path"] = json!(temp.path().join("elsewhere.json"));
    let reply = call(&bridge, "council", &args);
    assert_eq!(
        reply["structuredContent"]["error"]["kind"], "invalid_argument",
        "{reply}"
    );
}

fn launch(
    home: &Path,
    record: &Path,
    extra: &[&str],
    words: &[&str],
) -> (Value, std::process::Output) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_axiom"));
    c.env("AXIOM_HOME", home)
        .env("AXIOM_DATA_HOME", home)
        .env_remove(council::ENV)
        .env(
            "AXIOM_CLAUDE_CMD",
            serde_json::to_string(&common::fake("fake_claude")).unwrap(),
        )
        .env("FAKE_CLAUDE_RECORD", record)
        .stdin(Stdio::null());
    c.arg("wish").args(extra).args(words);
    let output = c.output().unwrap();
    let v = serde_json::from_slice(&fs::read(record).unwrap()).unwrap();
    (v, output)
}

/// Run in a separate temp home each time; the log path and plugin dir are the same per home.
fn argv_of(v: &Value) -> Vec<String> {
    v["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn the_switch_off_leaves_the_launch_exactly_as_today() {
    let temp = common::temp();
    let record = temp.path().join("record.json");
    let (off, _) = launch(temp.path(), &record, &[], &["fix", "the", "bug"]);
    let argv = argv_of(&off);
    let plugin = axiom::engine::plugin_dir(temp.path()).unwrap();
    let config = axiom::session::Config {
        claude: Ok(common::fake("fake_claude")),
        home: temp.path().to_path_buf(),
        bin: PathBuf::new(),
        clock: axiom::session::Clock::system(),
    };
    let expected = axiom::session::command(&config, &plugin, None, false).unwrap();
    assert_eq!(argv, expected[common::fake("fake_claude").len()..].to_vec());
    assert!(off["env"]["AXIOM_ADVANCED_RESEARCH"].is_null(), "{off}");
    assert_eq!(
        off["messages"][0]["message"]["content"][0]["text"],
        "/chronos:wish fix the bug"
    );
    assert!(!argv.iter().any(|a| a == "--append-system-prompt"));
    assert!(!argv.iter().any(|a| a.contains("__council")));

    let (on, output) = launch(
        temp.path(),
        &record,
        &["--advanced-research"],
        &["fix", "the", "bug"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("capped at $5.00"));
    let on_argv = argv_of(&on);
    assert_eq!(on["env"]["AXIOM_ADVANCED_RESEARCH"], "1");
    assert_eq!(
        on["messages"][0]["message"]["content"][0]["text"],
        "/chronos:wish fix the bug"
    );
    let at = on_argv
        .iter()
        .position(|a| a == "--append-system-prompt")
        .unwrap();
    assert_eq!(on_argv[at + 1], council::SYSTEM_PROMPT);
    let tools_at = on_argv.iter().position(|a| a == "--allowedTools").unwrap();
    assert_eq!(
        on_argv[tools_at + 1],
        format!("{},{}", argv[tools_at + 1], council::TOOL)
    );
    let mut without: Vec<String> = on_argv.clone();
    without.drain(at..at + 2);
    without[tools_at] = argv[tools_at].clone();
    without[tools_at + 1] = argv[tools_at + 1].clone();
    assert_eq!(without, argv, "the switch adds nothing else");
}

#[test]
fn words_in_a_wish_cannot_turn_the_council_on() {
    let temp = common::temp();
    let record = temp.path().join("record.json");
    let (off, _) = launch(
        temp.path(),
        &record,
        &[],
        &["--", "--advanced-research", "[advanced-research]"],
    );
    let argv = argv_of(&off);
    assert!(off["env"]["AXIOM_ADVANCED_RESEARCH"].is_null());
    assert!(!argv.iter().any(|a| a == "--append-system-prompt"));
}

#[test]
fn a_resumed_session_keeps_its_switch_and_only_its_switch() {
    let temp = common::temp();
    let record = temp.path().join("record.json");
    let session = "12345678-1234-1234-1234-123456789abc";
    let (_, output) = launch(temp.path(), &record, &["--advanced-research"], &["fix"]);
    assert!(output.status.success());
    assert!(council::switch_remembered(temp.path(), session));

    let config = axiom::session::Config {
        claude: Ok(common::fake("fake_claude")),
        home: temp.path().to_path_buf(),
        bin: PathBuf::new(),
        clock: axiom::session::Clock::system(),
    };
    let project = temp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    std::env::set_var("FAKE_CLAUDE_RECORD", &record);
    let spawned =
        axiom::session::spawn(&config, "wish", "go on", &project, None, Some(session)).unwrap();
    wait_for_record(&record);
    let v: Value = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
    assert_eq!(v["env"]["AXIOM_ADVANCED_RESEARCH"], "1");
    assert!(argv_of(&v).iter().any(|a| a == "--append-system-prompt"));
    drop(spawned);

    assert!(!council::switch_remembered(temp.path(), "other-session"));
}

fn wait_for_record(record: &Path) {
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if let Ok(bytes) = fs::read(record) {
            if serde_json::from_slice::<Value>(&bytes).is_ok_and(|v| {
                !v["messages"].as_array().unwrap().is_empty()
                    && v["argv"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|a| a == "--resume")
            }) {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("the record was never written");
}

#[test]
fn the_advanced_research_flag_is_refused_on_resume() {
    let temp = common::temp();
    let o = Command::new(env!("CARGO_BIN_EXE_axiom"))
        .env("AXIOM_HOME", temp.path())
        .env("AXIOM_DATA_HOME", temp.path())
        .args(["wish", "--resume", "abc", "--advanced-research"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("--resume"));
}

#[test]
fn the_bridge_process_serves_the_tool_only_with_the_switch() {
    use std::io::Write;
    let run = |switched: bool| -> Value {
        let temp = common::temp();
        let mut c = Command::new(env!("CARGO_BIN_EXE_axiom"));
        c.arg("bridge")
            .env("AXIOM_DATA_HOME", temp.path())
            .env_remove(council::ENV)
            .env("AXIOM_CLAUDE_CMD", "[\"no-such-claude\"]")
            .env("AXIOM_AGY_CMD", "[\"no-such-agy\"]")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if switched {
            c.env(council::ENV, "1");
        }
        let mut child = c.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        serde_json::from_slice(output.stdout.split(|b| *b == b'\n').next().unwrap()).unwrap()
    };
    let names = |reply: &Value| -> Vec<String> {
        reply["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect()
    };
    let off = run(false);
    assert_eq!(off["result"]["tools"], tools());
    assert!(names(&run(true)).contains(&"council".to_string()));
}

#[test]
fn cli_caps_require_opt_in_and_override_only_the_child_environment() {
    for (flag, value) in [
        ("--fable-cap", "20"),
        ("--astra-cap", "600000"),
        ("--council-rounds", "5"),
    ] {
        let temp = common::temp();
        let output = Command::new(env!("CARGO_BIN_EXE_axiom"))
            .args(["wish", flag, value, "fix"])
            .env("AXIOM_HOME", temp.path())
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("require --advanced-research"));
    }
    let temp = common::temp();
    let record = temp.path().join("record.json");
    let output = Command::new(env!("CARGO_BIN_EXE_axiom"))
        .args([
            "wish",
            "--fable-cap",
            "20",
            "--advanced-research",
            "--astra-cap",
            "600000",
            "--council-rounds",
            "5",
            "fix",
        ])
        .env("AXIOM_HOME", temp.path())
        .env("AXIOM_DATA_HOME", temp.path())
        .env(
            "AXIOM_CLAUDE_CMD",
            serde_json::to_string(&common::fake("fake_claude")).unwrap(),
        )
        .env("FAKE_CLAUDE_RECORD", &record)
        .env("AXIOM_FABLE_CAP_USD", "overridden-invalid")
        .env("AXIOM_ARGON_TOKEN_CAP", "40000")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let v: Value = serde_json::from_slice(&fs::read(record).unwrap()).unwrap();
    for (key, value) in [
        ("AXIOM_FABLE_CAP_USD", "20"),
        ("AXIOM_ASTRA_TOKEN_CAP", "600000"),
        ("AXIOM_ARGON_TOKEN_CAP", "40000"),
        ("AXIOM_COUNCIL_ROUNDS", "5"),
    ] {
        assert_eq!(v["env"][key], value);
    }
    assert!(String::from_utf8_lossy(&output.stderr).contains("$20.00, Astra at 600k tokens, Argon at 40k tokens for one review, up to 5 rounds per phase"));
}

#[test]
fn bridge_refuses_invalid_environment_without_serving_or_calling_members() {
    for (name, value) in [
        ("AXIOM_FABLE_CAP_USD", "201"),
        ("AXIOM_ASTRA_TOKEN_CAP", "-1"),
        ("AXIOM_ARGON_TOKEN_CAP", "NaN"),
        ("AXIOM_COUNCIL_ROUNDS", "0"),
    ] {
        let temp = common::temp();
        let output = Command::new(env!("CARGO_BIN_EXE_axiom"))
            .arg("bridge")
            .env("AXIOM_DATA_HOME", temp.path())
            .env(council::ENV, "1")
            .env(name, value)
            .env("AXIOM_CLAUDE_CMD", "[\"no-real-claude\"]")
            .env("AXIOM_AGY_CMD", "[\"no-real-agy\"]")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains(name) && error.contains(&format!("{value:?}")),
            "{error}"
        );
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn legacy_finished_councils_are_not_paid_for_again() {
    let case = Case::new();
    let setup = case.setup("ok", "ok");
    case.run(&setup, &Astra::ok()).unwrap();
    let mut saved = case.stored();
    saved.as_object_mut().unwrap().remove("phases");
    for r in saved["rounds"].as_array_mut().unwrap() {
        r.as_object_mut().unwrap().remove("phase");
        r.as_object_mut().unwrap().remove("turns");
    }
    for c in saved["claims"].as_array_mut().unwrap() {
        for k in ["phase", "round", "author", "history"] {
            c.as_object_mut().unwrap().remove(k);
        }
    }
    fs::write(
        case.plan.with_file_name("council.json"),
        serde_json::to_vec(&saved).unwrap(),
    )
    .unwrap();
    let before = case.calls().len();
    assert_eq!(case.run(&setup, &Astra::ok()).unwrap()["reused"], true);
    assert_eq!(case.calls().len(), before);
}
#[test]
fn many_rounds_keep_a_bounded_digest_and_each_stances_phase() {
    let case = Case::new();
    case.run(&case.setup("ok", "disagree"), &Astra::ok())
        .unwrap();
    let mut stored: council::Council = serde_json::from_value(case.stored()).unwrap();
    let mut round = stored.rounds[1].clone();
    stored.rounds.clear();
    for n in 1..=1000 {
        round.n = n;
        stored.rounds.push(round.clone());
    }
    let result = council::digest(&stored, &case.plan.with_file_name("council.json"), false);
    assert!(result.to_string().chars().count() <= DIGEST_MAX_CHARS);
    assert_eq!(result["rounds"][0]["through"], 1000);
    assert!(result["open"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["stance_phases"]["argon"]["phase"] == "B"));
}

#[test]
fn rejected_new_review_claims_stay_open_for_the_core() {
    let case = Case::new();
    let astra = |r: &CrossRequest| {
        let mut reply = Astra::ok().answer(r);
        if r.prompt.contains("COUNCIL PHASE C") {
            let ids = Regex::new(r"(?m)^(C\d+) \[Position").unwrap();
            reply.output.as_mut().unwrap()["resolutions"]=json!(ids.captures_iter(&r.prompt).map(|c|json!({"claim":c[1].to_string(),"decision":"rejected","reason":"the premise is contradicted by src/mod.py:12","change":""})).collect::<Vec<_>>());
        }
        reply
    };
    let result = council::run(
        &case.setup("ok,settle", "add-claims"),
        &case.request(),
        &astra,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result["phases"][2]["agreement"], true);
    for claim in claims(&case).iter().filter(|c| c["author"] == "argon") {
        assert_ne!(claim["status"], "consensus");
        assert_eq!(claim["stances"]["astra"], "disagree");
        assert_eq!(claim["stances"]["fable"], "disagree");
        assert_eq!(claim["stances"]["argon"], "author");
    }
}

#[test]
fn missing_approval_or_claim_reviews_do_not_silently_count_as_agreement() {
    for (fable, argon, member) in [
        ("missing-approval", "ok", "fable"),
        ("ok", "partial-review", "argon"),
    ] {
        let case = Case::new();
        let result = case.run(&case.setup(fable, argon), &Astra::ok()).unwrap();
        assert_eq!(case.member_status(member), "malformed", "{result}");
        assert_eq!(result["degraded"], true);
        if member == "argon" {
            assert!(result["note"]
                .as_str()
                .unwrap()
                .contains("Argon did not review"));
        }
    }
}

#[test]
fn astra_can_revise_a_claim_before_fable_and_the_single_review() {
    let case = Case::new();
    let original = fs::read(&case.plan).unwrap();
    let astra = |r: &CrossRequest| {
        let mut reply = Astra::ok().answer(r);
        reply.output.as_mut().unwrap()["resolutions"] = json!([{"claim":"C01","decision":"accepted","reason":"the rebuttal identified unbounded growth","change":"use a bounded cache with eviction"}]);
        reply
    };
    let result = council::run(
        &case.setup("ok", "ok"),
        &case.request(),
        &astra,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result["phases"][0]["agreement"], true);
    assert!(case.calls_of("fable", 2)[0]["prompt"]
        .as_str()
        .unwrap()
        .contains("use a bounded cache with eviction"));
    assert!(case.calls_of("argon", 1)[0]["prompt"]
        .as_str()
        .unwrap()
        .contains("use a bounded cache with eviction"));
    assert_eq!(fs::read(&case.plan).unwrap(), original);
}

#[test]
fn accepted_review_objections_require_a_matching_change_from_both_members() {
    let case = Case::new();
    let astra = |r: &CrossRequest| {
        let mut reply = Astra::ok().answer(r);
        if r.prompt.contains("COUNCIL PHASE C") {
            let ids = Regex::new(r"(?m)^(C\d+) \[Position").unwrap();
            reply.output.as_mut().unwrap()["resolutions"]=json!(ids.captures_iter(&r.prompt).map(|c|json!({"claim":c[1].to_string(),"decision":"accepted","reason":"apply the review change","change":"bound the cache at 100 entries"})).collect::<Vec<_>>());
        }
        reply
    };
    let result = council::run(
        &case.setup("ok,settle,accept", "disagree,add-claims"),
        &case.request(),
        &astra,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result["phases"][2]["agreement"], true);
    assert_eq!(
        case.calls().iter().filter(|c| c["role"] == "argon").count(),
        1
    );
    assert!(result["agreed"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["phase"] == "B"
            && c["changes"]
                .as_array()
                .is_some_and(|changes| changes.len() == 2)));
    assert!(claims(&case)
        .iter()
        .filter(|c| c["author"] == "argon")
        .all(|c| c["stances"]["argon"] == "author"));
}

#[test]
fn argon_envelopes_and_fable_timeouts_keep_their_original_guards() {
    for scenario in ["shape-top", "shape-object"] {
        let case = Case::new();
        assert_eq!(
            case.run(&case.setup("ok", scenario), &Astra::ok()).unwrap()["outcome"],
            "full"
        );
    }
    let case = Case::new();
    let mut setup = case.setup("hang", "ok");
    setup.round1_timeout = Duration::from_millis(400);
    case.run(&setup, &Astra::ok()).unwrap();
    assert_eq!(case.member_status("fable"), "timeout");
    assert!(!base::proc::pid_alive(
        case.calls_of("fable", 1)[0]["pid"].as_u64().unwrap() as u32
    ));
}
#[test]
fn authorship_and_checkable_evidence_still_determine_claim_status() {
    let case = Case::new();
    case.run(&case.setup("disagree", "disagree"), &Astra::ok())
        .unwrap();
    for c in claims(&case) {
        assert_eq!(
            c["stances"]
                .as_object()
                .unwrap()
                .values()
                .filter(|v| **v == "author")
                .count(),
            1
        );
        if c["author"] == "astra" {
            assert_eq!(c["status"], "disputed");
            assert_eq!(c["dissent"].as_array().unwrap().len(), 2);
        } else {
            assert_eq!(c["status"], "majority");
        }
    }
    let case = Case::new();
    case.run(&case.setup("disagree-bare", "disagree-bare"), &Astra::ok())
        .unwrap();
    for c in claims(&case).iter().filter(|c| c["author"] == "astra") {
        assert_eq!(c["stances"]["fable"], "unsupported");
        assert_eq!(c["status"], "disputed");
        assert!(c["dissent"].as_array().unwrap().is_empty());
    }
}
#[test]
fn ruling_guards_still_refuse_consensus_bad_decisions_and_empty_reasons() {
    let case = Case::new();
    case.run(&case.setup("disagree", "ok"), &Astra::ok())
        .unwrap();
    let all = claims(&case);
    let open = all.iter().find(|c| c["status"] != "consensus").unwrap()["id"]
        .as_str()
        .unwrap();
    let agreed = all.iter().find(|c| c["status"] == "consensus").unwrap()["id"]
        .as_str()
        .unwrap();
    for (id, decision, reason) in [
        (agreed, "upheld", "checked"),
        (open, "maybe", "checked"),
        (open, "upheld", " "),
    ] {
        assert!(council::rule(&case.plan, &[(id.into(), decision.into(), reason.into())]).is_err());
    }
    council::rule(
        &case.plan,
        &[(open.into(), "upheld".into(), "checked".into())],
    )
    .unwrap();
    assert_eq!(
        claims(&case).iter().find(|c| c["id"] == open).unwrap()["ruling"]["decision"],
        "upheld"
    );
}
