use axiom::orch::*;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

struct Fixture {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    store: PathBuf,
}

fn git(repo: &Path, args: &[&str]) {
    assert!(Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .unwrap()
        .success());
}
fn numbered(n: usize) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let store = temp.path().join("store");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(store.join("inbox")).unwrap();
        git(&repo, &["init", "-q"]);
        fs::write(repo.join("a.txt"), numbered(200)).unwrap();
        fs::write(repo.join("outside.txt"), "outside text\n").unwrap();
        fs::write(
            store.join("modules.md"),
            "## m\nPURPOSE x\nPATHS a.txt new.txt big.txt\nDEPENDS none\nCONTRACT the module promises\nACCEPT test `echo ok`\n\n## n\nPURPOSE y\nPATHS b.txt\nDEPENDS none\nCONTRACT other\nACCEPT test `echo ok`\n",
        )
        .unwrap();
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
        Self {
            _temp: temp,
            repo,
            store,
        }
    }
    fn add(&self, unit: &str, track: &str, scope: &str) {
        let brief = self.store.join(format!("{unit}.brief"));
        fs::write(
            &brief,
            format!("GOAL build {unit}\nSCOPE {scope}\nVERIFY echo ok\n"),
        )
        .unwrap();
        let mut rows = fs::read_to_string(self.store.join("units.tsv")).unwrap();
        rows.push_str(&format!(
            "{unit}\t{track}\tpending\t\t\t\t{}\n",
            brief.display()
        ));
        fs::write(self.store.join("units.tsv"), rows).unwrap();
    }
    fn work(&self, unit: &str, model: &str, change: impl FnOnce(&Path), report: Value) {
        let Begin::Run { attempt_id, .. } =
            begin_unit(&self.store, unit, "terra", &self.repo, false).unwrap()
        else {
            panic!("unit was reused");
        };
        change(&self.repo);
        finish_unit(
            &self.store,
            unit,
            &attempt_id,
            &self.repo,
            "terra",
            model,
            &report,
        )
        .unwrap();
    }
    fn built(&self) {
        self.add("u", "m", "a.txt new.txt");
        self.work(
            "u",
            "gpt-6.1-terra",
            |repo| {
                let text = numbered(200).replace("line 100\n", "line one hundred\n");
                fs::write(repo.join("a.txt"), text).unwrap();
                fs::write(repo.join("new.txt"), "brand new file\n").unwrap();
            },
            json!({"status":"completed","file_changes":[{"path":"a.txt"},{"path":"new.txt"}]}),
        );
    }
    fn orch(&self, args: &[&str]) -> Output {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        run(&self.store, &args, false, false)
    }
    fn repo_arg(&self) -> String {
        self.repo.to_string_lossy().into_owned()
    }
    fn verdict(&self, unit: &str, body: &Value) -> String {
        let path = self.store.join(format!("{unit}-verdict.json"));
        fs::write(&path, body.to_string()).unwrap();
        path.to_string_lossy().into_owned()
    }
    fn review(&self, unit: &str, path: &str) -> Output {
        self.orch(&[
            "review",
            unit,
            "--repo",
            &self.repo_arg(),
            "--reviewer",
            "chronos:reviewer",
            "--verdict-file",
            path,
        ])
    }
}

#[test]
fn run_row_keeps_the_scope_as_it_was_before_the_worker() {
    let f = Fixture::new();
    f.built();
    let rows = evidence(&f.store, "u").unwrap();
    let run = rows.iter().find(|r| r.kind == "run").unwrap();
    assert!(run.commit.starts_with("tree:"), "{}", run.commit);
    let verify = rows.iter().find(|r| r.kind == "verify").unwrap();
    assert_ne!(run.commit, verify.commit);
    assert_eq!(base_snapshot(&f.repo, &["missing.txt".into()]).len(), 45);
}

#[test]
fn pack_has_contract_changed_lines_and_new_files_only() {
    let f = Fixture::new();
    f.built();
    let info = review_pack(&f.store, "u", &f.repo).unwrap();
    assert_eq!(info.files, 2);
    assert!(info.omitted.is_empty());
    let text = fs::read_to_string(&info.path).unwrap();
    assert_eq!(text.len(), info.bytes);
    let verify = evidence(&f.store, "u")
        .unwrap()
        .into_iter()
        .find(|r| r.kind == "verify")
        .unwrap();
    for present in [
        verify.commit.as_str(),
        "the module promises",
        "GOAL build u",
        "-line 100",
        "+line one hundred",
        "\n line 92\n",
        "\n line 108\n",
        "Every changed file follows with 8 lines of surrounding code; a new file appears in full.",
        "+brand new file",
        "- a.txt  +1 -1",
        "- new.txt  +1 -0",
    ] {
        assert!(text.contains(present), "{present}\n{text}");
    }
    for absent in ["\n line 91\n", "\n line 109\n", "outside text"] {
        assert!(!text.contains(absent), "{absent}");
    }
}

#[test]
fn a_diff_over_the_budget_is_named_for_direct_reading() {
    let f = Fixture::new();
    f.add("u", "m", "a.txt big.txt");
    f.work(
        "u",
        "gpt-6.1-terra",
        |repo| {
            fs::write(repo.join("a.txt"), "changed\n").unwrap();
            fs::write(repo.join("big.txt"), "big line of text\n".repeat(8000)).unwrap();
        },
        json!({"status":"completed"}),
    );
    let info = review_pack(&f.store, "u", &f.repo).unwrap();
    assert_eq!(info.omitted, ["big.txt"]);
    assert!(info.bytes < 90_000, "{}", info.bytes);
    let text = fs::read_to_string(&info.path).unwrap();
    assert!(text.contains("- big.txt  +8000 -0  (read this file directly)"));
    assert!(!text.contains("big line of text"));
    assert!(text.contains("+changed"));
}

#[test]
fn a_change_git_cannot_show_is_still_named() {
    let f = Fixture::new();
    fs::write(f.repo.join(".gitignore"), "sub/ignored.txt\n").unwrap();
    f.add("u", "m", "sub/");
    f.work(
        "u",
        "gpt-6.1-terra",
        |repo| {
            fs::create_dir_all(repo.join("sub")).unwrap();
            fs::write(repo.join("sub/a.txt"), "changed\n").unwrap();
            fs::write(repo.join("sub/ignored.txt"), "secret\n").unwrap();
        },
        json!({"status":"completed","file_changes":[{"path":"sub/a.txt"},{"path":"sub/ignored.txt"}]}),
    );
    let info = review_pack(&f.store, "u", &f.repo).unwrap();
    let text = fs::read_to_string(info.path).unwrap();
    assert!(text.contains("git does not show as changed"));
    assert!(text.contains(": sub/ignored.txt\n"), "{text}");
    assert!(!text.contains("secret"));
}

#[test]
fn a_unit_without_a_base_snapshot_is_reviewed_from_its_files() {
    let f = Fixture::new();
    f.built();
    let path = f.store.join("reports/u.evidence.tsv");
    let text = fs::read_to_string(&path).unwrap();
    let cleared: Vec<String> = text
        .lines()
        .map(|l| {
            let mut c: Vec<&str> = l.split('\t').collect();
            if c[0] == "run" {
                c[3] = "";
            }
            c.join("\t")
        })
        .collect();
    fs::write(&path, cleared.join("\n") + "\n").unwrap();
    let info = review_pack(&f.store, "u", &f.repo).unwrap();
    assert_eq!(info.files, 0);
    let text = fs::read_to_string(info.path).unwrap();
    assert!(text.contains("review every file in SCOPE directly: a.txt, new.txt"));
    assert!(!text.contains("+line one hundred"));
}

#[test]
fn pack_needs_a_passing_verify_and_a_real_tree() {
    let f = Fixture::new();
    f.add("u", "m", "a.txt");
    assert_eq!(
        review_pack(&f.store, "u", &f.repo).unwrap_err().kind,
        "not_built"
    );
    assert_eq!(
        review_pack(&f.store, "zz", &f.repo).unwrap_err().kind,
        "not_found"
    );
    let f = Fixture::new();
    f.built();
    let path = f.store.join("reports/u.evidence.tsv");
    let text = fs::read_to_string(&path).unwrap();
    let tree = evidence(&f.store, "u").unwrap()[0].commit.clone();
    fs::write(&path, text.replacen(&tree, "tree:0123456789", 1)).unwrap();
    assert_eq!(
        review_pack(&f.store, "u", &f.repo).unwrap_err().kind,
        "git_failed"
    );
}

#[test]
fn a_verdict_needs_a_pack_of_the_verified_tree() {
    let f = Fixture::new();
    f.built();
    record_accept(&f.store, "m", &f.repo).unwrap();
    let verdict = f.verdict(
        "u",
        &json!({"unit":"u","tree":verified_tree(&f.store, "u"),"verdict":"pass","findings":[]}),
    );
    let before = evidence(&f.store, "u").unwrap().len();
    fs::remove_file(pack_path(&f.store, "u")).unwrap();
    let refused = f.review("u", &verdict);
    assert_eq!(refused.exit_code, 1);
    assert!(refused.stderr.contains("run orch review-pack first"));
    fs::write(pack_path(&f.store, "u"), "an old pack of tree:other").unwrap();
    assert_eq!(f.review("u", &verdict).exit_code, 1);
    assert_eq!(evidence(&f.store, "u").unwrap().len(), before);
    let made = f.orch(&["review-pack", "u", "--repo", &f.repo_arg()]);
    assert_eq!(made.exit_code, 0, "{}", made.stderr);
    assert!(made.stdout.contains("files=2\tread-directly=0"));
    let accepted = f.review("u", &verdict);
    assert_eq!(accepted.exit_code, 0, "{}", accepted.stderr);
    assert!(accepted.stdout.contains("u\tverified"));
}

#[test]
fn review_stays_cross_family_with_a_pack() {
    let f = Fixture::new();
    f.add("u", "m", "a.txt");
    f.work(
        "u",
        "claude-opus-5-5",
        |repo| fs::write(repo.join("a.txt"), "changed\n").unwrap(),
        json!({"status":"completed"}),
    );
    record_accept(&f.store, "m", &f.repo).unwrap();
    review_pack(&f.store, "u", &f.repo).unwrap();
    let verdict = f.verdict(
        "u",
        &json!({"unit":"u","tree":verified_tree(&f.store, "u"),"verdict":"pass","findings":[]}),
    );
    assert!(f.review("u", &verdict).stderr.contains("same family"));
}

fn report(f: &Fixture, extra: &[&str]) -> String {
    let repo = f.repo_arg();
    let mut args = vec!["report", "--repo", &repo];
    args.extend(extra);
    let out = f.orch(&args);
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    out.stdout
}

#[test]
fn digest_carries_what_the_final_report_lists() {
    let f = Fixture::new();
    f.built();
    record_accept(&f.store, "m", &f.repo).unwrap();
    review_pack(&f.store, "u", &f.repo).unwrap();
    let fail = json!({"unit":"u","tree":verified_tree(&f.store, "u"),"verdict":"fail","findings":[
        {"check":"untested","severity":"major","location":"a.txt:100","evidence":"no test covers it","fix":"add one"},
        {"check":"platform","severity":"minor","location":"a.txt:1","evidence":"small thing","fix":"x"}
    ]});
    assert_eq!(f.review("u", &f.verdict("u", &fail)).exit_code, 0);
    f.add("waiting", "n", "b.txt");
    f.add("fixer", "m", "new.txt");
    fs::write(f.store.join("decisions.md"), "- chose X: because Y\n").unwrap();
    fs::create_dir_all(f.store.join("astra")).unwrap();
    fs::write(
        f.store.join("astra/plan-final.md"),
        "# Plan\n\n## Changes\n\nsomething\n\n## Coverage\n\nread: src\nnot read: docs\n\n## After\n\nno\n",
    )
    .unwrap();
    assert_eq!(
        record_full(&f.store, &f.repo, Some("echo all-good"))
            .unwrap()
            .exit_code,
        0
    );
    let text = report(&f, &[]);
    for present in [
        "units: 3 (pending 2, review-failed 1)",
        "not verified:\n",
        "  u  review-failed\n",
        "  waiting  pending\n",
        "acceptance:\n  m  pass\n  n  not run\n",
        "full test: pass echo all-good\n",
        "sol: did not run\n",
        "changed files (2):\n  a.txt\n  new.txt\n",
        "open findings (1; 1 minor",
        "u  major  untested  a.txt:100: no test covers it",
        "decisions:\n- chose X: because Y\n",
        "## Coverage\n\nread: src\nnot read: docs\n",
    ] {
        assert!(text.contains(present), "{present}\n{text}");
    }
    assert!(!text.contains("## After"));
    assert!(!text.contains("small thing"));
}

#[test]
fn digest_names_sol_and_marks_built_units_unreviewed() {
    let f = Fixture::new();
    f.add("s", "m", "a.txt");
    f.work(
        "s",
        "gpt-6.1-sol",
        |repo| fs::write(repo.join("a.txt"), "x\n").unwrap(),
        json!({"status":"completed"}),
    );
    let text = report(&f, &[]);
    assert!(text.contains("sol: ran (s)"), "{text}");
    assert!(text.contains("  s  unreviewed\n"), "{text}");
    assert!(text.contains("full test: not run\n"), "{text}");
    assert!(text.contains("decisions: none recorded\n"), "{text}");
}

#[test]
fn module_digest_covers_only_that_module() {
    let f = Fixture::new();
    f.built();
    f.add("other", "n", "b.txt");
    fs::write(f.store.join("decisions.md"), "- chose X\n").unwrap();
    let text = report(&f, &["--module", "m"]);
    assert!(text.contains("units: 1 ("), "{text}");
    assert!(text.contains("acceptance:\n  m  not run\n"), "{text}");
    for absent in ["other", "  n  ", "decisions", "full test", "sol:"] {
        assert!(!text.contains(absent), "{absent}\n{text}");
    }
    let repo = f.repo_arg();
    let missing = f.orch(&["report", "--repo", &repo, "--module", "nope"]);
    assert_eq!(missing.exit_code, 2);
    assert_eq!(f.orch(&["report"]).exit_code, 1);
}

#[test]
fn digest_stays_bounded_however_large_the_run() {
    let f = Fixture::new();
    f.add("u", "m", "a.txt src/");
    let many: Vec<Value> = (0..600)
        .map(|i| json!({"path": format!("src/file_{i:04}.py")}))
        .collect();
    f.work(
        "u",
        "gpt-6.1-terra",
        |repo| {
            fs::write(repo.join("a.txt"), "x\n").unwrap();
            fs::create_dir_all(repo.join("src")).unwrap();
            fs::write(repo.join("src/file_0000.py"), "x\n").unwrap();
        },
        json!({"status":"completed","file_changes":many,"commands":vec!["cmd"; 40000]}),
    );
    fs::write(f.store.join("decisions.md"), "決".repeat(40000)).unwrap();
    let findings: Vec<Value> = (0..200)
        .map(|i| json!({"check":"untested","severity":"blocker","location":format!("l{i}"),"evidence":"e".repeat(5000)}))
        .collect();
    record_accept(&f.store, "m", &f.repo).unwrap();
    review_pack(&f.store, "u", &f.repo).unwrap();
    let verdict = f.verdict(
        "u",
        &json!({"unit":"u","tree":verified_tree(&f.store, "u"),"verdict":"fail","findings":findings}),
    );
    assert_eq!(f.review("u", &verdict).exit_code, 0);
    let reports: u64 = fs::read_dir(f.store.join("reports"))
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().len())
        .sum();
    assert!(reports > 1_000_000, "{reports}");
    let text = report(&f, &[]);
    assert!(text.chars().count() < 45_000, "{}", text.chars().count());
    // 600 reported paths, plus a.txt, which only git shows
    assert!(text.contains("changed files (601):"));
    assert!(text.contains("... 401 more in reports/<unit>.json"));
    assert!(text.contains("... 170 more in reports/<unit>.verdict.json"));
    assert!(text.contains("cut at 16000 characters; the rest is in"));
    assert!(text.contains("decisions.md"));
}

fn verified_tree(store: &Path, unit: &str) -> String {
    evidence(store, unit)
        .unwrap()
        .into_iter()
        .rev()
        .find(|r| r.kind == "verify")
        .unwrap()
        .commit
}

#[test]
fn an_old_verdict_cannot_pass_a_new_pack() {
    let f = Fixture::new();
    f.built();
    record_accept(&f.store, "m", &f.repo).unwrap();
    let pack = review_pack(&f.store, "u", &f.repo).unwrap();
    let first = pack.tree;
    assert!(fs::read_to_string(&pack.path)
        .unwrap()
        .contains(&format!("\nTree reviewed: {first}\n")));
    let old = f.verdict(
        "u",
        &json!({"unit":"u","tree":first,"verdict":"pass","findings":[]}),
    );
    fs::write(f.repo.join("a.txt"), "rewritten after the first review\n").unwrap();
    assert_eq!(record_verify(&f.store, "u", &f.repo).unwrap().exit_code, 0);
    record_accept(&f.store, "m", &f.repo).unwrap();
    let second = review_pack(&f.store, "u", &f.repo).unwrap().tree;
    assert_ne!(first, second);
    let before = evidence(&f.store, "u").unwrap().len();
    let refused = f.review("u", &old);
    assert_eq!(refused.exit_code, 1, "{}", refused.stdout);
    assert!(
        refused
            .stderr
            .contains(&format!("the verdict reviewed {first}")),
        "{}",
        refused.stderr
    );
    assert_eq!(evidence(&f.store, "u").unwrap().len(), before);
    let untreed = f.verdict("u", &json!({"unit":"u","verdict":"pass","findings":[]}));
    assert!(f.review("u", &untreed).stderr.contains("missing tree"));
    let current = f.verdict(
        "u",
        &json!({"unit":"u","tree":second,"verdict":"pass","findings":[]}),
    );
    let accepted = f.review("u", &current);
    assert_eq!(accepted.exit_code, 0, "{}", accepted.stderr);
    assert!(accepted.stdout.contains("u\tverified"));
}

#[test]
fn a_large_deletion_keeps_its_last_version_readable() {
    let f = Fixture::new();
    fs::create_dir_all(f.repo.join("sub")).unwrap();
    fs::write(f.repo.join("sub/a.txt"), "a\n").unwrap();
    fs::write(
        f.repo.join("sub/big.txt"),
        "big line of text\n".repeat(8000),
    )
    .unwrap();
    f.add("u", "m", "sub/");
    f.work(
        "u",
        "gpt-6.1-terra",
        |repo| {
            fs::write(repo.join("sub/a.txt"), "changed\n").unwrap();
            fs::remove_file(repo.join("sub/big.txt")).unwrap();
        },
        json!({"status":"completed"}),
    );
    let info = review_pack(&f.store, "u", &f.repo).unwrap();
    assert_eq!(info.omitted, ["sub/big.txt"]);
    let text = fs::read_to_string(&info.path).unwrap();
    let line = text
        .lines()
        .find(|l| l.starts_with("- sub/big.txt  +0 -8000"))
        .unwrap_or_else(|| panic!("{text}"));
    let (_, kept) = line
        .split_once("deleted; its last version is at ")
        .unwrap_or_else(|| panic!("{line}"));
    let kept = kept.trim_end_matches(')');
    assert_eq!(
        fs::read_to_string(kept).unwrap(),
        "big line of text\n".repeat(8000)
    );
    assert!(!text.contains("big line of text"));
    assert!(text.contains("+changed"));
}

#[test]
fn a_unit_that_deletes_a_file_named_in_scope_gets_a_snapshot() {
    let f = Fixture::new();
    fs::write(f.repo.join("gone.txt"), "old\n").unwrap();
    f.add("u", "m", "a.txt gone.txt");
    f.work(
        "u",
        "gpt-6.1-terra",
        |repo| fs::remove_file(repo.join("gone.txt")).unwrap(),
        json!({"status":"completed"}),
    );
    assert!(verified_tree(&f.store, "u").starts_with("tree:"));
    let info = review_pack(&f.store, "u", &f.repo).unwrap();
    let text = fs::read_to_string(&info.path).unwrap();
    assert!(text.contains("- gone.txt  +0 -1\n"), "{text}");
    assert!(text.contains("-old\n"), "{text}");
}

#[test]
fn digest_lists_changes_the_worker_did_not_report() {
    let f = Fixture::new();
    fs::write(f.repo.join(".gitignore"), "sub/ignored.txt\n").unwrap();
    f.add("u", "m", "a.txt new.txt sub/");
    f.work(
        "u",
        "gpt-6.1-terra",
        |repo| {
            fs::write(repo.join("a.txt"), "changed\n").unwrap();
            fs::write(repo.join("new.txt"), "new\n").unwrap();
            fs::create_dir_all(repo.join("sub")).unwrap();
            fs::write(repo.join("sub/ignored.txt"), "hidden from git\n").unwrap();
        },
        json!({"status":"completed","file_changes":[{"path":"sub/ignored.txt"}]}),
    );
    let text = report(&f, &[]);
    assert!(
        text.contains("changed files (3):\n  a.txt\n  new.txt\n  sub/ignored.txt\n"),
        "{text}"
    );
    let path = f.store.join("reports/u.evidence.tsv");
    let rows = fs::read_to_string(&path).unwrap();
    let tree = verified_tree(&f.store, "u");
    fs::write(&path, rows.replace(&tree, "tree:0123456789")).unwrap();
    let text = report(&f, &[]);
    assert!(text.contains("changed files (1):\n"), "{text}");
    assert!(
        text.contains("u: git cannot compare its snapshots"),
        "{text}"
    );
}

fn set_state(f: &Fixture, unit: &str, state: &str) {
    let path = f.store.join("units.tsv");
    let rows = fs::read_to_string(&path).unwrap();
    fs::write(
        path,
        rows.replace(
            &format!("{unit}\tm\tpending\t"),
            &format!("{unit}\tm\t{state}\t"),
        ),
    )
    .unwrap();
}

fn put_verdict(f: &Fixture, unit: &str, findings: Value) {
    fs::create_dir_all(f.store.join("reports")).unwrap();
    fs::write(
        f.store.join(format!("reports/{unit}.verdict.json")),
        json!({"unit":unit,"tree":"tree:old","verdict":"fail","findings":findings}).to_string(),
    )
    .unwrap();
}

fn row(kind: &str, command: &str, tree: &str) -> Evidence {
    Evidence {
        kind: kind.into(),
        command: command.into(),
        exit_code: 0,
        commit: tree.into(),
        ts: "2026-01-01".into(),
    }
}

#[test]
fn digest_classifies_one_and_multi_level_chains() {
    for (states, answered) in [
        (vec![("a", "superseded:b"), ("b", "verified")], true),
        (
            vec![
                ("a", "superseded:b"),
                ("b", "superseded:c"),
                ("c", "verified"),
            ],
            true,
        ),
        (vec![("a", "superseded:b"), ("b", "pending")], false),
        (vec![("a", "superseded:missing")], false),
    ] {
        let f = Fixture::new();
        for (unit, state) in &states {
            f.add(unit, "m", "a.txt");
            set_state(&f, unit, state);
        }
        put_verdict(
            &f,
            "a",
            json!([{"severity":"major","check":"correctness","location":"a:1","evidence":"chain finding"}]),
        );
        let text = report(&f, &[]);
        let count = states
            .iter()
            .filter(|(_, s)| s.starts_with("superseded"))
            .count();
        assert!(
            text.contains(&format!("superseded units: {count}\n")),
            "{text}"
        );
        assert!(!text.contains("  a  superseded"), "{text}");
        if answered {
            assert!(text.contains("open findings (0;"), "{text}");
            assert!(text.contains("answered by later units: 1\n"), "{text}");
            assert!(!text.contains("chain finding"), "{text}");
        } else {
            assert!(text.contains("open findings (1;"), "{text}");
            assert!(
                text.contains("a  major  correctness  a:1: chain finding"),
                "{text}"
            );
            assert!(!text.contains("answered by later units:"), "{text}");
        }
    }
}

#[test]
fn digest_separates_needs_ruled_minor_and_open() {
    let f = Fixture::new();
    f.add("u", "m", "a.txt");
    put_verdict(
        &f,
        "u",
        json!([
            {"severity":"major","check":"untested","needs":"smoke","location":"a:1","evidence":"smoke evidence"},
            {"severity":"blocker","check":"untested","needs":"manual","location":"a:2","evidence":"manual evidence"},
            {"severity":"major","check":"correctness","location":"a:3","evidence":"waived evidence"},
            {"severity":"minor","check":"style","location":"a:4","evidence":"minor evidence"},
            {"severity":"blocker","check":"correctness","location":"a:5","evidence":"open evidence"}
        ]),
    );
    append_evidence(&f.store, "u", &row("review", "reviewer", "tree:old")).unwrap();
    append_evidence(
        &f.store,
        "u",
        &findings::waive_row(3, "accepted reason", "tree:old", false),
    )
    .unwrap();
    let text = report(&f, &[]);
    for present in [
        "open findings (1; 1 minor findings across 1 units",
        "u  blocker  correctness  a:5: open evidence",
        "needs a smoke or manual check (2):",
        "u  major  smoke  a:1: smoke evidence",
        "u  blocker  manual  a:2: manual evidence",
        "ruled (1):",
        "u  #3  major  a:3: accepted reason",
    ] {
        assert!(text.contains(present), "{present}\n{text}");
    }
    assert!(!text.contains("waived evidence"), "{text}");
    assert!(!text.contains("minor evidence"), "{text}");
}

#[test]
fn re_review_pack_contains_only_new_diff_and_unwaived_serious_findings() {
    let f = Fixture::new();
    f.built();
    let first = verified_tree(&f.store, "u");
    put_verdict(
        &f,
        "u",
        json!([
            {"severity":"blocker","check":"safety","location":"a:1","evidence":"old blocker"},
            {"severity":"major","check":"logic","location":"a:2","evidence":"old major"},
            {"severity":"minor","check":"style","location":"a:3","evidence":"old minor"},
            {"severity":"major","check":"logic","location":"a:4","evidence":"old waived"}
        ]),
    );
    append_evidence(&f.store, "u", &row("review", "reviewer", &first)).unwrap();
    append_evidence(
        &f.store,
        "u",
        &findings::waive_row(4, "waived", &first, true),
    )
    .unwrap();
    let same_tree = review_pack(&f.store, "u", &f.repo).unwrap();
    let same_text = fs::read_to_string(same_tree.path).unwrap();
    assert_eq!(same_tree.files, 2);
    assert!(same_text.contains("+line one hundred"));
    assert!(!same_text.contains("## Earlier findings"));
    let text = fs::read_to_string(f.repo.join("a.txt"))
        .unwrap()
        .replace("line 150\n", "second edit\n");
    fs::write(f.repo.join("a.txt"), text).unwrap();
    assert_eq!(record_verify(&f.store, "u", &f.repo).unwrap().exit_code, 0);
    let pack = review_pack(&f.store, "u", &f.repo).unwrap();
    let text = fs::read_to_string(pack.path).unwrap();
    assert_eq!(pack.files, 1);
    for present in [
        &format!("Earlier review at: {first}"),
        "## Earlier findings",
        "1. blocker safety a:1: old blocker",
        "2. major logic a:2: old major",
        "+second edit",
    ] {
        assert!(text.contains(present), "{present}\n{text}");
    }
    for absent in ["old minor", "old waived", "+line one hundred", "new.txt"] {
        // new.txt is in the original brief; only its diff and file listing must disappear.
        if absent == "new.txt" {
            assert!(!text.contains("diff --git a/new.txt"));
            assert!(!text.contains("- new.txt  +"));
        } else {
            assert!(!text.contains(absent), "{absent}\n{text}");
        }
    }
}

#[test]
fn fix_pack_lists_old_unwaived_serious_findings() {
    let f = Fixture::new();
    f.add("old", "m", "a.txt");
    put_verdict(
        &f,
        "old",
        json!([
            {"severity":"blocker","check":"safety","location":"a:1","evidence":"fix blocker"},
            {"severity":"major","check":"logic","location":"a:2","evidence":"fix major"},
            {"severity":"minor","check":"style","location":"a:3","evidence":"fix minor"},
            {"severity":"major","check":"logic","location":"a:4","evidence":"fix waived"}
        ]),
    );
    append_evidence(&f.store, "old", &row("review", "reviewer", "tree:old")).unwrap();
    append_evidence(
        &f.store,
        "old",
        &findings::waive_row(4, "reason", "tree:old", true),
    )
    .unwrap();
    f.add("fix", "m", "a.txt");
    f.work(
        "fix",
        "gpt-6.1-terra",
        |repo| fs::write(repo.join("a.txt"), "fixed\n").unwrap(),
        json!({"status":"completed"}),
    );
    append_evidence(&f.store, "fix", &row(FIXES_KIND, "old", "")).unwrap();
    let text = fs::read_to_string(review_pack(&f.store, "fix", &f.repo).unwrap().path).unwrap();
    for present in [
        "Fixes unit: old",
        "## Findings to fix",
        "1. blocker safety a:1: fix blocker",
        "2. major logic a:2: fix major",
        "The review checks these findings and the changed lines only.",
        "+fixed",
    ] {
        assert!(text.contains(present), "{present}\n{text}");
    }
    assert!(!text.contains("fix minor"), "{text}");
    assert!(!text.contains("fix waived"), "{text}");
}
