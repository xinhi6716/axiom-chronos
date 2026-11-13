mod common;

use axiom::{
    bridge::server::{call, Bridge, Config},
    goal, hook, orch,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

fn member_setup() {
    static MEMBER: OnceLock<tempfile::TempDir> = OnceLock::new();
    MEMBER.get_or_init(|| {
        let dir = common::temp();
        let script = dir.path().join("member.json");
        fs::write(&script, "[]").unwrap();
        let mut cmd = common::fake("fake_member");
        cmd.push(format!("--fake-script={}", script.display()));
        std::env::set_var("AXIOM_CLAUDE_CMD", serde_json::to_string(&cmd).unwrap());
        dir
    });
}

struct Fixture {
    temp: tempfile::TempDir,
    repo: PathBuf,
    store: PathBuf,
    brief: PathBuf,
    log: PathBuf,
    bridge: Bridge,
}
impl Fixture {
    fn new() -> Self {
        member_setup();
        let temp = common::temp();
        let repo = temp.path().join("repo");
        let store = orch::store_for(temp.path(), "demo").unwrap();
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::create_dir_all(store.join("inbox")).unwrap();
        fs::write(repo.join("src/mod.py"), "fixture\n").unwrap();
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
                .current_dir(&repo)
                .status()
                .unwrap()
                .success());
        }
        fs::write(
            store.join("modules.md"),
            "## m\nPURPOSE x\nPATHS src/mod.py\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
        )
        .unwrap();
        let brief = store.join("u1.brief");
        fs::write(&brief, "GOAL build\nRISK trivial\nSCOPE src/mod.py\nVERIFY echo ok\n").unwrap();
        fs::write(
            store.join("units.tsv"),
            format!(
                "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu1\tm\tpending\t\t\t\t{}\n",
                brief.display()
            ),
        )
        .unwrap();
        let script = temp.path().join("codex.json");
        let log = temp.path().join("codex.jsonl");
        fs::write(&script, json!([
            {"match":"AXIOM-ENGINE adjudicator needs_check", "reply":{"decision":"pass","reason":"capture confirms the check", "evidence":["E1"]}},
            {"match":"AXIOM-ENGINE verifier", "reply":{"findings":[{"finding":1,"status":"rejected","reason":"the contract is already satisfied"}]}}
        ]).to_string()).unwrap();
        let mut config = Config::from_env();
        config.data_home = Some(temp.path().into());
        let mut cmd = common::fake("fake_codex");
        cmd.push(format!("--fake-script={}", script.display()));
        cmd.push(format!("--fake-log={}", log.display()));
        config.codex = Ok(cmd);
        config.wsl = None;
        config.on_windows = false;
        let bridge = Bridge::new(config);
        Self {
            temp,
            repo,
            store,
            brief,
            log,
            bridge,
        }
    }
    fn invoke(&self, tool: &str, args: Value) -> Value {
        call(&self.bridge, tool, &args)["structuredContent"].clone()
    }
    fn orch(&self, args: Value) -> Value {
        self.invoke("orch", json!({"project":"demo","args":args}))
    }
    fn open_goal(&self) {
        let mut g = goal::open(
            self.temp.path(),
            "s",
            "build",
            Some(self.repo.to_str().unwrap()),
        )
        .unwrap();
        g.project = Some("demo".into());
        g.wish_id = None;
        fs::write(
            self.temp.path().join("goals/s.json"),
            serde_json::to_vec(&g).unwrap(),
        )
        .unwrap();
    }
    fn build_and_review(&self, verdict: &str, finding: Value) {
        let result = self.invoke(
            "codex_run",
            json!({"project":"demo","unit":"u1","role":"terra","cwd":self.repo,"cycle":true}),
        );
        assert_eq!(result["next"], "review", "cycle: {result}");
        let verify = orch::evidence(&self.store, "u1")
            .unwrap()
            .into_iter()
            .rev()
            .find(|e| e.kind == "verify")
            .unwrap();
        let file = self.temp.path().join("verdict.json");
        fs::write(
            &file,
            json!({"unit":"u1","tree":verify.commit,"verdict":verdict,"findings":[finding]})
                .to_string(),
        )
        .unwrap();
        let review = self.orch(json!([
            "review",
            "u1",
            "--repo",
            self.repo,
            "--reviewer",
            "chronos:reviewer",
            "--verdict-file",
            file
        ]));
        assert_eq!(review["status"], "completed", "review: {review}");
    }
    fn events(&self) -> Vec<Value> {
        lines(&self.store.join("events.jsonl"))
    }
    fn turn(&self, text: &str, effort: &str) -> Vec<Value> {
        let log = lines(&self.log);
        let turns: Vec<_> = log
            .iter()
            .filter(|v| {
                v["method"] == "turn/start" && v["params"]["input"].to_string().contains(text)
            })
            .collect();
        assert_eq!(turns.len(), 1, "turns: {log:?}");
        let turn = turns[0];
        assert_eq!(turn["params"]["effort"], effort);
        log
    }
}
fn lines(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn of_kind<'a>(events: &'a [Value], kind: &str) -> Vec<&'a Value> {
    events.iter().filter(|e| e["kind"] == kind).collect()
}

#[test]
fn needs_manual_wish_finishes_through_adjudicator() {
    let f = Fixture::new();
    let policy = f.orch(json!(["policy", "set", "adjudicator", "on"]));
    let capture = f.temp.path().join("capture.txt");
    fs::write(&capture, "E1: checked manually\n").unwrap();
    f.build_and_review(
        "pass",
        json!({"severity":"major","check":"untested","needs":"manual","evidence":capture}),
    );
    f.open_goal();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let full = f.orch(json!([
        "fulltest",
        "--repo",
        f.repo,
        "--command",
        "echo ok"
    ]));
    assert_eq!(full["status"], "completed", "fulltest: {full}");
    let result = f.invoke("goal", json!({"session":"s","status":"done"}));
    assert_eq!(result["goal"]["status"], "done", "goal: {result}");
    assert_eq!(
        policy["status"], "completed",
        "adjudicator policy: {policy}"
    );
    let decisions = fs::read_to_string(f.store.join("decisions.md")).unwrap_or_default();
    assert!(decisions
        .lines()
        .any(|line| line.contains("adjudicator") && line.contains("u1")));
    let events = f.events();
    let adjudication = of_kind(&events, "adjudication");
    assert_eq!(adjudication.len(), 1);
    assert_eq!(adjudication[0]["data"]["on_behalf_of_person"], true);
    assert_eq!(adjudication[0]["data"]["decision"], "pass");
    let log = f.turn("AXIOM-ENGINE adjudicator needs_check", "max");
    let turn = log
        .iter()
        .find(|v| {
            v["method"] == "turn/start"
                && v["params"]["input"]
                    .to_string()
                    .contains("AXIOM-ENGINE adjudicator needs_check")
        })
        .unwrap();
    let thread = &turn["params"]["threadId"];
    assert_eq!(
        log.iter()
            .filter(|v| v["method"] == "turn/start" && v["params"]["threadId"] == *thread)
            .count(),
        1
    );
    let start = log
        .iter()
        .rev()
        .find(|v| v["method"] == "thread/start" && v["time"].as_f64() < turn["time"].as_f64())
        .unwrap();
    assert!(
        start["params"].get("developerInstructions").is_none(),
        "{start}"
    );
}

#[test]
fn approval_runs_exactly_once() {
    let f = Fixture::new();
    let marker = f.temp.path().join("marker.txt");
    let program = format!(
        "from pathlib import Path; p=Path({:?}); p.open('a').write('once\\n')",
        marker.to_string_lossy()
    );
    let request = f.invoke("approval", json!({"project":"demo","op":"request","action":{"command":[common::python(),"-c",program],"cwd":f.repo},"reason":"run once"}));
    assert_eq!(request["status"], "requested", "approval: {request}");
    let id = request["id"].as_str().unwrap();
    let digest = request["digest"].as_str().unwrap();
    let denied = f.invoke(
        "approval",
        json!({"project":"demo","op":"approve","id":id,"digest":digest}),
    );
    assert_eq!(denied["error"]["kind"], "person_only", "{denied}");
    let cli = || {
        Command::new(env!("CARGO_BIN_EXE_axiom"))
            .args([
                "approval",
                "approve",
                id,
                "--project",
                "demo",
                "--digest",
                digest,
            ])
            .env("AXIOM_DATA_HOME", f.temp.path())
            .output()
            .unwrap()
    };
    let first = cli();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(fs::read_to_string(&marker).unwrap().lines().count(), 1);
    let _second = cli();
    let _execute = f.invoke("approval", json!({"project":"demo","op":"execute","id":id}));
    assert_eq!(fs::read_to_string(&marker).unwrap().lines().count(), 1);
    assert_eq!(of_kind(&f.events(), "approval_executed").len(), 1);
}

#[test]
fn interjection_goes_sent_received_applied() {
    let f = Fixture::new();
    f.open_goal();
    hook::handle(
        &json!({"hook_event_name":"UserPromptSubmit","session_id":"s","prompt":"Please export the data"}),
        Some(f.temp.path()),
        false,
    );
    let events = f.events();
    let sent = of_kind(&events, "interjection_sent");
    let received = of_kind(&events, "interjection_received");
    assert_eq!(sent.len(), 1, "events: {events:?}");
    assert_eq!(received.len(), 1);
    assert_eq!(sent[0]["data"]["id"], "ij-1");
    assert_eq!(received[0]["data"]["id"], "ij-1");
    let classified = f.invoke("goal", json!({"session":"s","classify":"amend"}));
    assert_eq!(classified["status"], "completed", "{classified}");
    let events = f.events();
    assert_eq!(of_kind(&events, "interjection_classified").len(), 1);
    assert!(of_kind(&events, "interjection_applied").is_empty());
    let applied = f.invoke("goal", json!({"session":"s","interjection":{"id":"ij-1","applied":{"units":["u2"],"scope":["src/export.py"]}}}));
    assert_eq!(applied["status"], "completed", "{applied}");
    let events = f.events();
    let kinds = [
        "interjection_sent",
        "interjection_received",
        "interjection_classified",
        "interjection_applied",
    ];
    let selected: Vec<_> = kinds
        .iter()
        .map(|kind| {
            let found = of_kind(&events, kind);
            assert_eq!(found.len(), 1);
            found[0]
        })
        .collect();
    assert_eq!(selected[3]["data"]["links"]["units"], json!(["u2"]));
    assert_eq!(
        selected[3]["data"]["links"]["scope"],
        json!(["src/export.py"])
    );
    assert!(selected
        .windows(2)
        .all(|pair| pair[0]["seq"].as_u64() < pair[1]["seq"].as_u64()));
}

#[test]
fn rejected_major_finding_spawns_no_fix_unit() {
    let f = Fixture::new();
    f.build_and_review(
        "fail",
        json!({"severity":"major","check":"contract","evidence":"src/mod.py"}),
    );
    let before = fs::read(f.store.join("units.tsv")).unwrap();
    let result = f.orch(json!([
        "unit", "add", "u1-fix", "--track", "m", "--brief", f.brief, "--fixes", "u1"
    ]));
    assert_eq!(result["status"], "failed", "fix: {result}");
    assert_eq!(result["error"]["kind"], "findings_rejected", "{result}");
    assert_eq!(fs::read(f.store.join("units.tsv")).unwrap(), before);
    let events = f.events();
    let verified = of_kind(&events, "finding_verified");
    assert_eq!(verified.len(), 1);
    assert_eq!(verified[0]["data"]["status"], "rejected");
    f.turn("AXIOM-ENGINE verifier", "low");
}
