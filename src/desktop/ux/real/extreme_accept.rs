//! Tests for extreme; owned by module extreme-seam.
use super::extreme::CodexTurnPolicy;
use crate::{
    codex::{extreme, AppServer, RunRequest},
    desktop::{
        demo,
        doctor::extreme::{policy_on_install, policy_on_toggle},
        i18n::Lang,
        settings::Settings,
        ux::seams::TurnPolicy,
    },
};
use serde_json::Value;
use std::{
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

fn python() -> String {
    if let Ok(p) = std::env::var("AXIOM_TEST_PYTHON") {
        return p;
    }
    let names = if cfg!(windows) {
        ["python", "python3"]
    } else {
        ["python3", "python"]
    };
    names
        .into_iter()
        .find(|p| {
            crate::base::proc::cmd(p)
                .arg("--version")
                .output()
                .is_ok_and(|o| o.status.success())
        })
        .expect("Python not found")
        .into()
}
fn fake(log: &Path) -> Vec<String> {
    vec![
        python(),
        "-I".into(),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fakes/fake_codex.py")
            .to_string_lossy()
            .into_owned(),
        format!("--fake-log={}", log.display()),
    ]
}
fn request(cwd: &Path, brief: &str) -> RunRequest {
    RunRequest {
        brief: brief.into(),
        cwd: cwd.to_string_lossy().into_owned(),
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
fn starts(log: &Path) -> Vec<Value> {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["method"] == "turn/start")
        .collect()
}

#[test]
fn toggle_is_shared_snapshotted_and_requested_as_fast_on_the_next_turn() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("slots");
    let log = d.path().join("fake.log");
    let policy = CodexTurnPolicy::new(home.clone());
    let server = Arc::new(AppServer::new(fake(&log)).with_slots_home(home.clone()));

    let turn = policy.begin_turn();
    assert!(!turn.extreme);
    let release = d.path().join("release");
    let req = request(d.path(), &format!("WAIT_FILE {}", release.display()));
    let worker_server = server.clone();
    let worker = std::thread::spawn(move || worker_server.run(&req).unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    while starts(&log).is_empty() {
        assert!(Instant::now() < deadline, "fake did not start turn");
        std::thread::sleep(Duration::from_millis(20));
    }
    let next = policy.try_set_extreme(true).unwrap();
    assert!(next.extreme);
    assert!(!turn.extreme, "the running turn keeps its snapshot");
    fs::write(&release, "").unwrap();
    let first = worker.join().unwrap();
    assert!(first.get("service_tier").is_none());
    assert!(policy.begin_turn().extreme);
    let second = server.run(&request(d.path(), "second")).unwrap();
    assert_eq!(second["service_tier"]["applied"], true);
    let seen = starts(&log);
    assert!(seen[0]["params"].get("serviceTier").is_none());
    assert_eq!(seen[1]["params"]["serviceTier"], "fast");
    assert_eq!(policy.fast_cost(), None);
    server.close();

    // a restart and the CLI see the same setting
    assert!(CodexTurnPolicy::new(home.clone()).current().extreme);
    extreme::set(&home, false).unwrap();
    assert!(!policy.current().extreme);
    extreme::set(&home, true).unwrap();
    assert!(policy.current().extreme);
    assert_eq!(
        policy.current(),
        CodexTurnPolicy::new(home.clone()).current()
    );
    assert!(CodexTurnPolicy::new(home).begin_turn().extreme);
}

#[test]
fn installation_and_toggles_share_the_cli_setting() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("slots");
    // The CLI uses this same codex::extreme::set entry point.
    extreme::set(&home, true).unwrap();
    let policy = Arc::new(CodexTurnPolicy::new(home.clone()));
    let mut state = demo::state();
    let mut prefs = Settings::default();
    state.ux.seams.turn_policy = Some(policy.clone());
    policy_on_install(&mut state, &mut prefs);
    assert!(prefs.ux.extreme);
    assert!(Arc::ptr_eq(
        state.ux.seams.turn_policy.as_ref().unwrap(),
        &(policy.clone() as Arc<dyn TurnPolicy>)
    ));
    assert!(policy_on_toggle(&mut state, &mut prefs, false, Lang::En));
    assert!(!prefs.ux.extreme);
    assert!(!extreme::load(&home).on);
    assert!(policy_on_toggle(&mut state, &mut prefs, true, Lang::En));
    assert!(prefs.ux.extreme);
    assert!(extreme::load(&home).on);
    let mut restarted = demo::state();
    restarted.ux.seams.turn_policy = Some(Arc::new(CodexTurnPolicy::new(home.clone())));
    prefs.ux.extreme = false;
    policy_on_install(&mut restarted, &mut prefs);
    assert!(prefs.ux.extreme);
    extreme::set(&home, false).unwrap();
    policy_on_install(&mut restarted, &mut prefs);
    assert!(!prefs.ux.extreme);
    assert!(!policy.current().extreme);
}

#[test]
fn unwritable_slots_home_reports_and_keeps_the_setting() {
    let d = tempfile::tempdir().unwrap();
    let blocker = d.path().join("file");
    fs::write(&blocker, "x").unwrap();
    let policy = CodexTurnPolicy::new(blocker.join("slots"));
    let err = policy.try_set_extreme(true).unwrap_err();
    assert_eq!(err.kind, "io");
    assert!(!policy.set_extreme(true).extreme);
    assert!(!policy.current().extreme);
    for (lang, prefix) in [
        (Lang::En, "Could not save extreme mode: "),
        (Lang::ZhTw, "無法儲存極限模式："),
    ] {
        let mut state = demo::state();
        state.ux.seams.turn_policy = Some(Arc::new(CodexTurnPolicy::new(blocker.join("slots"))));
        let mut prefs = Settings::default();
        assert!(!prefs.ux.extreme);
        assert!(!policy_on_toggle(&mut state, &mut prefs, true, lang));
        assert!(!prefs.ux.extreme);
        assert_eq!(
            state.errors.last().unwrap().message.as_ref(),
            format!("{prefix}{}", err.message)
        );
    }
}
