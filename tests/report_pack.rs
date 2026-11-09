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
        "line 80\n",
        "line 120\n",
        "+brand new file",
        "- a.txt  +1 -1",
        "- new.txt  +1 -0",
    ] {
        assert!(text.contains(present), "{present}\n{text}");
    }
    for absent in ["\n line 79\n", "\n line 121\n", "outside text"] {
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
    let verdict = f.verdict("u", &json!({"unit":"u","verdict":"pass","findings":[]}));
    let before = evidence(&f.store, "u").unwrap().len();
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
    let verdict = f.verdict("u", &json!({"unit":"u","verdict":"pass","findings":[]}));
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
    let fail = json!({"unit":"u","verdict":"fail","findings":[
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
        &json!({"unit":"u","verdict":"fail","findings":findings}),
    );
    assert_eq!(f.review("u", &verdict).exit_code, 0);
    let reports: u64 = fs::read_dir(f.store.join("reports"))
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().len())
        .sum();
    assert!(reports > 1_000_000, "{reports}");
    let text = report(&f, &[]);
    assert!(text.chars().count() < 45_000, "{}", text.chars().count());
    assert!(text.contains("changed files (600):"));
    assert!(text.contains("... 400 more in reports/<unit>.json"));
    assert!(text.contains("... 170 more in reports/<unit>.verdict.json"));
    assert!(text.contains("cut at 16000 characters; the rest is in"));
    assert!(text.contains("decisions.md"));
}
