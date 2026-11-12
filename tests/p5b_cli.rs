mod common;
use axiom::{
    cli::research_budget,
    codex::extreme::{self, Check, TierStatus},
    research_program::{
        self,
        budget::Budget,
        history::{self, RunSummary},
        PROGRAM_TYPE,
    },
    seams::FakeApproval,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

fn command(home: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_axiom"));
    c.env("AXIOM_HOME", home)
        .env("AXIOM_DATA_HOME", home)
        .stdin(Stdio::null());
    c
}
fn ok(out: &Output) {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
fn json_out(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap()
}
fn module(home: &Path) {
    let dir = home.join("workshop/tiny");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("manifest.json"), json!({"name":"tiny","version":"1","entry":["python","main.py"],"permissions":[],"panel":{"title":"tiny","description":"test"}}).to_string()).unwrap();
    fs::write(dir.join("main.py"), "import sys\nsys.exit(0)\n").unwrap();
}

#[test]
fn workshop_lifecycle() {
    let t = common::temp();
    let home = t.path();
    module(home);
    let out = command(home).args(["workshop", "list"]).output().unwrap();
    ok(&out);
    assert_eq!(json_out(&out)[0]["name"], "tiny");
    let out = command(home)
        .args(["workshop", "run", "tiny"])
        .env("AXIOM_CLAUDE_CMD", "invalid")
        .env("AXIOM_CODEX_CMD", "invalid")
        .output()
        .unwrap();
    ok(&out);
    assert_eq!(json_out(&out)["status"], "running");
    let out = command(home)
        .args(["workshop", "kill", "tiny"])
        .output()
        .unwrap();
    ok(&out);
    assert_eq!(json_out(&out)["killed"], true);
    let out = command(home)
        .args(["workshop", "status", "tiny"])
        .output()
        .unwrap();
    ok(&out);
    assert_eq!(json_out(&out)["killed"], true);
    assert_eq!(
        command(home)
            .args(["workshop", "run", "tiny"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(1)
    );
    let out = command(home)
        .args(["workshop", "revive", "tiny"])
        .output()
        .unwrap();
    ok(&out);
    assert_eq!(json_out(&out)["killed"], false);
    ok(&command(home)
        .args(["workshop", "run", "tiny"])
        .output()
        .unwrap());
    fs::write(
        home.join("workshop/tiny/main.py"),
        "import sys\nsys.exit(7)\n",
    )
    .unwrap();
    assert_eq!(
        command(home)
            .args(["workshop", "run", "tiny"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(1)
    );
}

#[test]
fn extreme_states() {
    let t = common::temp();
    let home = t.path();
    for (action, on) in [("status", false), ("on", true)] {
        let out = command(home).args(["extreme", action]).output().unwrap();
        ok(&out);
        let v = json_out(&out);
        assert_eq!(v["on"], on);
        assert_eq!(v["active"], false);
        assert!(v["last_check"].is_null());
    }
    extreme::record_check(
        home,
        &Check {
            status: TierStatus::Accepted,
            detail: "ok".into(),
            at: "now".into(),
            turn: None,
        },
    )
    .unwrap();
    let out = command(home).args(["extreme", "status"]).output().unwrap();
    ok(&out);
    assert_eq!(json_out(&out)["active"], true);
    let out = command(home).args(["extreme", "off"]).output().unwrap();
    ok(&out);
    assert_eq!(json_out(&out)["active"], false);
    assert_eq!(json_out(&out)["on"], false);
}

#[test]
fn estimates() {
    let t = common::temp();
    let home = t.path();
    let out = command(home)
        .args(["research-budget", "estimate"])
        .output()
        .unwrap();
    ok(&out);
    assert!(String::from_utf8_lossy(&out.stdout).contains("no history for wish_program"));
    history::append(
        home,
        &RunSummary {
            run_id: "run".into(),
            program_type: PROGRAM_TYPE.into(),
            models: research_budget::default_models(),
            per_provider: BTreeMap::new(),
            completed: true,
            at: "now".into(),
        },
    )
    .unwrap();
    let out = command(home)
        .args(["research-budget", "estimate"])
        .output()
        .unwrap();
    ok(&out);
    assert!(String::from_utf8_lossy(&out.stdout).contains("estimate from 1 past runs"));
    let out = command(home)
        .args(["research-budget", "estimate", "--models", "z,a"])
        .output()
        .unwrap();
    ok(&out);
    assert!(String::from_utf8_lossy(&out.stdout).contains("a, z: no estimate"));
}

#[test]
fn budget_raise_and_usage() {
    let t = common::temp();
    let store = t.path().join("run");
    let path = research_program::program_dir(&store).join("budget.json");
    let out = command(t.path())
        .args(["research-budget", "raise", "--store"])
        .arg(&store)
        .args(["--cap", "10"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(!path.exists());
    Budget::open(&path, "run", 5.0).unwrap();
    let before = fs::read(&path).unwrap();
    let out = command(t.path())
        .args(["research-budget", "raise", "--store"])
        .arg(&store)
        .args(["--cap", "10"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("raise denied"));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(
        research_budget::raise(&store, 10.0, &FakeApproval::denying())
            .unwrap()
            .is_none()
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(
        research_budget::raise(&store, 10.0, &FakeApproval::approving())
            .unwrap()
            .unwrap()
            .to_usd,
        10.0
    );
    assert_eq!(Budget::open(&path, "run", 0.0).unwrap().cap_usd(), 10.0);
    for args in [
        vec!["workshop", "unknown"],
        vec!["extreme", "unknown"],
        vec!["research-budget", "unknown"],
        vec!["research-budget", "raise", "--store", "x"],
    ] {
        assert_eq!(
            command(t.path())
                .args(&args)
                .output()
                .unwrap()
                .status
                .code(),
            Some(2),
            "{args:?}"
        );
    }
}

#[test]
fn advanced_wish_prints_estimate() {
    let t = common::temp();
    for advanced in [false, true] {
        let mut c = command(t.path());
        c.arg("wish");
        if advanced {
            c.arg("--advanced-research");
        }
        c.arg("test");
        let out = c
            .env(
                "AXIOM_CLAUDE_CMD",
                serde_json::to_string(&common::fake("fake_claude")).unwrap(),
            )
            .env("FAKE_CLAUDE_RECORD", t.path().join("record.json"))
            .output()
            .unwrap();
        ok(&out);
        assert_eq!(
            String::from_utf8_lossy(&out.stderr).contains("no history for wish_program"),
            advanced
        );
    }
}

#[test]
fn advanced_research_missing_data_home_does_not_block_launch() {
    let t = common::temp();
    let out = command(t.path())
        .args(["research", "--advanced-research", "test"])
        .env_remove("AXIOM_DATA_HOME")
        .env_remove("CLAUDE_PLUGIN_DATA")
        .env(
            "AXIOM_CLAUDE_CMD",
            serde_json::to_string(&common::fake("fake_claude")).unwrap(),
        )
        .env("FAKE_CLAUDE_RECORD", t.path().join("record.json"))
        .output()
        .unwrap();
    ok(&out);
    assert!(String::from_utf8_lossy(&out.stderr)
        .contains("estimate unavailable: AXIOM_DATA_HOME or CLAUDE_PLUGIN_DATA must be set"));
}
