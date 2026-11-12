use axiom::graph::{
    code::{refs_at, sync, SyncOptions},
    GraphStore,
};
use serde::Deserialize;
use std::{fs, path::Path, process::Command};
#[derive(Deserialize)]
struct Case {
    id: String,
    refs: Vec<ExpectedRef>,
}
#[derive(Deserialize)]
struct ExpectedRef {
    file: String,
    line: u32,
    name: String,
    nth: usize,
    expect: Expect,
}
#[derive(Deserialize)]
struct Expect {
    resolution: String,
    target: Option<String>,
}
#[derive(Default, Debug)]
struct Score {
    expected: usize,
    correct: usize,
    wrong: usize,
}
impl Score {
    fn add(
        &mut self,
        expected: &str,
        target: Option<&str>,
        actual: &str,
        actual_target: Option<&str>,
    ) {
        if expected == "resolved" {
            self.expected += 1;
        }
        if expected == "resolved" && actual == "resolved" && target == actual_target {
            self.correct += 1;
        }
        if actual == "resolved" && (expected != "resolved" || target != actual_target) {
            self.wrong += 1;
        }
    }
    fn passes(&self) -> bool {
        self.wrong == 0 && self.expected > 0 && self.correct * 10 >= self.expected * 7
    }
}
fn copy(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for e in fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let p = e.path();
        let d = dst.join(e.file_name());
        if p.is_dir() {
            copy(&p, &d)
        } else {
            fs::copy(p, d).unwrap();
        }
    }
}
#[test]
fn detects_wrong_certain_edge() {
    let mut s = Score::default();
    s.add("unresolved", None, "resolved", Some("wrong"));
    assert_eq!(s.wrong, 1);
    assert!(!s.passes());
}
#[test]
fn all_unresolved_fails_recall() {
    let mut score = Score::default();
    for _ in 0..10 {
        score.add("resolved", Some("target"), "unresolved", None);
    }
    assert_eq!(score.wrong, 0);
    assert!(!score.passes());
}
#[test]
fn holdout_through_index() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/graph/holdout/resolution");
    let home = tempfile::tempdir().unwrap();
    let store = GraphStore::open(home.path()).unwrap();
    let mut scores = Vec::new();
    for lang in ["rust", "python", "ts", "js"] {
        let mut score = Score::default();
        let mut cases: Vec<_> = fs::read_dir(root.join(lang))
            .unwrap()
            .map(|x| x.unwrap().path())
            .collect();
        cases.retain(|p| p.is_dir());
        cases.sort();
        let expected_cases = cases.len();
        let mut scanned = 0;
        for case_dir in cases {
            scanned += 1;
            let case: Case =
                serde_json::from_slice(&fs::read(case_dir.join("case.json")).unwrap()).unwrap();
            let repo = tempfile::tempdir().unwrap();
            copy(&case_dir.join("files"), repo.path());
            assert!(Command::new("git")
                .args(["init", "-q"])
                .current_dir(repo.path())
                .status()
                .unwrap()
                .success());
            let report = sync(&store, repo.path(), None, &SyncOptions::default()).unwrap();
            assert!(!report.partial, "{}: {:?}", case.id, report.partial_reason);
            for expected in case.refs {
                let rows = refs_at(&store, &report.scope, &expected.file, expected.line).unwrap();
                let actual = rows
                    .iter()
                    .filter(|r| r.name == expected.name)
                    .nth(expected.nth - 1);
                let resolution = actual.map_or("missing", |r| r.resolution.as_str());
                let target = actual
                    .and_then(|r| r.target.as_deref())
                    .map(|s| s.trim_start_matches("sym:"));
                score.add(
                    &expected.expect.resolution,
                    expected.expect.target.as_deref(),
                    resolution,
                    target,
                );
                if resolution == "resolved"
                    && (expected.expect.resolution != "resolved"
                        || expected.expect.target.as_deref() != target)
                {
                    eprintln!(
                        "wrong {} {}:{} {}: {:?}",
                        case.id, expected.file, expected.line, expected.name, target
                    );
                }
            }
        }
        assert_eq!(scanned, expected_cases, "{lang}");
        eprintln!(
            "{lang}: correct {}/{} wrong {}",
            score.correct, score.expected, score.wrong
        );
        scores.push((lang, score));
    }
    assert!(
        scores.iter().all(|(_, s)| s.passes()),
        "holdout thresholds not met: {scores:?}"
    );
}
