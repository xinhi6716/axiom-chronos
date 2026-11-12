use axiom::{goal, hook, orch::*};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap().map(Result::unwrap) {
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(&path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}

struct Fx {
    _t: tempfile::TempDir,
    home: PathBuf,
    repo: PathBuf,
    store: PathBuf,
}

fn row(kind: &str, command: &str, commit: &str) -> Evidence {
    Evidence {
        kind: kind.into(),
        command: command.into(),
        exit_code: 0,
        commit: commit.into(),
        ts: "1".into(),
    }
}

impl Fx {
    /// Units `a` and `b` of track `m`, both built and verified on the current tree.
    fn new(worker: &str) -> Self {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        let repo = t.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        fs::write(repo.join("a.txt"), "one").unwrap();
        fs::write(repo.join("b.txt"), "one").unwrap();
        let g = goal::open(&home, "s", "wish", Some(repo.to_str().unwrap())).unwrap();
        let store = store_for(&home, g.project.as_deref().unwrap()).unwrap();
        fs::create_dir_all(store.join("reports")).unwrap();
        fs::create_dir_all(store.join("agents")).unwrap();
        let mut units = "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n".to_string();
        for (id, file) in [("a", "a.txt"), ("b", "b.txt")] {
            let brief = store.join(format!("{id}.brief"));
            fs::write(&brief, format!("GOAL x\nSCOPE {file}\nVERIFY echo ok\n")).unwrap();
            units.push_str(&format!("{id}\tm\tpending\t\t\t\t{}\n", brief.display()));
        }
        fs::write(store.join("units.tsv"), units).unwrap();
        for (id, file) in [("a", "a.txt"), ("b", "b.txt")] {
            append_evidence(&store, id, &row("run", worker, "")).unwrap();
            let tree = snapshot(&repo, &[file.into()]);
            append_evidence(&store, id, &row("verify", "echo ok", &tree)).unwrap();
            review_pack(&store, id, &repo).unwrap();
        }
        Self {
            _t: t,
            home,
            repo,
            store,
        }
    }
    fn tree(&self, unit: &str) -> String {
        evidence(&self.store, unit)
            .unwrap()
            .into_iter()
            .rfind(|r| r.kind == "verify")
            .unwrap()
            .commit
    }
    fn verdict(&self, unit: &str, findings: Value) {
        fs::write(
            verdict_path(&self.store, unit),
            json!({"unit":unit,"tree":self.tree(unit),"verdict":"pass","findings":findings})
                .to_string(),
        )
        .unwrap();
    }
    fn bind(&self, id: &str, unit: &str) {
        fs::write(
            self.store.join("agents").join(format!("{id}.reviewer")),
            unit,
        )
        .unwrap();
    }
    fn reviews(&self, unit: &str) -> usize {
        evidence(&self.store, unit)
            .unwrap()
            .iter()
            .filter(|r| r.kind == "review")
            .count()
    }
    fn state(&self, unit: &str) -> String {
        derive_state(&evidence(&self.store, unit).unwrap(), false)
    }
    fn fire(&self, event: &Value) -> Option<String> {
        hook::handle(event, Some(&self.home), false).map(|v| {
            v["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .unwrap()
                .to_string()
        })
    }
}

fn agent_event(id: Option<&str>, subagent: &str) -> Value {
    let mut e = json!({"hook_event_name":"PostToolUse","session_id":"s","tool_name":"Agent",
        "tool_input":{"subagent_type":subagent,"prompt":"review unit a"},
        "tool_response":{"status":"completed"}});
    if let Some(id) = id {
        e["tool_response"]["agentId"] = json!(id);
    }
    e
}

#[test]
fn happy_path_records_and_completes_the_module() {
    let f = Fx::new("gpt-6.1-terra");
    f.bind("r1", "a");
    f.verdict(
        "a",
        json!([{"check":"d","severity":"minor","location":"a.txt:2","evidence":"nit"}]),
    );
    let text = f
        .fire(&agent_event(Some("r1"), "chronos:reviewer"))
        .unwrap();
    assert!(
        text.starts_with(
            "Chronos recorded the review.
"
        ),
        "{text}"
    );
    assert!(
        text.contains("a	verified	next=done	verdict blocker=0 major=0 minor=1"),
        "{text}"
    );
    assert!(text.contains("paths: pack="), "{text}");
    assert!(
        text.contains(&format!(
            "paths: pack={} verdict={} report={}",
            pack_path(&f.store, "a").display(),
            verdict_path(&f.store, "a").display(),
            f.store.join("reports").join("a.json").display()
        )),
        "{text}"
    );
    assert!(!text.contains("blocker and major"), "{text}");
    assert!(!text.contains("module m done"), "{text}");
    assert_eq!(f.reviews("a"), 1);
    let review = evidence(&f.store, "a").unwrap().pop().unwrap();
    assert_eq!(
        (
            review.kind.as_str(),
            review.command.as_str(),
            review.exit_code
        ),
        ("review", "chronos:reviewer", 0)
    );
    f.bind("r2", "b");
    f.verdict("b", json!([]));
    let text = f
        .fire(&agent_event(Some("r2"), "chronos:reviewer"))
        .unwrap();
    assert!(
        text.contains("module m done: a=verified, b=verified"),
        "{text}"
    );
    assert_eq!(f.state("b"), "verified");
}

#[test]
fn module_done_ignores_superseded_units() {
    let f = Fx::new("gpt-6.1-terra");
    let path = f.store.join("units.tsv");
    let units = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("{units}old\tm\tsuperseded:a\t\t\t\t\n")).unwrap();
    for (id, unit) in [("r1", "a"), ("r2", "b")] {
        f.bind(id, unit);
        f.verdict(unit, json!([]));
        let response = hook::handle(
            &agent_event(Some(id), "chronos:reviewer"),
            Some(&f.home),
            false,
        )
        .unwrap();
        let text = response["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert_eq!(
            response,
            json!({"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": text}})
        );
        if unit == "b" {
            assert!(
                text.contains("module m done: a=verified, b=verified"),
                "{text}"
            );
            assert!(!text.contains("old="), "{text}");
        }
    }
}

#[test]
fn serious_findings_are_listed_and_the_unit_needs_a_fix() {
    let f = Fx::new("gpt-6.1-terra");
    f.bind("r1", "a");
    f.verdict(
        "a",
        json!([{"check":"c","severity":"major","location":"a.txt:1","evidence":"proof"}]),
    );
    let text = f
        .fire(&agent_event(Some("r1"), "chronos:reviewer"))
        .unwrap();
    assert!(
        text.contains("a	review-failed	next=fix	verdict blocker=0 major=1 minor=0"),
        "{text}"
    );
    assert!(
        text.contains(
            "blocker and major findings:
  #1 major c a.txt:1: proof"
        ),
        "{text}"
    );
    assert!(!text.contains("module m done"), "{text}");
    assert_eq!(f.reviews("a"), 1);
    assert_ne!(f.state("a"), "verified");
}

#[test]
fn clean_pass_is_verified_and_agent_id_is_sanitized() {
    let f = Fx::new("gpt-6.1-terra");
    f.bind("r1", "a");
    f.verdict("a", json!([]));
    let text = f
        .fire(&agent_event(Some("r1/../"), "chronos:reviewer"))
        .unwrap();
    assert!(text.contains("a\tverified\tnext=done"), "{text}");
    assert_eq!(f.state("a"), "verified");
}

#[test]
fn other_agents_and_missing_goals_record_nothing() {
    let f = Fx::new("gpt-6.1-terra");
    f.bind("r1", "a");
    f.verdict("a", json!([]));
    let before = files(&f.home);
    assert!(f.fire(&agent_event(Some("r1"), "chronos:critic")).is_none());
    let mut e = agent_event(Some("r1"), "chronos:reviewer");
    e["tool_name"] = json!("Bash");
    assert!(f.fire(&e).is_none());
    e["tool_name"] = json!("Task");
    e["session_id"] = json!("no-such-session");
    assert!(f.fire(&e).is_none());
    assert_eq!(files(&f.home), before);
    goal::interrupt(&f.home, "s", "stop").unwrap();
    let before = files(&f.home);
    assert!(f
        .fire(&agent_event(Some("r1"), "chronos:reviewer"))
        .is_none());
    assert_eq!(files(&f.home), before);
    assert_eq!(f.reviews("a"), 0);
    assert_eq!(f.state("a"), "built");
}

fn rejected(f: &Fx, text: &str, needle: &str) {
    assert!(
        text.starts_with("Chronos did not record the review of a: "),
        "{text}"
    );
    assert!(text.contains(needle), "{text}");
    assert!(
        text.ends_with("Call orch review yourself after fixing it."),
        "{text}"
    );
    let rows = evidence(&f.store, "a").unwrap();
    assert_eq!(
        rows.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(),
        ["run", "verify"]
    );
    assert_ne!(f.state("a"), "verified");
}

#[test]
fn missing_verdict_file_is_not_recorded() {
    let f = Fx::new("gpt-6.1-terra");
    f.bind("r1", "a");
    let text = f
        .fire(&agent_event(Some("r1"), "chronos:reviewer"))
        .unwrap();
    rejected(&f, &text, "does not exist");
}

#[test]
fn needs_null_is_accepted_like_an_omitted_needs() {
    let f = Fx::new("gpt-6.1-terra");
    f.bind("r1", "a");
    f.verdict(
        "a",
        json!([{"check":"c","severity":"minor","location":"a.txt:1","evidence":"e","needs":null}]),
    );
    let text = f
        .fire(&agent_event(Some("r1"), "chronos:reviewer"))
        .unwrap();
    assert!(text.starts_with("Chronos recorded the review."), "{text}");
    assert!(text.contains("a\tverified\tnext=done\tverdict blocker=0 major=0 minor=1"), "{text}");
    assert_eq!(f.reviews("a"), 1);
    assert_eq!(f.state("a"), "verified");
}

#[test]
fn stale_tree_and_stale_pack_never_reach_verified() {
    let f = Fx::new("gpt-6.1-terra");
    f.bind("r1", "a");
    f.verdict("a", json!([]));
    fs::write(f.repo.join("a.txt"), "changed during review").unwrap();
    let text = f
        .fire(&agent_event(Some("r1"), "chronos:reviewer"))
        .unwrap();
    rejected(&f, &text, "stale review");
    // a verdict for another tree
    fs::write(f.repo.join("a.txt"), "one").unwrap();
    fs::write(
        verdict_path(&f.store, "a"),
        json!({"unit":"a","tree":"other","verdict":"pass","findings":[]}).to_string(),
    )
    .unwrap();
    let text = f
        .fire(&agent_event(Some("r1"), "chronos:reviewer"))
        .unwrap();
    rejected(&f, &text, "reviewed other");
    // a missing pack
    f.verdict("a", json!([]));
    fs::remove_file(pack_path(&f.store, "a")).unwrap();
    let text = f
        .fire(&agent_event(Some("r1"), "chronos:reviewer"))
        .unwrap();
    rejected(&f, &text, "review pack");
}

#[test]
fn same_family_reviewer_is_not_recorded() {
    let f = Fx::new("claude-opus");
    f.bind("r1", "a");
    f.verdict("a", json!([]));
    let text = f
        .fire(&agent_event(Some("r1"), "chronos:reviewer"))
        .unwrap();
    rejected(&f, &text, "same family");
}

#[test]
fn unbound_reviewer_gets_a_not_recorded_reason_and_writes_nothing() {
    let f = Fx::new("gpt-6.1-terra");
    f.verdict("a", json!([]));
    let before = files(&f.home);
    let text = f
        .fire(&agent_event(Some("unknown"), "chronos:reviewer"))
        .unwrap();
    assert!(
        text.starts_with("Chronos did not record the review: "),
        "{text}"
    );
    assert_eq!(f.reviews("a"), 0);
    assert_eq!(files(&f.home), before);
}

#[test]
fn prompt_path_fallback_needs_a_bound_marker() {
    let f = Fx::new("gpt-6.1-terra");
    f.verdict("a", json!([]));
    let mut e = agent_event(None, "chronos:reviewer");
    e["tool_input"]["prompt"] = json!(format!(
        "Review unit a, pack {}",
        pack_path(&f.store, "a").display()
    ));
    let text = f.fire(&e).unwrap();
    assert!(
        text.starts_with("Chronos did not record the review: "),
        "{text}"
    );
    assert_eq!(f.reviews("a"), 0);
    f.bind("r1", "a");
    e["tool_response"]["agentId"] = json!("unknown");
    let before = files(&f.home);
    assert!(f
        .fire(&e)
        .unwrap()
        .starts_with("Chronos did not record the review: "));
    assert_eq!(files(&f.home), before);
    e["tool_response"]
        .as_object_mut()
        .unwrap()
        .remove("agentId");
    e["tool_input"]["prompt"] = json!("review /wrong/reports/a.review.md");
    assert!(f
        .fire(&e)
        .unwrap()
        .starts_with("Chronos did not record the review: "));
    e["tool_input"]["prompt"] = json!(format!(
        "{} {}",
        pack_path(&f.store, "a").display(),
        pack_path(&f.store, "b").display()
    ));
    assert!(f
        .fire(&e)
        .unwrap()
        .starts_with("Chronos did not record the review: "));
    assert_eq!(files(&f.home), before);
    e["tool_name"] = json!("Task");
    e["tool_input"]["prompt"] = json!(format!("Review {}.", pack_path(&f.store, "a").display()));
    let text = f.fire(&e).unwrap();
    assert!(text.contains("a\tverified\tnext=done"), "{text}");
    assert_eq!(f.reviews("a"), 1);
}

#[test]
fn hooks_json_registers_post_tool_use_for_agent_calls() {
    let hooks: Value = serde_json::from_str(include_str!("../engine/hooks/hooks.json")).unwrap();
    let cmd = json!({"type":"command", "command":"\"${AXIOM_BIN:-axiom}\" hook"});
    assert_eq!(
        hooks,
        json!({"hooks": {
            "UserPromptSubmit": [{"hooks":[cmd.clone()]}],
            "SessionStart": [{"matcher":"compact", "hooks":[cmd.clone()]}],
            "Stop": [{"hooks":[cmd.clone()]}],
            "PreToolUse": [{"matcher":"*", "hooks":[cmd.clone()]}],
            "PostToolUse": [{"matcher":"Agent|Task", "hooks":[cmd]}]
        }})
    );
}

#[test]
fn goal_cwd_takes_priority_and_event_cwd_is_the_fallback() {
    let f = Fx::new("gpt-6.1-terra");
    f.bind("r1", "a");
    f.verdict("a", json!([]));
    let mut e = agent_event(Some("r1"), "chronos:reviewer");
    e["cwd"] = json!(f.home);
    assert!(f.fire(&e).unwrap().starts_with("Chronos recorded"));

    let f = Fx::new("gpt-6.1-terra");
    f.bind("r1", "a");
    f.verdict("a", json!([]));
    let path = f.home.join("goals").join("s.json");
    let mut g: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    g["cwd"] = Value::Null;
    fs::write(path, g.to_string()).unwrap();
    e["cwd"] = json!(f.repo);
    assert!(f.fire(&e).unwrap().starts_with("Chronos recorded"));
    assert_eq!(f.state("a"), "verified");
}

#[test]
fn active_goal_without_project_is_silent_and_does_not_write() {
    let f = Fx::new("gpt-6.1-terra");
    f.bind("r1", "a");
    f.verdict("a", json!([]));
    let path = f.home.join("goals").join("s.json");
    let mut g: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    g["project"] = Value::Null;
    fs::write(path, g.to_string()).unwrap();
    let before = files(&f.home);
    assert!(f
        .fire(&agent_event(Some("r1"), "chronos:reviewer"))
        .is_none());
    assert_eq!(files(&f.home), before);
}

#[test]
fn stdin_entrypoint_accepts_post_tool_use() {
    let f = Fx::new("gpt-6.1-terra");
    f.bind("r1", "a");
    f.verdict("a", json!([]));
    let mut child = Command::new(env!("CARGO_BIN_EXE_axiom"))
        .arg("hook")
        .env("AXIOM_DATA_HOME", &f.home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            agent_event(Some("r1"), "chronos:reviewer")
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    assert!(value["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .starts_with("Chronos recorded"));
    assert_eq!(f.state("a"), "verified");
}

#[test]
fn findings_use_the_review_command_cutoff() {
    let f = Fx::new("gpt-6.1-terra");
    f.bind("r1", "a");
    f.verdict(
        "a",
        json!([
            {"check":"c","severity":"blocker","location":"a","evidence":"x".repeat(300)},
            {"check":"d","severity":"major","location":"b","evidence":"proof"}
        ]),
    );
    let text = f
        .fire(&agent_event(Some("r1"), "chronos:reviewer"))
        .unwrap();
    let line = text
        .lines()
        .find(|line| line.contains("#1 blocker"))
        .unwrap()
        .trim();
    assert_eq!(line.chars().count(), 241);
    assert!(line.ends_with('…'));
    assert!(text.contains("#2 major d b: proof"));
    assert_ne!(f.state("a"), "verified");
}
