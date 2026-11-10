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
        temp.path().join("engine").join(format!(
            "{}-{}",
            env!("CARGO_PKG_VERSION"),
            engine::build_id()
        ))
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
    for name in ["plan", "audit"] {
        strict(&json_asset(&format!("research/{name}.schema.json")));
    }
    assert_eq!(
        json_asset("research/review.schema.json")["properties"]["verdict"]["enum"],
        json!(["pass", "revise", "reject"])
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
    for (agent, model) in [
        ("architect", "opus"),
        ("dispatcher", "sonnet"),
        ("astra", "sonnet"),
        ("critic", "sonnet"),
        ("reviewer", "sonnet"),
        ("memory-keeper", "haiku"),
    ] {
        let text = asset(&format!("agents/{agent}.md"));
        let front = text.split("---").nth(1).unwrap();
        assert!(
            front
                .lines()
                .any(|line| line.trim() == format!("model: {model}")),
            "{agent}"
        );
    }
    let plugin = json_asset(".claude-plugin/plugin.json");
    assert_eq!(plugin["version"], "2.0.0");
    assert_eq!(
        plugin["mcpServers"]["bridge"],
        json!({"command":"${AXIOM_BIN:-axiom}","args":["bridge"]})
    );
    let hooks = json_asset("hooks/hooks.json");
    assert_eq!(hooks["hooks"]["SessionStart"][0]["matcher"], "compact");
    for event in ["UserPromptSubmit", "SessionStart", "Stop", "PreToolUse"] {
        assert_eq!(
            hooks["hooks"][event][0]["hooks"][0]["command"],
            "\"${AXIOM_BIN:-axiom}\" hook"
        );
    }
    assert_eq!(hooks["hooks"]["PreToolUse"][0]["matcher"], "*");
    let marketplace: Value = serde_json::from_str(
        &fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(".claude-plugin/marketplace.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(marketplace["plugins"][0]["source"], "./engine");
    assert_eq!(marketplace["plugins"][0]["version"], "2.0.0");
    for (path, text) in engine::FILES {
        for forbidden in ["AXIOM_PYTHON", "bun", "vendor", "orch.ts"] {
            assert!(!text.contains(forbidden), "{path}: {forbidden}");
        }
    }
}

#[test]
fn builds_with_one_version_never_share_or_clean_an_engine_directory() {
    let temp = tempfile::tempdir().unwrap();
    let older = temp.path().join("engine/1.0.0/agents/old.md");
    fs::create_dir_all(older.parent().unwrap()).unwrap();
    fs::write(&older, "a running older build still reads this").unwrap();
    let sibling = temp.path().join("engine/1.0.0-000000000000/rules.md");
    fs::create_dir_all(sibling.parent().unwrap()).unwrap();
    fs::write(&sibling, "another build's rules").unwrap();

    let a = engine::install(temp.path(), "1.0.0", &[("rules.md", "build a")]).unwrap();
    let b = engine::install(temp.path(), "1.0.0", &[("rules.md", "build b")]).unwrap();
    assert_ne!(a, b);
    assert_eq!(fs::read_to_string(a.join("rules.md")).unwrap(), "build a");
    assert_eq!(fs::read_to_string(b.join("rules.md")).unwrap(), "build b");
    assert_eq!(
        engine::install(temp.path(), "1.0.0", &[("rules.md", "build a")]).unwrap(),
        a
    );
    assert_eq!(fs::read_to_string(a.join("rules.md")).unwrap(), "build a");
    assert_eq!(
        fs::read_to_string(&older).unwrap(),
        "a running older build still reads this"
    );
    assert_eq!(
        fs::read_to_string(&sibling).unwrap(),
        "another build's rules"
    );
    let name = a.file_name().unwrap().to_str().unwrap();
    assert!(
        Regex::new(r"^1\.0\.0-[0-9a-f]{12}$")
            .unwrap()
            .is_match(name),
        "{name}"
    );
}

#[test]
fn content_id_follows_every_path_and_byte() {
    let id = engine::content_id;
    assert_eq!(id(&[("a", "b")]), id(&[("a", "b")]));
    assert_ne!(id(&[("a", "b")]), id(&[("a", "c")]));
    assert_ne!(id(&[("a", "b")]), id(&[("c", "b")]));
    assert_ne!(id(&[("ab", "c")]), id(&[("a", "bc")]));
    assert_ne!(id(&[("a", "b"), ("c", "d")]), id(&[("a", "bcd")]));
    assert_eq!(engine::build_id(), id(engine::FILES));
}

#[test]
fn every_release_manifest_carries_the_crate_version() {
    let version = env!("CARGO_PKG_VERSION");
    assert_eq!(json_asset(".claude-plugin/plugin.json")["version"], version);
    let marketplace: Value = serde_json::from_str(
        &fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(".claude-plugin/marketplace.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(marketplace["plugins"][0]["version"], version);
}

fn with_plugin_version(root: &Path, version: &str) {
    let file = root.join(".claude-plugin/plugin.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    manifest["version"] = json!(version);
    fs::write(file, manifest.to_string()).unwrap();
}

#[test]
fn plugin_of_another_version_is_refused_naming_both_and_the_fix() {
    let temp = tempfile::tempdir().unwrap();
    let root = engine::plugin_dir(temp.path()).unwrap();
    assert_eq!(
        engine::check_plugin(&root).unwrap(),
        engine::PluginCheck {
            version: env!("CARGO_PKG_VERSION").into(),
            differing: vec![],
        }
    );
    with_plugin_version(&root, "0.3.0-codex");
    let reason = engine::check_plugin(&root).unwrap_err();
    for part in [
        "0.3.0-codex",
        env!("CARGO_PKG_VERSION"),
        "claude plugin update chronos@axiom-chronos",
        "AXIOM_BIN",
        "axiom doctor",
    ] {
        assert!(reason.contains(part), "{reason}");
    }
}

#[test]
fn plugin_without_a_readable_version_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("plugin");
    let reason = engine::check_plugin(&root).unwrap_err();
    assert!(reason.contains("cannot read"), "{reason}");
    assert!(reason.contains(env!("CARGO_PKG_VERSION")), "{reason}");
    fs::create_dir_all(root.join(".claude-plugin")).unwrap();
    for (manifest, part) in [("{", "not valid JSON"), ("{}", "declares no version")] {
        fs::write(root.join(".claude-plugin/plugin.json"), manifest).unwrap();
        let reason = engine::check_plugin(&root).unwrap_err();
        assert!(reason.contains(part), "{reason}");
    }
}

#[test]
fn plugin_files_are_compared_by_content_but_not_line_endings() {
    let temp = tempfile::tempdir().unwrap();
    let root = engine::plugin_dir(temp.path()).unwrap();
    for line_ending in ["\n", "\r\n"] {
        for (path, text) in engine::FILES {
            let copy = text.replace("\r\n", "\n").replace('\n', line_ending);
            fs::write(root.join(path), copy).unwrap();
        }
        assert_eq!(
            engine::check_plugin(&root).unwrap().differing,
            Vec::<&str>::new(),
            "{line_ending:?}"
        );
    }
    fs::write(root.join("DISPATCH.md"), "other rules").unwrap();
    fs::remove_file(root.join("agents/critic.md")).unwrap();
    assert_eq!(
        engine::check_plugin(&root).unwrap().differing,
        ["DISPATCH.md", "agents/critic.md"]
    );
}
