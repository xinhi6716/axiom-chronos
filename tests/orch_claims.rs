use axiom::orch::{self, Begin};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

struct Fixture {
    root: PathBuf,
    store: PathBuf,
    repo: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "orch-claims-{}-{}",
            std::process::id(),
            axiom::base::unique()
        ));
        let store = root.join("orchestrate/demo");
        let repo = root.join("repo");
        fs::create_dir_all(store.join("reports")).unwrap();
        fs::create_dir_all(store.join("inbox")).unwrap();
        fs::create_dir_all(&repo).unwrap();
        let brief = store.join("a.brief");
        fs::write(
            &brief,
            "GOAL test\nSCOPE a.txt\nVERIFY echo evidence && exit 7\n",
        )
        .unwrap();
        fs::write(
            store.join("units.tsv"),
            format!(
                "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\na\tm\tpending\t\t\t\t{}\n",
                brief.display()
            ),
        )
        .unwrap();
        Self { root, store, repo }
    }
    fn add_overlapping_unit(&self) {
        let brief = self.store.join("b.brief");
        fs::write(&brief, "GOAL test\nSCOPE a.txt\nVERIFY echo ok\n").unwrap();
        use std::io::Write;
        writeln!(
            fs::OpenOptions::new()
                .append(true)
                .open(self.store.join("units.tsv"))
                .unwrap(),
            "b\tm\tpending\t\t\t\t{}",
            brief.display()
        )
        .unwrap();
    }
    fn begin(&self) -> String {
        match orch::begin_unit(&self.store, "a", "worker", &self.repo, false).unwrap() {
            Begin::Run { attempt_id, .. } => attempt_id,
            Begin::Reused => panic!("unexpected reuse"),
        }
    }
    fn cli(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_axiom"))
            .env("AXIOM_DATA_HOME", &self.root)
            .env_remove("ORCH_STORE")
            .arg("orch")
            .arg("--store")
            .arg(&self.store)
            .args(args)
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn persisted_claim_is_unknown_in_another_process_and_release_checks_id() {
    let f = Fixture::new();
    f.add_overlapping_unit();
    let id = f.begin();
    let claims = fs::read_to_string(f.store.join("claims.tsv")).unwrap();
    let mut lines = claims.lines();
    assert_eq!(lines.next(), Some("unit\tattempt\tscope\tstarted"));
    let fields: Vec<_> = lines.next().unwrap().split('\t').collect();
    assert_eq!(&fields[..3], &["a", &id, "a.txt"]);
    assert!(!fields[3].is_empty());
    assert!(lines.next().is_none());
    for unit in ["a", "b"] {
        assert_eq!(
            orch::begin_unit(&f.store, unit, "worker", &f.repo, false)
                .err()
                .unwrap()
                .kind,
            "busy"
        );
    }
    assert!(!orch::ready_units(&f.store)
        .unwrap()
        .contains(&"b".to_string()));
    assert!(orch::waiting_units(&f.store)
        .unwrap()
        .iter()
        .any(|w| w.unit == "b"));
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "fresh_process_claim", "--nocapture"])
        .env("CLAIM_TEST_STORE", &f.store)
        .env("CLAIM_TEST_REPO", &f.repo)
        .env("CLAIM_TEST_ID", &id)
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}",
        String::from_utf8_lossy(&child.stdout)
    );
    let output = f.cli(&["--json", "ready"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ready"], json!([]));
    assert_eq!(value["waiting"][0]["next"], format!("unknown {id}"));
    assert!(!f
        .cli(&["release", "a", "--attempt", "wrong"])
        .status
        .success());
    assert!(f.cli(&["release", "a", "--attempt", &id]).status.success());
    assert_eq!(orch::ready_units(&f.store).unwrap(), vec!["a"]);
}

#[test]
fn vanished_claim_blocks_overlap_and_stale_attempt_cannot_release_newer() {
    let f = Fixture::new();
    fs::write(
        f.store.join("claims.tsv"),
        "unit\tattempt\tscope\tstarted\r\na\t1-vanished\ta.txt\t2026-01-01T00:00:00Z\r\n",
    )
    .unwrap();
    let err = orch::begin_unit(&f.store, "a", "worker", &f.repo, false)
        .err()
        .unwrap();
    assert_eq!(err.kind, "outcome_unknown");
    assert!(err.message.contains("1-vanished"));
    assert!(orch::ready_units(&f.store).unwrap().is_empty());
    assert!(f
        .cli(&["release", "a", "--attempt", "1-vanished"])
        .status
        .success());
    let newer = f.begin();
    assert!(!orch::abort_unit(&f.store, "a", "1-vanished"));
    assert!(orch::finish_unit(
        &f.store,
        "a",
        "1-vanished",
        &f.repo,
        "worker",
        "model",
        &json!({"status":"completed"})
    )
    .is_err());
    assert_eq!(orch::waiting_units(&f.store).unwrap()[0].next, "running");
    assert!(orch::abort_unit(&f.store, "a", &newer));
}

#[test]
fn verify_output_is_readable_and_failure_is_recorded() {
    let f = Fixture::new();
    let id = f.begin();
    let result = orch::finish_unit(
        &f.store,
        "a",
        &id,
        &f.repo,
        "worker",
        "model",
        &json!({"status":"completed", "final_message":"worker done"}),
    )
    .unwrap();
    assert_eq!(result.state, "failed");
    assert_eq!(result.verify_exit, Some(7));
    let path = result.verify_output.unwrap();
    assert!(fs::read_to_string(&path).unwrap().contains("evidence"));
    let report = fs::read_to_string(f.store.join("reports/a.md")).unwrap();
    assert_eq!(
        report,
        format!(
            "worker done\nBRIDGE VERIFY exit=7 output={}\n",
            path.display()
        )
    );
}

#[test]
fn fresh_process_claim() {
    let Some(store) = std::env::var_os("CLAIM_TEST_STORE") else {
        return;
    };
    let repo = std::env::var_os("CLAIM_TEST_REPO").unwrap();
    let id = std::env::var("CLAIM_TEST_ID").unwrap();
    for unit in ["a", "b"] {
        let err = orch::begin_unit(Path::new(&store), unit, "worker", Path::new(&repo), false)
            .err()
            .unwrap();
        assert_eq!(err.kind, "outcome_unknown");
        assert!(err.message.contains(&id));
    }
}

#[test]
fn verify_output_truncation() {
    let f = Fixture::new();
    let expected = "01234567\r\n".repeat(2100);
    fs::write(f.repo.join("long-output.txt"), &expected).unwrap();
    #[cfg(windows)]
    let command = "type long-output.txt";
    #[cfg(not(windows))]
    let command = "cat long-output.txt";
    fs::write(
        f.store.join("a.brief"),
        format!("GOAL test\nSCOPE a.txt\nVERIFY {command}\n"),
    )
    .unwrap();
    let id = f.begin();
    let result = orch::finish_unit(
        &f.store,
        "a",
        &id,
        &f.repo,
        "worker",
        "model",
        &json!({"status":"completed"}),
    )
    .unwrap();
    assert_eq!(result.verify_exit, Some(0));
    assert_eq!(
        fs::read_to_string(result.verify_output.unwrap()).unwrap(),
        expected[1000..]
    );
    assert_eq!(result.verify_tail, expected[19000..]);
}

#[test]
fn verify_not_run_reasons() {
    for report in [
        json!({"status":"failed"}),
        json!({"status":"completed", "file_changes":[{"path":"outside.txt"}]}),
    ] {
        let f = Fixture::new();
        let id = f.begin();
        let result =
            orch::finish_unit(&f.store, "a", &id, &f.repo, "worker", "model", &report).unwrap();
        assert_eq!(result.verify_exit, None);
        let reason = if report["status"] == "failed" {
            "worker did not complete"
        } else {
            "changes outside scope"
        };
        assert_eq!(
            fs::read_to_string(f.store.join("reports/a.md")).unwrap(),
            format!("BRIDGE VERIFY not run: {reason}\n")
        );
    }
}

#[test]
fn verify_spawn_error_is_reported() {
    let f = Fixture::new();
    fs::write(
        f.store.join("a.brief"),
        "GOAL test\nSCOPE a.txt\nVERIFY echo \0bad\n",
    )
    .unwrap();
    let id = f.begin();
    let result = orch::finish_unit(
        &f.store,
        "a",
        &id,
        &f.repo,
        "worker",
        "model",
        &json!({"status":"completed"}),
    )
    .unwrap();
    assert_eq!(result.verify_exit, None);
    let report = fs::read_to_string(f.store.join("reports/a.md")).unwrap();
    assert!(report.starts_with("BRIDGE VERIFY not run: "));
    assert!(report.trim_end().len() > "BRIDGE VERIFY not run: ".len());
}

#[test]
fn legacy_stores_and_handoff_fields() {
    for name in ["old", "old-crlf"] {
        let store = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/orch/stores")
            .join(name);
        assert!(!orch::read_units(&store).unwrap().is_empty());
        assert!(!store.join("claims.tsv").exists());
    }
    let f = Fixture::new();
    assert!(!f.store.join("claims.tsv").exists());
    assert_eq!(orch::ready_units(&f.store).unwrap(), vec!["a"]);
    assert!(orch::waiting_units(&f.store).unwrap().is_empty());
    assert!(!f.store.join("claims.tsv").exists());
    let dispatch = include_str!("../engine/DISPATCH.md");
    let block = dispatch
        .split("## HANDOFF")
        .nth(1)
        .unwrap()
        .split("```")
        .nth(1)
        .unwrap();
    let mut lines = block.lines().filter(|line| !line.trim().is_empty());
    assert_eq!(lines.next(), Some("HANDOFF"));
    let fields: Vec<_> = lines
        .map(|line| line.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(fields, ["RESULT", "PATHS", "COMMANDS", "OPEN", "EVIDENCE"]);
    let dispatcher = include_str!("../engine/agents/dispatcher.md");
    assert!(dispatcher.contains("A downstream brief's CONTEXT carries only the HANDOFF blocks of the units its module DEPENDS on for upstream unit results."));
}
