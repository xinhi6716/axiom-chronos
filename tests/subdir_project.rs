mod common;

use axiom::orch::*;
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn numbered(n: usize) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

struct Fixture {
    _temp: tempfile::TempDir,
    project: PathBuf,
    store: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp_root = common::repo_root().join(".axiom-tmp/subdir-paths-1");
        fs::create_dir_all(&temp_root).unwrap();
        let temp = tempfile::tempdir_in(temp_root).unwrap();
        let root = temp.path().join("repo");
        let project = root.join("proj");
        let store = temp.path().join("store");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(store.join("inbox")).unwrap();
        git(&root, &["init", "-q"]);
        fs::write(project.join("a.txt"), numbered(200)).unwrap();
        fs::write(project.join("gone.txt"), numbered(9000)).unwrap();
        fs::write(root.join("outside.txt"), "outside\n").unwrap();
        git(&root, &["add", "."]);
        git(
            &root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "initial",
            ],
        );
        fs::write(store.join("modules.md"), "## m\nPURPOSE x\nPATHS a.txt new.txt gone.txt\nDEPENDS none\nCONTRACT the module promises\nACCEPT test `echo ok`\n").unwrap();
        let brief = store.join("u.brief");
        fs::write(
            &brief,
            format!(
                "GOAL build u\nSCOPE a.txt new.txt gone.txt\nVERIFY \"{}\" -c \"print('ok')\"\n",
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
        fs::write(
            store.join("ledger.tsv"),
            "pr\tsha\tverdict\tevidence\tverifier\tts\n",
        )
        .unwrap();
        Self {
            _temp: temp,
            project,
            store,
        }
    }

    fn work(&self, outside: bool) -> Finished {
        let Begin::Run { attempt_id, .. } =
            begin_unit(&self.store, "u", "terra", &self.project, false).unwrap()
        else {
            panic!("unit was reused")
        };
        fs::write(
            self.project.join("a.txt"),
            numbered(200).replace("line 100\n", "line one hundred\n"),
        )
        .unwrap();
        fs::write(self.project.join("new.txt"), "brand new file\n").unwrap();
        fs::remove_file(self.project.join("gone.txt")).unwrap();
        if outside {
            fs::write(self.project.join("../outside.txt"), "changed\n").unwrap();
        }
        finish_unit(&self.store, "u", &attempt_id, &self.project, "terra", "gpt-6.1-terra", &json!({"status":"completed", "file_changes":[{"path":"a.txt"},{"path":self.project.join("new.txt").to_string_lossy()}]})).unwrap()
    }
}

#[test]
fn project_relative_scope_pack_and_digest() {
    let f = Fixture::new();
    let finished = f.work(false);
    assert_eq!(finished.outside_scope, Vec::<String>::new());
    assert_eq!(finished.verify_exit, Some(0));
    let rows = evidence(&f.store, "u").unwrap();
    assert_eq!(
        rows.iter().find(|r| r.kind == "scope").unwrap().exit_code,
        0
    );
    assert_eq!(
        rows.iter().find(|r| r.kind == "verify").unwrap().exit_code,
        0
    );
    let info = review_pack(&f.store, "u", &f.project).unwrap();
    assert_eq!(info.files, 3);
    assert_eq!(info.omitted, ["gone.txt"]);
    let text = fs::read_to_string(info.path)
        .unwrap()
        .replace('\r', "")
        .replace('\\', "/");
    for expected in [
        "- a.txt",
        "- new.txt",
        "- gone.txt",
        "+line one hundred",
        "a/a.txt",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
    assert!(!text.contains("proj/"), "{text}");
    assert!(!text.contains("git does not show as changed"), "{text}");
    let line = text.lines().find(|l| l.starts_with("- gone.txt ")).unwrap();
    let copy = line
        .split_once("deleted; its last version is at ")
        .unwrap()
        .1
        .trim_end_matches(')');
    assert_eq!(
        fs::read_to_string(copy).unwrap().replace('\r', ""),
        numbered(9000)
    );
    let digest = digest(&f.store, &f.project, None)
        .unwrap()
        .replace('\r', "")
        .replace('\\', "/");
    assert!(
        digest.contains("changed files (3):\n  a.txt\n  gone.txt\n  new.txt\n"),
        "{digest}"
    );
    for path in ["  a.txt", "  gone.txt", "  new.txt"] {
        assert_eq!(digest.lines().filter(|line| *line == path).count(), 1);
    }
    assert!(
        !digest.lines().any(|line| line.contains("proj/")),
        "{digest}"
    );
}

#[test]
fn outside_project_is_outside_scope() {
    let f = Fixture::new();
    let finished = f.work(true);
    assert_eq!(finished.outside_scope, ["../outside.txt"]);
    assert_eq!(finished.verify_exit, None);
    let rows = evidence(&f.store, "u").unwrap();
    assert_eq!(
        rows.iter().find(|r| r.kind == "scope").unwrap().exit_code,
        1
    );
    assert!(!rows.iter().any(|r| r.kind == "verify"));
}
