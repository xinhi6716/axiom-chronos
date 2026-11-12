use axiom::{engine::policy, goal, hook, orch, review::roles};
use serde_json::{json, Value};
use std::{fs, path::PathBuf, process::Command};

struct Fx {
    _temp: tempfile::TempDir,
    home: PathBuf,
    repo: PathBuf,
    store: PathBuf,
}

impl Fx {
    fn new(pm_qa: bool, worker: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        fs::write(repo.join("a.txt"), "one").unwrap();
        let goal = goal::open(&home, "s", "wish", Some(repo.to_str().unwrap())).unwrap();
        let store = orch::store_for(&home, goal.project.as_deref().unwrap()).unwrap();
        fs::create_dir_all(store.join("agents")).unwrap();
        let brief = store.join("a.brief");
        fs::write(&brief, "GOAL lifecycle\nSCOPE a.txt\nVERIFY echo ok\n").unwrap();
        fs::write(
            store.join("units.tsv"),
            format!(
                "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\na\tm\tpending\t\t\t\t{}\n",
                brief.display()
            ),
        )
        .unwrap();
        policy::set(&store, "pm_qa", if pm_qa { "on" } else { "off" }).unwrap();
        orch::append_evidence(
            &store,
            "a",
            &orch::Evidence {
                kind: "run".into(),
                command: worker.into(),
                exit_code: 0,
                commit: String::new(),
                ts: "1".into(),
            },
        )
        .unwrap();
        assert_eq!(
            orch::record_verify(&store, "a", &repo).unwrap().exit_code,
            0
        );
        orch::review_pack(&store, "a", &repo).unwrap();
        Self {
            _temp: temp,
            home,
            repo,
            store,
        }
    }
    fn rows(&self) -> Vec<orch::Evidence> {
        orch::evidence(&self.store, "a").unwrap()
    }
    fn state(&self) -> String {
        orch::derive_state(&self.rows(), false)
    }
    fn tree(&self) -> String {
        self.rows()
            .into_iter()
            .rev()
            .find(|r| r.kind == "verify")
            .unwrap()
            .commit
    }
    fn verdict(&self, reviewer: &str, findings: Value) {
        fs::write(
            orch::review_verdict_path(&self.store, "a", reviewer),
            json!({"unit":"a","tree":self.tree(),"verdict":"pass","findings":findings}).to_string(),
        )
        .unwrap();
    }
    fn bind(&self) {
        fs::write(self.store.join("agents/r1.reviewer"), "a").unwrap();
    }
    fn fire(&self, reviewer: &str, id: Option<&str>, prompt: &str) -> String {
        let mut event = json!({"hook_event_name":"PostToolUse","session_id":"s","tool_name":"Agent","tool_input":{"subagent_type":reviewer,"prompt":prompt},"tool_response":{"status":"completed"}});
        if let Some(id) = id {
            event["tool_response"]["agentId"] = json!(id);
        }
        hook::handle(&event, Some(&self.home), false).unwrap()["hookSpecificOutput"]
            ["additionalContext"]
            .as_str()
            .unwrap()
            .to_string()
    }
    fn reviews(&self) -> Vec<orch::Evidence> {
        self.rows()
            .into_iter()
            .filter(|r| r.kind == "review")
            .collect()
    }
}

#[test]
fn false_green_major_finding_records_failure() {
    let f = Fx::new(true, "gpt-6.1-terra");
    f.bind();
    f.verdict("chronos:reviewer", json!([]));
    f.fire("chronos:reviewer", Some("r1"), "");
    f.verdict("chronos:pm-qa", json!([{"check":"lifecycle","severity":"major","location":"a.txt:1","evidence":"false green: required review is missing"}]));
    let text = f.fire("chronos:pm-qa", Some("r1"), "");
    assert!(text.contains("review-failed"), "{text}");
    let reviews = f.reviews();
    assert_eq!(
        (reviews[1].command.as_str(), reviews[1].exit_code),
        ("chronos:pm-qa", 1)
    );
    assert_eq!(f.state(), "review-failed");
    let events = fs::read_to_string(f.store.join("events.jsonl")).unwrap();
    let event = events
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|event| {
            event["kind"] == "review_recorded" && event["data"]["reviewer"] == "chronos:pm-qa"
        })
        .unwrap();
    assert_eq!(event["data"]["review_id"], "a#2");
    assert_eq!(event["data"]["role"], "pm-qa");
    assert_eq!(event["data"]["tree"], f.tree());
    assert_eq!(event["data"]["pass"], false);
    assert_eq!(event["data"]["findings"][0]["severity"], "major");
}

#[test]
fn missing_required_verdict_cannot_verify() {
    let f = Fx::new(true, "gpt-6.1-terra");
    f.bind();
    f.verdict("chronos:reviewer", json!([]));
    f.fire("chronos:reviewer", Some("r1"), "");
    assert_ne!(f.state(), "verified");
    let text = f.fire("chronos:pm-qa", Some("r1"), "");
    assert!(text.starts_with("Chronos did not record"), "{text}");
    assert_eq!(f.reviews().len(), 1);
    assert_ne!(f.state(), "verified");
    assert!(f
        .rows()
        .iter()
        .any(|r| r.kind == "require" && r.command == "chronos:pm-qa" && r.commit == f.tree()));
}

#[test]
fn disabled_policy_preserves_reviewer_behavior() {
    let f = Fx::new(false, "gpt-6.1-terra");
    let items = roles::review_items(&f.store, "a").unwrap();
    assert_eq!(
        items
            .iter()
            .map(|i| i.reviewer.as_str())
            .collect::<Vec<_>>(),
        ["chronos:reviewer"]
    );
    f.bind();
    f.verdict("chronos:reviewer", json!([]));
    assert!(f
        .fire("chronos:reviewer", Some("r1"), "")
        .contains("verified"));
    assert_eq!(f.state(), "verified");
    assert_eq!(
        f.reviews()
            .iter()
            .map(|r| r.command.as_str())
            .collect::<Vec<_>>(),
        ["chronos:reviewer"]
    );
    assert!(!f
        .rows()
        .iter()
        .any(|r| r.kind == "require" && r.command == "chronos:pm-qa"));
}

#[test]
fn malformed_and_stale_verdicts_are_refused() {
    let f = Fx::new(true, "gpt-6.1-terra");
    f.bind();
    let path = orch::review_verdict_path(&f.store, "a", "chronos:pm-qa");
    let malformed =
        json!({"unit":"a","tree":f.tree(),"verdict":"pass","findings":[{"needs":"invalid"}]})
            .to_string();
    assert!(orch::parse_verdict(&malformed).is_err());
    fs::write(&path, malformed).unwrap();
    assert!(f
        .fire("chronos:pm-qa", Some("r1"), "")
        .contains("invalid needs"));
    fs::write(
        &path,
        json!({"unit":"a","tree":"stale","verdict":"pass","findings":[]}).to_string(),
    )
    .unwrap();
    assert!(f
        .fire("chronos:pm-qa", Some("r1"), "")
        .starts_with("Chronos did not record"));
    f.verdict("chronos:pm-qa", json!([]));
    fs::write(f.repo.join("a.txt"), "changed").unwrap();
    assert!(f
        .fire("chronos:pm-qa", Some("r1"), "")
        .contains("stale review"));
    assert!(f.reviews().is_empty());
    assert_ne!(f.state(), "verified");
}

#[test]
fn prompt_fallback_requires_marker_and_accepts_pm_qa_verdict_path_or_pack() {
    for verdict_path in [true, false] {
        let f = Fx::new(true, "gpt-6.1-terra");
        f.verdict("chronos:pm-qa", json!([]));
        let path = if verdict_path {
            orch::review_verdict_path(&f.store, "a", "chronos:pm-qa")
        } else {
            orch::pack_path(&f.store, "a")
        };
        let prompt = format!("Review {}.", path.display());
        assert!(f
            .fire("chronos:pm-qa", None, &prompt)
            .starts_with("Chronos did not record"));
        f.bind();
        assert!(f
            .fire("chronos:pm-qa", Some("unknown"), &prompt)
            .starts_with("Chronos did not record"));
        assert!(f
            .fire("chronos:pm-qa", None, &prompt)
            .starts_with("Chronos recorded"));
        assert_eq!(f.reviews()[0].command, "chronos:pm-qa");
    }
}

#[test]
fn prompt_binding_rejects_wrong_or_ambiguous_paths() {
    let f = Fx::new(true, "gpt-6.1-terra");
    f.bind();
    f.verdict("chronos:pm-qa", json!([]));
    let pm = orch::review_verdict_path(&f.store, "a", "chronos:pm-qa");
    let other = orch::pack_path(&f.store, "b");
    for prompt in [
        format!("{}/extra", pm.display()),
        format!("{} {}", pm.display(), other.display()),
        "review /wrong/reports/a.pm-qa.verdict.json".to_string(),
    ] {
        assert!(f
            .fire("chronos:pm-qa", None, &prompt)
            .starts_with("Chronos did not record"));
    }
    assert!(f
        .fire("chronos:reviewer", None, &pm.display().to_string())
        .starts_with("Chronos did not record"));
    assert!(f.reviews().is_empty());
}

#[test]
fn pm_qa_keeps_same_family_gate() {
    let f = Fx::new(true, "claude-opus");
    f.bind();
    f.verdict("chronos:pm-qa", json!([]));
    assert!(f
        .fire("chronos:pm-qa", Some("r1"), "")
        .contains("same family"));
    assert!(f.reviews().is_empty());
}
