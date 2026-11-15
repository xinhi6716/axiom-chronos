mod common;
use axiom::bridge::{call, Bridge, Config};
use serde_json::{json, Value};
use std::fs;

#[test]
fn schedule_claims_roots_and_replays_cursor() {
    let temp = common::temp();
    let home = temp.path();
    let store = home.join("orchestrate/demo");
    fs::create_dir_all(&store).unwrap();
    fs::write(
        store.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n",
    )
    .unwrap();
    fs::write(store.join("modules.md"), "## fast\nPURPOSE x\nPATHS fast/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## slow\nPURPOSE x\nPATHS slow/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## later\nPURPOSE x\nPATHS later/\nDEPENDS fast\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
    let bridge = Bridge::new(Config {
        data_home: Some(home.into()),
        codex: Ok(common::fake_codex(&[])),
        wsl: None,
        on_windows: false,
    });
    let run = |args: Value| {
        let result = call(&bridge, "orch", &json!({"project":"demo", "args":args}));
        let body = &result["structuredContent"];
        assert_eq!(body["status"], "completed", "{body}");
        serde_json::from_str::<Value>(body["stdout"].as_str().unwrap()).unwrap()
    };
    let first = run(json!(["--json", "schedule", "next", "--timeout", "0"]));
    assert_eq!(first["start"], json!(["fast", "slow"]));
    assert_eq!(first["waiting"][0]["module"], "later");
    let replay = run(json!([
        "--json",
        "schedule",
        "next",
        "--cursor",
        "0",
        "--timeout",
        "0"
    ]));
    assert_eq!(replay["start"], first["start"]);
    assert_eq!(replay["cursor"], first["cursor"]);
}

#[test]
fn old_verified_event_does_not_hide_regressed_module() {
    use axiom::orch::{events, lifecycle::scheduler};
    let temp = common::temp();
    let store = temp.path().join("orchestrate/demo");
    fs::create_dir_all(&store).unwrap();
    fs::write(
        store.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n",
    )
    .unwrap();
    fs::write(
        store.join("modules.md"),
        "## fast\nPURPOSE x\nPATHS fast/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
    )
    .unwrap();
    events::append_event(
        &store,
        events::MODULE_VERIFIED,
        Some("fast"),
        None,
        json!({}),
    )
    .unwrap();
    assert_eq!(scheduler::claim_ready(&store).unwrap(), vec!["fast"]);
}

#[test]
fn check_transition_emits_unit_state() {
    use axiom::orch::{self, events, Evidence};
    let temp = common::temp();
    let store = temp.path().join("orchestrate/demo");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&store).unwrap();
    fs::create_dir_all(&repo).unwrap();
    let brief = store.join("u.brief");
    fs::write(&brief, "GOAL x\nSCOPE x\nVERIFY echo ok\nRISK trivial\n").unwrap();
    fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tpending\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    fs::write(
        store.join("modules.md"),
        "## m\nPURPOSE x\nPATHS x\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
    )
    .unwrap();
    let row = |kind: &str, exit_code| Evidence {
        kind: kind.into(),
        command: "worker".into(),
        exit_code,
        commit: "tree:1".into(),
        ts: "2026-01-01T00:00:00Z".into(),
    };
    orch::append_evidence(&store, "u", &row("run", 0)).unwrap();
    orch::append_check(&store, "u", &repo, &row("verify", 1)).unwrap();
    let events = events::read_events(&store, 0).unwrap();
    assert!(
        events.iter().any(|e| e.kind == events::UNIT_STATE
            && e.data["state"] == "failed"
            && e.data["previous"] == "pending"),
        "{events:?}"
    );
}

#[test]
fn releasing_last_unit_claim_notifies_a_verified_module() {
    use axiom::orch::{self, events, Evidence};
    let temp = common::temp();
    let store = temp.path().join("orchestrate/demo");
    fs::create_dir_all(&store).unwrap();
    let brief = store.join("u.brief");
    fs::write(&brief, "GOAL x\nSCOPE x\nVERIFY echo ok\nRISK trivial\n").unwrap();
    fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tverified\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    fs::write(
        store.join("modules.md"),
        "## m\nPURPOSE x\nPATHS x\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
    )
    .unwrap();
    fs::write(
        store.join("claims.tsv"),
        "unit\tattempt\tscope\tstarted\tmodule\twish\tpid\tpeers\nu\ta1\tx\tt\tm\tdemo\t0\t\n",
    )
    .unwrap();
    for (kind, command) in [
        ("run", "terra"),
        ("verify", "verify"),
        ("accept", "accept"),
        ("review", "chronos:reviewer"),
    ] {
        orch::append_evidence(
            &store,
            "u",
            &Evidence {
                kind: kind.into(),
                command: command.into(),
                exit_code: 0,
                commit: "tree:1".into(),
                ts: "2026-01-01T00:00:00Z".into(),
            },
        )
        .unwrap();
    }
    events::append_event(&store, events::MODULE_CLAIMED, Some("m"), None, json!({})).unwrap();
    orch::release_claim(&store, "u", "a1").unwrap();
    assert!(events::read_events(&store, 0)
        .unwrap()
        .iter()
        .any(|e| e.kind == events::MODULE_VERIFIED));
}

#[test]
fn verified_fast_module_unlocks_while_slow_worker_is_running() {
    use axiom::orch;
    use std::{
        process::Command,
        sync::Arc,
        thread,
        time::{Duration, Instant},
    };
    let temp = common::temp();
    let home = temp.path();
    let repo = home.join("repo");
    let store = home.join("orchestrate/demo");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(store.join("inbox")).unwrap();
    for args in [
        vec!["init", "-q"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
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
    let mut units = String::from("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n");
    for (unit, delay) in [
        ("fast", 100),
        ("slow", 4000),
        ("later", 100),
        ("rogue", 100),
    ] {
        let brief = store.join(format!("{unit}.brief"));
        let write = if unit == "rogue" {
            "outside.txt".to_string()
        } else {
            format!("{unit}.txt")
        };
        fs::write(&brief, format!("GOAL build\nSCOPE {unit}.txt\nVERIFY echo ok\nRISK trivial\nFAKE_DELAY_MS {delay}\nFAKE_WRITE {write}\n")).unwrap();
        units.push_str(&format!(
            "{unit}\t{unit}\tpending\t\t\t\t{}\n",
            brief.display()
        ));
    }
    fs::write(store.join("units.tsv"), units).unwrap();
    fs::write(store.join("modules.md"), "## fast\nPURPOSE x\nPATHS fast.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## slow\nPURPOSE x\nPATHS slow.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## later\nPURPOSE x\nPATHS later.txt\nDEPENDS fast\nCONTRACT x\nACCEPT test `echo ok`\n## rogue\nPURPOSE x\nPATHS rogue.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
    let bridge = Arc::new(Bridge::new(Config {
        data_home: Some(home.into()),
        codex: Ok(common::fake_codex(&[])),
        wsl: None,
        on_windows: false,
    }));
    let schedule = |cursor: Option<u64>| {
        let mut args = vec![
            "--json".to_string(),
            "schedule".into(),
            "next".into(),
            "--timeout".into(),
            "0".into(),
        ];
        if let Some(c) = cursor {
            args.extend(["--cursor".into(), c.to_string()]);
        }
        let body = call(&bridge, "orch", &json!({"project":"demo","args":args}))
            ["structuredContent"]
            .clone();
        assert_eq!(body["status"], "completed", "{body}");
        serde_json::from_str::<Value>(body["stdout"].as_str().unwrap()).unwrap()
    };
    let first = schedule(None);
    assert_eq!(first["start"], json!(["fast", "slow", "rogue"]));
    let spawn = |unit: &'static str| {
        let b = Arc::clone(&bridge);
        let repo = repo.clone();
        thread::spawn(move || {
            call(
                &b,
                "codex_run",
                &json!({"project":"demo","unit":unit,"role":"terra","cwd":repo}),
            )["structuredContent"]
                .clone()
        })
    };
    let slow = spawn("slow");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !fs::read_to_string(store.join("claims.tsv"))
        .unwrap_or_default()
        .contains("slow\t")
    {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(20));
    }
    let fast = spawn("fast").join().unwrap();
    assert_eq!(fast["status"], "completed", "{fast}");
    let before = schedule(Some(first["cursor"].as_u64().unwrap()));
    assert_eq!(before["start"], json!([]));
    let verify = orch::evidence(&store, "fast")
        .unwrap()
        .into_iter()
        .rev()
        .find(|e| e.kind == "verify")
        .unwrap();
    let verdict = store.join("fast.verdict.json");
    fs::write(
        &verdict,
        json!({"unit":"fast","tree":verify.commit,"verdict":"pass","findings":[]}).to_string(),
    )
    .unwrap();
    let review = call(&bridge, "orch", &json!({"project":"demo","args":["--json","review","fast","--repo",repo,"--reviewer","chronos:reviewer","--verdict-file",verdict]}))["structuredContent"].clone();
    assert_eq!(review["status"], "completed", "{review}");
    let next = schedule(Some(before["cursor"].as_u64().unwrap()));
    assert_eq!(next["start"], json!(["later"]));
    assert!(next["running"].as_array().unwrap().contains(&json!("slow")));
    assert!(
        !slow.is_finished(),
        "slow module finished before dependent was started"
    );
    let rogue = spawn("rogue").join().unwrap();
    assert_eq!(rogue["error"]["kind"], "outside_scope", "{rogue}");
    assert_eq!(rogue["outside_scope"], json!(["outside.txt"]));
    let slow_result = slow.join().unwrap();
    assert_eq!(slow_result["status"], "completed", "{slow_result}");
    assert_eq!(fast["outside_scope_count"], 0);
    assert_eq!(slow_result["outside_scope_count"], 0);
}

#[test]
fn unchanged_accept_preserves_reviewed_module_gate() {
    use axiom::orch::{self, lifecycle::scheduler, Evidence};
    let temp = common::temp();
    let store = temp.path().join("orchestrate/demo");
    fs::create_dir_all(&store).unwrap();
    let brief = store.join("u.brief");
    fs::write(&brief, "GOAL x\nSCOPE x\nVERIFY echo ok\nRISK trivial\n").unwrap();
    fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tverified\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    fs::write(
        store.join("modules.md"),
        "## m\nPURPOSE x\nPATHS x\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
    )
    .unwrap();
    for (kind, command) in [
        ("run", "terra"),
        ("verify", "verify"),
        ("accept", "accept"),
        ("review", "chronos:reviewer"),
        ("accept", "accept"),
    ] {
        orch::append_evidence(
            &store,
            "u",
            &Evidence {
                kind: kind.into(),
                command: command.into(),
                exit_code: 0,
                commit: "tree:1".into(),
                ts: "2026-01-01T00:00:00Z".into(),
            },
        )
        .unwrap();
    }
    assert!(scheduler::module_verified(&store, "m").unwrap());
}

#[test]
fn hooks_only_allow_claimed_background_dispatchers_and_core_scheduling() {
    use axiom::{goal, hook, orch};
    let temp = common::temp();
    let home = temp.path();
    let repo = home.join("repo");
    fs::create_dir_all(&repo).unwrap();
    let goal = goal::open(home, "s", "test", Some(repo.to_str().unwrap())).unwrap();
    goal::set_mode(home, "s", "change").unwrap();
    let store = orch::store_for(home, goal.project.as_deref().unwrap()).unwrap();
    fs::create_dir_all(&store).unwrap();
    fs::write(
        store.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n",
    )
    .unwrap();
    fs::write(store.join("modules.md"), "## fast\nPURPOSE x\nPATHS fast/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## slow\nPURPOSE x\nPATHS slow/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
    let event = |subagent: &str, prompt: &str| json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":"Agent","tool_input":{"subagent_type":subagent,"prompt":prompt,"run_in_background":true}});
    let denied = |event: &Value| {
        hook::handle(event, Some(home), false)
            .is_some_and(|v| v["hookSpecificOutput"]["permissionDecision"] == "deny")
    };
    fs::remove_file(store.join("modules.md")).unwrap();
    assert!(denied(&event("chronos:dispatcher", "Build fast")));
    fs::write(store.join("modules.md"), "## fast\nPURPOSE x\nPATHS fast/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## slow\nPURPOSE x\nPATHS slow/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
    assert!(denied(&event("chronos:dispatcher", "Build fast")));
    orch::lifecycle::scheduler::claim(&store, "fast").unwrap();
    assert!(!denied(&event("chronos:dispatcher", "Build fast")));
    assert!(denied(&event("chronos:dispatcher", "Build slow")));
    assert!(denied(&event("chronos:dispatcher", "Build unknown")));
    assert!(denied(&event("chronos:memory-keeper", "Build fast")));
    for (args, expected) in [
        (json!(["schedule", "next"]), true),
        (json!(["--json", "schedule", "claim", "fast"]), true),
        (json!(["setup", "status"]), true),
        (
            json!(["schedule", "release", "fast", "--reason", "x"]),
            false,
        ),
        (json!(["activity", "start"]), false),
    ] {
        let e = json!({"hook_event_name":"PreToolUse","session_id":"s","agent_id":"a","tool_name":"mcp__plugin_chronos_bridge__orch","tool_input":{"args":args}});
        assert_eq!(denied(&e), expected, "{e}");
    }
}

#[test]
fn dependent_waits_for_successful_accept_review_and_all_briefs() {
    use axiom::orch::{events, lifecycle::scheduler};
    let temp = common::temp();
    let store = temp.path().join("orchestrate/demo");
    fs::create_dir_all(store.join("reports")).unwrap();
    fs::write(store.join("modules.md"), "## a\nPURPOSE x\nPATHS a/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## b\nPURPOSE x\nPATHS b/\nDEPENDS a\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
    let brief = store.join("u.brief");
    fs::write(&brief, "GOAL x\nSCOPE a/x\nVERIFY echo ok\nRISK trivial\n").unwrap();
    let units = |second: &str| {
        format!("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\ta\tbuilt\t\t\t\t{}\nv\ta\tbuilt\t\t\t\t{second}\n",brief.display())
    };
    fs::write(store.join("units.tsv"), units("")).unwrap();
    let evidence = |name: &str, accept: i32, review: Option<i32>| {
        let mut data=format!("kind\tcommand\texit_code\tcommit\tts\nrun\tterra\t0\ttree:1\tt\nverify\tverify\t0\ttree:1\tt\naccept\taccept\t{accept}\ttree:1\tt\n");
        if let Some(code) = review {
            data.push_str(&format!("review\tchronos:reviewer\t{code}\ttree:1\tt\n"));
        }
        fs::write(
            store.join("reports").join(format!("{name}.evidence.tsv")),
            data,
        )
        .unwrap();
    };
    assert_eq!(scheduler::claim_ready(&store).unwrap(), vec!["a"]);
    evidence("u", 1, Some(0));
    assert!(scheduler::claim_ready(&store).unwrap().is_empty());
    evidence("u", 0, None);
    assert!(scheduler::claim_ready(&store).unwrap().is_empty());
    evidence("u", 0, Some(1));
    assert!(scheduler::claim_ready(&store).unwrap().is_empty());
    evidence("u", 0, Some(0));
    evidence("v", 0, Some(0));
    assert!(scheduler::claim_ready(&store).unwrap().is_empty());
    fs::write(store.join("units.tsv"), units(brief.to_str().unwrap())).unwrap();
    events::append_event(&store, events::MODULE_VERIFIED, Some("a"), None, json!({})).unwrap();
    events::append_event(&store, events::MODULE_VERIFIED, Some("a"), None, json!({})).unwrap();
    assert_eq!(scheduler::claim_ready(&store).unwrap(), vec!["b"]);
    assert!(scheduler::claim_ready(&store).unwrap().is_empty());
}

#[test]
fn stopped_and_cutoff_goals_hold_claims_until_resumed() {
    use axiom::{
        goal,
        orch::{self, lifecycle::scheduler},
    };
    use std::time::Duration;
    let temp = common::temp();
    let home = temp.path();
    let repo = home.join("repo");
    fs::create_dir_all(&repo).unwrap();
    let g = goal::open(home, "s", "test", Some(repo.to_str().unwrap())).unwrap();
    let store = orch::store_for(home, g.project.as_deref().unwrap()).unwrap();
    fs::create_dir_all(&store).unwrap();
    fs::write(
        store.join("modules.md"),
        "## a\nPURPOSE x\nPATHS a/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
    )
    .unwrap();
    fs::write(
        store.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n",
    )
    .unwrap();
    goal::interject(home, "s", "stop").unwrap();
    goal::classify(home, "s", "stop").unwrap();
    let stopped = scheduler::next(&store, None, Duration::ZERO).unwrap();
    assert_eq!(stopped.held.as_deref(), Some("stopped"));
    assert!(stopped.start.is_empty());
    assert_eq!(scheduler::claim(&store, "a").unwrap_err().kind, "held");
    goal::reopen(home, "s").unwrap();
    goal::cut_off(home, "s", 2).unwrap();
    let cut = scheduler::next(&store, None, Duration::ZERO).unwrap();
    assert_eq!(cut.held.as_deref(), Some("goal_blocked"));
    assert!(cut.start.is_empty());
    goal::reopen(home, "s").unwrap();
    assert_eq!(
        scheduler::next(&store, None, Duration::ZERO).unwrap().start,
        vec!["a"]
    );
}

#[test]
fn disk_space_hold_masks_claims_until_reopened() {
    use axiom::{
        goal,
        orch::{self, events, lifecycle::scheduler},
    };
    use std::time::Duration;

    let temp = common::temp();
    let home = temp.path();
    let repo = home.join("repo");
    fs::create_dir_all(&repo).unwrap();
    let g = goal::open(home, "s", "test", Some(repo.to_str().unwrap())).unwrap();
    let store = orch::store_for(home, g.project.as_deref().unwrap()).unwrap();
    fs::create_dir_all(&store).unwrap();
    fs::write(
        store.join("modules.md"),
        "## a\nPURPOSE x\nPATHS a/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
    )
    .unwrap();
    fs::write(
        store.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n",
    )
    .unwrap();
    let line = "fake disk: free_bytes=0 reserve_bytes=0";
    goal::disk_pause(home, "s", line).unwrap().unwrap();
    let held = scheduler::next(&store, None, Duration::ZERO).unwrap();
    assert_eq!(held.held.as_deref(), Some("disk_space"));
    assert!(held.start.is_empty());
    assert!(held.running.is_empty());
    assert!(!events::read_events(&store, 0)
        .unwrap()
        .iter()
        .any(|event| event.kind == events::MODULE_CLAIMED));

    goal::reopen(home, "s").unwrap();
    let resumed = scheduler::next(&store, None, Duration::ZERO).unwrap();
    assert_eq!(resumed.start, vec!["a"]);

    goal::disk_pause(home, "s", line).unwrap().unwrap();
    let held_replay = scheduler::next(&store, None, Duration::ZERO).unwrap();
    assert_eq!(held_replay.held.as_deref(), Some("disk_space"));
    assert!(held_replay.start.is_empty());
    assert_eq!(held_replay.running, vec!["a"]);
}

#[test]
fn release_cancels_activities_and_cursor_handles_truncated_replay() {
    use axiom::orch::{events, lifecycle::scheduler};
    use std::time::Duration;
    let temp = common::temp();
    let home = temp.path();
    let store = home.join("orchestrate/demo");
    fs::create_dir_all(&store).unwrap();
    fs::write(
        store.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n",
    )
    .unwrap();
    fs::write(
        store.join("modules.md"),
        "## a\nPURPOSE x\nPATHS a/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
    )
    .unwrap();
    let bridge = Bridge::new(Config {
        data_home: Some(home.into()),
        codex: Ok(common::fake_codex(&[])),
        wsl: None,
        on_windows: false,
    });
    let run = |args: Value| {
        call(&bridge, "orch", &json!({"project":"demo","args":args}))["structuredContent"].clone()
    };
    let first = run(json!(["--json", "schedule", "next", "--timeout", "0"]));
    assert_eq!(first["status"], "completed");
    let a = run(json!([
        "--json",
        "activity",
        "start",
        "--module",
        "a",
        "--actor",
        "dispatcher",
        "--phase",
        "brief"
    ]));
    assert_eq!(a["status"], "completed");
    assert_eq!(events::open_activities(&store).unwrap().len(), 1);
    let release = run(json!([
        "--json", "schedule", "release", "a", "--reason", "retry"
    ]));
    assert_eq!(release["status"], "completed");
    assert!(events::open_activities(&store).unwrap().is_empty());
    assert!(events::read_events(&store, 0)
        .unwrap()
        .iter()
        .any(|e| e.kind == events::ACTIVITY_FINISH && e.data["outcome"] == "cancelled"));
    assert!(scheduler::claim_ready(&store).unwrap().is_empty());
    scheduler::claim(&store, "a").unwrap();
    for _ in 0..21 {
        events::append_event(&store, events::MODULE_CLAIMED, Some("a"), None, json!({})).unwrap();
    }
    events::append_event(&store, events::HEARTBEAT, None, None, json!({})).unwrap();
    let reply = scheduler::next(&store, None, Duration::ZERO).unwrap();
    assert_eq!(reply.start, vec!["a"]);
    assert_eq!(reply.events.len(), 20);
    assert!(reply.events_truncated);
    assert!(reply.events.iter().all(|e| ![
        events::ACTIVITY_START,
        events::ACTIVITY_FINISH,
        events::HEARTBEAT
    ]
    .contains(&e.kind.as_str())));
    let resumed = scheduler::next(&store, Some(reply.cursor), Duration::ZERO).unwrap();
    assert_eq!(reply.cursor, reply.events.last().unwrap().seq);
    assert_eq!(resumed.start, vec!["a"]);
    assert!(!resumed.timed_out);
    assert!(!resumed.events_truncated);
    assert_eq!(resumed.events.len(), 4);
    assert_eq!(resumed.cursor, events::last_seq(&store).unwrap());
}

#[test]
fn scheduled_workers_respect_codex_slot_cap() {
    use std::{
        process::Command,
        sync::Arc,
        thread,
        time::{Duration, Instant},
    };
    if std::env::var_os("DAG_SLOT_CHILD").is_none() {
        let result = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "scheduled_workers_respect_codex_slot_cap",
                "--nocapture",
            ])
            .env("DAG_SLOT_CHILD", "1")
            .env("AXIOM_CODEX_MAX_PARALLEL", "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    let temp = common::temp();
    let home = temp.path();
    let repo = home.join("repo");
    let store = home.join("orchestrate/demo");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(store.join("inbox")).unwrap();
    for args in [
        vec!["init", "-q"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
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
    let mut units = String::from("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n");
    let mut map = String::new();
    for unit in ["one", "two"] {
        let brief = store.join(format!("{unit}.brief"));
        fs::write(&brief,format!("GOAL x\nSCOPE {unit}.txt\nVERIFY echo ok\nRISK trivial\nFAKE_DELAY_MS 1200\nFAKE_WRITE {unit}.txt\n")).unwrap();
        units.push_str(&format!(
            "{unit}\t{unit}\tpending\t\t\t\t{}\n",
            brief.display()
        ));
        map.push_str(&format!("## {unit}\nPURPOSE x\nPATHS {unit}.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n"));
    }
    fs::write(store.join("units.tsv"), units).unwrap();
    fs::write(store.join("modules.md"), map).unwrap();
    let bridge = Arc::new(Bridge::new(Config {
        data_home: Some(home.into()),
        codex: Ok(common::fake_codex(&[])),
        wsl: None,
        on_windows: false,
    }));
    let first = call(
        &bridge,
        "orch",
        &json!({"project":"demo","args":["--json","schedule","next","--timeout","0"]}),
    );
    assert_eq!(first["structuredContent"]["status"], "completed");
    let launch = |unit: &'static str| {
        let b = Arc::clone(&bridge);
        let repo = repo.clone();
        thread::spawn(move || {
            call(
                &b,
                "codex_run",
                &json!({"project":"demo","unit":unit,"role":"terra","cwd":repo}),
            )["structuredContent"]
                .clone()
        })
    };
    let one = launch("one");
    let deadline = Instant::now() + Duration::from_secs(10);
    while axiom::codex::slots::occupancy(home).unwrap().total == 0 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(20));
    }
    let two = launch("two");
    let a = one.join().unwrap();
    let b = two.join().unwrap();
    assert_eq!(a["status"], "completed", "{a}");
    assert_eq!(b["status"], "completed", "{b}");
    assert!(b["slot_wait_ms"].as_u64().unwrap_or(0) > 200, "{b}");
}

#[test]
fn setup_commands_route_through_the_bridge() {
    let temp = common::temp();
    let home = temp.path();
    let repo = home.join("repo");
    fs::create_dir_all(repo.join(".axiom")).unwrap();
    fs::write(
        repo.join(".axiom/setup.json"),
        json!({"version":1,"steps":[{"id":"index","argv":["unused"],"plan_independent":false}]})
            .to_string(),
    )
    .unwrap();
    let bridge = Bridge::new(Config {
        data_home: Some(home.into()),
        codex: Ok(common::fake_codex(&[])),
        wsl: None,
        on_windows: false,
    });
    let run = |args: Value| {
        let result = call(&bridge, "orch", &json!({"project":"demo","args":args}))
            ["structuredContent"]
            .clone();
        assert_eq!(result["status"], "completed", "{result}");
        serde_json::from_str::<Value>(result["stdout"].as_str().unwrap()).unwrap()
    };
    let started = run(json!(["--json", "setup", "start", "--repo", repo]));
    assert_eq!(started[0]["id"], "index");
    assert_eq!(started[0]["state"], "skipped");
    assert_eq!(run(json!(["--json", "setup", "status"]))[0]["id"], "index");
    assert_eq!(
        run(json!(["--json", "setup", "keep", "index"]))[0]["state"],
        "skipped"
    );
    assert_eq!(run(json!(["--json", "setup", "cancel"]))[0]["id"], "index");
}

#[test]
fn simultaneous_schedulers_claim_each_module_once() {
    use axiom::orch::{events, lifecycle::scheduler};
    use std::{
        sync::{Arc, Barrier},
        thread,
    };
    let temp = common::temp();
    let store = temp.path().join("orchestrate/demo");
    fs::create_dir_all(&store).unwrap();
    fs::write(
        store.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n",
    )
    .unwrap();
    fs::write(store.join("modules.md"),"## a\nPURPOSE x\nPATHS a/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## b\nPURPOSE x\nPATHS b/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
    let barrier = Arc::new(Barrier::new(8));
    let mut handles = vec![];
    for _ in 0..8 {
        let barrier = Arc::clone(&barrier);
        let store = store.clone();
        handles.push(thread::spawn(move || {
            barrier.wait();
            scheduler::claim_ready(&store).unwrap()
        }));
    }
    let claims: Vec<_> = handles
        .into_iter()
        .flat_map(|h| h.join().unwrap())
        .collect();
    assert_eq!(claims.len(), 2);
    let logged = events::read_events(&store, 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == events::MODULE_CLAIMED)
        .count();
    assert_eq!(logged, 2);
}

#[test]
fn explicit_claim_and_release_report_each_error() {
    use axiom::orch::{self, lifecycle::scheduler, Evidence};
    let temp = common::temp();
    let store = temp.path().join("orchestrate/demo");
    fs::create_dir_all(&store).unwrap();
    let brief = store.join("a.brief");
    fs::write(&brief, "GOAL x\nSCOPE a\nVERIFY echo ok\nRISK trivial\n").unwrap();
    fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\ta\tverified\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    fs::write(store.join("modules.md"), "## a\nPURPOSE x\nPATHS a\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## b\nPURPOSE x\nPATHS b\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## c\nPURPOSE x\nPATHS c\nDEPENDS a, b\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
    let err = scheduler::claim(&store, "c").unwrap_err();
    assert_eq!(err.kind, "not_ready");
    assert!(
        err.message.contains("unverified dependencies: a, b"),
        "{err}"
    );
    assert_eq!(
        scheduler::claim(&store, "missing").unwrap_err().kind,
        "not_found"
    );
    assert_eq!(
        scheduler::release(&store, "b", "test").unwrap_err().kind,
        "not_claimed"
    );
    scheduler::claim(&store, "b").unwrap();
    assert_eq!(
        scheduler::claim(&store, "b").unwrap_err().kind,
        "already_claimed"
    );
    for kind in ["run", "verify", "accept", "review"] {
        orch::append_evidence(
            &store,
            "u",
            &Evidence {
                kind: kind.into(),
                command: if kind == "run" {
                    "terra"
                } else {
                    "claude-review"
                }
                .into(),
                exit_code: 0,
                commit: "tree:1".into(),
                ts: "t".into(),
            },
        )
        .unwrap();
    }
    assert_eq!(
        scheduler::claim(&store, "a").unwrap_err().kind,
        "already_verified"
    );
}

#[test]
fn next_waits_for_release_and_positive_timeout() {
    use axiom::orch::{events, lifecycle::scheduler};
    use std::{
        thread,
        time::{Duration, Instant},
    };
    let temp = common::temp();
    let store = temp.path().join("orchestrate/demo");
    fs::create_dir_all(&store).unwrap();
    fs::write(
        store.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n",
    )
    .unwrap();
    fs::write(
        store.join("modules.md"),
        "## a\nPURPOSE x\nPATHS a\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
    )
    .unwrap();
    scheduler::claim(&store, "a").unwrap();
    let cursor = events::last_seq(&store).unwrap();
    let other_store = store.clone();
    let writer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        scheduler::release(&other_store, "a", "retry").unwrap();
    });
    let started = Instant::now();
    let reply = scheduler::next(&store, Some(cursor), Duration::from_secs(3)).unwrap();
    let elapsed = started.elapsed();
    writer.join().unwrap();
    assert!(
        elapsed >= Duration::from_millis(100) && elapsed < Duration::from_secs(2),
        "{elapsed:?}"
    );
    assert!(!reply.timed_out);
    assert!(reply
        .events
        .iter()
        .any(|e| e.kind == events::MODULE_RELEASED));
    let started = Instant::now();
    let timeout = scheduler::next(&store, Some(reply.cursor), Duration::from_millis(150)).unwrap();
    assert!(timeout.timed_out);
    assert!(started.elapsed() >= Duration::from_millis(150));
}

#[test]
fn review_transition_without_claim_emits_one_module_verified() {
    use axiom::orch::{self, events, Evidence};
    let temp = common::temp();
    let store = temp.path().join("orchestrate/demo");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&store).unwrap();
    fs::create_dir_all(&repo).unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    fs::write(repo.join("a"), "content").unwrap();
    let brief = store.join("u.brief");
    fs::write(&brief, "GOAL x\nSCOPE a\nVERIFY echo ok\nRISK trivial\n").unwrap();
    fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tbuilt\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    fs::write(
        store.join("modules.md"),
        "## m\nPURPOSE x\nPATHS a\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
    )
    .unwrap();
    let commit = orch::snapshot(&repo, &["a".into()]);
    for kind in ["run", "verify", "accept"] {
        orch::append_evidence(
            &store,
            "u",
            &Evidence {
                kind: kind.into(),
                command: "terra".into(),
                exit_code: 0,
                commit: commit.clone(),
                ts: "t".into(),
            },
        )
        .unwrap();
    }
    for _ in 0..2 {
        orch::record_review(&store, &["u".into()], "claude-review", true, &repo).unwrap();
    }
    let logged = events::read_events(&store, 0).unwrap();
    assert_eq!(
        logged
            .iter()
            .filter(|e| e.kind == events::MODULE_VERIFIED && e.module.as_deref() == Some("m"))
            .count(),
        1,
        "{logged:?}"
    );
}

#[test]
fn schedule_and_activity_commands_support_text_and_timeout_validation() {
    use axiom::orch::events;
    let temp = common::temp();
    let home = temp.path();
    let store = home.join("orchestrate/demo");
    fs::create_dir_all(&store).unwrap();
    fs::write(
        store.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n",
    )
    .unwrap();
    fs::write(
        store.join("modules.md"),
        "## a\nPURPOSE x\nPATHS a\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
    )
    .unwrap();
    let bridge = Bridge::new(Config {
        data_home: Some(home.into()),
        codex: Ok(common::fake_codex(&[])),
        wsl: None,
        on_windows: false,
    });
    let run = |args: Value| {
        call(&bridge, "orch", &json!({"project":"demo", "args":args}))["structuredContent"].clone()
    };
    let bad = run(json!(["schedule", "next", "--timeout", "1801"]));
    assert_eq!(bad["status"], "failed", "{bad}");
    assert_eq!(bad["error"]["kind"], "invalid_argument", "{bad}");
    assert!(
        bad["stderr"].as_str().unwrap().contains("invalid_argument"),
        "{bad}"
    );
    let claim = run(json!(["schedule", "claim", "a"]));
    assert_eq!(claim["status"], "completed", "{claim}");
    assert!(claim["stdout"].as_str().unwrap().contains('a'));
    let start = run(json!([
        "activity", "start", "--module", "a", "--actor", "core", "--phase", "review"
    ]));
    assert_eq!(start["status"], "completed", "{start}");
    let id = start["stdout"].as_str().unwrap().trim();
    assert!(id.starts_with('a'));
    let finish = run(json!([
        "--json",
        "activity",
        "finish",
        "--module",
        "a",
        "--actor",
        "core",
        "--phase",
        "review",
        "--id",
        id,
        "--outcome",
        "finished"
    ]));
    assert_eq!(finish["status"], "completed", "{finish}");
    let value: Value = serde_json::from_str(finish["stdout"].as_str().unwrap()).unwrap();
    assert_eq!(value["event"]["data"]["activity"], id);
    assert_eq!(value["event"]["data"]["outcome"], "finished");
    assert!(events::open_activities(&store).unwrap().is_empty());
}
