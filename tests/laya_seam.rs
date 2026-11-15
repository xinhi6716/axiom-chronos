#[allow(unused_imports)]
use axiom::{
    evolve::{golden, harness, receipts},
    laya::{curate, infer, ort, pipeline, router, shadow, tokenizer, train},
    usage::laya,
};
use serde_json::{json, Value};
use std::{fs, path::PathBuf, process::Command};

fn assert_no_laya(value: &Value) {
    assert!(!value.as_object().unwrap().contains_key("laya"));
}

// Later units supersede the contract's stub text ("not built yet" and empty
// usage stubs); only the exit code 2 contract is retained.
#[test]
fn stubs_and_paths() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(axiom::laya::laya_dir(home.path()), home.path().join("laya"));
    assert_eq!(pipeline::cli(&[]), 2);
    assert_eq!(laya::cli_text(home.path()).unwrap(), "");
    assert_eq!(laya::cli_json(home.path()).unwrap(), None);
}

// Later units supersede the contract's stub text ("not built yet" and empty
// usage stubs); only the exit code 2 contract is retained.
#[test]
fn laya_command_dispatches_to_pipeline_cli() {
    let output = Command::new(env!("CARGO_BIN_EXE_axiom"))
        .args(["laya", "anything"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr).unwrap().starts_with("axiom: laya: unknown subcommand"));
}

#[test]
fn usage_json_preserves_ledger_output() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let data = root.path().join("data");
    let config = root.path().join("claude");
    let projects = config.join("projects");
    let project = projects.join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("session.jsonl"), "").unwrap();
    let locations = axiom::usage::Locations { home: home.clone(), projects };
    let ledger = axiom::usage::ledger(&locations, Some("session")).unwrap();
    let expected = format!("{}\n", serde_json::to_string_pretty(&ledger).unwrap());
    let output = Command::new(env!("CARGO_BIN_EXE_axiom"))
        .args(["usage", "--json", "--session", "session"])
        .env("AXIOM_HOME", &home)
        .env("AXIOM_DATA_HOME", &data)
        .env("CLAUDE_CONFIG_DIR", &config)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.stdout, expected.as_bytes());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_no_laya(&value);
}

struct PopulatedUsage {
    _root: tempfile::TempDir,
    home: PathBuf,
    data: PathBuf,
    config: PathBuf,
    ledger: axiom::usage::Ledger,
}

fn populated_usage() -> PopulatedUsage {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let data = root.path().join("data");
    let config = root.path().join("claude");
    let projects = config.join("projects");
    let project = projects.join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("session.jsonl"), "").unwrap();
    let locations = axiom::usage::Locations { home: home.clone(), projects };
    let ledger = axiom::usage::ledger(&locations, Some("session")).unwrap();
    laya::record(
        &data,
        &laya::UsageEvent {
            id: "event-1".into(),
            ts_ms: 123,
            model_version: "test-model".into(),
            kind: "inference".into(),
            backend: "cpu".into(),
            tokens: Some(7),
            gpu_hours: Some(0.5),
            compute_units: Some(2.0),
        },
    )
    .unwrap();
    PopulatedUsage { _root: root, home, data, config, ledger }
}

impl PopulatedUsage {
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_axiom"));
        command
            .env("AXIOM_HOME", &self.home)
            .env("AXIOM_DATA_HOME", &self.data)
            .env("CLAUDE_CONFIG_DIR", &self.config);
        command
    }
}

#[test]
fn usage_json_includes_populated_laya_and_preserves_ledger() {
    let setup = populated_usage();
    let output = setup
        .command()
        .args(["usage", "--json", "--session", "session"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let mut value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["laya"], laya::cli_json(&setup.data).unwrap().unwrap());
    value.as_object_mut().unwrap().remove("laya");
    assert_eq!(value, serde_json::to_value(&setup.ledger).unwrap());
}

#[test]
fn usage_text_appends_populated_laya_to_ledger() {
    let setup = populated_usage();
    let output = setup
        .command()
        .args(["usage", "--session", "session"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let laya_text = laya::cli_text(&setup.data).unwrap();
    assert!(!laya_text.is_empty());
    assert!(laya_text.contains("test-model"));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("{}{}", axiom::usage::render(&setup.ledger), laya_text)
    );
}

#[test]
#[should_panic]
fn absence_helper_rejects_laya_key() {
    assert_no_laya(&json!({"laya": null}));
}
