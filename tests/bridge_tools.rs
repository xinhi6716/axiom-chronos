mod common;

use axiom::{
    bridge::{call, tools, Bridge, Config},
    orch,
};
use serde_json::json;
use std::fs;

fn config(home: &std::path::Path) -> Config {
    Config {
        data_home: Some(home.into()),
        codex: Ok(common::fake("fake_codex")),
        wsl: None,
        on_windows: false,
    }
}

#[test]
fn tool_contract_and_rewrite_evidence() {
    let temp = common::temp();
    let marker = temp.path().join("started");
    let wrapper = temp.path().join("marker.py");
    fs::write(&wrapper, "import pathlib,runpy,sys\npathlib.Path(sys.argv[1]).write_text('started')\nsys.argv=sys.argv[2:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n").unwrap();
    let mut cfg = config(temp.path());
    cfg.codex = Ok(vec![
        common::python(),
        wrapper.to_string_lossy().into_owned(),
        marker.to_string_lossy().into_owned(),
        common::repo_root().join("tests/fakes/fake_codex.py").to_string_lossy().into_owned(),
    ]);
    let b = Bridge::new(cfg);
    let descriptors = tools();
    let list = descriptors.as_array().unwrap();
    let names: Vec<_> = list.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "codex_run",
            "orch",
            "run_on_linux",
            "memory_record",
            "memory_search",
            "memory_get",
            "memory_digest",
            "goal",
            "bridge_status"
        ]
    );
    assert!(!names.contains(&"review_code"));
    assert!(!names.contains(&"gemini_preview"));
    assert_eq!(
        list.iter().find(|t| t["name"] == "bridge_status").unwrap()["description"],
        "Report the Codex executable and app-server status."
    );
    let description = list.iter().find(|t| t["name"] == "codex_run").unwrap()["description"]
        .as_str()
        .unwrap();
    let phrase = "Unit mode: pass exactly `project`, `unit`, `role` (terra or sol) and `cwd`, plus `track` and `brief_file` (the brief's path in the store) to register a new unit in the same call; the bridge reads the registered brief, runs its VERIFY (cmd on Windows, sh on Linux) and, when every unit of the module has passed VERIFY, runs the module ACCEPT and returns it as `accept`. Brief mode is for role astra: pass `brief`, `cwd`, `project` and `save_to`.";
    assert!(description.contains(phrase));
    let stripped = description.replace(phrase, "");
    assert!(!stripped.contains(phrase));
    let clean = |text: &str| {
        let lower = text.to_lowercase();
        !lower.contains("gemini") && !lower.contains("rewrite")
            && !lower.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').any(|word| word == "agy")
    };
    let serialized = descriptors.to_string();
    assert!(clean(&serialized));
    assert!(!clean(&(serialized + "gemini")));
    let path = temp.path().join("orchestrate/demo/reports/_stages.evidence.tsv");
    for (name, args) in [
        ("gemini_preview", json!({"prompt":"hello","project":"demo","stage":"rewrite"})),
        ("review_code", json!({})),
    ] {
        let removed = call(&b, name, &args);
        assert_eq!(removed["structuredContent"]["status"], "bridge_error");
        assert_eq!(removed["structuredContent"]["error"]["kind"], "unknown_tool");
        assert!(!path.exists());
        assert!(!marker.exists());
    }
    let status = call(&b, "bridge_status", &json!({}));
    assert_eq!(status["structuredContent"]["status"], "completed");
    assert!(status["structuredContent"].get("codex").is_some());
    assert!(status["structuredContent"].get("agy").is_none());
    assert!(marker.exists());
}

#[test]
fn legacy_second_review_evidence_remains_readable() {
    let temp = common::temp();
    let store = temp.path();
    fs::create_dir_all(store.join("reports")).unwrap();
    fs::write(
        store.join("reports/_stages.evidence.tsv"),
        "kind\tcommand\texit_code\tcommit\tts\nsecond_review\tclaude-critic\t0\t\t1\n",
    )
    .unwrap();
    assert!(orch::astra_gate(store).is_err());
    orch::record_stage(store, "astra", "gpt-6-astra", true).unwrap();
    fs::create_dir_all(store.join("astra")).unwrap();
    for name in ["plan.json", "review.json", "ruling.md", "plan-final.md"] {
        fs::write(store.join("astra").join(name), "content").unwrap();
    }
    assert!(orch::astra_gate(store).is_ok());
    let rows = fs::read_to_string(store.join("reports/_stages.evidence.tsv")).unwrap();
    assert!(rows
        .lines()
        .any(|line| line.starts_with("second_review\tclaude-critic\t0\t")));
}
