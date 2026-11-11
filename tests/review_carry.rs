use axiom::orch::*;
use std::{fs, path::PathBuf, process::Command, sync::Mutex};

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct CarryEnv(Option<std::ffi::OsString>);
impl CarryEnv {
    fn set(value: Option<&str>) -> Self {
        let old = std::env::var_os(CARRY_ENV);
        if let Some(value) = value {
            std::env::set_var(CARRY_ENV, value);
        } else {
            std::env::remove_var(CARRY_ENV);
        }
        Self(old)
    }
}
impl Drop for CarryEnv {
    fn drop(&mut self) {
        if let Some(value) = &self.0 {
            std::env::set_var(CARRY_ENV, value);
        } else {
            std::env::remove_var(CARRY_ENV);
        }
    }
}

fn row(kind: &str, command: &str, exit_code: i32, commit: &str, ts: &str) -> Evidence {
    Evidence {
        kind: kind.into(),
        command: command.into(),
        exit_code,
        commit: commit.into(),
        ts: ts.into(),
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    store: PathBuf,
    repo: PathBuf,
    old: String,
}
impl Fixture {
    fn new(other_track: &str, review_code: i32, run_ts: &str) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&store).unwrap();
        fs::create_dir_all(&repo).unwrap();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        fs::write(repo.join("a.txt"), "old a").unwrap();
        fs::write(repo.join("b.txt"), "old b").unwrap();
        let brief = store.join("brief.txt");
        fs::write(&brief, "GOAL x\nSCOPE a.txt b.txt\nVERIFY echo ok\n").unwrap();
        fs::write(store.join("units.tsv"), format!("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tpending\t\t\t\t{}\nx\t{other_track}\tpending\t\t\t\t{}\n", brief.display(), brief.display())).unwrap();
        fs::write(
            store.join("modules.md"),
            "## m\nPURPOSE x\nPATHS a.txt b.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
        )
        .unwrap();
        let old = snapshot(&repo, &["a.txt".into(), "b.txt".into()]);
        for r in [
            row("run", "gpt-worker", 0, &old, "1"),
            row("verify", "test", 0, &old, "2"),
            row("accept", "test", 0, &old, "3"),
            row("review", "claude-review", review_code, &old, "4"),
        ] {
            append_evidence(&store, "u", &r).unwrap();
        }
        append_evidence(&store, "x", &row("run", "gpt-worker", 0, &old, run_ts)).unwrap();
        Self {
            _tmp: tmp,
            store,
            repo,
            old,
        }
    }
    fn change(&self, path: &str) -> String {
        fs::write(self.repo.join(path), "changed").unwrap();
        snapshot(&self.repo, &["a.txt".into(), "b.txt".into()])
    }
    fn x_tree(&self, tree: &str) {
        append_evidence(&self.store, "x", &row("verify", "test", 0, tree, "6")).unwrap();
    }
    fn last(&self) -> Evidence {
        evidence(&self.store, "u").unwrap().pop().unwrap()
    }
}

fn assert_carried(f: &Fixture, tree: &str) {
    let last = f.last();
    assert_eq!(last.kind, "review");
    assert_eq!(last.command, "claude-review carried-by:x");
    assert_eq!(last.commit, tree);
    assert_eq!(carried_by(&last), Some(vec!["x".into()]));
}

#[test]
fn carries_through_accept_and_verify() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _env = CarryEnv::set(None);
    let f = Fixture::new("m", 0, "5");
    let new = f.change("a.txt");
    f.x_tree(&new);
    record_accept(&f.store, "m", &f.repo).unwrap();
    assert_carried(&f, &new);
    assert_eq!(
        derive_state(&evidence(&f.store, "u").unwrap(), true),
        "verified"
    );

    let f = Fixture::new("m", 0, "5");
    let new = f.change("a.txt");
    f.x_tree(&new);
    record_verify(&f.store, "u", &f.repo).unwrap();
    assert_carried(&f, &new);
    assert_eq!(
        derive_state(&evidence(&f.store, "u").unwrap(), false),
        "verified"
    );
    assert_eq!(
        read_units(&f.store)
            .unwrap()
            .iter()
            .find(|u| u.id == "u")
            .unwrap()
            .state,
        "built"
    );
}

#[test]
fn carry_requires_all_conditions() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _env = CarryEnv::set(None);
    for (track, code, ts, covered) in [
        ("other", 0, "5", true),
        ("m", 1, "5", true),
        ("m", 0, "3", true),
        ("m", 0, "5", false),
    ] {
        let f = Fixture::new(track, code, ts);
        let new = f.change("a.txt");
        if covered {
            f.x_tree(&new);
        } else {
            f.x_tree(&f.old);
        }
        record_verify(&f.store, "u", &f.repo).unwrap();
        assert_eq!(
            f.last().kind,
            "verify",
            "track={track} code={code} ts={ts} covered={covered}"
        );
    }
    let _disabled = CarryEnv::set(Some("0"));
    let f = Fixture::new("m", 0, "5");
    let new = f.change("a.txt");
    f.x_tree(&new);
    record_verify(&f.store, "u", &f.repo).unwrap();
    assert_eq!(f.last().kind, "verify");
}

#[test]
fn waivers_and_carried_reviews_obey_row_order_and_latest_check() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _env = CarryEnv::set(None);
    let f = Fixture::new("m", 1, "5");
    let waiver = row(WAIVE_KIND, "1 accepted reason", 0, &f.old, "5");
    append_evidence(&f.store, "u", &waiver).unwrap();
    assert_eq!(
        derive_state(&evidence(&f.store, "u").unwrap(), true),
        "verified"
    );
    let new = f.change("a.txt");
    f.x_tree(&new);
    record_accept(&f.store, "m", &f.repo).unwrap();
    assert_carried(&f, &new);
    assert_eq!(
        derive_state(&evidence(&f.store, "u").unwrap(), true),
        "verified"
    );
    let mut rows = evidence(&f.store, "u").unwrap()[..4].to_vec();
    rows.insert(3, waiver.clone());
    assert_eq!(derive_state(&rows, true), "review-failed");
    rows.remove(3);
    rows.push(row(WAIVE_KIND, "1 partial", 1, &f.old, "5"));
    assert_eq!(derive_state(&rows, true), "review-failed");
    rows.push(row(WAIVE_KIND, "1 other", 0, "tree:other", "6"));
    assert_eq!(derive_state(&rows, true), "review-failed");
    assert_eq!(
        carried_by(&row(FIXES_KIND, "u carried-by:x", 0, "", "")),
        None
    );
}

#[test]
fn repeated_carry_merges_ids_and_accepts_superseded_units() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _env = CarryEnv::set(None);
    let f = Fixture::new("m", 0, "5");
    let first = f.change("a.txt");
    f.x_tree(&first);
    record_verify(&f.store, "u", &f.repo).unwrap();
    assert_carried(&f, &first);
    let brief = f.store.join("brief.txt");
    let units_path = f.store.join("units.tsv");
    let mut units = fs::read_to_string(&units_path).unwrap();
    units.push_str(&format!("y\tm\tsuperseded:u\t\t\t\t{}\n", brief.display()));
    fs::write(units_path, units).unwrap();
    append_evidence(&f.store, "y", &row("run", "gpt-worker", 0, &first, "9999")).unwrap();
    let second = f.change("b.txt");
    append_evidence(&f.store, "y", &row("verify", "test", 0, &second, "9999")).unwrap();
    let ids = append_check(
        &f.store,
        "u",
        &f.repo,
        &row("verify", "test", 0, &second, "9999"),
    )
    .unwrap();
    assert_eq!(ids, Some(vec!["x".into(), "y".into()]));
    assert_eq!(f.last().command, "claude-review carried-by:x,y");
    assert_eq!(f.last().commit, second);
}

#[test]
fn partially_covered_diff_does_not_carry() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _env = CarryEnv::set(None);
    let f = Fixture::new("m", 0, "5");
    let partial = f.change("a.txt");
    f.x_tree(&partial);
    f.change("b.txt");
    record_verify(&f.store, "u", &f.repo).unwrap();
    assert_eq!(f.last().kind, "verify");
}

#[test]
fn carried_review_keeps_reviewer_family_and_must_match_latest_check() {
    for reviewer in ["opus", "sonnet", "haiku"] {
        let rows = vec![
            row("run", "claude-worker", 0, "", "1"),
            row("verify", "test", 0, "tree:old", "2"),
            row("accept", "test", 0, "tree:new", "3"),
            row(
                "review",
                &format!("{reviewer}{CARRIED_MARK}x"),
                0,
                "tree:new",
                "4",
            ),
        ];
        assert_eq!(derive_state(&rows, true), "built", "{reviewer}");
    }

    let rows = vec![
        row("run", "gpt-worker", 0, "", "1"),
        row("verify", "test", 0, "tree:old", "2"),
        row("accept", "test", 0, "tree:new", "3"),
        row("review", "claude-review carried-by:x", 0, "tree:old", "4"),
    ];
    assert_eq!(derive_state(&rows, true), "built");
}
