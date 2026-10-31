#[path = "../common/mod.rs"]
mod common;
use axiom::{
    bridge::{call, Bridge, Config},
    orch,
};
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command, sync::Arc};

fn bridge(home: &Path) -> Bridge {
    Bridge::new(Config {
        data_home: Some(home.into()),
        codex: Ok(common::fake("fake_codex")),
        agy: Ok(common::fake("fake_agy")),
        wsl: None,
        on_windows: false,
    })
}
fn invoke(b: &Bridge, args: Value) -> Value {
    call(b, "codex_run", &args)["structuredContent"].clone()
}
fn fixture(home: &Path, units: &[(&str, &str)]) -> (std::path::PathBuf, std::path::PathBuf) {
    let repo = home.join("repo");
    let store = orch::store_for(home, "demo").unwrap();
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&store).unwrap();
    fs::create_dir_all(store.join("inbox")).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    fs::write(repo.join("tracked.txt"), "fixture").unwrap();
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(repo.join("src/mod.py"), "fixture").unwrap();
    assert!(Command::new("git")
        .args(["add", "tracked.txt", "src/mod.py"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            "fixture"
        ])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    let mut rows = String::from("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n");
    for (unit, brief) in units {
        let path = store.join(format!("{unit}.brief"));
        fs::write(&path, brief).unwrap();
        rows.push_str(&format!("{unit}\tm\tpending\t\t\t\t{}\n", path.display()));
    }
    fs::write(store.join("units.tsv"), rows).unwrap();
    (repo, store)
}
#[test]
fn writing_unit_obeys_active_goal_intent() {
    let temp = common::temp();
    let home = temp.path();
    let (repo, store) = fixture(
        home,
        &[(
            "intentunit",
            "GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\n",
        )],
    );
    let b = bridge(home);
    let args = json!({"project":"demo","unit":"intentunit","cwd":repo,"role":"terra"});
    let mut g = axiom::goal::open(home, "intent", "build", Some(repo.to_str().unwrap())).unwrap();
    g.project = Some("demo".into());
    for mode in [None, Some("review"), Some("answer")] {
        g.mode = mode.map(str::to_owned);
        fs::write(
            home.join("goals/intent.json"),
            serde_json::to_vec(&g).unwrap(),
        )
        .unwrap();
        let result = invoke(&b, args.clone());
        assert_eq!(result["error"]["kind"], "goal_mode", "{mode:?}: {result}");
    }
    g.mode = Some("change".into());
    fs::write(
        home.join("goals/intent.json"),
        serde_json::to_vec(&g).unwrap(),
    )
    .unwrap();
    axiom::goal::interject(home, "intent", "wait").unwrap();
    let result = invoke(&b, args.clone());
    assert_eq!(result["error"]["kind"], "pending_interjection", "{result}");
    assert!(fs::read_to_string(store.join("units.tsv"))
        .unwrap()
        .contains("\tpending\t"));
    let readonly = invoke(
        &b,
        json!({"cwd":repo,"role":"terra","sandbox":"read-only","brief":"inspect"}),
    );
    assert_eq!(readonly["status"], "completed", "{readonly}");
    fs::write(
        home.join("goals/intent.json"),
        serde_json::to_vec(&g).unwrap(),
    )
    .unwrap();
    let result = invoke(&b, args);
    assert_eq!(result["state"], "built", "{result}");
}
#[test]
fn unit_build_reuse_and_gates() {
    let temp = common::temp();
    let home = temp.path();
    let (repo, store) = fixture(
        home,
        &[(
            "bridgeunit",
            "GOAL build\nSCOPE src/mod.py\nVERIFY echo bridge-verify\n",
        )],
    );
    let log = home.join("codex.log");
    let b = logged_bridge(home, &log);
    let args = json!({"project":"demo","unit":"bridgeunit","cwd":repo,"role":"terra"});
    let r = invoke(&b, args.clone());
    assert_eq!(r["state"], "built", "{r}");
    assert_eq!(r["verify_exit"], 0);
    assert!(store.join("reports/bridgeunit.md").is_file());
    assert!(store.join("reports/bridgeunit.json").is_file());
    assert!(fs::read_dir(store.join("inbox"))
        .unwrap()
        .any(|e| fs::read_to_string(e.unwrap().path())
            .unwrap()
            .contains("bridgeunit.md")));
    assert!(orch::evidence(&store, "bridgeunit")
        .unwrap()
        .iter()
        .any(|e| e.kind == "verify" && e.command == "echo bridge-verify"));
    orch::record_review(&store, &["bridgeunit".into()], "claude-review", true, &repo).unwrap();
    let evidence = fs::read(store.join("reports/bridgeunit.evidence.tsv")).unwrap();
    let turns = fs::read_to_string(&log)
        .unwrap()
        .matches("turn/start")
        .count();
    let reused = invoke(&b, args);
    assert_eq!(
        fs::read(store.join("reports/bridgeunit.evidence.tsv")).unwrap(),
        evidence
    );
    assert_eq!(
        fs::read_to_string(&log)
            .unwrap()
            .matches("turn/start")
            .count(),
        turns
    );
    assert_eq!(reused["status"], "completed");
    assert_eq!(reused["state"], "verified");
    assert_eq!(reused["unit"], "bridgeunit");
    assert_eq!(reused["reused"], true, "{reused}");
    let sol = invoke(
        &b,
        json!({"project":"demo","unit":"bridgeunit","cwd":repo,"role":"terra","model":"gpt-6-sol"}),
    );
    assert_eq!(sol["error"]["kind"], "invalid_argument");
}
#[test]
fn sol_and_astra_restrictions() {
    let temp = common::temp();
    let home = temp.path();
    let (repo, store) = fixture(
        home,
        &[("gatedunit", "GOAL build\nSCOPE gated.txt\nVERIFY echo ok\n")],
    );
    let b = bridge(home);
    let mut g = axiom::goal::open(
        home,
        "session",
        "[astra] build",
        Some(repo.to_str().unwrap()),
    )
    .unwrap();
    g.project = Some("demo".into());
    g.mode = Some("change".into());
    fs::write(
        home.join("goals/session.json"),
        serde_json::to_vec(&g).unwrap(),
    )
    .unwrap();
    let gated = invoke(
        &b,
        json!({"project":"demo","unit":"gatedunit","cwd":repo,"role":"terra"}),
    );
    assert_eq!(gated["error"]["kind"], "astra_gate");
    let dest = store.join("astra/plan.json");
    let bad = invoke(
        &b,
        json!({"project":"demo","cwd":repo,"role":"terra","brief":"test","save_to":dest,"output_schema":{"type":"object"}}),
    );
    assert_eq!(bad["error"]["kind"], "invalid_argument");
    assert!(!dest.exists());
    let good = invoke(
        &b,
        json!({"project":"demo","cwd":repo,"role":"astra","sandbox":"read-only","model":"gpt-6-astra","brief":"test","save_to":dest,"output_schema":{"type":"object"}}),
    );
    assert_eq!(good["status"], "completed", "{good}");
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(dest).unwrap()).unwrap()["answer"],
        "ok"
    );
}
#[test]
fn independent_units_overlap() {
    let temp = common::temp();
    let home = temp.path();
    let (repo, _) = fixture(
        home,
        &[
            (
                "overlapone",
                "GOAL BARRIER\nSCOPE one.txt\nVERIFY echo ok\n",
            ),
            (
                "overlaptwo",
                "GOAL BARRIER\nSCOPE two.txt\nVERIFY echo ok\n",
            ),
        ],
    );
    let log = home.join("codex.log");
    let b = Arc::new(logged_bridge(home, &log));
    let jobs: Vec<_> = ["overlapone", "overlaptwo"]
        .into_iter()
        .map(|unit| {
            let b = b.clone();
            let repo = repo.clone();
            std::thread::spawn(move || {
                invoke(
                    &b,
                    json!({"project":"demo","unit":unit,"cwd":repo,"role":"terra"}),
                )
            })
        })
        .collect();
    for job in jobs {
        let r = job.join().unwrap();
        assert_eq!(r["state"], "built", "{r}");
    }
    let events: Vec<Value> = fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let starts: Vec<_> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| e["method"] == "turn/start")
        .map(|(i, _)| i)
        .collect();
    let ends: Vec<_> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| e["method"] == "fake/complete")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(starts.len(), 2);
    assert_eq!(ends.len(), 2);
    assert!(starts.iter().max() < ends.iter().min());
    for start in events.iter().filter(|e| e["method"] == "turn/start") {
        let thread = &start["params"]["threadId"];
        assert_eq!(
            events
                .iter()
                .filter(|e| e["method"] == "fake/complete"
                    && e["params"]["threadId"] == *thread
                    && e["params"]["turnId"].is_string())
                .count(),
            1
        );
    }
    let serial = common::temp();
    let (repo, _) = fixture(
        serial.path(),
        &[
            (
                "overlapone",
                "GOAL BARRIER\nSCOPE one.txt\nVERIFY echo ok\n",
            ),
            (
                "overlaptwo",
                "GOAL BARRIER\nSCOPE two.txt\nVERIFY echo ok\n",
            ),
        ],
    );
    let b = bridge(serial.path());
    let first = invoke(
        &b,
        json!({"project":"demo","unit":"overlapone","cwd":repo,"role":"terra"}),
    );
    assert_eq!(first["status"], "failed", "{first}");
    let second = invoke(
        &b,
        json!({"project":"demo","unit":"overlaptwo","cwd":repo,"role":"terra"}),
    );
    assert_eq!(second["status"], "completed", "{second}");
}
#[test]
fn codex_role_and_validation_and_limit() {
    let temp = common::temp();
    let b = bridge(temp.path());
    let ok = invoke(
        &b,
        json!({"cwd":temp.path(),"role":"terra","sandbox":"read-only","brief":"hello"}),
    );
    assert_eq!(ok["model"], "gpt-6.10-terra", "{ok}");
    for args in [
        json!({"cwd":temp.path(),"brief":"hi"}),
        json!({"cwd":"missing","role":"terra","sandbox":"read-only","brief":"hi"}),
        json!({"cwd":temp.path(),"role":"terra"}),
    ] {
        assert_eq!(invoke(&b, args)["error"]["kind"], "invalid_argument");
    }
    let limited = call(
        &b,
        "codex_run",
        &json!({"cwd":temp.path(),"role":"terra","sandbox":"read-only","brief":"LIMIT"}),
    );
    assert_eq!(limited["isError"], true);
    assert_eq!(limited["structuredContent"]["status"], "failed");
}
#[test]
fn astra_rejects_destinations_outside_store() {
    let temp = common::temp();
    let (repo, store) = fixture(temp.path(), &[]);
    let b = bridge(temp.path());
    for dest in [
        temp.path().join("outside.json"),
        Path::new("relative.json").to_path_buf(),
        store.join("../escape.json"),
    ] {
        let r = invoke(
            &b,
            json!({"project":"demo","cwd":repo,"role":"astra","sandbox":"read-only","brief":"test","save_to":dest,"output_schema":{"type":"object"}}),
        );
        assert_eq!(r["error"]["kind"], "invalid_argument", "{r}");
        assert!(!dest.exists());
        assert!(!store.join("reports/_stages.evidence.tsv").exists());
    }
}
#[test]
fn unit_failures_release_claims_and_refuse_invalid_inputs() {
    let temp = common::temp();
    let (repo, _) = fixture(
        temp.path(),
        &[
            (
                "verifyunit",
                "GOAL build\nSCOPE failure-only.txt\nVERIFY exit 1\n",
            ),
            (
                "scopeunit",
                "GOAL TOUCH_OUTSIDE\nSCOPE allowed.txt\nVERIFY echo ok\n",
            ),
            (
                "limitunit",
                "GOAL LIMIT\nSCOPE limit-only.txt\nVERIFY echo ok\n",
            ),
        ],
    );
    let wrapper = temp.path().join("failure_codex.py");
    fs::write(&wrapper, "import sys\np=sys.argv[1]\ns=open(p,encoding='utf8').read().replace(\"'path': 'src/mod.py'\", \"'path': 'failure-only.txt'\")\ns=s.replace(\"turn_id = thread_id + '-turn'\", \"turn_id = thread_id + '-turn'\\n        if 'TOUCH_OUTSIDE' in text:\\n            open(os.path.join(thread_params[thread_id]['cwd'], 'outside.txt'), 'w').write('changed')\")\nexec(compile(s,p,'exec'))\n").unwrap();
    let b = Bridge::new(Config {
        data_home: Some(temp.path().into()),
        codex: Ok(vec![
            common::python(),
            wrapper.to_string_lossy().into_owned(),
            common::repo_root()
                .join("tests/fakes/fake_codex.py")
                .to_string_lossy()
                .into_owned(),
        ]),
        agy: Ok(common::fake("fake_agy")),
        wsl: None,
        on_windows: false,
    });
    for (unit, kind) in [
        ("verifyunit", "verify_failed"),
        ("scopeunit", "outside_scope"),
        ("limitunit", "usage_limit"),
    ] {
        let args = json!({"project":"demo","unit":unit,"cwd":repo,"role":"terra"});
        let response = call(&b, "codex_run", &args);
        let r = &response["structuredContent"];
        assert_eq!(response["isError"], true, "{response}");
        assert_eq!(r["status"], "failed", "{r}");
        if unit != "limitunit" {
            assert_eq!(r["error"]["kind"], kind, "{r}");
        } else {
            assert!(r["error"].is_object(), "{r}");
        }
        if unit == "verifyunit" {
            assert_eq!(r["verify_exit"], 1);
            assert!(r["verify_tail"].is_string());
        }
        if unit == "scopeunit" {
            assert!(!r["outside_scope"].as_array().unwrap().is_empty());
            assert!(repo.join("outside.txt").exists());
        }
        let retry = invoke(&b, args);
        assert_eq!(retry["attempt"], 2, "{retry}");
    }
    for extra in [
        json!({"role":"terra","brief":"x"}),
        json!({"role":"astra"}),
        json!({"model":"gpt-6-terra"}),
        json!({"role":"terra","sandbox":"read-only"}),
    ] {
        let mut args = json!({"project":"demo","unit":"verifyunit","cwd":repo});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert_eq!(invoke(&b, args)["error"]["kind"], "invalid_argument");
    }
}

fn logged_bridge(home: &Path, log: &Path) -> Bridge {
    let wrapper = home.join("logged_codex.py");
    fs::write(&wrapper, "import os,runpy,sys\nos.environ['FAKE_CODEX_LOG']=sys.argv[1]\nsys.argv=sys.argv[2:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n").unwrap();
    Bridge::new(Config {
        data_home: Some(home.into()),
        codex: Ok(vec![
            common::python(),
            wrapper.to_string_lossy().into_owned(),
            log.to_string_lossy().into_owned(),
            common::repo_root()
                .join("tests/fakes/fake_codex.py")
                .to_string_lossy()
                .into_owned(),
        ]),
        agy: Ok(common::fake("fake_agy")),
        wsl: None,
        on_windows: false,
    })
}
#[test]
fn astra_gate_requires_every_prerequisite() {
    for missing in 0..7 {
        let temp = common::temp();
        let home = temp.path();
        let (repo, store) = fixture(
            home,
            &[(
                "gateunit",
                "GOAL build\nSCOPE gate-only.txt\nVERIFY echo ok\n",
            )],
        );
        let mut g =
            axiom::goal::open(home, "s", "[astra] build", Some(repo.to_str().unwrap())).unwrap();
        g.project = Some("demo".into());
        g.mode = Some("change".into());
        fs::write(home.join("goals/s.json"), serde_json::to_vec(&g).unwrap()).unwrap();
        if missing != 0 {
            orch::record_stage(&store, "astra", "gpt-6-astra", true).unwrap();
        }
        if missing != 1 {
            orch::record_stage(&store, "second_review", "review", true).unwrap();
        }
        for (i, name) in ["plan.json", "review.json", "ruling.md", "plan-final.md"]
            .iter()
            .enumerate()
        {
            if missing != i + 2 {
                fs::create_dir_all(store.join("astra")).unwrap();
                fs::write(store.join("astra").join(name), "ok").unwrap();
            }
        }
        let r = invoke(
            &bridge(home),
            json!({"project":"demo","unit":"gateunit","cwd":repo,"role":"terra"}),
        );
        if missing == 6 {
            assert_eq!(r["attempt"], 1, "{r}");
        } else {
            assert_eq!(r["error"]["kind"], "astra_gate", "missing {missing}: {r}");
        }
    }
}
#[test]
fn sol_can_assemble_without_failing_fulltest() {
    let temp = common::temp();
    let (repo, _store) = fixture(
        temp.path(),
        &[("solunit", "GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\n")],
    );
    let b = bridge(temp.path());
    let args =
        json!({"project":"demo","unit":"solunit","cwd":repo,"role":"sol","model":"gpt-6-sol"});
    let r = invoke(&b, args);
    assert_eq!(r["state"], "built", "{r}");
}

fn wait_pids(path: &Path) -> Vec<u32> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(ids) = fs::read_to_string(path)
            .ok()
            .and_then(|s| {
                s.split_whitespace()
                    .map(str::parse::<u32>)
                    .collect::<Result<Vec<_>, _>>()
                    .ok()
            })
            .filter(|ids| ids.len() == 2)
        {
            return ids;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "verifier never started: {}",
            path.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn cancellation_reaps_running_verify_accept_and_fulltest_without_cancelling_other_bridge() {
    for mode in ["verify", "accept", "fulltest"] {
        let temp = common::temp();
        let home = temp.path();
        let script = home.join("verifier.py");
        fs::write(&script, "import os,sys,time,subprocess\nfrom pathlib import Path\np=subprocess.Popen([sys.executable,'-c','import time;time.sleep(60)'])\nPath(sys.argv[1]).write_text(f'{os.getpid()} {p.pid}')\nwhile not Path(sys.argv[2]).exists(): time.sleep(.01)\np.terminate()\np.wait()\n").unwrap();
        let pids = home.join("pids");
        let release = home.join("release");
        let cmd = format!(
            "\"{}\" \"{}\" \"{}\" \"{}\"",
            common::python(),
            script.display(),
            pids.display(),
            release.display()
        );
        let (repo, store) = fixture(
            home,
            &[(
                "cancelunit",
                &format!("GOAL build\nSCOPE src/mod.py\nVERIFY {cmd}\n"),
            )],
        );
        fs::write(store.join("modules.md"), format!("## m\nPURPOSE x\nPATHS src/mod.py\nDEPENDS none\nCONTRACT x\nACCEPT test `{cmd}`\nFULL TEST `{cmd}`\n")).unwrap();
        let b = Arc::new(bridge(home));
        let other_home = home.join("other");
        fs::create_dir(&other_home).unwrap();
        fs::create_dir_all(orch::store_for(&other_home, "demo").unwrap()).unwrap();
        let other = Arc::new(bridge(&other_home));
        let other_pids = home.join("other-pids");
        let other_release = home.join("other-release");
        let other_cmd = format!(
            "\"{}\" \"{}\" \"{}\" \"{}\"",
            common::python(),
            script.display(),
            other_pids.display(),
            other_release.display()
        );
        let other_job = {
            let b = other.clone();
            let repo = repo.clone();
            std::thread::spawn(move || {
                call(
                    &b,
                    "orch",
                    &json!({"project":"demo","args":["fulltest","--repo",repo,"--command",other_cmd]}),
                )
            })
        };
        let other_ids = wait_pids(&other_pids);
        let job = {
            let b = b.clone();
            let repo = repo.clone();
            std::thread::spawn(move || {
                if mode == "verify" {
                    invoke(
                        &b,
                        json!({"project":"demo","unit":"cancelunit","cwd":repo,"role":"terra"}),
                    )
                } else {
                    let args = if mode == "accept" {
                        json!(["accept", "m", "--repo", repo])
                    } else {
                        json!(["fulltest", "--repo", repo])
                    };
                    call(&b, "orch", &json!({"project":"demo","args":args}))["structuredContent"]
                        .clone()
                }
            })
        };
        let ids = wait_pids(&pids);
        // VERIFY only starts after the worker's completed model report was persisted.
        if mode == "verify" {
            let report: Value =
                serde_json::from_slice(&fs::read(store.join("reports/cancelunit.json")).unwrap())
                    .unwrap();
            assert_eq!(report["status"], "completed");
        }
        b.cancel();
        let result = job.join().unwrap();
        assert_eq!(result["status"], "failed", "{mode}: {result}");
        for pid in ids {
            assert!(
                !axiom::base::proc::pid_alive(pid),
                "{mode}: verifier process {pid} survived cancellation"
            );
        }
        for pid in other_ids {
            assert!(
                axiom::base::proc::pid_alive(pid),
                "other Bridge's verifier was cancelled"
            );
        }
        fs::write(&other_release, "release").unwrap();
        let other_result = other_job.join().unwrap();
        assert_eq!(
            other_result["structuredContent"]["status"], "completed",
            "{other_result}"
        );
        let rows = if mode == "fulltest" {
            fs::read_to_string(store.join("reports/_full.evidence.tsv")).unwrap()
        } else {
            fs::read_to_string(store.join("reports/cancelunit.evidence.tsv")).unwrap()
        };
        assert!(
            rows.lines().any(|line| line.starts_with(&format!(
                "{}\t",
                if mode == "fulltest" { "full" } else { mode }
            )) && line.split('\t').nth(2) != Some("0")),
            "{rows}"
        );
        assert!(orch::read_units(&store)
            .unwrap()
            .iter()
            .all(|u| u.state != "verified"));
        // A late verification cannot cross the cancellation boundary and launch.
        let late = call(
            &b,
            "orch",
            &json!({"project":"demo","args":["fulltest","--repo",repo,"--command","echo late > late-marker"]}),
        );
        assert_eq!(late["structuredContent"]["status"], "failed");
        assert!(!repo.join("late-marker").exists());
    }
}
#[test]
fn astra_save_records_only_astra() {
    let temp = common::temp();
    let (repo, store) = fixture(temp.path(), &[]);
    let b = bridge(temp.path());
    let dest = store.join("astra/plan.json");
    let bad = invoke(
        &b,
        json!({"project":"demo","cwd":repo,"role":"terra","model":"gpt-6-terra","brief":"test","save_to":dest,"output_schema":{"type":"object"}}),
    );
    assert_eq!(bad["error"]["kind"], "invalid_argument");
    assert!(!dest.exists());
    assert!(!store.join("reports/_stages.evidence.tsv").exists());
    let ok = invoke(
        &b,
        json!({"project":"demo","cwd":repo,"role":"astra","sandbox":"read-only","model":"gpt-6-astra","brief":"test","save_to":dest,"output_schema":{"type":"object"}}),
    );
    assert_eq!(ok["status"], "completed", "{ok}");
    assert!(ok.get("structured_output").is_none());
    assert!(ok.get("final_message").is_none());
    assert_eq!(ok["saved_to"], json!(dest));
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(dest).unwrap()).unwrap()["answer"],
        "ok"
    );
    assert!(
        fs::read_to_string(store.join("reports/_stages.evidence.tsv"))
            .unwrap()
            .lines()
            .any(|r| r.starts_with("astra\tgpt-6-astra\t0\t"))
    );
}
#[test]
fn astra_missing_structured_output_records_failure() {
    let temp = common::temp();
    let (repo, store) = fixture(temp.path(), &[]);
    let dest = store.join("astra/missing.json");
    let r = invoke(
        &bridge(temp.path()),
        json!({"project":"demo","cwd":repo,"role":"astra","sandbox":"read-only","brief":"test","save_to":dest}),
    );
    assert_eq!(r["status"], "failed", "{r}");
    assert!(!dest.exists());
    assert!(
        fs::read_to_string(store.join("reports/_stages.evidence.tsv"))
            .unwrap()
            .lines()
            .any(|r| r.starts_with("astra\t") && r.contains("\t1\t"))
    );
}
#[test]
fn astra_preserves_coverage() {
    let temp = common::temp();
    let (repo, store) = fixture(temp.path(), &[]);
    let wrapper = temp.path().join("coverage.py");
    fs::write(&wrapper, "import sys\np=sys.argv[1]\ns=open(p,encoding='utf8').read().replace(\"json.dumps({'answer': 'ok'})\", \"json.dumps({'answer': 'ok', 'coverage': {'units': 3}})\")\nexec(compile(s,p,'exec'))\n").unwrap();
    let c = Config {
        data_home: Some(temp.path().into()),
        codex: Ok(vec![
            common::python(),
            wrapper.to_string_lossy().into_owned(),
            common::repo_root()
                .join("tests/fakes/fake_codex.py")
                .to_string_lossy()
                .into_owned(),
        ]),
        agy: Ok(common::fake("fake_agy")),
        wsl: None,
        on_windows: false,
    };
    let dest = store.join("astra/coverage.json");
    let r = invoke(
        &Bridge::new(c),
        json!({"project":"demo","cwd":repo,"role":"astra","sandbox":"read-only","brief":"test","save_to":dest,"output_schema":{"type":"object"}}),
    );
    assert_eq!(r["coverage"], json!({"units":3}), "{r}");
}

#[test]
fn unbound_writing_run_is_refused_before_codex_starts() {
    let temp = common::temp();
    let (repo, store) = fixture(temp.path(), &[]);
    let marker = temp.path().join("started");
    let wrapper = temp.path().join("marker.py");
    fs::write(&wrapper, "import pathlib,runpy,sys\npathlib.Path(sys.argv[1]).write_text('started')\nsys.argv=sys.argv[2:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n").unwrap();
    let b = Bridge::new(Config {
        data_home: Some(temp.path().into()),
        codex: Ok(vec![
            common::python(),
            wrapper.to_string_lossy().into_owned(),
            marker.to_string_lossy().into_owned(),
            common::repo_root()
                .join("tests/fakes/fake_codex.py")
                .to_string_lossy()
                .into_owned(),
        ]),
        agy: Ok(common::fake("fake_agy")),
        wsl: None,
        on_windows: false,
    });
    for choice in [
        json!({"role":"terra"}),
        json!({"role":"sol","sandbox":"workspace-write"}),
        json!({"model":"gpt-6-terra","sandbox":"workspace-write"}),
    ] {
        let mut args = json!({"cwd":repo,"brief":"hello"});
        args.as_object_mut()
            .unwrap()
            .extend(choice.as_object().unwrap().clone());
        let r = invoke(&b, args);
        assert_eq!(r["error"]["kind"], "unbound_write", "{r}");
        assert!(!marker.exists());
    }
    let r = invoke(
        &b,
        json!({"project":"demo","cwd":repo,"role":"astra","sandbox":"read-only","brief":"hello","save_to":store.join("plan.json"),"output_schema":{"type":"object"}}),
    );
    assert_eq!(r["status"], "completed", "{r}");
    assert!(marker.exists());
}

#[test]
fn creative_goal_requires_architect() {
    let temp = common::temp();
    let (repo, store) = fixture(
        temp.path(),
        &[(
            "creativeunit",
            "GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\n",
        )],
    );
    let mut goal = axiom::goal::open(
        temp.path(),
        "creative",
        "[creative] build",
        Some(repo.to_str().unwrap()),
    )
    .unwrap();
    goal.project = Some("demo".into());
    goal.mode = Some("change".into());
    fs::write(
        temp.path().join("goals/creative.json"),
        serde_json::to_vec(&goal).unwrap(),
    )
    .unwrap();
    let b = bridge(temp.path());
    let args = json!({"project":"demo","unit":"creativeunit","cwd":repo,"role":"terra"});
    assert_eq!(
        invoke(&b, args.clone())["error"]["kind"],
        "architect_required"
    );
    fs::write(store.join("modules.md"), "architecture").unwrap();
    assert_eq!(invoke(&b, args.clone())["error"]["kind"], "astra_gate");
    orch::record_stage(&store, "astra", "gpt-6-astra", true).unwrap();
    orch::record_stage(&store, "second_review", "review", true).unwrap();
    fs::create_dir_all(store.join("astra")).unwrap();
    for name in ["plan.json", "review.json", "ruling.md", "plan-final.md"] {
        fs::write(store.join("astra").join(name), "ok").unwrap();
    }
    let result = invoke(&b, args);
    assert_eq!(result["state"], "built", "{result}");
}

#[test]
fn uncertain_writer_is_quarantined_before_claim_release_across_bridges() {
    for scenario in ["CRASH", "NO_TURN_ID", "REFUSE_INTERRUPT"] {
        let temp = common::temp();
        let (repo, store) = fixture(
            temp.path(),
            &[(
                "uncertain-writer",
                &format!("GOAL {scenario}\nSCOPE src/mod.py\nVERIFY echo ok\n"),
            )],
        );
        let b = bridge(temp.path());
        let args = json!({"project":"demo","unit":"uncertain-writer","role":"terra","cwd":repo,"timeout_s":1});
        let result = invoke(&b, args.clone());
        assert_eq!(result["error"]["kind"], "turn_unconfirmed", "{result}");
        assert!(store.join("quarantine").is_dir());
        assert!(orch::running().contains(&"uncertain-writer".into()));
        assert!(!store.join("reports/uncertain-writer.evidence.tsv").exists());
        let other = bridge(temp.path());
        assert_eq!(invoke(&other, args)["error"]["kind"], "quarantined");
        drop(b); // closes/reaps the test app-server before explicit recovery
        orch::recover_quarantine(&store).unwrap();
        let retry = orch::begin_unit(&store, "uncertain-writer", "terra", &repo, false).unwrap();
        assert!(matches!(retry, orch::Begin::Run { .. }));
        orch::abort_unit(&store, "uncertain-writer");
    }
}

#[test]
fn actual_worker_model_not_requested_model_controls_evidence() {
    let temp = common::temp();
    let (repo, store) = fixture(
        temp.path(),
        &[("mismatch", "GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\n")],
    );
    let wrapper = temp.path().join("mismatch.py");
    fs::write(&wrapper, format!("from pathlib import Path\np={}\ns=Path(p).read_text(encoding='utf-8').replace(\"'actual-model' if params.get('model') == 'requested-model' else params.get('model')\",\"'gpt-6-luna'\")\nexec(compile(s,p,'exec'))\n", serde_json::to_string(&common::repo_root().join("tests/fakes/fake_codex.py").to_string_lossy()).unwrap())).unwrap();
    let mut cfg = Config {
        data_home: Some(temp.path().into()),
        codex: Ok(common::fake("fake_codex")),
        agy: Ok(common::fake("fake_agy")),
        wsl: None,
        on_windows: false,
    };
    cfg.codex = Ok(vec![
        common::python(),
        "-I".into(),
        wrapper.to_string_lossy().into_owned(),
    ]);
    let b = Bridge::new(cfg);
    let result = invoke(
        &b,
        json!({"project":"demo","unit":"mismatch","role":"sol","model":"gpt-6-sol","cwd":repo}),
    );
    assert_eq!(result["error"]["kind"], "thread_policy", "{result}");
    assert!(orch::evidence(&store, "mismatch").unwrap().is_empty());
    assert!(!store.join(".worker.claim").exists());
    assert!(!orch::current_verified_unit(&store, "mismatch", &repo).unwrap());
    assert!(!store.join("quarantine").exists());
}

#[test]
fn missing_actual_model_never_starts_a_turn_or_credits_requested_model() {
    let temp = common::temp();
    let (repo, store) = fixture(
        temp.path(),
        &[(
            "unknown-model",
            "GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\n",
        )],
    );
    let wrapper = temp.path().join("missing_model.py");
    let log = temp.path().join("requests.jsonl");
    fs::write(&wrapper, format!("from pathlib import Path\nimport os\nos.environ['FAKE_CODEX_LOG']={}\np={}\ns=Path(p).read_text(encoding='utf-8').replace(\"'actual-model' if params.get('model') == 'requested-model' else params.get('model')\",\"None\")\nassert s!=Path(p).read_text(encoding='utf-8')\nexec(compile(s,p,'exec'))\n", serde_json::to_string(&log.to_string_lossy()).unwrap(), serde_json::to_string(&common::repo_root().join("tests/fakes/fake_codex.py").to_string_lossy()).unwrap())).unwrap();
    let b = Bridge::new(Config {
        data_home: Some(temp.path().into()),
        codex: Ok(vec![
            common::python(),
            "-I".into(),
            wrapper.to_string_lossy().into_owned(),
        ]),
        agy: Ok(common::fake("fake_agy")),
        wsl: None,
        on_windows: false,
    });
    let result = invoke(
        &b,
        json!({"project":"demo","unit":"unknown-model","role":"terra","cwd":repo}),
    );
    assert_eq!(result["error"]["kind"], "thread_policy", "{result}");
    assert!(result["error"]["message"]
        .as_str()
        .unwrap()
        .contains("turn not started"));
    assert!(orch::evidence(&store, "unknown-model").unwrap().is_empty());
    assert!(!store.join(".worker.claim").exists());
    let events: Vec<Value> = fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert!(events.iter().any(|v| v["method"] == "thread/start"));
    assert!(!events.iter().any(|v| v["method"] == "turn/start"));
}

#[test]
fn astra_high_verifier_is_isolated_and_rejects_writing_or_effort_overrides() {
    let temp = common::temp();
    let (repo, _) = fixture(
        temp.path(),
        &[("u", "GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\n")],
    );
    let log = temp.path().join("verifier.log");
    let b = logged_bridge(temp.path(), &log);
    let base = json!({"cwd":repo,"role":"astra-verify","sandbox":"read-only","task_kind":"verification","brief":"verify tracked.txt"});
    for kind in ["verification", "measurement"] {
        let mut args = base.clone();
        args["task_kind"] = json!(kind);
        let r = invoke(&b, args);
        assert_eq!(r["status"], "completed", "{r}");
        assert_eq!(r["requested_model"], "gpt-6-astra");
        assert_eq!(r["model"], "gpt-6-astra");
        assert_eq!(r["requested_effort"], "high");
        assert!(r["actual_effort"].is_null());
    }
    let events: Vec<Value> = fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    for e in events.iter().filter(|e| e["method"] == "thread/start") {
        assert_eq!(e["params"]["sandbox"], "read-only");
        assert_eq!(e["params"]["approvalPolicy"], "never");
        assert_eq!(e["params"]["config"]["features"]["multi_agent"], false);
        assert_eq!(e["params"]["config"]["features"]["apps"], false);
    }
    assert!(events
        .iter()
        .filter(|e| e["method"] == "turn/start")
        .all(|e| e["params"]["effort"] == "high"));
    let count = events.len();
    for (key, value) in [
        ("sandbox", json!("workspace-write")),
        ("unit", json!("u")),
        ("effort", json!("max")),
        ("task_kind", json!("improvement")),
        ("model", json!("gpt-6-terra")),
        ("lean", json!(false)),
        ("keep_plugins", json!(["unsafe"])),
        ("developer_instructions", json!("write product")),
    ] {
        let mut args = base.clone();
        args[key] = value;
        assert_eq!(
            invoke(&b, args)["error"]["kind"],
            "invalid_argument",
            "{key}"
        );
    }
    let astra =
        json!({"cwd":repo,"role":"astra","sandbox":"read-only","brief":"research","effort":"high"});
    assert_eq!(invoke(&b, astra)["error"]["kind"], "invalid_argument");
    assert_eq!(fs::read_to_string(&log).unwrap().lines().count(), count);
    assert_eq!(
        fs::read_to_string(repo.join("tracked.txt")).unwrap(),
        "fixture"
    );
}

fn pipeline_bridge(home: &Path, hold: u64, fault: &str) -> Bridge {
    Bridge::new(Config {
        data_home: Some(home.into()),
        codex: Ok(vec![
            common::python(),
            "-I".into(),
            common::repo_root()
                .join("tests/bridge/fake_dispatcher.py")
                .to_string_lossy()
                .into_owned(),
            "--scenario".into(),
            (if fault.starts_with("host-callback") {
                "pipeline-dispatcher"
            } else {
                "pipeline"
            })
            .into(),
            "--hold-s".into(),
            hold.to_string(),
            "--fault".into(),
            fault.into(),
        ]),
        agy: Err(axiom::base::Failure::new("disabled", "test")),
        wsl: None,
        on_windows: cfg!(windows),
    })
}
fn pipeline_fixture(home: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let (repo,store)=fixture(home,&[("checkpointed","GOAL Verify a Rust product\nSCOPE src/lib.rs src/mod.py\nVERIFY cargo test --locked --offline\nDELIVERY verification\nOUTPUTS src/lib.rs\n")]);
    let product = home.join("demo");
    fs::rename(&repo, &product).unwrap();
    let repo = product;
    fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname=\"host-fixture\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    )
    .unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n").unwrap();
    fs::write(repo.join(".gitignore"), "/target\n").unwrap();
    assert!(Command::new("cargo")
        .current_dir(&repo)
        .args(["generate-lockfile", "--offline"])
        .status()
        .unwrap()
        .success());
    (repo, store)
}
fn pipeline_checkpoint(store: &Path, repo: &Path, round: u32) {
    pipeline_checkpoint_with_severity(store, repo, round, "normal");
}
fn pipeline_checkpoint_with_severity(store: &Path, repo: &Path, round: u32, severity: &str) {
    orch::checkpoint(store,serde_json::from_value(json!({"task":"checkpointed","module":"m","round":round,"owner":"terra","dependencies":[],"outputs":["src/lib.rs"],"evidence":["tests"],"next_action":"build","issue":"issue","proposal":"proposal","evidence_version":round,"contract":orch::contract_identity(store,"checkpointed").unwrap(),"repo":repo,"severity":severity,"findings":[]})).unwrap()).unwrap();
}
fn seed_execution(b: &Bridge) {
    let (id,_)=axiom::goal::find_active(b.home().unwrap(),"demo").unwrap().unwrap();
    let dir=b.home().unwrap().join("authority").join(id);fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("state.json"),json!({"phase":"execution","revision":1,"research":null,"reason":"trusted downstream fixture"}).to_string()).unwrap();
}
fn pipeline_core_status(b: &Bridge, repo: &Path) -> Value {
    let begin = call(
        b,
        "session",
        &json!({"action":"begin","project":repo,"text":"verify Rust product"}),
    );
    assert_eq!(begin["structuredContent"]["status"], "completed", "{begin}");
    // Downstream pipeline fixture starts after the independently tested review/planning prefix.
    let dir=b.home().unwrap().join("authority").join(begin["structuredContent"]["session"].as_str().unwrap());
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("state.json"),json!({"phase":"execution","revision":1,"research":null,"reason":"trusted downstream fixture"}).to_string()).unwrap();

    let mode = call(
        b,
        "session",
        &json!({"action":"call","tool":"goal","arguments":{"mode":"change"}}),
    );
    assert_eq!(
        mode["structuredContent"]["goal"]["mode"], "change",
        "{mode}"
    );
    let dispatched = call(
        b,
        "session",
        &json!({"action":"call","tool":"Agent","arguments":{"subagent_type":"chronos:dispatcher","project":"demo","cwd":repo,"prompt":"Return bounded host evidence; do not dispatch another writer."}}),
    );
    assert_eq!(
        dispatched["structuredContent"]["status"], "completed",
        "{dispatched}"
    );
    call(b, "session", &json!({"action":"status"}))["structuredContent"].clone()
}
#[test]
fn checkpointed_bridge_lifecycle_separates_unit_release_from_five_product_rounds() {
    let temp = common::temp();
    let home = temp.path();
    let (repo, store) = pipeline_fixture(home);
    let brief = store.join("second.brief");
    fs::copy(store.join("checkpointed.brief"), &brief).unwrap();
    let mut rows = fs::read_to_string(store.join("units.tsv")).unwrap();
    rows.push_str(&format!("second\tm\tpending\t\t\t\t{}\n", brief.display()));
    fs::write(store.join("units.tsv"), rows).unwrap();
    let b = pipeline_bridge(home, 0, "");
    let operator = pipeline_bridge(home, 0, "");
    let product_command = |args: Value, _attached: bool| {
        let arguments = json!({"project":"demo","args":args});
        let response = call(&operator,"orch",&arguments);
        response["structuredContent"].clone()
    };
    let transition = |round: u32, verification: &str, assessment: &str, action: &str| {
        product_command(json!(["wait","product-transition","--run","product","--round",round.to_string(),"--verifications",verification,"--assessment",assessment,"--action",action]), true)
    };
    let finish_product = || {
        // Historical product-controller acceptance uses the unattached operator;
        // attached core additionally requires the new role workflow submit gate.
        let session=call(&b,"session",&json!({"action":"status"}))["structuredContent"]["session"].clone();
        call(&operator,"goal",&json!({"session":session,"status":"done"}))["structuredContent"].clone()
    };
    let checkpoint = |task: &str, round: u32| {
        orch::checkpoint(&store, serde_json::from_value(json!({"task":task,"module":"m","round":round,"owner":"terra","dependencies":[],"outputs":["src/lib.rs"],"evidence":["tests"],"next_action":"build","issue":"issue","proposal":"proposal","evidence_version":round,"contract":orch::contract_identity(&store,task).unwrap(),"repo":repo,"severity":"normal","findings":[]})).unwrap()).unwrap();
    };
    let mut previous_verification: Option<String> = None;
    assert!(orch::done_gate_in_repo(&store, true, Some(&repo)).is_err());
    for round in 1..=5 {
        let checkpoint_args = json!(["wait","product-checkpoint","--run","product"]);
        let result = product_command(checkpoint_args.clone(), round > 1);
        assert_eq!(result["status"], "completed", "{result}");
        assert_ne!(product_command(checkpoint_args, round > 1)["status"], "completed");
        // The fixture reports unresolved issues on task attempt five. Use a
        // second already-enrolled unit to prove product rounds are not attempts.
        let task = if round < 5 { "checkpointed" } else { "second" };
        let task_round = if round < 5 { round } else { round - 3 };
        checkpoint(task, task_round);
        let args = json!({"project":"demo","unit":task,"cwd":repo,"role":"terra"});
        let built = invoke(&operator, args.clone());
        assert_eq!(built["state"], "awaiting_core", "round {round}: {built}");
        assert_eq!(invoke(&operator, args.clone())["operation_id"], built["operation_id"]);
        let state = orch::wait_state(&store).unwrap();
        let d = state
            .events
            .iter()
            .rev()
            .find(|e| e.kind == "dispatch")
            .unwrap();
        let verification_id = state
            .events
            .iter()
            .rev()
            .find(|e| e.kind == "verification")
            .unwrap()
            .id
            .as_str();
        let product_assessment = json!({"valid_issues":Vec::<&str>::new(),"severity":"normal","decision":"finish"}).to_string();
        assert!(orch::product_transition(&store, "product", round, vec![verification_id.into()], &product_assessment, "continue").is_err());
        let mut forged = state
            .events
            .iter()
            .rev()
            .find(|e| e.kind == "build")
            .unwrap()
            .clone();
        forged.id = "forged-host-report".into();
        assert!(orch::wait_event(&store, forged).is_err());
        let assessment=json!({"valid_issues":Vec::<&str>::new(),"severity":"normal"}).to_string();
        // The replayable notification is now committed before codex_run returns.
        // A stale identity still cannot acknowledge that outbox delivery.
        assert!(state.events.iter().any(|e| e.kind == "notification" && e.dispatch_id == d.id));
        assert!(
            orch::core_verdict(&store, task, "not-the-verification", true, &assessment).is_err()
        );
        let status = if round == 1 {
            pipeline_core_status(&b, &repo)
        } else {
            call(&b, "session", &json!({"action":"status"}))["structuredContent"].clone()
        };
        let notices = status["core_notices"].as_array().unwrap();
        assert_eq!(notices.len(), 1, "{status}");
        assert_eq!(notices[0]["dispatch_id"], d.id);
        let replay = call(&b, "session", &json!({"action":"status"}));
        assert_eq!(
            replay["structuredContent"]["core_notices"],
            status["core_notices"]
        );
        let original = fs::read(repo.join("src/lib.rs")).unwrap();
        fs::write(repo.join("src/lib.rs"), "changed source").unwrap();
        assert!(
            orch::core_verdict(&store, task, verification_id, true, &assessment).is_err()
        );
        fs::write(repo.join("src/lib.rs"), original).unwrap();
        let package = state
            .events
            .iter()
            .rev()
            .find(|e| e.kind == "package")
            .unwrap();
        let report: Value =
            serde_json::from_str(package.evidence.trim_start_matches("host:")).unwrap();
        let path = Path::new(report["package_path"].as_str().unwrap());
        let bytes = fs::read(path).unwrap();
        fs::write(path, b"forged package").unwrap();
        assert!(
            orch::core_verdict(&store, task, verification_id, true, &assessment).is_err()
        );
        fs::write(path, bytes).unwrap();
        let approved =
            orch::core_verdict(&store, task, verification_id, true, &assessment).unwrap();
        assert_eq!(
            orch::core_verdict(&store, task, verification_id, true, &assessment)
                .unwrap()
                .events
                .len(),
            approved.events.len()
        );
        assert!(
            orch::core_verdict(&store, task, verification_id, false, &assessment)
                .is_err()
        );
        orch::complete_round(&store, task, verification_id).unwrap();
        let count = orch::wait_state(&store).unwrap().events.len();
        orch::complete_round(&store, task, verification_id).unwrap();
        assert_eq!(orch::wait_state(&store).unwrap().events.len(), count);
        assert!(orch::current_verified_unit(&store, task, &repo).unwrap());
        if round == 1 {
            checkpoint("second", 1);
            let second = invoke(&operator, json!({"project":"demo","unit":"second","cwd":repo,"role":"terra"}));
            assert_eq!(second["state"], "awaiting_core", "{second}");
            let second_state = orch::wait_state(&store).unwrap();
            let id = &second_state.events.iter().rev().find(|e| e.task == "second" && e.kind == "verification").unwrap().id;
            call(&b, "session", &json!({"action":"status"}));
            orch::core_verdict(&store, "second", id, true, &assessment).unwrap();
            orch::complete_round(&store, "second", id).unwrap();
            // Even a complete engineering handoff cannot turn unit releases
            // into product rounds or bypass the product minimum.
            orch::record_full(&store, &repo, Some("cargo test --locked --offline")).unwrap();
            orch::record_stage(&store, "rewrite", "core", true).unwrap();
            assert!(orch::current_verified_unit(&store, "second", &repo).unwrap());
            let error = orch::done_gate_in_repo(&store, true, Some(&repo)).unwrap_err();
            assert!(error.message.contains("product rounds"), "{error:?}");
            let refused = finish_product();
            assert_eq!(refused["error"]["kind"], "not_done", "{refused}");
        }
        // Check stale/forged evidence only after all engineering gates pass,
        // so a missing unit release cannot mask the rejection being tested.
        if let Some(old) = &previous_verification {
            assert_ne!(transition(round, old, &product_assessment, "continue")["status"], "completed");
        }
        assert_ne!(transition(round, "forged-verification", &product_assessment, "continue")["status"], "completed");
        assert_ne!(transition(round + 1, verification_id, &product_assessment, "finish")["status"], "completed");
        if round < 5 {
            assert_ne!(transition(round, verification_id, &product_assessment, "finish")["status"], "completed");
            assert!(orch::done_gate_in_repo(&store, true, Some(&repo)).is_err());
            assert_eq!(finish_product()["error"]["kind"], "not_done");
        }
        let action = if round < 5 { "continue" } else { "finish" };
        let original = fs::read(repo.join("src/lib.rs")).unwrap();
        fs::write(repo.join("src/lib.rs"), "stale product source").unwrap();
        assert_ne!(transition(round, verification_id, &product_assessment, action)["status"], "completed");
        fs::write(repo.join("src/lib.rs"), original).unwrap();
        let result = transition(round, verification_id, &product_assessment, action);
        assert_eq!(result["status"], "completed", "{result}");
        let replay = orch::wait_state(&store).unwrap();
        assert_eq!(replay.product_runs[0].rounds.len(), round as usize);
        assert_eq!(orch::product_policy_finished(&replay, "product"), round >= 5);
        assert_eq!(orch::current_checkpoint(&replay, task).unwrap().round, task_round);
        previous_verification = Some(verification_id.to_string());
    }
    // Reopening the persisted journal rejects both skipped rounds and forged
    // transition evidence; do not rely only on in-memory command validation.
    let journal = store.join("core-wait.json");
    let original = fs::read(&journal).unwrap();
    for forged_evidence in [false, true] {
        let mut replay: orch::WaitState = serde_json::from_slice(&original).unwrap();
        if forged_evidence {
            replay.product_runs[0].rounds[4].transition.as_mut().unwrap().verification_ids = vec!["forged-verification".into()];
        } else {
            replay.product_runs[0].rounds[4].round = 6;
        }
        fs::write(&journal, serde_json::to_vec(&replay).unwrap()).unwrap();
        assert!(orch::wait_state(&store).is_err());
        assert!(orch::done_gate_in_repo(&store, true, Some(&repo)).is_err());
        fs::write(&journal, &original).unwrap();
    }
    assert!(orch::product_checkpoint(&store, "product").is_err());
    assert_eq!(invoke(&operator, json!({"project":"demo","unit":"second","cwd":repo,"role":"terra"}))["reused"], true);
    // Policy finish alone cannot bypass the existing product handoff.
    assert!(orch::done_gate_in_repo(&store, true, Some(&repo)).is_err());
    assert_eq!(finish_product()["error"]["kind"], "not_done");
    let full = product_command(json!(["fulltest","--repo",repo,"--command","cargo test --locked --offline"]), true);
    assert_eq!(full["status"], "completed", "{full}");
    assert!(orch::done_gate_in_repo(&store, true, Some(&repo)).is_err());
    assert_eq!(finish_product()["error"]["kind"], "not_done");
    orch::record_stage(&store, "rewrite", "core", true).unwrap();
    assert!(orch::done_gate_in_repo(&store, true, Some(&repo)).is_ok());
    orch::record_stage(&store, "rewrite", "core", false).unwrap();
    assert!(orch::done_gate_in_repo(&store, true, Some(&repo)).is_err());
    assert_eq!(finish_product()["error"]["kind"], "not_done");
    orch::record_stage(&store, "rewrite", "core", true).unwrap();
    assert!(orch::done_gate_in_repo(&store, true, Some(&repo)).is_ok());
    let original_brief = fs::read(&brief).unwrap();
    fs::write(&brief, [original_brief.as_slice(), b"changed contract\n"].concat()).unwrap();
    assert!(!orch::current_verified_unit(&store, "second", &repo).unwrap());
    assert!(orch::done_gate_in_repo(&store, true, Some(&repo)).is_err());
    fs::write(&brief, original_brief).unwrap();
    assert!(orch::done_gate_in_repo(&store, true, Some(&repo)).is_ok());
    let original_source = fs::read(repo.join("src/lib.rs")).unwrap();
    fs::write(repo.join("src/lib.rs"), "stale source").unwrap();
    assert!(orch::done_gate_in_repo(&store, true, Some(&repo)).is_err());
    assert_eq!(finish_product()["error"]["kind"], "not_done");
    fs::write(repo.join("src/lib.rs"), original_source).unwrap();
    // Successful historical verification cannot authorize a handoff or reuse
    // once the Host receipt is missing or tampered with.
    let state = orch::wait_state(&store).unwrap();
    let package = state.events.iter().rev().find(|e| e.task == "second" && e.kind == "package").unwrap();
    let evidence: Value = serde_json::from_str(package.evidence.trim_start_matches("host:")).unwrap();
    let receipt_path = Path::new(evidence["verification_receipt"]["path"].as_str().unwrap());
    let original_receipt = fs::read(receipt_path).unwrap();
    for missing in [true, false] {
        if missing { fs::remove_file(receipt_path).unwrap(); }
        else { fs::write(receipt_path, b"{}").unwrap(); }
        assert!(!orch::current_verified_unit(&store, "second", &repo).unwrap());
        assert!(orch::done_gate_in_repo(&store, true, Some(&repo)).is_err());
        assert_eq!(finish_product()["error"]["kind"], "not_done");
        fs::write(receipt_path, &original_receipt).unwrap();
        assert!(orch::current_verified_unit(&store, "second", &repo).unwrap());
    }
    let finished = finish_product();
    assert_eq!(finished["status"], "completed", "{finished}");
    assert_eq!(finished["goal"]["status"], "done", "{finished}");
    checkpoint("second", 3);
    let replay = orch::wait_state(&store).unwrap();
    assert!(!orch::product_policy_finished(&replay, "product"));
    assert!(orch::done_gate_in_repo(&store, true, Some(&repo)).is_err());
    let next = product_command(json!(["wait","product-checkpoint","--run","product"]), false);
    assert_eq!(next["status"], "completed", "{next}");
    assert_eq!(orch::wait_state(&store).unwrap().product_runs[0].rounds.len(), 6);
}
#[test]
fn enrolled_host_missing_actual_effort_and_dead_worker_never_credit_or_redispatch() {
    for fault in ["missing-effort", "crash-worker"] {
        let temp = common::temp();
        let (repo, store) = pipeline_fixture(temp.path());
        pipeline_checkpoint(&store, &repo, 1);
        let b = pipeline_bridge(temp.path(), 0, fault);
        let args = json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra"});
        assert_ne!(invoke(&b, args.clone())["status"], "completed");
        assert_ne!(invoke(&b, args)["status"], "completed");
        let state = orch::wait_state(&store).unwrap();
        assert_eq!(
            state.events.iter().filter(|e| e.kind == "dispatch").count(),
            1
        );
        assert!(!state.events.iter().any(|e| e.kind == "build"));
        if fault == "crash-worker" {
            assert!(store.join("quarantine").exists());
        }
    }
}
#[test]
#[ignore = "product host waits a real >=180 seconds; explicit acceptance command"]
fn product_host_wait_180s() {
    let temp = common::temp();
    let (repo, store) = pipeline_fixture(temp.path());
    pipeline_checkpoint(&store, &repo, 1);
    let b = pipeline_bridge(temp.path(), 180, "");
    let start = std::time::Instant::now();
    let result = invoke(
        &b,
        json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra","timeout_s":240}),
    );
    assert_eq!(result["state"], "awaiting_core", "{result}");
    assert!(start.elapsed() >= std::time::Duration::from_secs(180));
    let state = orch::wait_state(&store).unwrap();
    let build = state.events.iter().find(|e| e.kind == "build").unwrap();
    let report: Value = serde_json::from_str(build.evidence.trim_start_matches("host:")).unwrap();
    assert!(report["elapsed_ms"].as_u64().unwrap() >= 180000);
    println!(
        "product-host evidence: {}",
        serde_json::to_string(&state).unwrap()
    );
}

#[test]
fn enrolled_worker_interruption_and_explicit_recovery_preserve_attempt_identity() {
    for fault in ["interrupt-worker", "crash-worker"] {
        let temp = common::temp();
        let (repo, store) = pipeline_fixture(temp.path());
        pipeline_checkpoint(&store, &repo, 1);
        let b = pipeline_bridge(temp.path(), 0, fault);
        let result = invoke(
            &b,
            json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra","timeout_s":1}),
        );
        assert_ne!(result["status"], "completed", "{result}");
        let state = orch::wait_state(&store).unwrap();
        assert!(state.events.iter().any(|e| e.kind == "suspended"));
        if fault == "crash-worker" {
            assert!(state.operations[0].executing, "{fault}: {result}");
            assert!(store.join("quarantine").exists(), "{fault}");
        } else {
            assert!(!state.operations[0].executing, "{fault}: {result}");
            assert!(!store.join(".worker.claim").exists(), "{fault}");
        }
        let replay = invoke(&b, json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra"}));
        assert_ne!(replay["status"], "completed", "{fault}: {replay}");
        assert_eq!(orch::wait_state(&store).unwrap().events.iter().filter(|e| e.kind == "dispatch").count(), 1);
        let dispatch = state.events.iter().find(|e| e.kind == "dispatch").unwrap();
        let mut recovery = dispatch.clone();
        recovery.id = "confirmed-stopped".into();
        recovery.kind = "recovery".into();
        recovery.actor = "core".into();
        recovery.evidence="Independent fixture controller observed terminal interrupt or fixture process exit; no remote service exists in this fixture".into();
        assert!(orch::recover_dispatch(&store, recovery.clone(), "unknown").is_err());
        orch::recover_dispatch(&store, recovery, "stopped").unwrap();
        assert!(!store.join("quarantine").exists());
        let before = orch::wait_state(&store).unwrap();
        let deadline = before.operations[0].deadline_ms;
        let retry = pipeline_bridge(temp.path(), 0, "typed-failed");
        let result = invoke(
            &retry,
            json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra","timeout_s":99999}),
        );
        assert_ne!(result["status"], "completed", "{result}");
        let after = orch::wait_state(&store).unwrap();
        assert_eq!(after.events.len(), before.events.len());
        assert_eq!(after.operations.len(), 1);
        assert_eq!(after.operations[0].deadline_ms, deadline);
        assert!(!temp.path().join("terminal-launches.log").exists());
        assert_eq!(
            orch::wait_state(&store)
                .unwrap()
                .events
                .iter()
                .filter(|e| e.kind == "dispatch")
                .count(),
            1
        );
    }
}

#[test]
fn host_testing_releases_unit_without_core_technical_authority() {
    let temp = common::temp();
    let (repo, store) = pipeline_fixture(temp.path());
    let prepared = orch::run(
        &store,
        &[
            "wait".into(),
            "prepare".into(),
            "--task".into(),
            "checkpointed".into(),
            "--repo".into(),
            repo.to_string_lossy().into_owned(),
            "--issue".into(),
            "issue".into(),
            "--proposal".into(),
            "proposal".into(),
            "--severity".into(),
            "normal".into(),
            "--min-rounds".into(),
            "3".into(),
        ],
        true,
        false,
    );
    assert_eq!(prepared.exit_code, 0, "{}", prepared.stderr);
    let b = pipeline_bridge(temp.path(), 0, "host-callback");
    let status = pipeline_core_status(&b, &repo);
    assert_eq!(
        status["core_notices"].as_array().unwrap().len(),
        0,
        "{status}"
    );
    let state = orch::wait_state(&store).unwrap();
    for kind in [
        "dispatch",
        "build",
        "assembly",
        "package",
        "verification",
        "notification",
    ] {
        assert_eq!(
            state.events.iter().filter(|e| e.kind == kind).count(),
            1,
            "{kind}"
        );
    }
    let verification = state
        .events
        .iter()
        .find(|e| e.kind == "verification")
        .unwrap();
    let assessment = json!({"valid_issues":[],"severity":"normal"}).to_string();
    let command = |verification_id: &str, assessment: &str| {
        call(
            &b,
            "session",
            &json!({"action":"call","tool":"orch","arguments":{"project":"demo","args":["wait","verdict","--task","checkpointed","--verification",verification_id,"--decision","approve","--assessment",assessment]}}),
        )
    };
    assert_ne!(
        command("stale-verification", &assessment)["structuredContent"]["status"],
        "completed"
    );
    assert_ne!(
        command(&verification.id, &assessment)["structuredContent"]["status"],
        "completed"
    );
    assert!(!state.events.iter().any(|e| e.kind=="core_approve"));
    for kind in ["test_approve","completion","release"] {
        let event=state.events.iter().find(|e| e.kind==kind).unwrap();
        assert_eq!(event.actor,"lifecycle");
    }
    assert!(call(&b,"orch",&json!({"project":"demo","args":["status"]}))["isError"] == true);
    assert!(orch::may_finish(
        &orch::wait_state(&store).unwrap(),
        "checkpointed"
    ));
    assert!(orch::wait_state(&store).unwrap().product_runs.is_empty());
    assert!(orch::done_gate_in_repo(&store, true, Some(&repo)).is_err());
    let prepare = vec!["wait".into(), "prepare".into(), "--task".into(), "checkpointed".into(), "--repo".into(), repo.to_string_lossy().into_owned(), "--issue".into(), "issue".into(), "--proposal".into(), "proposal".into()];
    let mut changed = prepare.clone();
    changed.extend(["--min-rounds".into(), "5".into()]);
    assert_ne!(orch::run(&store, &changed, true, false).exit_code, 0);
    assert_eq!(orch::run(&store, &prepare, true, false).exit_code, 0);
    assert_eq!(orch::current_checkpoint(&orch::wait_state(&store).unwrap(), "checkpointed").unwrap().min_rounds, 3);
    assert!(
        call(&b, "session", &json!({"action":"status"}))["structuredContent"]["core_notices"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn host_verify_preflight_io_failure_does_not_invent_a_running_process() {
    let temp = common::temp();
    let (repo, store) = pipeline_fixture(temp.path());
    let marker = repo.join("verify-started");
    let script = repo.join("verify_start_count.py");
    fs::write(&script, "from pathlib import Path\nPath('verify-started').write_text('started')\n").unwrap();
    let command = format!("\"{}\" verify_start_count.py", common::python());
    fs::write(store.join("checkpointed.brief"), format!("GOAL Verify fixture\nSCOPE src/lib.rs src/mod.py\nVERIFY {command}\nDELIVERY verification\nOUTPUTS src/lib.rs\n")).unwrap();
    pipeline_checkpoint(&store, &repo, 1);
    fs::create_dir_all(store.join("reports/checkpointed.md")).unwrap();
    let b = pipeline_bridge(temp.path(), 0, "");
    let result = invoke(&b, json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra"}));
    assert_eq!(result["outcome"], "failed", "{result}");
    assert_eq!(result["error"]["kind"], "io", "{result}");
    assert!(!marker.exists());
    let state = orch::wait_state(&store).unwrap();
    assert!(!state.operations[0].executing);
    assert_eq!(state.operations[0].outcome, "failed");
    assert!(!store.join(".worker.claim").exists());
    assert!(!store.join("quarantine").exists());
    assert!(!state.events.iter().any(|e| e.kind == "package"));
}

#[test]
fn enrolled_host_verification_receipt_records_failures_and_source_changes() {
    let mut targets = std::collections::HashSet::new();
    for mode in ["success", "failure", "stale"] {
        let temp = common::temp();
        // Keep Host report names within Win32's limit; only Cargo's nested
        // artifact paths are the regression under test.
        let root = temp.path().join("long-project-store-".repeat(2));
        let (repo, store) = pipeline_fixture(&root);
        let script = repo.join("verify_fixture.py");
        fs::write(&script, format!("import os, sys\nfrom pathlib import Path\nprint('receipt stdout')\nprint(os.environ['CARGO_TARGET_DIR'])\nprint('receipt stderr', file=sys.stderr)\n{}\nsys.exit({})\n", if mode == "stale" { "Path('src/lib.rs').write_text('changed')" } else { "pass" }, if mode == "failure" { 7 } else { 0 })).unwrap();
        let command = format!("\"{}\" verify_fixture.py", common::python());
        fs::write(store.join("checkpointed.brief"), format!("GOAL Verify fixture\nSCOPE src/lib.rs src/mod.py\nVERIFY {command}\nDELIVERY verification\nOUTPUTS src/lib.rs\n")).unwrap();
        pipeline_checkpoint(&store, &repo, 1);
        // A candidate unit run must not overwrite an earlier failed fulltest.
        let failed_full = b"kind\tcommand\texit_code\tcommit\tts\nfull\tcargo test --locked --offline\t101\told-source\t2026-10-31T00:00:00Z\n";
        fs::create_dir_all(store.join("reports")).unwrap();
        fs::write(store.join("reports/_full.evidence.tsv"), failed_full).unwrap();
        let b = pipeline_bridge(&root, 0, "");
        let result = invoke(&b, json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra"}));
        if mode == "success" { assert_eq!(result["state"], "awaiting_core", "{result}"); }
        else { assert_ne!(result["status"], "completed", "{result}"); }
        let receipt_path = fs::read_dir(store.join("reports")).unwrap().map(|e| e.unwrap().path()).find(|p| p.file_name().unwrap().to_string_lossy().starts_with("verify-checkpointed-") && p.extension().is_some_and(|e| e == "json")).unwrap();
        let receipt: Value = serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
        assert_eq!(receipt["command"], command);
        assert!(receipt["stdout"].as_str().unwrap().contains("receipt stdout"));
        let target = receipt["target_dir"].as_str().unwrap();
        assert!(targets.insert(target.to_owned()));
        assert!(target.encode_utf16().count() <= 78);
        assert!(!Path::new(target).starts_with(&store));
        assert!(Path::new(target).join(".chronos-binding.json").is_file());
        let binding: Value = serde_json::from_slice(&fs::read(Path::new(target).join(".chronos-binding.json")).unwrap()).unwrap();
        assert_eq!(binding["source"], fs::canonicalize(&repo).unwrap().to_string_lossy().as_ref());
        assert!(binding["execution"].as_str().unwrap().starts_with("verify:checkpointed:"));
        assert_eq!(fs::read(receipt_path.with_extension("stdout")).unwrap(), receipt["stdout"].as_str().unwrap().as_bytes());
        assert_eq!(fs::read(receipt_path.with_extension("stderr")).unwrap(), receipt["stderr"].as_str().unwrap().as_bytes());
        assert!(receipt["stdout"].as_str().unwrap().contains(target));
        assert!(receipt["stderr"].as_str().unwrap().contains("receipt stderr"));
        assert_eq!(receipt["exit_code"], if mode == "failure" { 7 } else { 0 });
        assert_eq!(receipt["source_before"] == receipt["source_after"], mode != "stale");
        let state = orch::wait_state(&store).unwrap();
        assert_eq!(state.events.iter().any(|e| e.kind == "verification"), mode == "success");
        assert!(!state.operations[0].executing, "{mode}: {result}");
        assert!(!store.join(".worker.claim").exists(), "{mode}");
        assert_eq!(fs::read(store.join("reports/_full.evidence.tsv")).unwrap(), failed_full);
        if mode == "success" {
            let package = state.events.iter().find(|e| e.kind == "package").unwrap();
            let evidence: Value = serde_json::from_str(package.evidence.trim_start_matches("host:")).unwrap();
            assert_eq!(evidence["verification_receipt"]["receipt"], receipt);
            assert_eq!(evidence["source_identity"], receipt["source_after"]);
            let commands = evidence["commands"].as_array().unwrap();
            let package_target = commands[0]["env"]["CARGO_TARGET_DIR"].as_str().unwrap();
            assert!(targets.insert(package_target.to_owned()));
            assert!(package_target.encode_utf16().count() <= 78);
            assert_ne!(package_target, target);
            assert_eq!(commands[1]["env"]["CARGO_TARGET_DIR"], package_target);
            assert!(Path::new(evidence["cargo_package_path"].as_str().unwrap()).starts_with(package_target));
        } else {
            assert!(!state.events.iter().any(|e| e.kind == "package"));
        }
    }
}

#[test]
fn enrolled_host_rejects_missing_or_tampered_receipt_and_stale_review_source() {
    for fault in ["missing-receipt", "tampered-receipt", "stale-review-source"] {
        let temp = common::temp();
        let (repo, store) = pipeline_fixture(temp.path());
        pipeline_checkpoint(&store, &repo, 1);
        let b = pipeline_bridge(temp.path(), 0, fault);
        let result = invoke(&b, json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra"}));
        assert_ne!(result["status"], "completed", "{fault}: {result}");
        let state = orch::wait_state(&store).unwrap();
        assert!(state.events.iter().any(|e| e.kind == "package"), "{fault}");
        assert!(!state.events.iter().any(|e| e.kind == "verification"), "{fault}");
        assert!(!orch::current_verified_unit(&store, "checkpointed", &repo).unwrap());
    }
}

#[test]
fn stage_restart_child() {
    let Some(home) = std::env::var_os("AXIOM_R1_RESTART_HOME") else { return };
    let home = Path::new(&home);
    let b = pipeline_bridge(home, 0, "");
    let result = invoke(&b, json!({"project":"demo","unit":"checkpointed","cwd":home.join("demo"),"role":"terra","stop_after":"build","timeout_s":300}));
    assert_eq!(result["outcome"], "blocked", "{result}");
    assert_eq!(result["stage"], "assembly");
    // Reconstruct the exact crash window: terminal boundary is durable but
    // the local process lease was not unlinked. The child then exits.
    let store = home.join("orchestrate/demo");
    let op = orch::wait_state(&store).unwrap().operations[0].clone();
    fs::write(store.join(".worker.claim"), &op.worker_lease).unwrap();
}

#[test]
fn durable_phase_boundaries_resume_after_process_restart_without_a_second_dispatch() {
    let temp = common::temp();
    let (repo, store) = pipeline_fixture(temp.path());
    pipeline_checkpoint(&store, &repo, 1);
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "codex_cases::stage_restart_child", "--nocapture"])
        .env("AXIOM_R1_RESTART_HOME", temp.path()).output().unwrap();
    assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    let initial = orch::wait_state(&store).unwrap().operations[0].clone();
    let deadline = initial.deadline_ms;
    for phase in ["assembly", "host_test", "package", "verification"] {
        let b = pipeline_bridge(temp.path(), 0, "");
        let response = call(&b, "codex_run", &json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra","stop_after":phase,"timeout_s":99999}));
        let result = &response["structuredContent"];
        assert_eq!(response["isError"], true, "{response}");
        assert_eq!(result["outcome"], "blocked", "{phase}: {result}");
        assert_eq!(result["operation_id"], initial.id);
        assert_eq!(result["deadline_ms"], deadline);
        assert!(!store.join(".worker.claim").exists());
    }
    let b = pipeline_bridge(temp.path(), 0, "");
    let args = json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra"});
    let result = invoke(&b, args.clone());
    assert_eq!(result["outcome"], "awaiting_core", "{result}");
    let replay = invoke(&b, args);
    assert_eq!(replay["delivery_id"], result["delivery_id"]);
    let state = orch::wait_state(&store).unwrap();
    for kind in ["dispatch", "build", "assembly", "package", "verification", "notification"] {
        assert_eq!(state.events.iter().filter(|e| e.kind == kind).count(), 1, "{kind}");
    }
    assert_eq!(state.operations.len(), 1);
    assert_eq!(state.operations[0].owner_generation, 6);
    assert_eq!(state.operations[0].deadline_ms, deadline);
    assert!(!orch::may_finish(&state, "checkpointed"));
}

#[test]
fn host_terminal_failure_is_not_an_unknown_writer_or_an_implicit_retry() {
    for fault in ["typed-unconfirmed", "typed-failed", "native-failed"] {
        let temp = common::temp();
        let (repo, store) = pipeline_fixture(temp.path());
        pipeline_checkpoint(&store, &repo, 1);
        let args = json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra","timeout_s":300});
        let b = pipeline_bridge(temp.path(), 0, fault);
        let result = invoke(&b, args.clone());
        assert_eq!(result["outcome"], "failed", "{fault}: {result}");
        assert_eq!(result["transport_status"], "completed");
        let state = orch::wait_state(&store).unwrap();
        let op = &state.operations[0];
        assert!(!op.executing, "{fault}");
        assert!(op.terminal_failure.is_some());
        assert!(!store.join(".worker.claim").exists(), "{fault}");
        assert!(!store.join("quarantine").exists());
        let deadline = op.deadline_ms;
        let journal = state.events.len();
        let launches = fs::read_to_string(temp.path().join("terminal-launches.log")).unwrap();
        assert_eq!(launches.lines().count(), 1);
        drop(b);
        let b = pipeline_bridge(temp.path(), 0, fault);
        let mut retry = args.clone();
        retry["timeout_s"] = json!(99999);
        retry["operation_id"] = result["operation_id"].clone();
        let replay = invoke(&b, retry);
        assert_eq!(replay["delivery_id"], result["delivery_id"], "{fault}: {replay}");
        let state = orch::wait_state(&store).unwrap();
        assert!(!state.operations[0].executing, "{fault}");
        assert!(state.operations[0].terminal_failure.is_some(), "{fault}");
        assert!(!store.join(".worker.claim").exists(), "{fault}");
        assert_eq!(state.operations.len(), 1);
        assert_eq!(state.operations[0].deadline_ms, deadline);
        assert_eq!(state.events.len(), journal);
        assert_eq!(state.events.iter().filter(|e| e.kind == "dispatch").count(), 1);
        assert_eq!(fs::read_to_string(temp.path().join("terminal-launches.log")).unwrap(), launches);
    }
}

#[test]
fn host_scope_rejection_and_confirmed_astra_failure_release_ownership() {
    for fault in ["scope", "native-failed"] {
        let temp = common::temp();
        let (repo, store) = pipeline_fixture(temp.path());
        pipeline_checkpoint(&store, &repo, 1);
        let args = json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra"});
        let b = pipeline_bridge(temp.path(), 0, "");
        if fault != "scope" {
            let mut boundary = args.clone();
            boundary["stop_after"] = json!("package");
            assert_eq!(invoke(&b, boundary)["outcome"], "blocked");
        }
        drop(b);
        let b = if fault == "scope" {
            let wrapper = temp.path().join("scope_fixture.py");
            let original = common::repo_root().join("tests/bridge/fake_dispatcher.py");
            fs::write(&wrapper, "import sys\np=sys.argv.pop(1)\ns=open(p,encoding='utf8').read()\ns=s.replace(\"        complete(tid,turn,final,status='failed' if fault=='native-failed' else 'completed')\", \"        event('item/completed',tid,turnId=turn,item={'type':'fileChange','id':'scope','status':'completed','changes':[{'path':'outside.txt','kind':{'type':'update'}}]})\\n        complete(tid,turn,final,status='failed' if fault=='native-failed' else 'completed')\")\n__file__=p\nexec(compile(s,p,'exec'))\n").unwrap();
            Bridge::new(Config { data_home: Some(temp.path().into()), codex: Ok(vec![common::python(), "-I".into(), wrapper.to_string_lossy().into_owned(), original.to_string_lossy().into_owned(), "--scenario".into(), "pipeline".into()]), agy: Err(axiom::base::Failure::new("disabled", "test")), wsl: None, on_windows: cfg!(windows) })
        } else { pipeline_bridge(temp.path(), 0, fault) };
        let result = invoke(&b, args.clone());
        assert_eq!(result["outcome"], "failed", "{fault}: {result}");
        let state = orch::wait_state(&store).unwrap();
        assert!(!state.operations[0].executing, "{fault}");
        assert!(!store.join(".worker.claim").exists(), "{fault}");
        assert!(!store.join("quarantine").exists(), "{fault}");
        assert_eq!(invoke(&b, args)["delivery_id"], result["delivery_id"]);
        assert_eq!(orch::wait_state(&store).unwrap().events.iter().filter(|e| e.kind == "dispatch").count(), 1);
    }
}

#[test]
fn typed_blocked_missing_truncated_and_foreign_results_never_advance_or_redispatch() {
    for fault in ["blocked-worker", "missing-result", "truncated-result", "wrong-identity"] {
        let temp = common::temp();
        let (repo, store) = pipeline_fixture(temp.path());
        pipeline_checkpoint(&store, &repo, 1);
        let b = pipeline_bridge(temp.path(), 0, fault);
        let args = json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra"});
        let response = call(&b, "codex_run", &args);
        let result = &response["structuredContent"];
        assert_eq!(response["isError"], true, "{fault}: {response}");
        assert_eq!(result["outcome"], if fault == "blocked-worker" { "blocked" } else { "protocol_incomplete" });
        let replay = invoke(&b, args);
        assert_eq!(replay["delivery_id"], result["delivery_id"], "{replay}");
        let state = orch::wait_state(&store).unwrap();
        assert!(!state.operations[0].executing, "{fault}");
        assert!(state.operations[0].terminal_failure.is_some(), "{fault}");
        assert!(!store.join(".worker.claim").exists(), "{fault}");
        assert_eq!(state.events.iter().filter(|e| e.kind == "dispatch").count(), 1);
        assert!(!state.events.iter().any(|e| e.kind == "build" || e.kind == "assembly"));
    }
}

#[test]
fn sol_terminal_blocked_and_failed_release_writer_without_retry() {
    for fault in ["blocked-worker", "native-failed"] {
        let temp = common::temp();
        let (repo, store) = pipeline_fixture(temp.path());
        pipeline_checkpoint(&store, &repo, 1);
        let args = json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra"});
        let b = pipeline_bridge(temp.path(), 0, "");
        let mut boundary = args.clone();
        boundary["stop_after"] = json!("build");
        assert_eq!(invoke(&b, boundary)["stage"], "assembly");
        drop(b);
        let b = pipeline_bridge(temp.path(), 0, fault);
        let result = invoke(&b, args.clone());
        assert_eq!(result["outcome"], if fault == "blocked-worker" { "blocked" } else { "failed" }, "{result}");
        let state = orch::wait_state(&store).unwrap();
        let op = &state.operations[0];
        assert!(!op.executing, "{fault}");
        assert_eq!(op.terminal_failure.as_ref().unwrap()["requested_role"], "sol");
        assert!(!store.join(".worker.claim").exists());
        assert_eq!(invoke(&b, args)["delivery_id"], result["delivery_id"]);
        assert_eq!(orch::wait_state(&store).unwrap().events.iter().filter(|e| e.kind == "assembly_intent").count(), 1);
    }
}

#[test]
fn enrolled_unconfirmed_interrupt_keeps_writer_quarantined() {
    let temp = common::temp();
    let (repo, store) = pipeline_fixture(temp.path());
    fs::write(store.join("checkpointed.brief"), "GOAL NO_CONFIRM\nSCOPE src/lib.rs src/mod.py\nVERIFY cargo test --locked --offline\nDELIVERY verification\nOUTPUTS src/lib.rs\n").unwrap();
    pipeline_checkpoint(&store, &repo, 1);
    let b = bridge(temp.path());
    let args = json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra","timeout_s":1});
    let result = invoke(&b, args.clone());
    assert_eq!(result["outcome"], "unconfirmed", "{result}");
    let state = orch::wait_state(&store).unwrap();
    assert!(state.operations[0].executing);
    assert!(store.join("quarantine").is_dir());
    assert!(store.join(".worker.claim").exists());
    let replay = invoke(&bridge(temp.path()), args);
    assert_ne!(replay["status"], "completed", "{replay}");
    assert_eq!(orch::wait_state(&store).unwrap().events.iter().filter(|e| e.kind == "dispatch").count(), 1);
    drop(b);
    orch::recover_quarantine(&store).unwrap();
}

#[test]
fn persisted_deadline_is_not_renewed_by_restart_or_larger_requested_timeout() {
    let temp = common::temp();
    let (repo, store) = pipeline_fixture(temp.path());
    pipeline_checkpoint(&store, &repo, 1);
    let b = pipeline_bridge(temp.path(), 0, "");
    let result = invoke(&b, json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra","stop_after":"build","timeout_s":3}));
    assert_eq!(result["outcome"], "blocked", "{result}");
    drop(b);
    std::thread::sleep(std::time::Duration::from_millis(3100));
    let b = pipeline_bridge(temp.path(), 0, "");
    let result = invoke(&b, json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra","timeout_s":99999}));
    assert_eq!(result["outcome"], "timeout", "{result}");
    let state = orch::wait_state(&store).unwrap();
    assert_eq!(state.events.iter().filter(|e| e.kind == "dispatch").count(), 1);
    assert!(!state.events.iter().any(|e| e.kind == "assembly"));
}

#[test]
fn actual_test_rejection_allows_dispatcher_repair_without_core_verdict() {
    let temp=common::temp();
    let (repo,store)=pipeline_fixture(temp.path());
    assert_eq!(orch::run(&store,&["wait".into(),"enroll".into()],true,false).exit_code,0);
    let prepared=orch::run(&store,&["wait".into(),"prepare".into(),"--task".into(),"checkpointed".into(),"--repo".into(),repo.to_string_lossy().into_owned(),"--issue".into(),"issue".into(),"--proposal".into(),"proposal".into(),"--min-rounds".into(),"5".into()],true,false);
    assert_eq!(prepared.exit_code,0,"{}",prepared.stderr);
    let b=pipeline_bridge(temp.path(),0,"host-callback-test-findings");
    call(&b,"session",&json!({"action":"begin","project":repo,"text":"repair test finding"}));
    seed_execution(&b);
    call(&b,"session",&json!({"action":"call","tool":"goal","arguments":{"mode":"change"}}));
    let result=call(&b,"session",&json!({"action":"call","tool":"Agent","arguments":{"subagent_type":"chronos:dispatcher","project":"demo","cwd":repo,"prompt":"Run tests and prepare a repair if they fail"}}));
    let state=orch::wait_state(&store).unwrap();
    let report=fs::read_to_string(result["structuredContent"]["saved_to"].as_str().unwrap()).unwrap();
    assert_eq!(state.checkpoints.last().unwrap().round,2,"{report}");
    assert!(state.events.iter().any(|e|e.kind=="test_reject"&&e.actor=="lifecycle"));
    assert!(!state.events.iter().any(|e|matches!(e.kind.as_str(),"core_approve"|"core_reject"|"release")));
    assert!(call(&b,"session",&json!({"action":"status"}))["structuredContent"]["core_notices"].as_array().unwrap().is_empty());
    assert!(!orch::may_finish(&state,"checkpointed"));
}

#[test]
fn independent_final_tester_unlocks_only_core_submission() {
    // Downstream fixture: the real prefix is covered by authority_cases.
    // Models are deterministic; Rust executes actual build, package and full tests.
    let temp=common::temp();
    let (repo,store)=pipeline_fixture(temp.path());
    pipeline_checkpoint(&store,&repo,1);
    let b=pipeline_bridge(temp.path(),0,"host-callback-final");
    pipeline_core_status(&b,&repo);
    let status=call(&b,"session",&json!({"action":"call","tool":"workflow","arguments":{"action":"status"}}));
    assert_eq!(status["structuredContent"]["workflow"]["phase"],"ready_submit","{status}");
    let submitted=call(&b,"session",&json!({"action":"call","tool":"workflow","arguments":{"action":"submit"}}));
    assert_eq!(submitted["structuredContent"]["phase"],"submitted","{submitted}");
    let done=call(&b,"session",&json!({"action":"call","tool":"goal","arguments":{"status":"done"}}));
    assert_eq!(done["isError"],false,"{done}");
    assert_eq!(done["structuredContent"]["goal"]["status"],"done","{done}");
    assert!(!orch::wait_state(&store).unwrap().events.iter().any(|e|e.kind=="core_approve"));
}

#[test]
fn rejected_final_tester_persists_findings_without_unlocking_core() {
    let temp=common::temp();
    let (repo,store)=pipeline_fixture(temp.path());
    pipeline_checkpoint(&store,&repo,1);
    let b=pipeline_bridge(temp.path(),0,"host-callback-final-reject");
    pipeline_core_status(&b,&repo);
    let status=call(&b,"session",&json!({"action":"call","tool":"workflow","arguments":{"action":"status"}}));
    assert_eq!(status["structuredContent"]["workflow"]["phase"],"execution","{status}");
    let submitted=call(&b,"session",&json!({"action":"call","tool":"workflow","arguments":{"action":"submit"}}));
    assert_eq!(submitted["isError"],true,"{submitted}");
    let authority=b.home().unwrap().join("authority");
    let artifacts:Vec<_>=fs::read_dir(authority).unwrap().flatten().flat_map(|d|
        fs::read_dir(d.path()).unwrap().flatten().map(|f|f.path()).collect::<Vec<_>>()
    ).filter(|p|p.file_name().unwrap().to_string_lossy().starts_with("tester-")).collect();
    assert_eq!(artifacts.len(),1);
    let saved:Value=serde_json::from_slice(&fs::read(&artifacts[0]).unwrap()).unwrap();
    assert!(saved["final_message"].as_str().unwrap().contains("Final acceptance evidence missing"));
}

#[test]
fn outer_dispatcher_budget_does_not_cut_a_long_owned_engineering_callback() {
    let temp = common::temp();
    let (repo, store) = pipeline_fixture(temp.path());
    pipeline_checkpoint(&store, &repo, 1);
    let b = pipeline_bridge(temp.path(), 13, "host-callback");
    assert_eq!(call(&b,"session",&json!({"action":"begin","project":repo,"text":"deadline regression"}))["isError"], false);
    seed_execution(&b);
    assert_eq!(call(&b,"session",&json!({"action":"call","tool":"goal","arguments":{"mode":"change"}}))["isError"], false);
    let result = call(&b,"session",&json!({"action":"call","tool":"Agent","arguments":{"subagent_type":"chronos:dispatcher","project":"demo","cwd":repo,"prompt":"run one owned engineering operation","timeout_s":1}}));
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["structuredContent"]["outcome"], "verified", "{result}");
    assert_eq!(orch::wait_state(&store).unwrap().operations.len(), 1);
}

#[test]
fn outer_completed_transport_cannot_mask_blocked_or_incomplete_domain_result() {
    for fault in ["host-callback-blocked-worker", "host-callback-missing-result"] {
        let temp = common::temp();
        let (repo, store) = pipeline_fixture(temp.path());
        pipeline_checkpoint(&store, &repo, 1);
        let b = pipeline_bridge(temp.path(), 0, fault);
        call(&b,"session",&json!({"action":"begin","project":repo,"text":"blocked domain regression"}));
        seed_execution(&b);
        call(&b,"session",&json!({"action":"call","tool":"goal","arguments":{"mode":"change"}}));
        let response = call(&b,"session",&json!({"action":"call","tool":"Agent","arguments":{"subagent_type":"chronos:dispatcher","project":"demo","cwd":repo,"prompt":"run one operation"}}));
        let result = &response["structuredContent"];
        assert_eq!(response["isError"], true, "{response}");
        assert_eq!(result["transport_status"], "completed", "{result}");
        assert_eq!(result["outcome"], if fault.ends_with("blocked-worker") { "blocked" } else { "protocol_incomplete" }, "{result}");
        let saved: Value = serde_json::from_slice(&fs::read(result["saved_to"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(saved["domain_result"], result["result"]);
        let op = &orch::wait_state(&store).unwrap().operations[0];
        assert_eq!(result["result"]["operations"][0]["delivery_id"], op.reply()["delivery_id"]);
        assert!(!op.receipts.contains_key("build"));
    }
}

#[test]
fn implementation_delivery_gate_uses_actual_tree_and_host_check_before_assembly() {
    for fault in ["delivery-native", "delivery-zero", "delivery-outside", "delivery-unrelated", "delivery-missing-output", "delivery-verify-fail", "delivery-positive"] {
        let temp = common::temp();
        let (repo, store) = pipeline_fixture(temp.path());
        let verify = if fault == "delivery-verify-fail" { "cargo test --locked --offline --definitely-invalid" } else { "cargo test --locked --offline" };
        fs::write(store.join("checkpointed.brief"), format!("GOAL Implement required artifact\nSCOPE src/lib.rs src/deliverable.rs\nVERIFY {verify}\nDELIVERY implementation\nOUTPUTS {}\n", if fault == "delivery-missing-output" { "src/lib.rs src/deliverable.rs" } else { "src/deliverable.rs" })).unwrap();
        let outputs = if fault == "delivery-missing-output" { vec!["src/lib.rs", "src/deliverable.rs"] } else { vec!["src/deliverable.rs"] };
        orch::checkpoint(&store, serde_json::from_value(json!({"task":"checkpointed","module":"m","round":1,"owner":"terra","dependencies":[],"outputs":outputs,"evidence":["tests"],"next_action":"build","issue":"issue","proposal":"proposal","evidence_version":1,"contract":orch::contract_identity(&store,"checkpointed").unwrap(),"repo":repo,"severity":"normal","findings":[]})).unwrap()).unwrap();
        let b = pipeline_bridge(temp.path(), 0, fault);
        let response = call(&b, "codex_run", &json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra","stop_after":"assembly"}));
        let state = orch::wait_state(&store).unwrap();
        let positive = fault == "delivery-positive";
        if !positive {
            assert!(!state.operations[0].executing, "{fault}: {response}");
            assert!(!store.join(".worker.claim").exists(), "{fault}");
        }
        assert_eq!(state.events.iter().filter(|e| e.kind == "build").count(), usize::from(positive), "{fault}: {response}");
        assert_eq!(state.events.iter().filter(|e| e.kind == "assembly").count(), usize::from(positive), "{fault}: {response}");
        let log = fs::read_to_string(temp.path().join("delivery-launches.log")).unwrap();
        assert_eq!(log.lines().filter(|l| l.contains("terra")).count(), 1, "{fault}");
        assert_eq!(log.lines().filter(|l| l.contains("sol")).count(), usize::from(positive), "{fault}");
        assert_eq!(log.lines().filter(|l| l.contains("astra")).count(), 0, "{fault}");
        if ["delivery-zero", "delivery-unrelated", "delivery-missing-output"].contains(&fault) {
            assert_eq!(response["structuredContent"]["outcome"], "blocked", "{fault}: {response}");
        }
        if positive { assert!(repo.join("src/deliverable.rs").is_file()); }
    }
}

#[test]
fn implementation_saved_build_requires_intact_bound_receipt_before_assembly() {
    for tamper in ["missing", "identity"] {
        let temp = common::temp();
        let (repo, store) = pipeline_fixture(temp.path());
        fs::write(store.join("checkpointed.brief"), "GOAL Implement\nSCOPE src/lib.rs src/deliverable.rs\nVERIFY cargo test --locked --offline\nDELIVERY implementation\nOUTPUTS src/deliverable.rs\n").unwrap();
        orch::checkpoint(&store, serde_json::from_value(json!({"task":"checkpointed","module":"m","round":1,"owner":"terra","dependencies":[],"outputs":["src/deliverable.rs"],"evidence":["tests"],"next_action":"build","issue":"issue","proposal":"proposal","evidence_version":1,"contract":orch::contract_identity(&store,"checkpointed").unwrap(),"repo":repo,"severity":"normal","findings":[]})).unwrap()).unwrap();
        let b = pipeline_bridge(temp.path(), 0, "delivery-positive");
        invoke(&b, json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra","stop_after":"build"}));
        let state = orch::wait_state(&store).unwrap();
        let receipt = &state.operations[0].receipts["build"]["delivery_receipt"];
        let path = std::path::PathBuf::from(receipt["path"].as_str().unwrap());
        if tamper == "missing" { fs::remove_file(path).unwrap(); } else {
            let mut value = receipt["receipt"].clone(); value["binding"]["operation_id"] = json!("foreign");
            fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
        }
        let result = invoke(&b, json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra"}));
        assert_eq!(result["outcome"], "failed", "{tamper}: {result}");
        let state = orch::wait_state(&store).unwrap();
        assert_eq!(state.events.iter().filter(|e| e.kind == "assembly").count(), 0);
        let log = fs::read_to_string(temp.path().join("delivery-launches.log")).unwrap();
        assert_eq!(log.lines().count(), 1, "{tamper}");
    }
}

#[test]
fn missing_live_delivery_never_dispatches_but_historical_brief_remains_readable() {
    let temp = common::temp();
    let (repo, store) = pipeline_fixture(temp.path());
    pipeline_checkpoint(&store, &repo, 1);
    fs::write(store.join("checkpointed.brief"), "GOAL Historical\nSCOPE src/lib.rs\nVERIFY cargo test --locked --offline\n").unwrap();
    assert!(orch::unit_brief(&store, "checkpointed").unwrap().delivery.is_none());
    let b = pipeline_bridge(temp.path(), 0, "delivery-zero");
    let response = call(&b, "codex_run", &json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra"}));
    assert_eq!(response["isError"], true, "{response}");
    assert_eq!(response["structuredContent"]["error"]["kind"], "invalid_argument", "{response}");
    let state = orch::wait_state(&store).unwrap();
    assert_eq!(state.events.iter().filter(|e| ["dispatch", "build", "assembly"].contains(&e.kind.as_str())).count(), 0);
    assert!(!temp.path().join("delivery-launches.log").exists());
}

#[test]
fn explicit_outputs_real_registration_prepare_and_build_boundary() {
    for mode in ["delivery-positive", "delivery-existing", "delivery-dir", "delivery-dir-delete", "delivery-empty", "delivery-wrong", "delivery-file-is-dir", "delivery-file-delete", "delivery-ignored-dir", "delivery-other"] {
        let temp = common::temp();
        let (repo, store) = pipeline_fixture(temp.path());
        fs::remove_file(store.join("units.tsv")).unwrap();
        assert_eq!(orch::run(&store, &["init".into()], true, false).exit_code, 0);
        fs::create_dir_all(repo.join("src/items")).unwrap();
        if mode == "delivery-dir-delete" {
            fs::write(repo.join("src/items/keep.rs"), "// keep\n").unwrap();
            fs::write(repo.join("src/items/remove.rs"), "// remove\n").unwrap();
        }
        if mode == "delivery-wrong" { fs::write(repo.join("src/items" ).join("wrong.rs"), "// wrong type\n").unwrap(); }
        if mode == "delivery-file-delete" { fs::write(repo.join("src/deliverable.rs"), "// required file\n").unwrap(); }
        if mode == "delivery-ignored-dir" {
            fs::write(repo.join(".gitignore"), "/target\n/src/items/\n").unwrap();
            fs::write(repo.join("src/items/ignored.rs"), "// not monitored\n").unwrap();
        }
        let outputs = match mode {
            "delivery-existing" => "src/lib.rs src/deliverable.rs",
            "delivery-dir" | "delivery-dir-delete" | "delivery-empty" | "delivery-ignored-dir" => "src/items/",
            "delivery-wrong" => "src/items/wrong.rs/",
            "delivery-file-is-dir" => "src/items",
            _ => "src/deliverable.rs",
        };
        let brief = store.join("checkpointed.brief");
        fs::write(&brief, format!("GOAL Implement\nSCOPE src/\nVERIFY cargo test --locked --offline\nDELIVERY implementation\nOUTPUTS {outputs}\n")).unwrap();
        let add = vec!["unit".into(), "add".into(), "checkpointed".into(), "--track".into(), "m".into(), "--brief".into(), brief.to_string_lossy().into_owned()];
        let added = orch::run(&store, &add, true, false);
        assert_eq!(added.exit_code, 0, "{}", added.stderr);
        let prepare = vec!["wait".into(), "prepare".into(), "--task".into(), "checkpointed".into(), "--repo".into(), repo.to_string_lossy().into_owned(), "--issue".into(), "issue".into(), "--proposal".into(), "proposal".into()];
        let prepared = orch::run(&store, &prepare, true, false);
        assert_eq!(prepared.exit_code, 0, "{}", prepared.stderr);
        let state = orch::wait_state(&store).unwrap();
        let checkpoint = orch::current_checkpoint(&state, "checkpointed").unwrap();
        assert_eq!(checkpoint.outputs, outputs.split_whitespace().collect::<Vec<_>>());
        assert_ne!(checkpoint.outputs, vec!["src/"]);
        let b = pipeline_bridge(temp.path(), 0, mode);
        let result = invoke(&b, json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra","stop_after":"build"}));
        let positive = ["delivery-positive", "delivery-existing", "delivery-dir", "delivery-dir-delete"].contains(&mode);
        let state = orch::wait_state(&store).unwrap();
        assert_eq!(state.events.iter().filter(|e| e.kind == "build").count(), usize::from(positive), "{mode}: {result}");
        assert_eq!(state.events.iter().filter(|e| e.kind == "assembly").count(), 0, "{mode}: {result}");
        let log = fs::read_to_string(temp.path().join("delivery-launches.log")).unwrap();
        assert_eq!(log.lines().filter(|s| s.contains("terra")).count(), 1);
        assert_eq!(log.lines().filter(|s| s.contains("sol") || s.contains("astra")).count(), 0);
        if positive { assert_eq!(result["stage"], "assembly", "{mode}: {result}"); }
        else {
            assert_eq!(result["outcome"], "blocked", "{mode}: {result}");
            let replay = invoke(&b, json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra","stop_after":"build"}));
            assert_eq!(replay["delivery_id"], result["delivery_id"], "{mode}: {replay}");
            assert_eq!(fs::read_to_string(temp.path().join("delivery-launches.log")).unwrap(), log);
        }
    }
}

#[test]
fn live_missing_outputs_and_forged_checkpoint_reject_before_dispatch() {
    for mode in ["missing-delivery", "missing", "invalid", "mismatch"] {
        let temp = common::temp();
        let (repo, store) = pipeline_fixture(temp.path());
        if mode == "mismatch" {
            pipeline_checkpoint(&store, &repo, 1);
        } else {
            let suffix = match mode {
                "missing-delivery" => "OUTPUTS src/lib.rs\n",
                "invalid" => "DELIVERY verification\nOUTPUTS ../outside\n",
                _ => "DELIVERY verification\n",
            };
            fs::write(store.join("checkpointed.brief"), format!("GOAL Verify a Rust product\nSCOPE src/lib.rs src/mod.py\nVERIFY cargo test --locked --offline\n{suffix}")).unwrap();
            let contract = if mode == "invalid" { "sha256:invalid".into() } else { orch::contract_identity(&store,"checkpointed").unwrap() };
            let c = serde_json::from_value(json!({"task":"checkpointed","module":"m","round":1,"owner":"terra","dependencies":[],"outputs":["src/lib.rs"],"evidence":["tests"],"next_action":"build","issue":"issue","proposal":"proposal","evidence_version":1,"contract":contract,"repo":repo,"severity":"normal","findings":[]})).unwrap();
            let error = orch::checkpoint(&store, c).unwrap_err();
            assert_eq!(error.kind, "invalid_argument", "{mode}: {error:?}");
            assert!(error.message.contains("OUTPUTS"), "{mode}: {error:?}");
            assert!(!store.join("core-wait.json").exists());
            // Forge a historical checkpoint to exercise live validation too.
            fs::write(store.join("checkpointed.brief"), "GOAL Verify a Rust product\nSCOPE src/lib.rs src/mod.py\nVERIFY cargo test --locked --offline\nDELIVERY verification\nOUTPUTS src/lib.rs\n").unwrap();
            pipeline_checkpoint(&store, &repo, 1);
            fs::write(store.join("checkpointed.brief"), format!("GOAL Verify a Rust product\nSCOPE src/lib.rs src/mod.py\nVERIFY cargo test --locked --offline\n{suffix}")).unwrap();
        }
        if mode == "mismatch" {
            let path = store.join("core-wait.json");
            let mut state: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            state["checkpoints"][0]["outputs"] = json!(["src/mod.py"]);
            fs::write(path, serde_json::to_vec(&state).unwrap()).unwrap();
        }
        let b = pipeline_bridge(temp.path(), 0, "delivery-zero");
        let result = invoke(&b, json!({"project":"demo","unit":"checkpointed","cwd":repo,"role":"terra","stop_after":"build"}));
        assert_eq!(result["error"]["kind"], "invalid_argument", "{mode}: {result}");
        let state = orch::wait_state(&store).unwrap();
        assert_eq!(state.events.iter().filter(|e| e.kind == "dispatch").count(), 0);
        assert!(!temp.path().join("delivery-launches.log").exists());
    }
}
