#![allow(dead_code)]
use std::path::PathBuf;
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
pub fn python() -> String {
    if let Ok(p) = std::env::var("AXIOM_TEST_PYTHON") {
        return p;
    }
    let names = if cfg!(windows) {
        ["python", "python3"]
    } else {
        ["python3", "python"]
    };
    names
        .into_iter()
        .find(|p| {
            std::process::Command::new(p)
                .arg("--version")
                .output()
                .is_ok_and(|o| o.status.success())
        })
        .expect("Python not found")
        .into()
}
pub fn fake(name: &str) -> Vec<String> {
    vec![
        python(),
        "-I".into(),
        repo_root()
            .join("tests/fakes")
            .join(format!("{name}.py"))
            .to_string_lossy()
            .into_owned(),
    ]
}
pub fn temp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}
