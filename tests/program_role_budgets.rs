mod common;

use axiom::{
    bridge::server::{self, Bridge, Config},
    council,
    engine::{kinds, policy},
    orch::{
        self,
        events::{self, Event},
    },
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Fixture {
    _temp: tempfile::TempDir,
    store: PathBuf,
    repo: PathBuf,
    codex_log: PathBuf,
    member_log: PathBuf,
    bridge: Bridge,
}

impl Fixture {
    fn new(limit: Option<&str>) -> Self {
        let temp = common::temp();
        let repo = temp.path().join("repo");
        let store = orch::store_for(temp.path(), "demo").unwrap();
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(store.join("inbox")).unwrap();
        fs::create_dir_all(store.join("reports")).unwrap();
        let script = temp.path().join("script.json");
        fs::write(&script, "[]").unwrap();
        let codex_log = temp.path().join("fake.jsonl");
        let member_log = temp.path().join("member.jsonl");
        let mut codex = common::fake("fake_codex");
        codex.push(format!("--fake-script={}", script.display()));
        codex.push(format!("--fake-log={}", codex_log.display()));
        let mut member = common::fake("fake_member");
        member.push(format!("--fake-script={}", script.display()));
        member.push(format!("--fake-log={}", member_log.display()));
        let mut config = Config::from_env();
        config.data_home = Some(temp.path().into());
        config.codex = Ok(codex);
        config.wsl = None;
        config.on_windows = false;
        let bridge =
            Bridge::new(config).with_council(council::Setup::new(Ok(member.clone()), Ok(member)));
        bridge.note_project_cwd("demo", repo.to_str().unwrap());
        if let Some(limit) = limit {
            policy::set(&store, "budget.council_member", limit).unwrap();
        }
        Self {
            _temp: temp,
            store,
            repo,
            codex_log,
            member_log,
            bridge,
        }
    }

    fn call(&self) -> Value {
        server::call(
            &self.bridge,
            "council",
            &json!({
                "project":"demo", "action":"program", "brief":"q", "cwd":self.repo
            }),
        )["structuredContent"]
            .clone()
    }

    fn events(&self) -> Vec<Event> {
        events::read_events(&self.store, 0).unwrap()
    }

    fn requests(&self) -> usize {
        lines(&self.member_log) + lines(&self.codex_log)
    }
}

fn lines(path: &Path) -> usize {
    fs::read_to_string(path).unwrap_or_default().lines().count()
}

#[test]
fn exhausted_refuses_before_any_member() {
    let f = Fixture::new(Some("0"));
    let result = f.call();
    assert_eq!(result["status"], "refused", "{result}");
    assert_eq!(result["error"]["kind"], "budget_exceeded");
    assert_eq!(lines(&f.member_log), 0);
    assert_eq!(lines(&f.codex_log), 0);
    assert!(!f.events().iter().any(|e| e.kind == kinds::ROLE_SPEND));
}

#[test]
fn budget_left_records_program_spend() {
    let f = Fixture::new(Some("1000"));
    let result = f.call();
    assert_ne!(result["status"], "refused", "{result}");
    let spend: Vec<_> = f
        .events()
        .into_iter()
        .filter(|e| e.kind == kinds::ROLE_SPEND)
        .collect();
    assert!(
        spend
            .iter()
            .any(|e| e.data["call"] == "program" && e.data["role"] == "council_member"),
        "{spend:?}"
    );
    assert!(spend.iter().all(|e| e.data["call"] == "program"));
}

#[test]
fn mid_run_exhaustion_stops_later_requests() {
    // Any council_member budget refuses every call after the first unpriced spend,
    // so a budgeted baseline depends on which member wins the race. Only an
    // unbudgeted run gives a fixed request count to compare against.
    let baseline = Fixture::new(None);
    baseline.call();
    let f = Fixture::new(Some("0.0000001"));
    f.call();
    let events = f.events();
    let first_spend = events
        .iter()
        .find(|e| e.kind == kinds::ROLE_SPEND)
        .expect("expected the first member's spend");
    assert!(first_spend.data["usd"].is_null());
    assert!(events.iter().any(|e| e.kind == kinds::BUDGET_EXCEEDED
        && e.data["call"] == "program"
        && e.data["role"] == "council_member"));
    assert!(
        events
            .iter()
            .filter(|e| e.kind == kinds::ROLE_SPEND)
            .count()
            <= baseline
                .events()
                .iter()
                .filter(|e| e.kind == kinds::ROLE_SPEND)
                .count()
    );
    assert!(
        f.requests() <= baseline.requests(),
        "mid-run: {}, baseline: {}",
        f.requests(),
        baseline.requests()
    );
}
