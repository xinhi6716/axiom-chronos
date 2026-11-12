mod common;
use axiom::{
    base::Failure,
    codex::{
        extreme::{self, TierStatus},
        AppServer, RunRequest,
    },
    usage::extensions,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

fn request(home: &Path, brief: &str) -> RunRequest {
    RunRequest {
        brief: brief.into(),
        cwd: home.to_string_lossy().into_owned(),
        model: "gpt-6.1-terra".into(),
        sandbox: "read-only".into(),
        effort: None,
        output_schema: None,
        developer_instructions: None,
        timeout_s: 20,
        lean: false,
        keep_plugins: vec![],
        web_search: false,
    }
}
fn starts(home: &Path) -> Vec<Value> {
    fs::read_to_string(home.join("fake.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["method"] == "turn/start")
        .collect()
}
fn wait_for(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !predicate() {
        assert!(Instant::now() < deadline, "fake did not start turn");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn classify_cases() {
    let ok = Ok(json!({}));
    assert_eq!(
        extreme::classify(&ok, &["unrelated warning".into()]).0,
        TierStatus::Accepted
    );
    let warning = "Warning: NOT ADVERTISED here".to_string();
    assert_eq!(
        extreme::classify(&ok, &[warning.clone()]),
        (TierStatus::NotAdvertised, warning)
    );
    for message in [
        "bad service_tier",
        "unsupported serviceTier",
        "fast tier unsupported",
    ] {
        let result = Err(Failure::new("rpc", message));
        let (status, detail) = extreme::classify(&result, &["not advertised".into()]);
        assert_eq!(status, TierStatus::Rejected);
        assert_eq!(extreme::report(status, &detail)["applied"], false);
    }
    assert_eq!(
        extreme::classify(&Err(Failure::new("rpc", "other")), &[]).0,
        TierStatus::Accepted
    );
}

#[test]
fn isolated_scenarios() {
    let home = common::temp();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "scenario_child", "--nocapture"])
        .env("EXTREME_TEST_HOME", home.path())
        .env("FAKE_CODEX_LOG", home.path().join("fake.log"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}
{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn scenario_child() {
    let Some(home) = std::env::var_os("EXTREME_TEST_HOME") else {
        return;
    };
    let home = Path::new(&home);
    let server =
        Arc::new(AppServer::new(common::fake_codex(&[])).with_slots_home(home.to_path_buf()));
    assert_eq!(extreme::load(home), extreme::Setting::default());
    let release = home.join("release");
    let req = request(home, &format!("WAIT_FILE {}", release.display()));
    let worker_server = server.clone();
    let worker = std::thread::spawn(move || worker_server.run(&req).unwrap());
    wait_for(|| starts(home).len() == 1);
    extreme::set(home, true).unwrap();
    fs::write(&release, "").unwrap();
    let first = worker.join().unwrap();
    assert_eq!(first["status"], "completed");
    assert!(first.get("service_tier").is_none());
    assert!(starts(home)[0]["params"].get("serviceTier").is_none());
    let second = server.run(&request(home, "second")).unwrap();
    assert_eq!(second["service_tier"]["status"], "accepted");
    assert_eq!(second["service_tier"]["applied"], true);
    assert_eq!(starts(home)[1]["params"]["serviceTier"], "fast");
    assert_eq!(
        extreme::load(home).last_check.unwrap().turn.as_deref(),
        second["turn_id"].as_str()
    );
    server.close();

    let restarted = AppServer::new(common::fake_codex(&[])).with_slots_home(home.to_path_buf());
    let third = restarted.run(&request(home, "restart")).unwrap();
    assert_eq!(third["service_tier"]["status"], "accepted");
    assert_eq!(starts(home)[2]["params"]["serviceTier"], "fast");
    extreme::set(home, false).unwrap();
    assert!(extreme::load(home).last_check.is_some());
    let fourth = restarted.run(&request(home, "off")).unwrap();
    assert!(fourth.get("service_tier").is_none());
    assert!(starts(home)[3]["params"].get("serviceTier").is_none());
    restarted.close();

    extreme::set(home, true).unwrap();
    let warning = AppServer::new(common::fake_codex(&["--fake-tier=not-advertised"]))
        .with_slots_home(home.to_path_buf());
    let r = warning.run(&request(home, "warning")).unwrap();
    assert_eq!(r["service_tier"]["status"], "not_advertised");
    assert_eq!(r["service_tier"]["applied"], false);
    assert!(r["service_tier"]["detail"]
        .as_str()
        .unwrap()
        .contains("not advertised"));
    warning.close();

    let reject = AppServer::new(common::fake_codex(&["--fake-tier=reject"]))
        .with_slots_home(home.to_path_buf());
    let r = reject.run(&request(home, "reject")).unwrap();
    assert_eq!(r["status"], "bridge_error");
    assert_eq!(r["error"]["kind"], "service_tier_rejected");
    assert_eq!(r["service_tier"]["status"], "rejected");
    assert_eq!(r["service_tier"]["applied"], false);
    assert_eq!(starts(home).len(), 6);
    assert_eq!(
        extreme::load(home).last_check.unwrap().status,
        TierStatus::Rejected
    );
    let (records, skipped) = extensions::read(home).unwrap();
    assert_eq!(skipped, 0);
    assert_eq!(records.len(), 3);
    for record in &records {
        assert_eq!(record.tier, extensions::Tier::Extreme);
        assert_eq!(record.provider, "codex");
        assert_eq!(record.model.as_deref(), Some("gpt-6.1-terra"));
        assert!(record.turn.is_some());
        assert_eq!(
            (
                record.input_tokens,
                record.cached_input_tokens,
                record.output_tokens
            ),
            (1200, 800, 40)
        );
        assert_eq!(record.usd, None);
    }
}
