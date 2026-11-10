use axiom::evolve::repo;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const DATE: &str = "2026-11-10";

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=Evolve Test",
            "-c",
            "user.email=evolve@example.test",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn fixture(parent: &Path, name: &str, manifest: &str) -> PathBuf {
    let dir = parent.join(name);
    fs::create_dir_all(dir.join("nested")).unwrap();
    git(&dir, &["init"]);
    fs::write(dir.join("Cargo.toml"), manifest).unwrap();
    fs::write(dir.join("nested/tracked"), "original").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-m", "fixture"]);
    dir
}

fn valid(parent: &Path, name: &str) -> PathBuf {
    fixture(parent, name, "[package]\nname = \"axiom-chronos\"\n")
}

#[test]
fn no_repo_and_stale_settings_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    assert_eq!(repo::resolve(&home, None).unwrap_err().kind, "no_repo");
    assert!(!home.exists());
    let dir = valid(tmp.path(), "repo");
    assert_eq!(repo::resolve(&home, Some(&dir)).unwrap(), dir);
    repo::remember(&home, &dir).unwrap();
    assert_eq!(
        repo::remembered(&home).unwrap().unwrap(),
        fs::canonicalize(&dir).unwrap()
    );
    assert_eq!(
        repo::resolve(&home, None).unwrap(),
        fs::canonicalize(&dir).unwrap()
    );
    let stale = tmp.path().join("missing");
    fs::write(
        home.join("evolve/settings.json"),
        serde_json::json!({"repo": stale}).to_string(),
    )
    .unwrap();
    assert_eq!(
        repo::check(&repo::resolve(&home, None).unwrap(), DATE)
            .unwrap_err()
            .kind,
        "not_git_repo"
    );
    let nonrepo = tmp.path().join("nonrepo");
    fs::create_dir(&nonrepo).unwrap();
    repo::remember(&home, &nonrepo).unwrap();
    assert_eq!(
        repo::check(&repo::resolve(&home, None).unwrap(), DATE)
            .unwrap_err()
            .kind,
        "not_git_repo"
    );
}

#[test]
fn requires_git_root_and_exact_package_name() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(
        repo::check(tmp.path(), DATE).unwrap_err().kind,
        "not_git_repo"
    );
    let dir = valid(tmp.path(), "valid");
    assert_eq!(
        repo::check(&dir.join("nested"), DATE).unwrap_err().kind,
        "not_repo_root"
    );
    for (name, manifest) in [
        ("wrong", "[package]\nname = \"another\"\n"),
        (
            "comment",
            "[package]\n# name = \"axiom-chronos\"\nname = \"wrong\"\n",
        ),
        (
            "other_table",
            "[package]\nversion = \"1\"\n[dependencies]\nname = \"axiom-chronos\"\n",
        ),
    ] {
        let dir = fixture(tmp.path(), name, manifest);
        assert_eq!(repo::check(&dir, DATE).unwrap_err().kind, "wrong_package");
    }
}

#[test]
fn rejects_untracked_unstaged_and_staged_changes() {
    let tmp = tempfile::tempdir().unwrap();
    for mode in ["untracked", "unstaged", "staged"] {
        let dir = valid(tmp.path(), mode);
        let file = if mode == "untracked" {
            "new"
        } else {
            "nested/tracked"
        };
        fs::write(dir.join(file), "change").unwrap();
        if mode == "staged" {
            git(&dir, &["add", file]);
        }
        assert_eq!(
            repo::check(&dir, DATE).unwrap_err().kind,
            "dirty_tree",
            "{mode}"
        );
        assert_eq!(git(&dir, &["branch", "--list", "evolve/*"]), "");
    }
}

#[test]
fn creates_only_new_branch_and_refuses_existing_one() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = valid(tmp.path(), "target");
    let other = valid(tmp.path(), "other");
    let original = git(&dir, &["branch", "--show-current"]);
    let commit = git(&dir, &["rev-parse", "HEAD"]);
    let other_branch = git(&other, &["branch", "--show-current"]);
    let other_head = git(&other, &["rev-parse", "HEAD"]);
    let info = repo::check(&dir, DATE).unwrap();
    assert_eq!(info.root, fs::canonicalize(&dir).unwrap());
    assert_eq!(info.original_branch, original);
    assert_eq!(info.base_commit, commit);
    assert_eq!(info.branch, "evolve/2026-11-10");
    repo::create_branch(&info).unwrap();
    assert_eq!(git(&dir, &["branch", "--show-current"]), info.branch);
    assert_eq!(git(&dir, &["rev-parse", &original]), commit);
    assert_eq!(git(&dir, &["status", "--porcelain"]), "");
    assert_eq!(repo::check(&dir, DATE).unwrap_err().kind, "branch_exists");
    assert_eq!(git(&other, &["branch", "--show-current"]), other_branch);
    assert_eq!(git(&other, &["rev-parse", "HEAD"]), other_head);
    assert_eq!(git(&other, &["branch", "--list", "evolve/*"]), "");
    assert_eq!(git(&other, &["status", "--porcelain"]), "");
}
