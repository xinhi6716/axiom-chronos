use super::*;
use std::{fs, path::Path, sync::Mutex};
static ENV_LOCK: Mutex<()> = Mutex::new(());
struct Restore(Option<std::ffi::OsString>);
impl Drop for Restore {
    fn drop(&mut self) {
        match &self.0 {
            Some(v) => std::env::set_var("AXIOM_LAYA_ORT_LIB", v),
            None => std::env::remove_var("AXIOM_LAYA_ORT_LIB"),
        }
    }
}
#[test]
fn candidates_and_router_config() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _restore = Restore(std::env::var_os("AXIOM_LAYA_ORT_LIB"));
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let repo = tmp.path().join("repo");
    let laya = home.join("laya");
    fs::create_dir_all(&laya).unwrap();
    fs::write(laya.join("router.json"), r#"{"runtime_lib":"router-lib"}"#).unwrap();
    std::env::set_var("AXIOM_LAYA_ORT_LIB", "env-lib");
    let paths = runtime_candidates(Some(&home), Some(&repo));
    assert_eq!(
        &paths[..4],
        &[
            Path::new("env-lib").to_path_buf(),
            Path::new("router-lib").to_path_buf(),
            laya.join("runtime").join(runtime_lib_name()),
            repo.join(".laya/runtime").join(runtime_lib_name())
        ]
    );
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        assert_eq!(
            paths[4],
            Path::new(&root).join("System32").join(runtime_lib_name())
        );
    }
    std::env::remove_var("AXIOM_LAYA_ORT_LIB");
    fs::write(laya.join("router.json"), "{}").unwrap();
    assert_eq!(
        runtime_candidates(Some(&home), Some(&repo))[0],
        laya.join("runtime").join(runtime_lib_name())
    );
}
#[test]
fn missing_runtime_and_bad_libraries() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _restore = Restore(std::env::var_os("AXIOM_LAYA_ORT_LIB"));
    std::env::remove_var("AXIOM_LAYA_ORT_LIB");
    let tmp = tempfile::tempdir().unwrap();
    // System32 may contain a real ORT installation; supply a nonexistent SystemRoot on Windows.
    #[cfg(windows)]
    let old_root = std::env::var_os("SystemRoot");
    #[cfg(windows)]
    std::env::set_var("SystemRoot", tmp.path());
    let err = find_runtime(Some(tmp.path()), Some(tmp.path())).unwrap_err();
    #[cfg(windows)]
    match old_root {
        Some(v) => std::env::set_var("SystemRoot", v),
        None => std::env::remove_var("SystemRoot"),
    }
    assert_eq!(err.kind, "laya_runtime_missing");
    assert!(
        err.message.contains("AXIOM_LAYA_ORT_LIB") && err.message.contains("tools/laya/fetch.py")
    );
    assert!(Runtime::load(&tmp.path().join("missing")).is_err());
    let file = tmp.path().join("not-a-library");
    fs::write(&file, b"not a library").unwrap();
    assert!(Runtime::load(&file).is_err());
}
#[test]
fn manifest_hash_and_negative_controls() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("nested").join("abc");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, b"abc").unwrap();
    assert_eq!(
        sha256_file(&file).unwrap(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let manifest = write_manifest(tmp.path(), "v1", "fused", "source", "rev").unwrap();
    assert_eq!(manifest, verify_dir(tmp.path()).unwrap());
    fs::write(&file, b"abd").unwrap();
    let err = verify_dir(tmp.path()).unwrap_err();
    assert_eq!(err.kind, "laya_sha256_mismatch");
    assert!(err.message.contains("nested/abc"));
    fs::remove_file(&file).unwrap();
    assert_eq!(
        verify_dir(tmp.path()).unwrap_err().kind,
        "laya_artifact_missing"
    );
    let mut manifest = manifest;
    manifest.format = 2;
    fs::write(
        tmp.path().join("axiom-laya.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert_eq!(
        verify_dir(tmp.path()).unwrap_err().kind,
        "laya_manifest_invalid"
    );
}
