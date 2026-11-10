mod common;

use axiom::{
    base,
    bridge::{call, tools, Bridge, Config},
    council::{self, CrossReply, CrossRequest, Setup, Stance, Status, DIGEST_MAX_CHARS},
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

fn stance(claim: &Value, member: &str) -> String {
    claim["stances"][member].as_str().unwrap().to_owned()
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
fn all_three_members_succeed() {
    let case = Case::new();
    let astra = Astra::ok();
    let digest = case.run(&case.setup("ok", "ok"), &astra).unwrap();
    assert_eq!(digest["outcome"], "full", "{digest}");
    assert_eq!(digest["degraded"], false);
    assert_eq!(digest["status"], "completed");
    valid_council(&case);
    let stored = case.stored();
    assert_eq!(
        stored["rounds"][0]["positions"].as_array().unwrap().len(),
        3
    );
    assert_eq!(
        stored["rounds"][1]["positions"].as_array().unwrap().len(),
        3
    );
    let all = claims(&case);
    assert_eq!(all.len(), 6);
    for claim in &all {
        assert_eq!(claim["status"], "consensus", "{claim}");
    }
    for member in ["astra", "fable", "argon"] {
        assert_eq!(case.member_status(member), "ok");
    }
    // Two rounds of repeated usage events: priced once per message, never the CLI's dollars.
    let expected = 2.0 * (1000.0 * 8.0 + 2000.0 * 0.20 + 4000.0 * 10.0 + 500.0 * 40.0) / 1e6;
    let usd = stored["cost"]["usd"].as_f64().unwrap();
    assert!((usd - expected).abs() < 1e-9, "{usd} vs {expected}");
    assert_eq!(stored["cost"]["cap_usd"], 5.0);
    assert_eq!(stored["members"][0]["model"], "gpt-6-astra");
    assert_eq!(stored["members"][0]["family"], "openai");
    assert_eq!(stored["members"][1]["family"], "anthropic");
    assert_eq!(stored["members"][2]["family"], "google");
    for position in stored["rounds"][0]["positions"].as_array().unwrap() {
        let bytes = fs::read(position["path"].as_str().unwrap()).unwrap();
        let digest = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&bytes));
        assert_eq!(position["sha256"], digest.as_str());
    }
}

#[test]
fn members_are_launched_read_only_with_the_given_flags() {
    let case = Case::new();
    case.run(&case.setup("ok", "ok"), &Astra::ok()).unwrap();
    let r1 = &case.calls_of("fable", 1)[0];
    let argv = r1["argv"].as_array().unwrap();
    assert_eq!(flag_value(argv, "--model"), "claude-fable-5-2");
    assert_eq!(flag_value(argv, "--output-format"), "stream-json");
    assert_eq!(flag_value(argv, "--setting-sources"), "local");
    assert_eq!(flag_value(argv, "--tools"), "Read,Glob,Grep");
    assert_eq!(flag_value(argv, "--allowedTools"), "Read,Glob,Grep");
    assert!(argv.iter().any(|a| a == "--verbose"));
    assert!(argv.iter().any(|a| a == "-p"));
    let r2 = &case.calls_of("fable", 2)[0];
    let argv = r2["argv"].as_array().unwrap();
    assert_eq!(flag_value(argv, "--tools"), "");
    assert!(!argv.iter().any(|a| a == "--allowedTools"));
    let argon = &case.calls_of("argon", 1)[0];
    let argv = argon["argv"].as_array().unwrap();
    assert_eq!(flag_value(argv, "--model"), "gemini-4-argon-high");
    assert_eq!(flag_value(argv, "--output-format"), "json");
    assert_eq!(flag_value(argv, "--mode"), "plan");
    assert!(argv.iter().any(|a| a == "--sandbox"));
    assert!(flag_value(argv, "--json-schema").ends_with("audit.schema.json"));
    assert!(flag_value(argv, "--print-timeout").ends_with('s'));
    assert_eq!(argv[argv.len() - 2], "--print");
}

#[test]
fn round_one_is_blind_and_round_two_is_anonymous() {
    let case = Case::new();
    let astra = Astra::ok();
    case.run(&case.setup("ok", "ok"), &astra).unwrap();
    for role in ["fable", "argon"] {
        let prompt = case.calls_of(role, 1)[0]["prompt"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(prompt.contains(BRIEF));
        assert!(!prompt.contains("astra evidence"), "{prompt}");
    }
    let fable_r2 = case.calls_of("fable", 2)[0]["prompt"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        fable_r2.contains("evidence one"),
        "fable must see astra's claims"
    );
    let lower = fable_r2.to_lowercase();
    for name in ["astra", "argon", "gemini", "fable", "claude"] {
        assert!(!lower.contains(name), "{name} leaked: {fable_r2}");
    }
    let own = Regex::new(r"fable\d").unwrap();
    assert!(
        !own.is_match(&fable_r2),
        "fable must not be shown its own claims"
    );
    let shown = astra.prompt.lock().unwrap().clone();
    assert!(shown.contains("[member]"), "names are masked: {shown}");
    assert!(!shown.to_lowercase().contains("claude"));
}

#[test]
fn fable_missing_runs_with_two_and_says_so() {
    let case = Case::new();
    let setup = Setup::new(absent(), case.member("ok"));
    let digest = case.run(&setup, &Astra::ok()).unwrap();
    assert_eq!(digest["outcome"], "degraded", "{digest}");
    assert_eq!(digest["degraded"], true);
    assert!(
        digest["note"].as_str().unwrap().contains("fable: missing"),
        "{digest}"
    );
    assert_eq!(case.member_status("fable"), "missing");
    assert!(case.calls().iter().all(|c| c["role"] == "argon"));
    let all = claims(&case);
    assert_eq!(all.len(), 4);
    for claim in &all {
        assert_eq!(stance(claim, "fable"), "absent");
        assert_eq!(claim["status"], "consensus", "{claim}");
    }
    valid_council(&case);
}

#[test]
fn argon_missing_runs_with_two_and_says_so() {
    let case = Case::new();
    let setup = Setup::new(case.member("ok"), absent());
    let digest = case.run(&setup, &Astra::ok()).unwrap();
    assert_eq!(digest["outcome"], "degraded");
    assert!(
        digest["note"].as_str().unwrap().contains("argon: missing"),
        "{digest}"
    );
    assert_eq!(case.member_status("argon"), "missing");
    assert!(case.calls().iter().all(|c| c["role"] == "fable"));
    assert_eq!(case.calls_of("fable", 2).len(), 1);
}

#[test]
fn astra_missing_blocks_before_anyone_is_paid() {
    let case = Case::new();
    fs::remove_file(&case.plan).unwrap();
    let astra = Astra::ok();
    let error = case.run(&case.setup("ok", "ok"), &astra).unwrap_err();
    assert_eq!(error.kind, "astra_missing");
    assert!(case.calls().is_empty());
    assert_eq!(astra.calls.load(Ordering::SeqCst), 0);
    assert!(!case.plan.with_file_name("council.json").exists());
    fs::write(&case.plan, "{\"not\":\"the schema\"}").unwrap();
    let error = case.run(&case.setup("ok", "ok"), &astra).unwrap_err();
    assert_eq!(error.kind, "astra_missing");
    assert!(case.calls().is_empty());
}

#[test]
fn fable_and_argon_both_missing_is_an_ordinary_wish() {
    let case = Case::new();
    let astra = Astra::ok();
    let setup = Setup::new(absent(), absent());
    let digest = case.run(&setup, &astra).unwrap();
    assert_eq!(digest["outcome"], "ordinary", "{digest}");
    assert_eq!(digest["stage_note"], "ordinary wish: no council");
    assert!(digest["note"].as_str().unwrap().contains("ordinary wish"));
    assert_eq!(
        astra.calls.load(Ordering::SeqCst),
        0,
        "no round 2 for one member"
    );
    assert!(claims(&case).is_empty());
    assert_eq!(case.stored()["degraded"], true);
    valid_council(&case);
}

#[test]
fn a_member_out_of_quota_is_named_and_the_rest_go_on() {
    let case = Case::new();
    case.run(&case.setup("quota", "ok"), &Astra::ok()).unwrap();
    assert_eq!(case.member_status("fable"), "quota");
    assert_eq!(case.stored()["cost"]["usd"].as_f64().unwrap() < 0.1, true);

    let case = Case::new();
    let digest = case.run(&case.setup("ok", "quota"), &Astra::ok()).unwrap();
    assert_eq!(case.member_status("argon"), "quota");
    assert_eq!(digest["outcome"], "degraded");

    let case = Case::new();
    let digest = case
        .run(&case.setup("ok", "ok"), &Astra::with(Status::Quota))
        .unwrap();
    assert_eq!(case.member_status("astra"), "quota");
    assert_eq!(digest["outcome"], "degraded");
    for claim in claims(&case) {
        if stance(&claim, "astra") != "author" {
            assert_eq!(stance(&claim, "astra"), "absent", "{claim}");
        }
    }

    let case = Case::new();
    case.run(&case.setup("ok", "r2-quota"), &Astra::ok())
        .unwrap();
    assert_eq!(case.member_status("argon"), "quota");
    assert_eq!(
        case.stored()["rounds"][0]["positions"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn the_cap_stops_round_two_from_starting() {
    let case = Case::new();
    // Round 1's stream shows little; its closing total shows $4.65 of the $5 spent.
    let digest = case
        .run(&case.setup("r1-near-cap", "ok"), &Astra::ok())
        .unwrap();
    assert_eq!(case.member_status("fable"), "budget_cut");
    assert!(
        case.calls_of("fable", 2).is_empty(),
        "round 2 must not be spawned"
    );
    let stored = case.stored();
    assert_eq!(stored["cost"]["cut"], json!(["fable:round2"]));
    assert!(stored["cost"]["usd"].as_f64().unwrap() > 4.0);
    assert_eq!(
        stored["rounds"][0]["positions"].as_array().unwrap().len(),
        3
    );
    assert_eq!(digest["outcome"], "degraded");
    for claim in claims(&case) {
        if stance(&claim, "fable") != "author" {
            assert_eq!(stance(&claim, "fable"), "absent", "{claim}");
        }
    }
    assert!(claims(&case).iter().any(|c| stance(c, "fable") == "author"));
    valid_council(&case);
}

#[test]
fn the_cap_kills_round_two_while_it_runs() {
    let case = Case::new();
    let mut setup = case.setup("r2-over-cap", "ok");
    setup.round2_timeout = Duration::from_secs(25);
    let started = std::time::Instant::now();
    case.run(&setup, &Astra::ok()).unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "killed at the cap, not by the timeout"
    );
    assert_eq!(case.member_status("fable"), "budget_cut");
    let round2 = case.calls_of("fable", 2);
    assert_eq!(round2.len(), 1);
    assert!(!base::proc::pid_alive(
        round2[0]["pid"].as_u64().unwrap() as u32
    ));
    let stored = case.stored();
    assert_eq!(stored["cost"]["cut"], json!(["fable:round2"]));
    assert!(stored["cost"]["usd"].as_f64().unwrap() >= 5.0);
    assert_eq!(
        stored["rounds"][0]["positions"].as_array().unwrap().len(),
        3
    );
    assert_eq!(
        stored["rounds"][1]["positions"].as_array().unwrap().len(),
        2
    );
}

#[test]
fn the_cap_kills_round_one_and_fable_counts_as_missing() {
    let case = Case::new();
    let mut setup = case.setup("r1-over-cap", "ok");
    setup.round1_timeout = Duration::from_secs(25);
    let started = std::time::Instant::now();
    case.run(&setup, &Astra::ok()).unwrap();
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(case.member_status("fable"), "budget_cut");
    assert!(case.calls_of("fable", 2).is_empty());
    let round1 = case.calls_of("fable", 1);
    assert!(!base::proc::pid_alive(
        round1[0]["pid"].as_u64().unwrap() as u32
    ));
    let stored = case.stored();
    assert_eq!(stored["cost"]["cut"], json!(["fable:round1"]));
    let positions = stored["rounds"][0]["positions"].as_array().unwrap();
    assert_eq!(positions.len(), 2);
    assert!(positions.iter().all(|p| p["member"] != "fable"));
    assert_eq!(claims(&case).len(), 4);
}

#[test]
fn a_model_the_table_cannot_price_fails_closed() {
    let case = Case::new();
    let mut setup = case.setup("unpriced", "ok");
    setup.fable_cap_usd = 0.05; // 1000 unpriced tokens of allowance; a call reports 7500.
    case.run(&setup, &Astra::ok()).unwrap();
    assert_eq!(case.member_status("fable"), "budget_cut");
    let stored = case.stored();
    assert!(stored["cost"]["unpriced_tokens"].as_u64().unwrap() > 0);
    assert_eq!(stored["cost"]["usd"].as_f64().unwrap(), 0.0);
}

#[test]
fn usage_that_is_never_reported_stops_round_two() {
    let case = Case::new();
    case.run(&case.setup("no-usage", "ok"), &Astra::ok())
        .unwrap();
    assert_eq!(case.member_status("fable"), "budget_cut");
    assert!(case.calls_of("fable", 2).is_empty());
    assert_eq!(case.stored()["cost"]["cut"], json!(["fable:round2"]));
}

#[test]
fn spend_carries_over_when_a_run_is_retried() {
    let case = Case::new();
    case.run(&case.setup("r1-near-cap", "ok"), &Astra::ok())
        .unwrap();
    fs::remove_file(case.plan.with_file_name("council.json")).unwrap();
    let before = case.calls().len();
    case.run(&case.setup("ok", "ok"), &Astra::ok()).unwrap();
    assert!(
        case.calls()[before..]
            .iter()
            .all(|c| c["role"] != "fable" || c["round"] == 1),
        "fable's round 2 must not be funded a second time"
    );
    assert_eq!(
        case.member_status("fable"),
        "budget_cut",
        "the budget is the brief's"
    );
}

#[test]
fn malformed_output_is_named_and_never_counted() {
    let case = Case::new();
    case.run(&case.setup("garbage", "ok"), &Astra::ok())
        .unwrap();
    assert_eq!(case.member_status("fable"), "malformed");
    assert_eq!(
        case.stored()["rounds"][0]["positions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let case = Case::new();
    case.run(&case.setup("wrong-shape", "ok"), &Astra::ok())
        .unwrap();
    assert_eq!(case.member_status("fable"), "malformed");

    let case = Case::new();
    case.run(&case.setup("ok", "garbage"), &Astra::ok())
        .unwrap();
    assert_eq!(case.member_status("argon"), "malformed");

    let case = Case::new();
    case.run(&case.setup("ok", "wrong-shape"), &Astra::ok())
        .unwrap();
    assert_eq!(case.member_status("argon"), "malformed");

    let case = Case::new();
    let digest = case
        .run(&case.setup("r2-garbage", "ok"), &Astra::ok())
        .unwrap();
    assert_eq!(case.member_status("fable"), "malformed");
    assert_eq!(digest["outcome"], "degraded");
    assert_eq!(
        case.stored()["rounds"][0]["positions"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    for claim in claims(&case) {
        if stance(&claim, "fable") != "author" {
            assert_eq!(stance(&claim, "fable"), "absent");
        }
    }
}

#[test]
fn argon_envelopes_and_crashes_and_timeouts() {
    for scenario in ["shape-top", "shape-object"] {
        let case = Case::new();
        let digest = case.run(&case.setup("ok", scenario), &Astra::ok()).unwrap();
        assert_eq!(digest["outcome"], "full", "{scenario}: {digest}");
    }
    let case = Case::new();
    case.run(&case.setup("ok", "crash"), &Astra::ok()).unwrap();
    assert_eq!(case.member_status("argon"), "failed");

    let case = Case::new();
    let mut setup = case.setup("hang", "ok");
    setup.round1_timeout = Duration::from_millis(400);
    case.run(&setup, &Astra::ok()).unwrap();
    assert_eq!(case.member_status("fable"), "timeout");

    let case = Case::new();
    let mut setup = case.setup("ok", "argon-tokens");
    setup.argon_token_cap = 1000;
    case.run(&setup, &Astra::ok()).unwrap();
    assert_eq!(case.member_status("argon"), "budget_cut");
    assert!(case.calls_of("argon", 2).is_empty());
    assert_eq!(case.stored()["cost"]["cut"], json!(["argon:round2"]));
}

#[test]
fn evidence_decides_what_is_disputed() {
    // Both other members dissent with evidence: the author stands alone.
    let case = Case::new();
    let astra = Astra::ok();
    let digest = case
        .run(&case.setup("disagree", "disagree"), &astra)
        .unwrap();
    let all = claims(&case);
    let authored_by = |c: &Value, m: &str| stance(c, m) == "author";
    for claim in all.iter().filter(|c| authored_by(c, "astra")) {
        assert_eq!(claim["status"], "disputed", "{claim}");
        assert_eq!(claim["dissent"].as_array().unwrap().len(), 2);
    }
    for claim in all.iter().filter(|c| authored_by(c, "fable")) {
        assert_eq!(stance(claim, "astra"), "agree");
        assert_eq!(stance(claim, "argon"), "disagree");
        assert_eq!(claim["status"], "majority", "{claim}");
    }
    assert_eq!(digest["counts"]["disputed"], 2);
    assert_eq!(digest["counts"]["majority"], 4);
    assert_eq!(digest["counts"]["consensus"], 0);
    assert_eq!(digest["stage_note"], "0 of 6 claims agreed, 6 disputed");

    // A bare "no" is not dissent, but neither is it support.
    let case = Case::new();
    case.run(&case.setup("disagree-bare", "disagree-bare"), &Astra::ok())
        .unwrap();
    for claim in claims(&case) {
        if stance(&claim, "astra") == "author" {
            assert_eq!(stance(&claim, "fable"), "unsupported");
            assert_eq!(claim["status"], "disputed", "{claim}");
            assert!(claim["dissent"].as_array().unwrap().is_empty());
        }
    }
}

#[test]
fn a_member_cannot_vote_on_its_own_claim_or_on_nothing() {
    let case = Case::new();
    case.run(&case.setup("ok", "ok"), &Astra::ok()).unwrap();
    for claim in claims(&case) {
        let authors = ["astra", "fable", "argon"]
            .iter()
            .filter(|m| claim["stances"][**m] == serde_json::to_value(Stance::Author).unwrap())
            .count();
        assert_eq!(authors, 1, "{claim}");
    }
}

#[test]
fn the_digest_is_bounded_and_holds_no_raw_position() {
    let case = Case::new();
    let digest = case
        .run(&case.setup("huge,disagree", "huge,disagree"), &Astra::ok())
        .unwrap();
    assert!(
        digest.to_string().chars().count() <= DIGEST_MAX_CHARS + 512,
        "{}",
        digest.to_string().len()
    );
    assert!(!digest.to_string().contains(&"x".repeat(250)));
    assert!(digest["counts"]["dropped_at_intake"].as_u64().unwrap() >= 8);
    assert!(digest["omitted_open"].as_u64().unwrap() > 0, "{digest}");
    let file = fs::read_to_string(digest["digest_path"].as_str().unwrap()).unwrap();
    assert!(file.chars().count() <= DIGEST_MAX_CHARS);
    assert!(claims(&case).len() <= 22);
    let r2 = &case.calls_of("fable", 2)[0];
    assert!(r2["prompt"].as_str().unwrap().encode_utf16().count() < 30_000);
    let raw =
        fs::read_to_string(case.plan.with_file_name("council").join("r1-fable.json")).unwrap();
    assert!(
        raw.contains(&"x".repeat(4000)),
        "the position itself is kept"
    );
}

#[test]
fn a_finished_council_is_never_paid_for_twice() {
    let case = Case::new();
    let astra = Astra::ok();
    let setup = case.setup("ok", "ok");
    let first = case.run(&setup, &astra).unwrap();
    assert_eq!(first["reused"], false);
    let spawned = case.calls().len();
    let second = case.run(&setup, &astra).unwrap();
    assert_eq!(second["reused"], true);
    assert_eq!(case.calls().len(), spawned);
    assert_eq!(astra.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn rulings_are_written_only_for_open_claims() {
    let case = Case::new();
    case.run(&case.setup("disagree", "ok"), &Astra::ok())
        .unwrap();
    let all = claims(&case);
    let open = all.iter().find(|c| c["status"] != "consensus").unwrap();
    let settled = all.iter().find(|c| c["status"] == "consensus").unwrap();
    let (open_id, settled_id) = (
        open["id"].as_str().unwrap().to_owned(),
        settled["id"].as_str().unwrap().to_owned(),
    );
    let ruling = |id: &str, decision: &str, reason: &str| {
        (id.to_string(), decision.to_string(), reason.to_string())
    };
    let reply = council::rule(
        &case.plan,
        &[ruling(&open_id, "upheld", "the check is real")],
    )
    .unwrap();
    assert!(!reply["unruled"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == open_id.as_str()));
    let stored = claims(&case);
    let written = stored.iter().find(|c| c["id"] == open_id.as_str()).unwrap();
    assert_eq!(written["ruling"]["decision"], "upheld");
    valid_council(&case);
    assert!(council::rule(&case.plan, &[ruling(&settled_id, "upheld", "x")]).is_err());
    assert!(council::rule(&case.plan, &[ruling("C99", "upheld", "x")]).is_err());
    assert!(council::rule(&case.plan, &[ruling(&open_id, "maybe", "x")]).is_err());
    assert!(council::rule(&case.plan, &[ruling(&open_id, "upheld", " ")]).is_err());
}

#[test]
fn runs_that_report_only_a_result_add_up_and_trip_the_cap() {
    let case = Case::new();
    // Each round costs $3 and reports it only in its closing result: $6 against the $5 cap.
    case.run(&case.setup("result-only", "ok"), &Astra::ok())
        .unwrap();
    assert_eq!(case.calls_of("fable", 2).len(), 1, "round 2 ran under the cap");
    let usd = case.stored()["cost"]["usd"].as_f64().unwrap();
    assert!((usd - 6.0).abs() < 0.01, "two $3 runs must add up: {usd}");
    // A retry now finds the budget spent.
    fs::remove_file(case.plan.with_file_name("council.json")).unwrap();
    let before = case.calls().len();
    case.run(&case.setup("result-only", "ok"), &Astra::ok())
        .unwrap();
    assert!(case.calls()[before..].iter().all(|c| c["role"] != "fable"));
}

#[test]
fn a_retry_with_the_budget_spent_starts_no_fable_process() {
    let case = Case::new();
    case.run(&case.setup("r1-over-cap", "ok"), &Astra::ok())
        .unwrap();
    fs::remove_file(case.plan.with_file_name("council.json")).unwrap();
    let before = case.calls().len();
    case.run(&case.setup("ok", "ok"), &Astra::ok()).unwrap();
    assert!(
        case.calls()[before..].iter().all(|c| c["role"] != "fable"),
        "no paid request may start once the budget is spent"
    );
    assert_eq!(case.member_status("fable"), "budget_cut");
}

#[test]
fn every_open_claim_can_be_ruled_through_successive_replies() {
    let case = Case::new();
    let digest = case
        .run(&case.setup("huge,disagree", "huge,disagree"), &Astra::ok())
        .unwrap();
    let open_total = claims(&case)
        .iter()
        .filter(|c| c["status"] != "consensus")
        .count();
    assert!(open_total > 16, "{open_total}");
    assert_eq!(digest["more_open"], true, "{digest}");
    let mut batch: Vec<Value> = digest["open"].as_array().unwrap().clone();
    let mut ruled = 0;
    while !batch.is_empty() {
        let rulings: Vec<_> = batch
            .iter()
            .map(|e| (e["id"].as_str().unwrap().to_owned(), "upheld".to_owned(), "checked".to_owned()))
            .collect();
        ruled += rulings.len();
        let reply = council::rule(&case.plan, &rulings).unwrap();
        assert!(reply.to_string().chars().count() <= DIGEST_MAX_CHARS, "reply is bounded");
        batch = reply["next"].as_array().expect("rule carries the next batch").clone();
        assert!(batch.len() <= 16);
        assert_eq!(reply["more_open"], !batch.is_empty() && reply["unruled_total"].as_u64().unwrap() > batch.len() as u64);
    }
    assert_eq!(ruled, open_total);
    assert!(claims(&case)
        .iter()
        .filter(|c| c["status"] != "consensus")
        .all(|c| c["ruling"].is_object()));
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
        digest.get("rounds").is_none(),
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
    assert_eq!(stored["cost"]["cut"], json!(["astra:round2"]));

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
