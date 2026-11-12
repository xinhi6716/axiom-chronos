mod common;
use axiom::{
    base,
    council::{self, CrossReply, CrossRequest, Setup, Status, DIGEST_MAX_CHARS},
};
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    sync::{atomic::AtomicBool, Mutex, OnceLock},
    time::Duration,
};

const VALID: &str =
    "{\"stances\":[{\"claim\":\"C01\",\"stance\":\"agree\",\"evidence\":\"src/a.rs:12\"}]}\n";
struct Case {
    temp: tempfile::TempDir,
    plan: PathBuf,
    log: PathBuf,
}
impl Case {
    fn new() -> Self {
        static HOME: OnceLock<tempfile::TempDir> = OnceLock::new();
        HOME.get_or_init(|| {
            let d = tempfile::tempdir().unwrap();
            std::env::set_var("AXIOM_DATA_HOME", d.path());
            std::env::set_var("AXIOM_HOME", d.path());
            d
        });
        let temp = tempfile::tempdir().unwrap();
        let plan = temp.path().join("plan.json");
        fs::write(&plan, serde_json::to_vec(&json!({"summary":"test","coverage":{"read":[],"not_read":[]},"out_of_scope":[],"changes":[{"id":"a1","area":"design","location":"src/a.rs","evidence":"Astra says use AXIOM_CODEX_MAX_PARALLEL and gpt-6.1-terra; by Argon; I am Fable; gpt-6.1-claude evidence","problem":"slow","change":"Fable proposes AXIOM_CODEX_MAX_PARALLEL with gpt-6.1-terra","verify":"test"}]})).unwrap()).unwrap();
        let log = temp.path().join("calls.jsonl");
        Self { temp, plan, log }
    }
    fn member(&self, scenario: &str) -> base::Res<Vec<String>> {
        let mut v = common::fake("fake_member");
        v.push(format!("--fake-scenario={scenario}"));
        v.push(format!("--fake-log={}", self.log.display()));
        Ok(v)
    }
    fn setup(&self, fable: &str, argon: &str) -> Setup {
        Setup::new(self.member(fable), self.member(argon))
    }
    fn run(&self, setup: &Setup) -> (Value, Vec<Value>) {
        let request = council::Request {
            brief: "AXIOM_CODEX_MAX_PARALLEL gpt-6.1-terra".into(),
            cwd: self.temp.path().into(),
            plan_path: self.plan.clone(),
            schema_path: common::repo_root().join("engine/research/audit.schema.json"),
        };
        let events = Mutex::new(Vec::new());
        let answer = |r: &CrossRequest| {
            let ids = regex::Regex::new(r"(?m)^(C\d+) \[Position").unwrap();
            let stances = ids
                .captures_iter(&r.prompt)
                .map(|c| json!({"claim":&c[1],"stance":"agree","evidence":""}))
                .collect::<Vec<_>>();
            CrossReply {
                status: Status::Ok,
                output: Some(json!({"stances":stances})),
                raw: Some("astra raw Ω\n".into()),
                tokens: 1,
                model: None,
                detail: None,
            }
        };
        let result =
            council::run_with_events(setup, &request, &answer, &AtomicBool::new(false), &|e| {
                events.lock().unwrap().push(e.clone())
            })
            .unwrap();
        (result, events.into_inner().unwrap())
    }
    fn calls(&self) -> Vec<Value> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    }
    fn stored(&self) -> Value {
        serde_json::from_slice(&fs::read(self.plan.with_file_name("council.json")).unwrap())
            .unwrap()
    }
    fn raw(&self, name: &str) -> Vec<u8> {
        fs::read(self.temp.path().join("council").join(name)).unwrap()
    }
}

#[test]
fn argon_repairs_format_once_without_new_turn() {
    for scenario in ["repair-once", "repair-schema", "partial-review"] {
        let c = Case::new();
        let (out, _) = c.run(&c.setup("no-claims", scenario));
        let calls: Vec<_> = c
            .calls()
            .into_iter()
            .filter(|v| v["role"] == "argon")
            .collect();
        assert_eq!(calls.len(), 2);
        let prompt = calls[1]["prompt"].as_str().unwrap();
        assert!(prompt.contains("REPAIR: The previous review failed validation:"));
        assert!(
            prompt.contains(if scenario == "repair-once" {
                "argon: stdout is not JSON"
            } else if scenario == "repair-schema" {
                "$.stances: required"
            } else {
                "missing per-claim stances"
            }),
            "{prompt}"
        );
        assert!(prompt.contains("same review reshaped") && prompt.contains("\"required\""));
        assert!(calls[0]["prompt"]
            .as_str()
            .unwrap()
            .contains("\"stances\":[{\"claim\":\"C1\""));
        let stored = c.stored();
        let rounds = stored["rounds"].as_array().unwrap();
        let review: Vec<_> = rounds.iter().filter(|r| r["phase"] == "B").collect();
        assert_eq!(review.len(), 1);
        assert_eq!(review[0]["turns"].as_array().unwrap().len(), 1);
        if scenario != "partial-review" {
            assert_eq!(out["degraded"], false);
        }
    }
    let c = Case::new();
    c.run(&c.setup("no-claims", "repair-long"));
    let calls = c.calls();
    let repair = calls
        .iter()
        .filter(|v| v["role"] == "argon")
        .nth(1)
        .unwrap()["prompt"]
        .as_str()
        .unwrap();
    assert!(
        repair.contains(&format!("$.{}: additional property", "x".repeat(350))),
        "{repair}"
    );
}

#[test]
fn argon_second_malformed_stops() {
    let c = Case::new();
    let (out, events) = c.run(&c.setup("no-claims", "repair-bad"));
    assert_eq!(c.calls().iter().filter(|v| v["role"] == "argon").count(), 2);
    let stored = c.stored();
    let turn = &stored["rounds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["phase"] == "B")
        .unwrap()["turns"][0];
    assert_eq!(turn["status"], "malformed");
    let detail = turn["detail"].as_str().unwrap();
    assert!(
        detail.contains("stdout is not JSON") && detail.contains("$.stances"),
        "{detail}"
    );
    assert_eq!(out["degraded"], true);
    assert_eq!(events.len(), 1);
}

fn newline(s: &str) -> String {
    if cfg!(windows) {
        s.replace("\n", "\r\n")
    } else {
        s.into()
    }
}

#[test]
fn council_preserves_raw_valid_invalid_and_repaired() {
    for (scenario, first, repaired) in [
        ("raw-valid", VALID, None),
        ("repair-once", "not JSON: 初回\n", Some(VALID)),
        ("repair-schema", "{\"wrong\":true}\n", Some(VALID)),
        (
            "repair-bad",
            "not JSON: 初回\n",
            Some("{\"stances\":false}\n"),
        ),
    ] {
        let c = Case::new();
        c.run(&c.setup("no-claims", scenario));
        assert_eq!(c.raw("B-r1-argon.raw.txt"), newline(first).as_bytes());
        if let Some(r) = repaired {
            assert_eq!(c.raw("B-r1-argon.repair.raw.txt"), newline(r).as_bytes());
        }
        assert!(!c.temp.path().join("council/A-r1-astra.raw.txt").exists());
    }
    let c = Case::new();
    c.run(&c.setup("raw-test,no-claims", "raw-valid"));
    let first = "{\"type\":\"assistant\",\"message\":{\"id\":\"fixed\",\"model\":\"claude-fable-5-2\",\"content\":[],\"usage\":{\"input_tokens\":1000,\"output_tokens\":500}}}\n";
    // Python and serde preserve object order differently; build the exact emitted text explicitly.
    let closing = format!("{{\"type\":\"result\",\"is_error\":false,\"result\":{}}}\n", serde_json::to_string("{\"stances\":[{\"claim\":\"C01\",\"stance\":\"agree\",\"evidence\":\"\"}],\"approved\":true,\"claims\":[]}").unwrap());
    assert_eq!(
        c.raw("A-r1-fable.raw.txt"),
        newline(&(first.to_owned() + &closing)).as_bytes()
    );
    let c = Case::new();
    c.run(&c.setup("ok", "raw-valid"));
    assert_eq!(c.raw("A-r2-astra.raw.txt"), "astra raw Ω\n".as_bytes());
}

#[test]
fn anonymisation_preserves_content_identifiers() {
    let c = Case::new();
    c.run(&c.setup("no-claims", "raw-valid"));
    let calls = c.calls();
    let prompt = calls.iter().find(|v| v["role"] == "argon").unwrap()["prompt"]
        .as_str()
        .unwrap();
    assert!(
        prompt.contains("AXIOM_CODEX_MAX_PARALLEL") && prompt.contains("gpt-6.1-terra"),
        "{prompt}"
    );
    assert!(
        prompt.contains("[member] says") && prompt.contains("[member] proposes"),
        "{prompt}"
    );
    assert!(!prompt.contains("Astra says") && !prompt.contains("Fable proposes"));
    assert!(prompt.contains("by [member]") && prompt.contains("I am [member]"));
    assert!(prompt.contains("gpt-6.1-claude evidence"));
    assert!(prompt.contains("[Position"));
}

#[test]
fn degraded_member_has_digest_reason_and_event() {
    for (kind, expected) in [
        ("missing", "missing"),
        ("quota", "quota"),
        ("timeout", "timeout"),
        ("budget", "budget_cut"),
        ("malformed", "malformed"),
    ] {
        let c = Case::new();
        let mut setup = match kind {
            "missing" => {
                let mut s = c.setup("no-claims", "raw-valid");
                s.fable = Err(base::Failure::new("missing", "not installed"));
                s
            }
            "quota" => c.setup("quota", "raw-valid"),
            "timeout" => {
                let mut s = c.setup("hang", "raw-valid");
                s.round1_timeout = Duration::from_millis(250);
                s
            }
            "budget" => c.setup("no-usage", "raw-valid"),
            _ => c.setup("no-claims", "repair-bad"),
        };
        setup.rounds = 1;
        let (out, events) = c.run(&setup);
        assert_eq!(events.len(), 1, "{kind}: {events:?}");
        let event = &events[0];
        assert_eq!(event["type"], "council");
        assert_eq!(event["subtype"], "member_degraded");
        assert_eq!(event["status"], expected);
        let d = &out["degraded_members"][0];
        for key in ["member", "phase", "round", "status", "reason"] {
            assert_eq!(d[key], event[key], "{kind}: {key}");
        }
        assert!(!d["reason"].as_str().unwrap().is_empty());
        assert!(d["reason"].as_str().unwrap().chars().count() <= 200);
        assert!(out.to_string().chars().count() <= DIGEST_MAX_CHARS);
        let disk: Value = serde_json::from_slice(
            &fs::read(c.plan.with_file_name("council-digest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(disk["degraded_members"], out["degraded_members"]);
        if kind == "missing" {
            assert!(!c.temp.path().join("council/A-r1-fable.raw.txt").exists());
        }
        if kind == "budget" {
            assert!(!c.raw("A-r1-fable.raw.txt").is_empty());
        }
    }
    let c = Case::new();
    let mut setup = c.setup("no-claims", "quota");
    setup.fable = Err(base::Failure::new("missing", "not installed"));
    let (out, events) = c.run(&setup);
    assert_eq!(events.len(), 2);
    assert_eq!(
        events
            .iter()
            .map(|e| e["member"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["fable", "argon"]
    );
    assert_eq!(out["degraded_members"].as_array().unwrap().len(), 2);
}
