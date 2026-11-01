use axiom::engine;
use std::{collections::BTreeSet, fs};
#[test]
fn source_package_is_complete_and_idempotent() {
    let t = tempfile::tempdir().unwrap();
    let dir = engine::plugin_dir(t.path()).unwrap();
    let mut seen = BTreeSet::new();
    for (path, text) in engine::FILES {
        assert!(seen.insert(path));
        assert_eq!(fs::read(dir.join(path)).unwrap(), text.as_bytes());
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join(".codex-plugin/plugin.json")).unwrap()).unwrap();
    assert_eq!(manifest["version"], env!("CARGO_PKG_VERSION"));
    let first = dir.join(engine::FILES[0].0);
    let before = fs::metadata(&first).unwrap().modified().unwrap();
    assert_eq!(engine::plugin_dir(t.path()).unwrap(), dir);
    assert_eq!(fs::metadata(first).unwrap().modified().unwrap(), before);
}
