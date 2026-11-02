use axiom::{goal, orch};
use std::{fs, path::Path};

const HEADER: &str = "kind\tcommand\texit_code\tcommit\tts\n";

fn setup(home: &Path) -> std::path::PathBuf {
    let store = home.join("orchestrate/project");
    fs::create_dir_all(store.join("reports")).unwrap();
    units(&store, "verified");
    fs::write(
        store.join("reports/unit.evidence.tsv"),
        format!("{HEADER}run\tworker\t0\t\t1000\n"),
    )
    .unwrap();
    fs::write(
        store.join("reports/_full.evidence.tsv"),
        format!("{HEADER}full\ttest\t0\t\t2000\n"),
    )
    .unwrap();
    store
}

fn units(store: &Path, state: &str) {
    fs::write(
        store.join("units.tsv"),
        format!("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nunit\tt\t{state}\t\t\t\t\n"),
    )
    .unwrap();
}

#[test]
fn done_without_rewrite_row() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let store = setup(home);
    assert!(!store.join("reports/_stages.evidence.tsv").exists());
    assert!(orch::done_gate(&store, true).is_ok());
    goal::open(home, "s", "wish", Some("project")).unwrap();
    assert_eq!(
        goal::set_status(home, "s", "done", None).unwrap()["status"],
        "completed"
    );

    units(&store, "pending");
    let error = orch::done_gate(&store, true).unwrap_err();
    assert_eq!(
        (error.kind.as_str(), error.message.as_str()),
        ("not_done", "unit=pending")
    );
    units(&store, "verified");
    fs::write(
        store.join("reports/_full.evidence.tsv"),
        format!("{HEADER}full\ttest\t0\t\t2000\nfull\ttest\t1\t\t3000\n"),
    )
    .unwrap();
    let error = orch::done_gate(&store, true).unwrap_err();
    assert_eq!(
        (error.kind.as_str(), error.message.as_str()),
        ("not_done", "missing successful, current full test")
    );
    fs::write(
        store.join("reports/_full.evidence.tsv"),
        format!("{HEADER}full\ttest\t0\t\t0999\n"),
    )
    .unwrap();
    assert_eq!(orch::done_gate(&store, true).unwrap_err().kind, "not_done");
    let missing = home.join("missing");
    assert!(orch::done_gate(&missing, false).is_ok());
    let error = orch::done_gate(&missing, true).unwrap_err();
    assert_eq!(
        (error.kind.as_str(), error.message.as_str()),
        ("not_done", "no verified units")
    );
}

#[test]
fn rewrite_stage_is_refused_second_review_kept() {
    let temp = tempfile::tempdir().unwrap();
    let store = setup(temp.path());
    for kind in ["rewrite", "unknown"] {
        let error = orch::record_stage(&store, kind, "model", true).unwrap_err();
        assert_eq!(
            (error.kind.as_str(), error.message.as_str()),
            ("invalid_argument", "invalid stage")
        );
        assert!(!store.join("reports/_stages.evidence.tsv").exists());
    }
    orch::record_stage(&store, "astra", "gpt-6-astra", true).unwrap();
    orch::record_stage(&store, "second_review", "claude-critic", true).unwrap();
    let stages = fs::read_to_string(store.join("reports/_stages.evidence.tsv")).unwrap();
    assert_eq!(stages.lines().count(), 3);
    assert!(stages.contains("astra\tgpt-6-astra\t0\t"));
    assert!(stages.contains("second_review\tclaude-critic\t0\t"));
}

#[test]
fn legacy_stage_rows_stay_readable() {
    let temp = tempfile::tempdir().unwrap();
    let store = setup(temp.path());
    let path = store.join("reports/_stages.evidence.tsv");
    let legacy = "kind\tcommand\texit_code\tcommit\tts\r\nsecond_review\tclaude-critic\t0\t\t1000\r\nrewrite\tmodel\t0\t\t1000\r\n";
    fs::write(&path, legacy).unwrap();
    assert_eq!(orch::astra_gate(&store).unwrap_err().kind, "astra_gate");
    orch::record_stage(&store, "astra", "gpt-6-astra", true).unwrap();
    fs::create_dir_all(store.join("astra")).unwrap();
    for file in ["plan.json", "review.json", "ruling.md", "plan-final.md"] {
        fs::write(store.join("astra").join(file), "ok").unwrap();
    }
    assert!(orch::astra_gate(&store).is_ok());
    assert!(orch::done_gate(&store, true).is_ok());
    assert!(fs::read(&path).unwrap().starts_with(legacy.as_bytes()));
}
