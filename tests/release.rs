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
        for required in ["beta", "0.2.0-beta.1", "interjection"] {
            assert!(lower.contains(required), "{name} missing {required}");
        }
    }
    let bridge = fs::read_to_string(root.join("docs/bridge.md")).unwrap();
    assert!(bridge.contains("axiom bridge"));
    assert!(!bridge.contains("orch.ts"));
}
