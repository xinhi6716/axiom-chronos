use std::fs;
use std::path::Path;

fn no_python(path: &Path) {
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            no_python(&path);
        } else {
            assert_ne!(
                path.extension().and_then(|s| s.to_str()),
                Some("py"),
                "{}",
                path.display()
            );
        }
    }
}

#[test]
fn rust_release_contract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    no_python(&root.join("src"));
    assert!(!root.join("pyproject.toml").exists());
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    for required in [
        "ubuntu-latest",
        "windows-latest",
        "setup-python",
        "cargo test --locked",
    ] {
        assert!(ci.contains(required), "CI missing {required}");
    }
    for name in ["README.md", "README.zh-TW.md"] {
        let readme = fs::read_to_string(root.join(name)).unwrap();
        let lower = readme.to_lowercase();
        for forbidden in ["pipx", "bun", "axiom_python", "--model", "`60`"] {
            assert!(!lower.contains(forbidden), "{name} contains {forbidden}");
        }
        for required in ["0.3.0-codex.dev.1"] {
            assert!(lower.contains(required), "{name} missing {required}");
        }
    }
    let bridge = fs::read_to_string(root.join("docs/bridge.md")).unwrap();
    assert!(bridge.contains("axiom bridge"));
    assert!(!bridge.contains("orch.ts"));
}

#[test]
fn codex_source_package_is_complete_and_versioned() {
    let home = tempfile::tempdir().unwrap();
    let package = axiom::engine::plugin_dir(home.path()).unwrap();
    assert!(package.starts_with(home.path()));
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(package.join(".codex-plugin/plugin.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["version"], env!("CARGO_PKG_VERSION"));
    for path in [
        "skills/wish/SKILL.md",
        "skills/research/SKILL.md",
        "assets/chronos.svg",
        "assets/chronos-dark.svg",
    ] {
        assert!(!fs::read(package.join(path)).unwrap().is_empty(), "{path}");
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for path in [
        ".codex-plugin/plugin.json",
        "skills/wish/SKILL.md",
        "skills/research/SKILL.md",
    ] {
        assert_eq!(
            fs::read(package.join(path)).unwrap(),
            fs::read(root.join("codex-plugin").join(path)).unwrap(),
            "{path}"
        );
    }
}
