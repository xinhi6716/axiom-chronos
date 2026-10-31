use crate::common;
use serde_json::{json, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
    time::Instant,
};
fn command(home: &std::path::Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_axiom"));
    c.env("AXIOM_HOME", home).env("AXIOM_DATA_HOME", home);
    c.env_remove("ORCH_STORE");
    c
}
#[test]
fn wait_prepare_rejects_invalid_minimum_rounds() {
    let t = common::temp();
    let store = t.path().join("store");
    let repo = t.path().join("repo");
    std::fs::create_dir(&store).unwrap();
    std::fs::create_dir(&repo).unwrap();
    let brief = store.join("brief");
    std::fs::write(&brief, "GOAL test\nSCOPE a.txt\nOUTPUTS a.txt\nDELIVERY implementation\nVERIFY echo ok\n").unwrap();
    std::fs::write(store.join("units.tsv"), format!("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nt\tm\tpending\t\t\t\t{}\n", brief.display())).unwrap();
    let invoke = |extra: &[&str]| command(t.path()).args(["orch", "--json", "--store"]).arg(&store)
        .args(["wait", "prepare", "--task", "t", "--repo"]).arg(&repo)
        .args(["--issue", "i", "--proposal", "p"]).args(extra).output().unwrap();
    for args in [&["--min-rounds"][..], &["--min-rounds", "0"], &["--min-rounds", "-1"], &["--min-rounds", "abc"], &["--min-rounds", "4294967296"], &["--min-rounds", "--severity", "normal"], &["--min-rounds", "3", "--min-rounds", "5"]] {
        assert!(!invoke(args).status.success(), "{args:?}");
    }
    let valid = invoke(&["--min-rounds", "3"]);
    assert!(valid.status.success(), "{}", String::from_utf8_lossy(&valid.stderr));
    assert_eq!(axiom::orch::wait_state(&store).unwrap().checkpoints[0].min_rounds, 3);
}
#[test]
fn wish_and_research() {
    let temp = common::temp();
    let record = temp.path().join("record.json");
    for (args, expected, turns) in [
        (
            vec!["wish", "--astra", "--no-architect", "add", "--pending"],
            "/chronos:wish [no-architect] [astra] add --pending",
            None,
        ),
        (
            vec!["research", "RRAM", "因果"],
            "/chronos:research RRAM 因果",
            Some("40"),
        ),
    ] {
        let o = command(temp.path())
            .args(args)
            .env(
                "AXIOM_CODEX_CMD",
                serde_json::to_string(&common::fake("fake_codex_session")).unwrap(),
            )
            .env("FAKE_CODEX_RECORD", &record)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let v: Value = serde_json::from_slice(&o.stdout).unwrap();
        assert_eq!(v["status"], "completed");
        assert_eq!(v["goal"]["wish"], expected.split_once(' ').unwrap().1);
        assert!(v["workflow"]
            .as_str()
            .unwrap()
            .contains(v["goal"]["wish"].as_str().unwrap()));
        assert_eq!(
            v["max_turns"].as_u64(),
            turns.map(|s| s.parse::<u64>().unwrap())
        );
        assert!(v["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "Agent"));
        assert!(
            !record.exists(),
            "CLI bootstrap must not start an extra core model"
        );
        assert!(!temp.path().join("sessions").exists());
    }
    let o = command(temp.path())
        .args(["wish", "x"])
        .env("PATH", "")
        .env_remove("AXIOM_CODEX_CMD")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("codex is not on PATH"));
}
#[test]
fn wish_rejects_model_without_starting_session() {
    let t = common::temp();
    let record = t.path().join("record.json");
    let o = command(t.path())
        .args(["wish", "--model", "opus", "x"])
        .env(
            "AXIOM_CODEX_CMD",
            serde_json::to_string(&common::fake("fake_codex_session")).unwrap(),
        )
        .env("FAKE_CODEX_RECORD", &record)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("unrecognized option: --model"));
    assert!(!record.exists());
}
fn pipe(
    exe: &std::path::Path,
    home: &std::path::Path,
    sub: &str,
    input: &str,
) -> std::process::Output {
    let mut p = Command::new(exe)
        .arg(sub)
        .env("AXIOM_HOME", home)
        .env("AXIOM_DATA_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    p.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    p.wait_with_output().unwrap()
}
#[test]
fn bridge_hook_and_unicode_path() {
    let t = common::temp();
    let dir = t.path().join("空 白");
    std::fs::create_dir(&dir).unwrap();
    let exe = dir.join(if cfg!(windows) { "axiom.exe" } else { "axiom" });
    std::fs::copy(env!("CARGO_BIN_EXE_axiom"), &exe).unwrap();
    assert_eq!(
        Command::new(&exe).arg("--version").output().unwrap().stdout,
        format!("axiom-chronos {} (beta)\n", env!("CARGO_PKG_VERSION")).as_bytes()
    );
    let input = format!(
        "{}\n{}\n",
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}})
    );
    let o = pipe(&exe, t.path(), "bridge", &input);
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(
        text.contains("protocolVersion") && text.contains("codex_run"),
        "{text}"
    );
    let o = pipe(
        &exe,
        t.path(),
        "hook",
        "{\"hook_event_name\":\"Stop\",\"session_id\":\"none\"}",
    );
    assert!(o.status.success());
    axiom::goal::open(t.path(), "s", "wish", None).unwrap();
    let o = pipe(
        &exe,
        t.path(),
        "hook",
        &json!({"hook_event_name":"Stop","session_id":"s"}).to_string(),
    );
    assert!(o.status.success());
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["decision"], "block");
    let o = pipe(&exe, t.path(), "hook", &json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":"mcp__plugin_chronos_bridge__codex_run","tool_input":{}}).to_string());
    assert!(o.status.success());
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
    let fake = serde_json::to_string(&common::fake("fake_codex_session")).unwrap();
    let o = Command::new(&exe)
        .arg("doctor")
        .env("AXIOM_HOME", t.path())
        .env("AXIOM_DATA_HOME", t.path())
        .env("AXIOM_CODEX_CMD", &fake)
        .env("AXIOM_AGY_CMD", &fake)
        .env("AXIOM_WSL_CMD", &fake)
        .env(
            "AXIOM_PWSH_CMD",
            serde_json::to_string(&vec!["pwsh"]).unwrap(),
        )
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    let mut times = Vec::new();
    for _ in 0..20 {
        let now = Instant::now();
        assert!(pipe(&exe, t.path(), "hook", "{}").status.success());
        times.push(now.elapsed());
    }
    times.sort();
    println!("hook cold start median: {:?}", (times[9] + times[10]) / 2);
}
#[test]
fn orch_and_memory() {
    let t = common::temp();
    let store = t.path().join("store");
    for args in [
        vec!["init"],
        vec!["unit", "add", "u1", "--track", "core"],
        vec!["status"],
    ] {
        let o = command(t.path())
            .arg("orch")
            .arg("--store")
            .arg(&store)
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
    let o = command(t.path())
        .args(["orch", "--json", "status"])
        .env("ORCH_STORE", &store)
        .output()
        .unwrap();
    assert!(o.status.success());
    assert!(String::from_utf8_lossy(&o.stdout).contains("u1"));
    let o = command(t.path())
        .args(["orch", "unit", "get", "nope"])
        .env("ORCH_STORE", &store)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("error:"));
    let o = command(t.path()).args(["orch", "status"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("orch: --store or ORCH_STORE is required"));
    axiom::memory::Memory::open(t.path()).unwrap();
    let db = rusqlite::Connection::open(t.path().join("memory/memory.db")).unwrap();
    db.execute("INSERT INTO entries (project, unit, title, summary, body, symbols, wish, files, project_dir, created) VALUES ('demo', 'u1', 'searchable title', 'searchable summary', 'searchable body', '[]', 'wish', '[]', '', 0)", []).unwrap();
    let o = command(t.path())
        .args(["memory", "search", "searchable", "--project", "demo"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stdout).contains("searchable"));
    for args in [vec!["memory", "get", "1"], vec!["memory", "digest", "demo"]] {
        let o = command(t.path()).args(args).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let _: Value = serde_json::from_slice(&o.stdout).unwrap();
    }
    let o = command(t.path())
        .args([
            "memory",
            "search",
            "searchable",
            "--project",
            "other",
            "--limit",
            "1",
        ])
        .output()
        .unwrap();
    assert!(o.status.success());
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["entries"].as_array().unwrap().len(), 0);
    db.execute("INSERT INTO entries (project, unit, title, summary, body, symbols, wish, files, project_dir, created) VALUES ('demo', 'u2', 'searchable second', '', '', '', '', '[]', '', 1)", []).unwrap();
    let o = command(t.path())
        .args([
            "memory",
            "search",
            "searchable",
            "--project",
            "demo",
            "--limit",
            "1",
        ])
        .output()
        .unwrap();
    assert!(o.status.success());
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["entries"].as_array().unwrap().len(), 1);
    let bad = t.path().join("file");
    std::fs::write(&bad, "x").unwrap();
    let o = command(&bad).args(["memory", "get", "1"]).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).starts_with("axiom: "));
}
#[test]
fn wish_usage_and_overrides() {
    let t = common::temp();
    for (args, message) in [
        (
            vec!["wish"],
            "axiom: wish: say what you want, e.g. axiom wish add a --stats flag to cli.py",
        ),
        (
            vec!["wish", "--no-architect"],
            "axiom: wish: say what you want, e.g. axiom wish add a --stats flag to cli.py",
        ),
        (
            vec!["unknown"],
            "axiom: unknown command or arguments: unknown",
        ),
        (vec![], "axiom: missing command"),
    ] {
        let o = command(t.path()).args(args).output().unwrap();
        assert_eq!(o.status.code(), Some(2));
        assert_eq!(String::from_utf8_lossy(&o.stderr).trim_end(), message);
    }
    let o = command(t.path())
        .args(["wish", "--project", "does-not-exist", "x"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("--project is not a directory"));
    let record = t.path().join("record.json");
    let o = command(t.path())
        .args(["wish", "--project"])
        .arg(t.path())
        .args(["--max-turns", "7", "text", "--model", "--max-turns"])
        .env(
            "AXIOM_CODEX_CMD",
            serde_json::to_string(&common::fake("fake_codex_session")).unwrap(),
        )
        .env("FAKE_CODEX_RECORD", &record)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["goal"]["wish"], "text --model --max-turns");
    assert_eq!(v["goal"]["cwd"], t.path().to_string_lossy().as_ref());
    assert_eq!(v["max_turns"], 7);
    assert!(!record.exists());
}

#[test]
fn creative_and_resume_usage() {
    let t = common::temp();
    let record = t.path().join("record");
    for (args, expected) in [
        (vec!["wish", "--creative"], "/chronos:wish [creative]"),
        (
            vec!["wish", "--creative", "invent", "x"],
            "/chronos:wish [creative] invent x",
        ),
        (
            vec!["wish", "--creative", "--no-architect", "--astra", "x"],
            "/chronos:wish [creative] x",
        ),
    ] {
        let o = command(t.path())
            .args(args)
            .env(
                "AXIOM_CODEX_CMD",
                serde_json::to_string(&common::fake("fake_codex_session")).unwrap(),
            )
            .env("FAKE_CODEX_RECORD", &record)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(o.status.success());
        let v: Value = serde_json::from_slice(&o.stdout).unwrap();
        assert_eq!(v["goal"]["wish"], expected.split_once(' ').unwrap().1);
        assert!(
            !record.exists(),
            "creative bootstrap must not start a core thread"
        );
    }
    for args in [
        vec!["wish", "--resume", "id", "text"],
        vec!["wish", "text", "--resume", "id"],
        vec!["wish", "--resume", "id", "--project", "."],
        vec!["wish", "--creative", "--resume", "id"],
        vec!["wish", "--resume", "id", "--astra"],
        vec!["wish", "--resume", "id", "--no-architect"],
    ] {
        let o = command(t.path()).args(args).output().unwrap();
        assert_eq!(o.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&o.stderr).contains("axiom: --resume takes only a session id")
        );
    }
}

#[test]
fn wait_cli_persists_core_verdicts_and_bound_explicit_recovery() {
    let t = common::temp();
    let store = t.path().join("wait-store");
    std::fs::create_dir(&store).unwrap();
    let file = t.path().join("input.json");
    let invoke = |action: &str, value: &Value, extra: &[&str]| {
        std::fs::write(&file, serde_json::to_vec(value).unwrap()).unwrap();
        command(t.path())
            .args(["orch", "--json", "--store"])
            .arg(&store)
            .args(["wait", action, "--file"])
            .arg(&file)
            .args(extra)
            .output()
            .unwrap()
    };
    let c = json!({"task":"task","module":"m","round":1,"owner":"terra","dependencies":[],"outputs":["out"],"evidence":["tests"],"next_action":"build","issue":"i","proposal":"p","evidence_version":1,"contract":"sha256:contract","repo":"","severity":"normal","findings":[]});
    assert!(invoke("checkpoint", &c, &[]).status.success());
    axiom::orch::commit_dispatch(&store, "task").unwrap();
    let state = axiom::orch::wait_state(&store).unwrap();
    let first = &state.events[0].id;
    let make = |kind: &str, actor: &str, dispatch: &str| json!({"id":format!("{dispatch}-{kind}"),"task":"task","round":1,"issue":"i","proposal":"p","evidence_version":1,"kind":kind,"evidence":"external process status confirmed","actor":actor,"dispatch_id":dispatch,"contract":"sha256:contract","artifact":"sha256:artifact"});
    let recovery = make("recovery", "core", first);
    assert!(!invoke("event", &recovery, &[]).status.success());
    assert!(!invoke("recover", &recovery, &["--confirmed", "maybe"])
        .status
        .success());
    assert!(
        invoke("recover", &recovery, &["--confirmed", "not_started"])
            .status
            .success()
    );
    axiom::orch::commit_dispatch(&store, "task").unwrap();
    assert!(!invoke("event", &make("build", "worker", first), &[])
        .status
        .success());
    let state = axiom::orch::wait_state(&store).unwrap();
    let second = &state.events.last().unwrap().id;
    assert!(!invoke("event", &make("completion", "core", second), &[])
        .status
        .success());
    for (kind, actor) in [
        ("build", "worker"),
        ("assembly", "sol"),
        ("verification", "astra-verify"),
        ("core_approve", "core"),
        ("completion", "core"),
        ("release", "core"),
    ] {
        assert!(
            invoke("event", &make(kind, actor, second), &[])
                .status
                .success(),
            "{kind}"
        );
    }
    let output = command(t.path())
        .args(["orch", "--json", "--store"])
        .arg(&store)
        .args(["wait", "show"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let state: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(state["schema_version"], 2);
    assert_eq!(state["events"].as_array().unwrap().len(), 9);
    let persisted = axiom::orch::wait_state(&store).unwrap();
    assert!(axiom::orch::may_finish(&persisted, "task"));
    assert!(!axiom::orch::product_policy_finished(&persisted, "run"));
    assert!(!invoke("event", &make("release", "core", second), &[])
        .status
        .success());
}

#[test]
fn wait_cli_explicitly_migrates_legacy_history_and_rejects_invalid_inputs() {
    let t = common::temp();
    let store = t.path().join("store");
    std::fs::create_dir(&store).unwrap();
    let file = t.path().join("checkpoint.json");
    let checkpoint = json!({"task":"task","module":"m","round":1,"owner":"terra","dependencies":[],"outputs":["out"],"evidence":["tests"],"next_action":"build","issue":"i","proposal":"p","evidence_version":1,"contract":"sha256:contract","repo":"","severity":"normal","findings":[]});
    std::fs::write(&file, serde_json::to_vec(&checkpoint).unwrap()).unwrap();
    let invoke = |args: &[&str]| {
        command(t.path())
            .args(["orch", "--json", "--store"])
            .arg(&store)
            .args(args)
            .output()
            .unwrap()
    };
    assert!(command(t.path())
        .args(["orch", "--json", "--store"])
        .arg(&store)
        .args(["wait", "checkpoint", "--file"])
        .arg(&file)
        .output().unwrap().status.success());
    axiom::orch::commit_dispatch(&store, "task").unwrap();
    let path = store.join("core-wait.json");
    let archive = store.join("core-wait.v1.json");
    let mut legacy: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    legacy["schema_version"] = 1.into();
    legacy.as_object_mut().unwrap().remove("product_runs");
    legacy.as_object_mut().unwrap().remove("engineering_enrolled");
    legacy["checkpoints"][0].as_object_mut().unwrap().remove("min_rounds");
    let bytes = serde_json::to_vec_pretty(&legacy).unwrap();
    std::fs::write(&path, &bytes).unwrap();

    for args in [vec!["wait", "show"], vec!["wait", "product-checkpoint", "--run", "run"]] {
        let rejected = invoke(&args);
        assert_eq!(rejected.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&rejected.stderr).contains("requires explicit wait migrate"));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(!archive.exists());
    }
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(!archive.exists());
    let migrated = invoke(&["wait", "migrate"]);
    assert!(migrated.status.success(), "{}", String::from_utf8_lossy(&migrated.stderr));
    let state: Value = serde_json::from_slice(&migrated.stdout).unwrap();
    assert_eq!(state["schema_version"], 2);
    assert_eq!(state["events"], legacy["events"]);
    let mut expected_checkpoints = legacy["checkpoints"].clone();
    expected_checkpoints[0]["min_rounds"] = 5.into();
    assert_eq!(state["checkpoints"], expected_checkpoints);
    assert_eq!(state["product_runs"], json!([]));
    assert_eq!(std::fs::read(&archive).unwrap(), bytes);
    let after = std::fs::read(&path).unwrap();
    assert!(invoke(&["wait", "migrate"]).status.success());
    assert_eq!(std::fs::read(&path).unwrap(), after);
    assert!(invoke(&["wait", "show"]).status.success());

    let product = invoke(&["wait", "product-checkpoint", "--run", "run"]);
    assert!(product.status.success(), "{}", String::from_utf8_lossy(&product.stderr));
    let state: Value = serde_json::from_slice(&product.stdout).unwrap();
    assert_eq!(state["schema_version"], 2);
    assert_eq!(state["events"], legacy["events"]);
    assert_eq!(state["product_runs"][0]["id"], "run");
    assert_eq!(state["product_runs"][0]["rounds"].as_array().unwrap().len(), 1);
    assert_eq!(state["product_runs"][0]["rounds"][0]["round"], 1);
    assert_eq!(state["product_runs"][0]["rounds"][0]["event_start"], legacy["events"].as_array().unwrap().len());
    assert!(state["product_runs"][0]["rounds"][0]["transition"].is_null());
    let product_bytes = std::fs::read(&path).unwrap();
    assert!(!invoke(&["wait", "product-checkpoint", "--run", "run"]).status.success());
    assert_eq!(std::fs::read(&path).unwrap(), product_bytes);
    assert_eq!(std::fs::read(&archive).unwrap(), bytes);

    std::fs::write(&path, &bytes).unwrap();
    std::fs::write(&archive, b"conflicting history").unwrap();
    assert!(!invoke(&["wait", "migrate"]).status.success());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(std::fs::read(&archive).unwrap(), b"conflicting history");
    legacy["schema_version"] = 99.into();
    let unknown = serde_json::to_vec(&legacy).unwrap();
    let unknown_store = t.path().join("unknown-store");
    std::fs::create_dir(&unknown_store).unwrap();
    let unknown_path = unknown_store.join("core-wait.json");
    let unknown_archive = unknown_store.join("core-wait.v1.json");
    std::fs::write(&unknown_path, &unknown).unwrap();
    let rejected = command(t.path())
        .args(["orch", "--json", "--store"])
        .arg(&unknown_store)
        .args(["wait", "migrate"])
        .output().unwrap();
    assert_eq!(rejected.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("unsupported or ambiguous legacy schema"));
    assert_eq!(std::fs::read(&unknown_path).unwrap(), unknown);
    assert!(!unknown_archive.exists());
    assert_eq!(std::fs::read(&archive).unwrap(), b"conflicting history");
}

#[test]
fn bridge_caller_host_round_trip_without_model_dispatch() {
    use std::io::{BufRead, BufReader};
    use std::sync::mpsc;
    use std::time::Duration;

    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let t = common::temp();
    // A launch guard, not a model fake: axiom cannot serve Codex app-server RPC.
    // Attachment and the local Host operation must not launch a model process.
    let mut child = ChildGuard(command(t.path())
        .arg("bridge")
        .env("AXIOM_CODEX_CMD", serde_json::to_string(&[env!("CARGO_BIN_EXE_axiom")]).unwrap())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn().unwrap());
    let stdout = child.0.stdout.take().unwrap();
    let (send, recv) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if send.send(line).is_err() { break; }
        }
    });
    let mut input = child.0.stdin.take().unwrap();
    let mut rpc = |id: u64, args: Value| {
        writeln!(input, "{}", json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"session","arguments":args}})).unwrap();
        input.flush().unwrap();
        let line = recv.recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|e| panic!("Host RPC {id} did not return: {e}"))
            .unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], id);
        assert!(response.get("error").is_none(), "{response}");
        response["result"].clone()
    };
    let absent = rpc(1, json!({"action":"status"}));
    assert_eq!(absent["isError"], true);
    assert_eq!(absent["structuredContent"]["error"]["kind"], "not_found");
    let begin = rpc(2, json!({"action":"begin","skill":"wish","project":t.path(),"text":"bounded host smoke","max_turns":4}));
    assert_eq!(begin["isError"], false, "{begin}");
    assert_eq!(begin["structuredContent"]["status"], "completed");
    let id = begin["structuredContent"]["session"].as_str().unwrap();
    assert!(!id.is_empty());
    let denied = rpc(3, json!({"action":"call","tool":"codex_run","arguments":{"role":"terra","actor":"dispatcher"}}));
    assert_eq!(denied["isError"], true);
    assert_eq!(denied["structuredContent"]["error"]["kind"], "permission_denied");
    let review = rpc(4, json!({"action":"call","tool":"goal","arguments":{"mode":"review"}}));
    assert_eq!(review["isError"], false, "{review}");
    assert_eq!(review["structuredContent"]["status"], "completed");
    let status = rpc(5, json!({"action":"status"}));
    assert_eq!(status["isError"], false);
    assert_eq!(status["structuredContent"]["session"], id);
    assert_eq!(status["structuredContent"]["goal"]["mode"], "review");
    assert_eq!(status["structuredContent"]["goal"]["mode_locked"], true);
    assert_eq!(rpc(6, json!({"action":"stop"}))["isError"], false);
    let stopped = rpc(7, json!({"action":"status"}));
    assert_eq!(stopped["isError"], true);
    assert_eq!(stopped["structuredContent"]["error"]["kind"], "not_found");
    drop(rpc);
    drop(input);
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() { break status; }
        assert!(Instant::now() < deadline, "Host did not shut down within 10 seconds");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success(), "Host exited with {status}");
    assert!(!t.path().join("sessions").exists(), "no model execution receipt expected");
}
