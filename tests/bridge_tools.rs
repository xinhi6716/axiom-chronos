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
        agy: Ok(common::fake("fake_agy")),
        wsl: None,
        on_windows: false,
    }
}

#[test]
fn tool_contract_and_rewrite_evidence() {
    let temp = common::temp();
    let count = temp.path().join("agy-count");
    let wrapper = temp.path().join("agy_wrapper.py");
    fs::write(&wrapper, "import os,runpy,sys\nos.environ['FAKE_AGY_COUNT']=sys.argv[1]\nrunpy.run_path(sys.argv[2],run_name='__main__')\n").unwrap();
    let mut config = config(temp.path());
    config.agy = Ok(vec![
        common::python(),
        wrapper.to_string_lossy().into_owned(),
        count.to_string_lossy().into_owned(),
        common::repo_root()
            .join("tests/fakes/fake_agy.py")
            .to_string_lossy()
            .into_owned(),
    ]);
    let b = Bridge::new(config);
    let descriptors = tools();
    let list = descriptors.as_array().unwrap();
    let names: Vec<_> = list.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "codex_run",
            "gemini_preview",
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
    let description = list.iter().find(|t| t["name"] == "codex_run").unwrap()["description"]
        .as_str()
        .unwrap();
    let phrase = "Unit mode: pass exactly `project`, `unit`, `role` (terra or sol) and `cwd`; the bridge reads the registered brief, runs its VERIFY (cmd on Windows, sh on Linux) and, when every unit of the module has passed VERIFY, runs the module ACCEPT and returns it as `accept`. Brief mode is for role astra: pass `brief`, `cwd`, `project` and `save_to`.";
    assert!(description.contains(phrase));
    let stripped = description.replace(phrase, "");
    assert!(!stripped.contains(phrase));
    let preview = list.iter().find(|t| t["name"] == "gemini_preview").unwrap();
    assert_eq!(
        preview["inputSchema"]["properties"]["stage"]["enum"],
        json!(["rewrite"])
    );
    let removed = call(&b, "review_code", &json!({}));
    assert_eq!(removed["structuredContent"]["status"], "bridge_error");
    assert_eq!(
        removed["structuredContent"]["error"]["kind"],
        "unknown_tool"
    );
    let invalid = call(
        &b,
        "gemini_preview",
        &json!({"prompt":"hello","project":"demo","stage":"second_review"}),
    );
    assert_eq!(invalid["structuredContent"]["status"], "bridge_error");
    assert_eq!(
        invalid["structuredContent"]["error"]["kind"],
        "invalid_argument"
    );
    assert_eq!(
        invalid["structuredContent"]["error"]["message"],
        "invalid stage"
    );
    let path = temp
        .path()
        .join("orchestrate/demo/reports/_stages.evidence.tsv");
    assert!(!path.exists());
    assert!(!count.exists());
    let ok = call(
        &b,
        "gemini_preview",
        &json!({"prompt":"hello","project":"demo","stage":"rewrite"}),
    );
    assert_eq!(ok["structuredContent"]["status"], "completed");
    let rows = fs::read_to_string(path).unwrap();
    assert!(rows
        .lines()
        .any(|line| line.starts_with("rewrite\t") && line.split('\t').nth(2) == Some("0")));
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
