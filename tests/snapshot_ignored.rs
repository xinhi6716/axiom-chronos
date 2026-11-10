mod common;

use axiom::orch::*;
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).replace('\r', "")
}

struct Fixture {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    store: PathBuf,
}

impl Fixture {
    fn new(scope: &str) -> Self {
        let temp = common::temp();
        let repo = temp.path().join("repo");
        let store = temp.path().join("store");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(store.join("inbox")).unwrap();
        git(&repo, &["init", "-q"]);
        fs::write(repo.join(".gitignore"), "conf/local.toml\nsub/x.txt\n").unwrap();
        fs::write(repo.join("a.txt"), "old a\n").unwrap();
        git(&repo, &["add", "--", ".gitignore"]);
        fs::write(
            store.join("units.tsv"),
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n",
        )
        .unwrap();
        fs::write(
            store.join("ledger.tsv"),
            "pr\tsha\tverdict\tevidence\tverifier\tts\n",
        )
        .unwrap();
        let brief = store.join("u.brief");
        fs::write(
            &brief,
            format!(
                "GOAL build u\nSCOPE {scope}\nVERIFY \"{}\" -c \"print('ok')\"\n",
                common::python()
            ),
        )
        .unwrap();
        fs::write(
            store.join("units.tsv"),
            format!(
                "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tpending\t\t\t\t{}\n",
                brief.display()
            ),
        )
        .unwrap();
        Self {
            _temp: temp,
            repo,
            store,
        }
    }

    fn work(&self, change: impl FnOnce(&Path)) {
        let Begin::Run { attempt_id, .. } =
            begin_unit(&self.store, "u", "terra", &self.repo, false).unwrap()
        else {
            panic!("unit reused")
        };
        change(&self.repo);
        finish_unit(
            &self.store,
            "u",
            &attempt_id,
            &self.repo,
            "terra",
            "gpt-6.1-terra",
            &json!({"status":"completed"}),
        )
        .unwrap();
    }

    fn verify_tree(&self) -> String {
        evidence(&self.store, "u")
            .unwrap()
            .into_iter()
            .rev()
            .find(|r| r.kind == "verify")
            .unwrap()
            .commit
    }

    fn tree_files(&self, tree: &str) -> Vec<String> {
        assert!(tree.starts_with("tree:"), "VERIFY tree: {tree:?}");
        git(&self.repo, &["ls-tree", "-r", "--name-only", &tree[5..]])
            .lines()
            .map(|s| s.replace('\\', "/"))
            .collect()
    }

    fn index(&self) -> Vec<u8> {
        fs::read(self.repo.join(".git/index")).unwrap()
    }
}

#[test]
fn named_ignored_file_is_reviewed_and_invalidates_review_when_changed() {
    let f = Fixture::new("a.txt conf/local.toml");
    fs::create_dir_all(f.repo.join("conf")).unwrap();
    fs::write(f.repo.join("conf/local.toml"), "old config\n").unwrap();
    let index = f.index();
    f.work(|repo| {
        fs::write(repo.join("a.txt"), "new a\n").unwrap();
        fs::write(repo.join("conf/local.toml"), "new config\n").unwrap();
    });
    let tree = f.verify_tree();
    assert!(f.tree_files(&tree).contains(&"conf/local.toml".into()));
    let pack = review_pack(&f.store, "u", &f.repo).unwrap();
    let text = fs::read_to_string(pack.path).unwrap().replace('\r', "");
    for line in ["conf/local.toml", "-old config\n", "+new config\n"] {
        assert!(text.contains(line), "{line}: {text}");
    }
    let verdict = f.store.join("verdict.json");
    fs::write(
        &verdict,
        json!({"unit":"u", "tree":tree, "verdict":"pass", "findings":[]}).to_string(),
    )
    .unwrap();
    let args: Vec<String> = [
        "review",
        "u",
        "--repo",
        &f.repo.to_string_lossy(),
        "--reviewer",
        "chronos:reviewer",
        "--verdict-file",
        &verdict.to_string_lossy(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let out = run(&f.store, &args, false, false);
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    assert!(out.stdout.contains("u\tverified"), "{}", out.stdout);
    fs::write(f.repo.join("conf/local.toml"), "changed again\n").unwrap();
    assert_eq!(record_verify(&f.store, "u", &f.repo).unwrap().exit_code, 0);
    assert_eq!(
        read_units(&f.store)
            .unwrap()
            .iter()
            .find(|u| u.id == "u")
            .unwrap()
            .state,
        "built"
    );
    assert_eq!(f.index(), index);
    assert!(!git(&f.repo, &["ls-files"])
        .lines()
        .any(|s| s == "conf/local.toml"));
}

#[test]
fn newly_created_named_ignored_file_is_in_tree_and_pack() {
    let f = Fixture::new("a.txt conf/local.toml");
    let index = f.index();
    f.work(|repo| {
        fs::create_dir_all(repo.join("conf")).unwrap();
        fs::write(repo.join("conf/local.toml"), "new config\n").unwrap();
    });
    assert!(f
        .tree_files(&f.verify_tree())
        .contains(&"conf/local.toml".into()));
    let text = fs::read_to_string(review_pack(&f.store, "u", &f.repo).unwrap().path)
        .unwrap()
        .replace('\r', "");
    assert!(text.contains("- conf/local.toml  +1 -0"), "{text}");
    assert!(text.contains("+new config\n"), "{text}");
    assert_eq!(f.index(), index);
}

#[test]
fn ignored_directory_contents_stay_out_while_named_file_is_included() {
    let f = Fixture::new("sub/");
    fs::create_dir_all(f.repo.join("sub")).unwrap();
    fs::write(f.repo.join("sub/a.txt"), "included\n").unwrap();
    fs::write(f.repo.join("sub/x.txt"), "ignored\n").unwrap();
    let index = f.index();
    let dir_tree = snapshot(&f.repo, &["sub/".into()]);
    let files = f.tree_files(&dir_tree);
    assert!(files.contains(&"sub/a.txt".into()));
    assert!(!files.contains(&"sub/x.txt".into()));
    // The old single-add algorithm must produce the same tree for ordinary paths.
    let reference_index = f.store.join("reference-index");
    let reference_git = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(&f.repo)
            .env("GIT_INDEX_FILE", &reference_index)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    reference_git(&["read-tree", "--empty"]);
    reference_git(&["add", "-A", "--", "a.txt", "sub/"]);
    assert_eq!(
        snapshot(&f.repo, &["a.txt".into(), "sub/".into()]),
        format!("tree:{}", reference_git(&["write-tree"]))
    );
    let named_tree = snapshot(&f.repo, &["sub/x.txt".into()]);
    assert!(f.tree_files(&named_tree).contains(&"sub/x.txt".into()));
    assert_eq!(f.index(), index);
}
