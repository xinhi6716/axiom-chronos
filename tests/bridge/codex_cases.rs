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
    orch::record_review(&store, &["bridgeunit".into()], "claude-review", true).unwrap();
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
    let sol = invoke(
        &b,
        json!({"project":"demo","unit":"gatedunit","cwd":repo,"role":"sol","model":"gpt-6-sol"}),
    );
    assert_eq!(sol["error"]["kind"], "sol_not_allowed");
    let mut g = axiom::goal::open(
        home,
        "session",
        "[astra] build",
        Some(repo.to_str().unwrap()),
    )
    .unwrap();
    g.project = Some("demo".into());
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
        json!({"project":"demo","cwd":repo,"role":"astra","model":"gpt-6-astra","brief":"test","save_to":dest,"output_schema":{"type":"object"}}),
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
            ("overlapone", "GOAL SLOW\nSCOPE one.txt\nVERIFY echo ok\n"),
            ("overlaptwo", "GOAL SLOW\nSCOPE two.txt\nVERIFY echo ok\n"),
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
    let starts: Vec<f64> = events
        .iter()
        .filter(|e| e["method"] == "turn/start")
        .map(|e| e["time"].as_f64().unwrap())
        .collect();
    let ends: Vec<f64> = events
        .iter()
        .filter(|e| e["method"] == "fake/complete")
        .map(|e| e["time"].as_f64().unwrap())
        .collect();
    assert_eq!(starts.len(), 2);
    assert_eq!(ends.len(), 2);
    assert!(
        starts.iter().copied().fold(f64::NEG_INFINITY, f64::max)
            < ends.iter().copied().fold(f64::INFINITY, f64::min)
    );
}
#[test]
fn codex_role_and_validation_and_limit() {
    let temp = common::temp();
    let b = bridge(temp.path());
    let ok = invoke(
        &b,
        json!({"cwd":temp.path(),"role":"terra","brief":"hello"}),
    );
    assert_eq!(ok["model"], "gpt-6.10-terra", "{ok}");
    for args in [
        json!({"cwd":temp.path(),"brief":"hi"}),
        json!({"cwd":"missing","role":"terra","brief":"hi"}),
        json!({"cwd":temp.path(),"role":"terra"}),
    ] {
        assert_eq!(invoke(&b, args)["error"]["kind"], "invalid_argument");
    }
    let limited = call(
        &b,
        "codex_run",
        &json!({"cwd":temp.path(),"role":"terra","brief":"LIMIT"}),
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
            json!({"project":"demo","cwd":repo,"role":"astra","brief":"test","save_to":dest,"output_schema":{"type":"object"}}),
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
    fs::write(&wrapper, "import os,sys\nos.environ['FAKE_CODEX_LOG']=sys.argv[1]\np=sys.argv[2]\ns=open(p,encoding='utf8').read()\ns=s.replace(\"def complete(thread_id, turn_id, status='completed', error=None):\", \"def complete(thread_id, turn_id, status='completed', error=None):\\n    with open(os.environ['FAKE_CODEX_LOG'], 'a', encoding='utf8') as f: f.write(json.dumps({'method':'fake/complete','time':time.monotonic()})+'\\\\n')\")\nexec(compile(s,p,'exec'))\n").unwrap();
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
fn sol_requires_failing_fulltest() {
    let temp = common::temp();
    let (repo, store) = fixture(
        temp.path(),
        &[(
            "solunit",
            "GOAL build\nSCOPE sol-only.txt\nVERIFY echo ok\n",
        )],
    );
    let b = bridge(temp.path());
    let args =
        json!({"project":"demo","unit":"solunit","cwd":repo,"role":"sol","model":"gpt-6-sol"});
    assert_eq!(invoke(&b, args.clone())["error"]["kind"], "sol_not_allowed");
    orch::record_full(&store, &repo, Some("exit 1")).unwrap();
    let r = invoke(&b, args);
    assert_eq!(r["state"], "built", "{r}");
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
        json!({"project":"demo","cwd":repo,"role":"astra","model":"gpt-6-astra","brief":"test","save_to":dest,"output_schema":{"type":"object"}}),
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
        json!({"project":"demo","cwd":repo,"role":"astra","brief":"test","save_to":dest}),
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
        json!({"project":"demo","cwd":repo,"role":"astra","brief":"test","save_to":dest,"output_schema":{"type":"object"}}),
    );
    assert_eq!(r["coverage"], json!({"units":3}), "{r}");
}
