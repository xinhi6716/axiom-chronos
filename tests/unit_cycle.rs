mod common;

use axiom::{
    bridge::{call, Bridge, Config},
    goal, orch,
};
use serde_json::{json, Value};
use std::{fs, path::PathBuf, process::Command};

struct Fixture {
    temp: tempfile::TempDir,
    repo: PathBuf,
    store: PathBuf,
    bridge: Bridge,
}

impl Fixture {
    fn new(units: &[(&str, &str)], accept: &str) -> Self {
        let temp = common::temp();
        let repo = temp.path().join("repo");
        let store = orch::store_for(temp.path(), "demo").unwrap();
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::create_dir_all(store.join("inbox")).unwrap();
        fs::write(repo.join("src/mod.py"), "fixture").unwrap();
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
        fs::write(store.join("modules.md"), format!("## m\nPURPOSE x\nPATHS src/mod.py\nDEPENDS none\nCONTRACT x\nACCEPT test `{accept}`\n")).unwrap();
        let mut rows = String::from("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n");
        for (id, verify) in units {
            let brief = store.join(format!("{id}.brief"));
            fs::write(
                &brief,
                format!("GOAL build\nSCOPE src/mod.py\nVERIFY {verify}\n"),
            )
            .unwrap();
            rows.push_str(&format!("{id}\tm\tpending\t\t\t\t{}\n", brief.display()));
        }
        fs::write(store.join("units.tsv"), rows).unwrap();
        let mut config = Config::from_env();
        config.data_home = Some(temp.path().into());
        config.codex = Ok(common::fake("fake_codex"));
        config.wsl = None;
        config.on_windows = false;
        let bridge = Bridge::new(config);
        Self {
            temp,
            repo,
            store,
            bridge,
        }
    }
    fn cycle(&self, unit: &str) -> Value {
        call(
            &self.bridge,
            "codex_run",
            &json!({"project":"demo","unit":unit,"role":"terra","cwd":self.repo,"cycle":true}),
        )["structuredContent"]
            .clone()
    }
}

#[test]
fn cycle_requires_unit_and_schema() {
    let f = Fixture::new(&[], "echo ok");
    let r = call(
        &f.bridge,
        "codex_run",
        &json!({"role":"astra","cycle":true}),
    );
    assert_eq!(
        r["structuredContent"]["error"]["message"],
        "cycle needs unit mode"
    );
    let tools = axiom::bridge::server::tools();
    let tool = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "codex_run")
        .unwrap();
    assert_eq!(
        tool["inputSchema"]["properties"]["cycle"]["type"],
        "boolean"
    );
}

#[test]
fn success_and_plain_share_bounded_shape() {
    for cycle in [false, true] {
        let f = Fixture::new(&[("one", "echo ok")], "echo accepted");
        let response = call(
            &f.bridge,
            "codex_run",
            &json!({"project":"demo","unit":"one","role":"terra","cwd":f.repo,"cycle":cycle}),
        );
        assert!(serde_json::to_vec(&response).unwrap().len() <= orch::summary::MAX_REPLY_BYTES);
        let r = &response["structuredContent"];
        assert_eq!(r["state"], "built", "{r}");
        for key in [
            "tests",
            "failing",
            "changed",
            "outside_scope_count",
            "report",
            "report_json",
            "usage",
            "slot_wait",
            "slot_wait_ms",
        ] {
            assert!(r.get(key).is_some(), "{key}: {r}");
        }
        for key in ["verify_tail", "file_changes"] {
            assert!(r.get(key).is_none());
        }
        assert!(r["accept"].get("output").is_none());
        assert!(r["accept"]["output_file"].as_str().is_some());
        assert!(PathBuf::from(r["pack"].as_str().unwrap()).is_file());
        if cycle {
            assert_eq!(r["next"], "review");
            assert_eq!(r["review"][0]["unit"], "one");
            assert!(r["paths"]["report"].is_string());
        } else {
            assert!(r.get("next").is_none());
        }
    }
}

#[test]
fn verify_accept_and_pack_failures() {
    let f = Fixture::new(&[("one", "echo failure marker && exit 1")], "echo ok");
    let r = f.cycle("one");
    assert_eq!(r["next"], "fix", "{r}");
    assert!(r["failing"].to_string().contains("failure marker"));
    let f = Fixture::new(&[("one", "echo ok")], "echo failure accept && exit 1");
    let r = f.cycle("one");
    assert_eq!(r["next"], "fix", "{r}");
    assert!(r["accept"]["failing"]
        .to_string()
        .contains("failure accept"));
    let f = Fixture::new(&[("one", "echo ok")], "echo ok");
    fs::create_dir_all(orch::pack_path(&f.store, "one")).unwrap();
    let r = f.cycle("one");
    assert_eq!(r["next"], "review", "{r}");
    assert!(r["pack"].is_null());
    assert!(r["pack_error"].is_string());
}

#[test]
fn pending_resume_and_module_review() {
    let f = Fixture::new(&[("one", "echo ok"), ("two", "echo ok")], "echo accepted");
    let first = f.cycle("one");
    assert_eq!(first["next"], "wait", "{first}");
    assert!(first["reason"]
        .as_str()
        .unwrap()
        .starts_with("units_pending: "));
    let n = orch::evidence(&f.store, "one")
        .unwrap()
        .iter()
        .filter(|r| r.kind == "run")
        .count();
    let resumed = f.cycle("one");
    assert_eq!(resumed["attempt"], first["attempt"]);
    assert_eq!(
        orch::evidence(&f.store, "one")
            .unwrap()
            .iter()
            .filter(|r| r.kind == "run")
            .count(),
        n
    );
    let second = f.cycle("two");
    assert_eq!(second["next"], "review", "{second}");
    assert_eq!(second["review"].as_array().unwrap().len(), 2, "{second}");
}

#[test]
fn lifecycle_resume_claim_and_changed_tree() {
    let f = Fixture::new(&[("one", "echo ok")], "echo ok");
    fs::write(
        f.store.join("one.brief"),
        "GOAL build\nSCOPE src/\nVERIFY echo ok\n",
    )
    .unwrap();
    let orch::Begin::Run { attempt_id, .. } =
        orch::begin_unit(&f.store, "one", "terra", &f.repo, false).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        orch::resume_built(&f.store, "one", &f.repo)
            .unwrap_err()
            .kind,
        "busy"
    );
    fs::write(f.repo.join("src/extra.py"), "new").unwrap();
    let result = orch::finish_unit(
        &f.store,
        "one",
        &attempt_id,
        &f.repo,
        "terra",
        "model",
        &json!({"status":"completed","file_changes":[{"path":"src/mod.py"}]}),
    )
    .unwrap();
    assert_eq!(result.changed, vec!["src/extra.py", "src/mod.py"]);
    assert!(orch::resume_built(&f.store, "one", &f.repo).unwrap());
    let resumed = f.cycle("one");
    assert_eq!(
        resumed["changed"]["sample"],
        json!(["src/extra.py", "src/mod.py"])
    );
    fs::write(f.repo.join("src/mod.py"), "changed").unwrap();
    assert!(!orch::resume_built(&f.store, "one", &f.repo).unwrap());
    let r = f.cycle("one");
    assert_eq!(r["attempt"], 2, "{r}");
}

#[test]
fn verify_file_contains_whole_capture() {
    let f = Fixture::new(
        &[(
            "one",
            &format!("echo FIRST_MARKER && echo \"test result: ok. 7 passed; 0 failed; 0 ignored;\" && {} -c \"print('x'*30000)\" && echo LAST_MARKER", common::python()),
        )],
        "echo ok",
    );
    let response = call(
        &f.bridge,
        "codex_run",
        &json!({"project":"demo","unit":"one","role":"terra","cwd":f.repo,"cycle":true}),
    );
    assert!(serde_json::to_vec(&response).unwrap().len() <= orch::summary::MAX_REPLY_BYTES);
    let r = &response["structuredContent"];
    assert_eq!(r["tests"]["passed"], 7);
    let output = fs::read_to_string(r["verify_output"].as_str().unwrap()).unwrap();
    assert!(output.contains("FIRST_MARKER"));
    assert!(output.contains("LAST_MARKER"));
}

#[test]
fn reused_and_carry_forward_stay_verified() {
    let f = Fixture::new(&[("one", "echo ok")], "echo accepted");
    let first = f.cycle("one");
    orch::record_review(&f.store, &["one".into()], "claude-review", true, &f.repo).unwrap();
    let rows = orch::evidence(&f.store, "one").unwrap();
    let evidence_file = fs::read(f.store.join("reports/one.evidence.tsv")).unwrap();
    let changed_file = fs::read(f.store.join("reports/one.changed.json")).unwrap();
    let reused = f.cycle("one");
    assert_eq!(reused["status"], "refused");
    assert_eq!(reused["error"]["kind"], "already_verified");
    assert_eq!(reused["state"], "verified", "{reused}");
    assert_eq!(reused["next"], "done");
    assert_eq!(reused["owner"]["attempt"], first["attempt"].to_string());
    assert_eq!(orch::evidence(&f.store, "one").unwrap().len(), rows.len());
    assert_eq!(fs::read(f.store.join("reports/one.evidence.tsv")).unwrap(), evidence_file);
    assert_eq!(fs::read(f.store.join("reports/one.changed.json")).unwrap(), changed_file);
    orch::record_accept(&f.store, "m", &f.repo).unwrap();
    let carried = f.cycle("one");
    assert_eq!(carried["next"], "done", "{carried}");
}

#[test]
fn accept_lists_other_review_units_when_trigger_review_is_carried() {
    let f = Fixture::new(&[("one", "echo ok")], "echo accepted");
    assert_eq!(f.cycle("one")["next"], "review");
    orch::record_review(&f.store, &["one".into()], "claude-review", true, &f.repo).unwrap();
    let verify = orch::evidence(&f.store, "one")
        .unwrap()
        .into_iter()
        .rev()
        .find(|r| r.kind == "verify")
        .unwrap();
    orch::append_evidence(&f.store, "one", &verify).unwrap();
    let brief = f.store.join("two.brief");
    fs::write(&brief, "GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\n").unwrap();
    let mut units = fs::read_to_string(f.store.join("units.tsv")).unwrap();
    units.push_str(&format!("two\tm\tpending\t\t\t\t{}\n", brief.display()));
    fs::write(f.store.join("units.tsv"), units).unwrap();
    let orch::Begin::Run { attempt_id, .. } =
        orch::begin_unit(&f.store, "two", "terra", &f.repo, false).unwrap()
    else {
        panic!()
    };
    orch::finish_unit(
        &f.store,
        "two",
        &attempt_id,
        &f.repo,
        "terra",
        "gpt-6-terra",
        &json!({"status":"completed"}),
    )
    .unwrap();
    let r = f.cycle("one");
    assert_eq!(r["accept"]["ran"], true, "{r}");
    assert_eq!(r["next"], "done", "{r}");
    assert_eq!(r["review"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(r["review"][0]["unit"], "two");
}

#[test]
fn resume_rebuilds_missing_pack_and_preserves_changes() {
    let f = Fixture::new(&[("one", "echo ok"), ("two", "echo ok")], "echo accepted");
    let first = f.cycle("one");
    fs::remove_file(first["pack"].as_str().unwrap()).unwrap();
    fs::remove_file(f.store.join("reports/one.changed.json")).unwrap();
    let r = f.cycle("one");
    assert!(PathBuf::from(r["pack"].as_str().unwrap()).is_file());
    assert_eq!(r["changed"], first["changed"]);
    assert_eq!(r["attempt"], first["attempt"]);
}

#[test]
fn resume_rebuilds_pack_older_than_verify() {
    let f = Fixture::new(&[("one", "echo ok"), ("two", "echo ok")], "echo accepted");
    let first = f.cycle("one");
    let pack = PathBuf::from(first["pack"].as_str().unwrap());
    fs::write(&pack, "obsolete pack").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    fs::write(first["verify_output"].as_str().unwrap(), "ok\n").unwrap();
    let resumed = f.cycle("one");
    assert_eq!(resumed["attempt"], first["attempt"]);
    assert!(fs::read_to_string(pack)
        .unwrap()
        .contains("# Review pack for unit one"));
}

#[test]
fn accept_error_keeps_next_from_recorded_evidence() {
    let f = Fixture::new(&[("one", "echo ok"), ("two", "echo ok")], "echo ok");
    for unit in ["one", "two"] {
        let orch::Begin::Run { attempt_id, .. } =
            orch::begin_unit(&f.store, unit, "terra", &f.repo, false).unwrap()
        else {
            panic!()
        };
        orch::finish_unit(
            &f.store,
            unit,
            &attempt_id,
            &f.repo,
            "terra",
            "model",
            &json!({"status":"completed"}),
        )
        .unwrap();
    }
    let script = f.temp.path().join("accept.py");
    let second = f.store.join("reports/two.evidence.tsv");
    fs::write(
        &script,
        format!(
            "import pathlib\np = pathlib.Path({:?})\np.unlink()\np.mkdir()\nraise SystemExit(1)\n",
            second.to_string_lossy()
        ),
    )
    .unwrap();
    fs::write(f.store.join("modules.md"), format!(
        "## m\nPURPOSE x\nPATHS src/mod.py\nDEPENDS none\nCONTRACT x\nACCEPT test `{} \"{}\"`\n",
        common::python(),
        script.display()
    )).unwrap();
    let result = f.cycle("one");
    assert_eq!(result["accept"]["reason"], "error", "{result}");
    assert_eq!(
        orch::summary::next_for(&f.store, "one").unwrap(),
        orch::summary::Next::Fix
    );
    assert_eq!(result["next"], "fix", "{result}");
}

#[test]
fn cycle_claim_conflicts_match_plain_errors() {
    let f = Fixture::new(&[("one", "echo ok"), ("two", "echo ok")], "echo ok");
    let orch::Begin::Run { attempt_id, .. } =
        orch::begin_unit(&f.store, "one", "terra", &f.repo, false).unwrap()
    else {
        panic!()
    };
    let r = f.cycle("two");
    assert_eq!(r["error"]["kind"], "busy", "{r}");
    orch::release_claim(&f.store, "one", &attempt_id).unwrap();
    fs::write(
        f.store.join("claims.tsv"),
        "unit\tattempt\tscope\tstarted\none\tunknown\tsrc/mod.py\ttime\n",
    )
    .unwrap();
    assert_eq!(f.cycle("two")["error"]["kind"], "busy");
    assert_eq!(f.cycle("one")["error"]["kind"], "outcome_unknown");
}

#[test]
fn cycle_preserves_pre_turn_gates() {
    let f = Fixture::new(&[("one", "echo ok")], "echo ok");
    let sol = call(
        &f.bridge,
        "codex_run",
        &json!({"project":"demo","unit":"one","role":"sol","cwd":f.repo,"cycle":true}),
    );
    assert_eq!(sol["structuredContent"]["error"]["kind"], "sol_not_allowed");
    let rows = fs::read_to_string(f.store.join("units.tsv")).unwrap();
    fs::write(
        f.store.join("units.tsv"),
        rows.replace("one\tm\tpending", "one\tunknown\tpending"),
    )
    .unwrap();
    assert_eq!(f.cycle("one")["error"]["kind"], "unknown_track");
    fs::write(
        f.store.join("units.tsv"),
        rows.replace("one\tm\tpending", "one\tm\tsuperseded:other"),
    )
    .unwrap();
    assert_eq!(f.cycle("one")["error"]["kind"], "superseded");
    fs::write(f.store.join("units.tsv"), &rows).unwrap();
    fs::write(f.store.join("modules.md"), "## dep\nPURPOSE x\nPATHS other/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## m\nPURPOSE x\nPATHS src/mod.py\nDEPENDS dep\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
    assert_eq!(f.cycle("one")["error"]["kind"], "not_ready");
    fs::write(
        f.store.join("modules.md"),
        "## m\nPURPOSE x\nPATHS src/mod.py\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
    )
    .unwrap();
    let mut g = goal::open(
        f.temp.path(),
        "session",
        "[astra] build",
        Some(f.repo.to_str().unwrap()),
    )
    .unwrap();
    g.project = Some("demo".into());
    g.wish_id = None;
    fs::write(
        f.temp.path().join("goals/session.json"),
        serde_json::to_vec(&g).unwrap(),
    )
    .unwrap();
    assert_eq!(f.cycle("one")["error"]["kind"], "astra_gate");
}

#[test]
fn cycle_fix_limit_and_goal_hold() {
    let f = Fixture::new(&[("one", "exit 1")], "echo ok");
    assert_eq!(f.cycle("one")["next"], "fix");
    assert_eq!(f.cycle("one")["next"], "fix");
    assert_eq!(f.cycle("one")["error"]["kind"], "fix_limit");
    let f = Fixture::new(&[("one", "echo ok")], "echo ok");
    orch::record_stage(&f.store, "astra", "gpt-6-astra", true).unwrap();
    fs::create_dir_all(f.store.join("astra")).unwrap();
    for name in ["plan.json", "review.json", "ruling.md", "plan-final.md"] {
        fs::write(f.store.join("astra").join(name), "ok").unwrap();
    }
    let mut g = goal::open(
        f.temp.path(),
        "session",
        "build",
        Some(f.repo.to_str().unwrap()),
    )
    .unwrap();
    g.project = Some("demo".into());
    g.wish_id = None;
    fs::write(
        f.temp.path().join("goals/session.json"),
        serde_json::to_vec(&g).unwrap(),
    )
    .unwrap();
    goal::interject(f.temp.path(), "session", "stop").unwrap();
    let r = f.cycle("one");
    assert_eq!(r["next"], "wait", "{r}");
    assert_eq!(r["reason"], "amend_pending:");
}

#[test]
fn cycle_reports_outside_scope() {
    let f = Fixture::new(&[("one", "echo ok")], "echo ok");
    fs::write(
        f.store.join("one.brief"),
        "GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\nCONTEXT\nFAKE_WRITE outside.txt\n",
    )
    .unwrap();
    let r = f.cycle("one");
    assert_eq!(r["error"]["kind"], "outside_scope", "{r}");
    assert_eq!(r["status"], "failed");
    assert_eq!(r["state"], "failed");
    assert_eq!(r["next"], "fix");
    assert_eq!(r["outside_scope_count"], 1);
    assert_eq!(r["outside_scope"], json!(["outside.txt"]));
    assert!(f.repo.join("outside.txt").is_file());
    assert!(r["verify_exit"].is_null());
    assert_eq!(r["accept"]["ran"], false);
}
