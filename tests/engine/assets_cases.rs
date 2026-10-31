use axiom::{bridge, engine};
use regex::Regex;
use serde_json::{json, Value};
use std::{collections::BTreeSet, fs, path::Path};

fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("engine")
}
fn files(dir: &Path, base: &Path, out: &mut BTreeSet<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files(&path, base, out);
        } else {
            out.insert(
                path.strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}
fn asset(path: &str) -> &'static str {
    engine::FILES.iter().find(|(p, _)| *p == path).unwrap().1
}
fn json_asset(path: &str) -> Value {
    serde_json::from_str(asset(path)).unwrap()
}

#[test]
fn embedded_files_match_disk_and_install_idempotently() {
    let mut disk = BTreeSet::new();
    files(&root(), &root(), &mut disk);
    let embedded: BTreeSet<_> = engine::FILES.iter().map(|(p, _)| p.to_string()).collect();
    assert_eq!(embedded.len(), engine::FILES.len());
    assert_eq!(disk, embedded);
    for (path, text) in engine::FILES {
        assert_eq!(fs::read(root().join(path)).unwrap(), text.as_bytes());
    }
    let temp = tempfile::tempdir().unwrap();
    let dir = engine::plugin_dir(temp.path()).unwrap();
    assert_eq!(
        dir,
        temp.path().join("engine").join(env!("CARGO_PKG_VERSION"))
    );
    for (path, text) in engine::FILES {
        assert_eq!(fs::read(dir.join(path)).unwrap(), text.as_bytes());
    }
    let first = dir.join(engine::FILES[0].0);
    let before = fs::metadata(&first).unwrap().modified().unwrap();
    assert_eq!(engine::plugin_dir(temp.path()).unwrap(), dir);
    assert_eq!(fs::metadata(&first).unwrap().modified().unwrap(), before);
    fs::write(&first, "changed").unwrap();
    engine::plugin_dir(temp.path()).unwrap();
    assert_eq!(fs::read_to_string(first).unwrap(), engine::FILES[0].1);
}

#[test]
fn codex_manifest_preserves_authorized_light_and_dark_artwork() {
    use sha2::{Digest, Sha256};
    let root = root().parent().unwrap().join("codex-plugin");
    let manifest: Value =
        serde_json::from_slice(&fs::read(root.join(".codex-plugin/plugin.json")).unwrap()).unwrap();
    for (field, path, expected) in [
        (
            "logo",
            "./assets/chronos.svg",
            "87e269750c24d6a095d2e8706c00cb17f498e57cf711eaeaff86c8351aa2e477",
        ),
        (
            "logoDark",
            "./assets/chronos-dark.svg",
            "4c2275607151611489b4f2a32a66d64ba2110f9dce01f9a20806f29c5a6f6938",
        ),
        (
            "composerIcon",
            "./assets/chronos.svg",
            "87e269750c24d6a095d2e8706c00cb17f498e57cf711eaeaff86c8351aa2e477",
        ),
    ] {
        assert_eq!(manifest["interface"][field], path);
        let mut bytes = fs::read(root.join(path)).unwrap();
        let digest = |b: &[u8]| format!("{:x}", Sha256::digest(b));
        assert_eq!(
            digest(&bytes),
            expected,
            "authorized artwork changed: {path}"
        );
        bytes[0] ^= 1;
        assert_ne!(digest(&bytes), expected); // Detect content changes, not just file presence.
    }
}

#[test]
fn references_tools_and_agents_exist() {
    let references = Regex::new(r#"\$\{CLAUDE_PLUGIN_ROOT\}/([^\s`"'\\)]+)"#).unwrap();
    let tools = Regex::new(r"mcp__plugin_([a-z0-9-]+)_([a-z0-9-]+)__([a-z_]+)").unwrap();
    let agents = Regex::new(r"chronos:([a-z-]+)").unwrap();
    let served = bridge::tools();
    let served = served.as_array().unwrap();
    for (path, text) in engine::FILES {
        for cap in references.captures_iter(text) {
            assert!(root().join(&cap[1]).is_file(), "{path}: {}", &cap[1]);
        }
        for cap in tools.captures_iter(text) {
            assert_eq!((&cap[1], &cap[2]), ("chronos", "bridge"));
            assert!(
                served.iter().any(|t| t["name"] == &cap[3]),
                "{path}: {}",
                &cap[3]
            );
        }
        for cap in agents.captures_iter(text) {
            let name = &cap[1];
            assert!(
                name == "bridge"
                    || root().join(format!("agents/{name}.md")).is_file()
                    || root().join(format!("skills/{name}")).is_dir(),
                "{path}: {name}"
            );
        }
    }
    let codex = served.iter().find(|t| t["name"] == "codex_run").unwrap();
    assert!(codex["description"]
        .as_str()
        .unwrap()
        .contains("${CLAUDE_PLUGIN_ROOT}/DISPATCH.md"));
    assert!(root().join("DISPATCH.md").is_file());
}

fn strict(node: &Value) {
    match node {
        Value::Object(map) => {
            if map.get("type") == Some(&json!("object")) {
                assert_eq!(map.get("additionalProperties"), Some(&json!(false)));
                let props = map["properties"].as_object().unwrap();
                let required: BTreeSet<_> = map["required"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap())
                    .collect();
                assert_eq!(required, props.keys().map(String::as_str).collect());
            }
            for value in map.values() {
                strict(value);
            }
        }
        Value::Array(items) => {
            for item in items {
                strict(item);
            }
        }
        _ => {}
    }
}
fn coverage_ok(schema: &Value) -> bool {
    schema["required"] == json!(["summary", "coverage", "changes", "out_of_scope"])
        && schema["properties"]
            .as_object()
            .is_some_and(|p| p.len() == 4)
        && schema["properties"]["coverage"]
            == json!({"type":"object", "additionalProperties":false, "required":["read", "not_read"], "properties":{"read":{"type":"array", "items":{"type":"string"}}, "not_read":{"type":"array", "items":{"type":"string"}}}})
}
#[test]
fn schemas_are_strict_and_coverage_has_negative_control() {
    for name in ["plan", "audit", "review"] {
        strict(&json_asset(&format!("research/{name}.schema.json")));
    }
    assert_eq!(
        json_asset("research/review.schema.json")["properties"]["verdict"]["enum"],
        json!(["pass", "revise", "reject"])
    );
    assert_eq!(
        json_asset("research/review.schema.json")["properties"]["findings"]["items"]["properties"]
            ["check"]["enum"],
        json!(["out_of_scope", "overclaim", "unfalsifiable", "complexity"])
    );
    let mut audit = json_asset("research/audit.schema.json");
    assert!(coverage_ok(&audit));
    audit["required"]
        .as_array_mut()
        .unwrap()
        .retain(|x| x != "coverage");
    assert!(!coverage_ok(&audit));
}

#[test]
fn models_and_binary_configuration() {
    for (agent, model, effort) in [
        ("architect", "astra", "high"),
        ("tester", "astra", "high"),
        ("dispatcher", "terra", "high"),
        ("astra", "astra", "max"),
        ("critic", "sol", "high"),
        ("reviewer", "sol", "high"),
        ("memory-keeper", "terra", "medium"),
        ("content", "luna", "low"),
    ] {
        let text = asset(&format!("agents/{agent}.md"));
        let front = text.split("---").nth(1).unwrap();
        assert!(
            front
                .lines()
                .any(|line| line.trim() == format!("model: {model}")),
            "{agent}"
        );
        assert!(
            front
                .lines()
                .any(|line| line.trim() == format!("effort: {effort}")),
            "{agent}"
        );
    }
    let plugin = json_asset(".claude-plugin/plugin.json");
    assert_eq!(plugin["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        plugin["mcpServers"]["bridge"],
        json!({"command":"${AXIOM_BIN:-axiom}","args":["bridge"]})
    );
    let hooks = json_asset("hooks/hooks.json");
    for event in ["UserPromptSubmit", "Stop", "PreToolUse"] {
        assert_eq!(
            hooks["hooks"][event][0]["hooks"][0]["command"],
            "\"${AXIOM_BIN:-axiom}\" hook"
        );
    }
    assert_eq!(hooks["hooks"]["PreToolUse"][0]["matcher"], "Write|Edit|NotebookEdit|Bash|AskUserQuestion|Agent|Task|mcp__plugin_chronos_bridge__codex_run");
    let marketplace: Value = serde_json::from_str(
        &fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(".claude-plugin/marketplace.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(marketplace["plugins"][0]["source"], "./engine");
    assert_eq!(marketplace["plugins"][0]["version"], env!("CARGO_PKG_VERSION"));
    for (path, text) in engine::FILES {
        for forbidden in ["AXIOM_PYTHON", "bun", "vendor", "orch.ts"] {
            assert!(!text.contains(forbidden), "{path}: {forbidden}");
        }
    }
}
