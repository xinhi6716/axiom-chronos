use axiom::{
    base,
    orch::{events, store},
    storage::{
        ledger::normalize,
        retention::{self, Mode},
        worktree::{self, Removal, State},
        Class, EntryState, Ledger, Owner, Policy,
    },
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}
struct Fixture {
    _tmp: TempDir,
    home: PathBuf,
    repo: PathBuf,
    owner: Owner,
}
impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        fs::write(
            repo.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(repo.join(".gitignore"), "/target/\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "initial"]);
        Self {
            _tmp: tmp,
            home,
            repo,
            owner: Owner {
                project: "project".into(),
                wish: Some("w-1234567890123-abcdef".into()),
            },
        }
    }
    fn worktree(&self, name: &str) -> PathBuf {
        let p = self._tmp.path().join(name);
        git(
            &self.repo,
            &["worktree", "add", "-b", name, p.to_str().unwrap()],
        );
        p
    }
    fn register(&self, p: &Path) {
        worktree::register(&self.home, p, &self.owner)
            .unwrap()
            .unwrap();
    }
    fn merge(&self, p: &Path, branch: &str) {
        fs::write(p.join(format!("{branch}.txt")), "change").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "work"]);
        git(&self.repo, &["merge", "--no-ff", branch, "-m", "merge"]);
    }
}
#[test]
fn merged_worktree_cleanup_requires_confirmation() {
    let f = Fixture::new();
    let p = f.worktree("one");
    fs::create_dir(p.join("target")).unwrap();
    f.register(&p);
    f.merge(&p, "one");
    assert_eq!(
        worktree::scan(&f.home, now()).unwrap()[0].state,
        State::Merged
    );
    let before = Ledger::open(&f.home)
        .unwrap()
        .get(&p)
        .unwrap()
        .unwrap()
        .meta;
    assert!(matches!(
        worktree::remove(&f.home, &p, false, now()).unwrap(),
        Removal::Planned { .. }
    ));
    assert!(p.exists() && p.join("target").exists());
    assert_eq!(
        before,
        Ledger::open(&f.home)
            .unwrap()
            .get(&p)
            .unwrap()
            .unwrap()
            .meta
    );
    let Removal::Removed { records } = worktree::remove(&f.home, &p, true, now()).unwrap() else {
        panic!()
    };
    assert!(!p.exists());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].reason, "user_confirmed");
    assert!(fs::read_to_string(retention::audit_log(&f.home))
        .unwrap()
        .contains("user_confirmed"));
    assert_eq!(
        Ledger::open(&f.home)
            .unwrap()
            .get(&p)
            .unwrap()
            .unwrap()
            .state,
        EntryState::Deleted
    );
}
#[test]
fn dirty_or_active_worktree_is_preserved() {
    let f = Fixture::new();
    let p = f.worktree("one");
    f.register(&p);
    fs::write(p.join("dirty"), "x").unwrap();
    assert_eq!(
        worktree::remove(&f.home, &p, true, now()).unwrap_err().kind,
        "worktree_busy"
    );
    assert!(p.exists());
    fs::remove_file(p.join("dirty")).unwrap();
    assert_eq!(
        worktree::remove(&f.home, &p, true, now()).unwrap_err().kind,
        "worktree_busy"
    );
    f.merge(&p, "one");
    let ledger = Ledger::open(&f.home).unwrap();
    let _lease = ledger.lease(&p).unwrap();
    assert_eq!(
        worktree::remove(&f.home, &p, true, now()).unwrap_err().kind,
        "worktree_busy"
    );
    assert!(p.exists());
}
#[test]
fn workers_share_repository_target() {
    let f = Fixture::new();
    let p = f.worktree("one");
    let q = f.worktree("two");
    let a = worktree::worker_env(&f.home, &p, &f.owner).unwrap();
    let b = worktree::worker_env(&f.home, &q, &f.owner).unwrap();
    assert_eq!(a, b);
    assert_eq!(a[0].0, "CARGO_TARGET_DIR");
    assert!(Path::new(&a[0].1).starts_with(normalize(&f.home.join("build-cache"))));
    let other = Fixture::new();
    assert_ne!(
        a,
        worktree::worker_env(&f.home, &other.repo, &f.owner).unwrap()
    );
    fs::remove_file(q.join("Cargo.toml")).unwrap();
    assert!(worktree::worker_env(&f.home, &q, &f.owner)
        .unwrap()
        .is_empty());
}
#[test]
fn shared_target_survives_one_worktree_removal() {
    let f = Fixture::new();
    let p = f.worktree("one");
    let q = f.worktree("two");
    f.register(&p);
    f.register(&q);
    let cache = worktree::shared_target(&f.home, &p, &f.owner)
        .unwrap()
        .unwrap();
    worktree::shared_target(&f.home, &q, &f.owner).unwrap();
    f.merge(&p, "one");
    worktree::remove(&f.home, &p, true, now()).unwrap();
    assert!(cache.exists());
    let entry = Ledger::open(&f.home).unwrap().get(&cache).unwrap().unwrap();
    assert_eq!(entry.meta["users"].as_array().unwrap().len(), 1);
    assert_eq!(
        entry.meta["users"][0],
        normalize(&q).to_string_lossy().as_ref()
    );
    assert!(
        !retention::plan(&f.home, &Policy::default(), Mode::Pressure, now())
            .unwrap()
            .items
            .iter()
            .any(|i| i.path == cache)
    );
}
#[test]
fn announce_merged_fires_once() {
    let f = Fixture::new();
    let p = f.worktree("one");
    let fresh = f.worktree("fresh");
    f.register(&p);
    f.register(&fresh);
    let wish_store = store::wish_store(&f.home, &f.owner.project, f.owner.wish.as_deref()).unwrap();
    fs::create_dir_all(&wish_store).unwrap();
    assert!(worktree::announce_merged(&f.home, now())
        .unwrap()
        .is_empty());
    f.merge(&p, "one");
    let announced = worktree::announce_merged(&f.home, now()).unwrap();
    assert_eq!(announced.len(), 1);
    assert_eq!(announced[0].path, normalize(&p));
    assert!(worktree::announce_merged(&f.home, now())
        .unwrap()
        .is_empty());
    assert_eq!(
        fs::read_to_string(f.home.join("storage/events.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    let entries = events::read_events(&wish_store, 0).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, worktree::EVENT_CLEANUP_AVAILABLE);
}
#[test]
fn adopt_registers_legacy_worktrees() {
    let f = Fixture::new();
    let p = f.worktree("legacy");
    let rows = worktree::adopt(&f.home, &f.repo).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, State::Active);
    let e = Ledger::open(&f.home).unwrap().get(&p).unwrap().unwrap();
    assert_eq!(e.class, Class::Worktree);
    assert_eq!(e.owner.project, base::slug_for_dir(&f.repo));
    assert!(e.meta.get("registered_head").is_none());
    assert!(worktree::register(&f.home, &f.repo, &f.owner)
        .unwrap()
        .is_none());
}
#[test]
fn non_merged_branch_is_not_merged() {
    let f = Fixture::new();
    let p = f.worktree("one");
    f.register(&p);
    fs::write(p.join("change"), "x").unwrap();
    git(&p, &["add", "."]);
    git(&p, &["commit", "-m", "unmerged"]);
    assert_eq!(
        worktree::scan(&f.home, now()).unwrap()[0].state,
        State::Active
    );
}
#[test]
fn abandoned_gone_and_state_metadata() {
    let f = Fixture::new();
    let p = f.worktree("one");
    fs::create_dir(p.join("target")).unwrap();
    f.register(&p);
    let ledger = Ledger::open(&f.home).unwrap();
    ledger.touch(&p, 1).unwrap();
    fs::create_dir_all(f.home.join("goals")).unwrap();
    fs::write(f.home.join("goals/goal.json"), serde_json::json!({"wish":"test", "project":"project", "wish_id":f.owner.wish, "status":"done"}).to_string()).unwrap();
    let t = 10 * 86_400_000;
    assert_eq!(
        worktree::scan(&f.home, t).unwrap()[0].state,
        State::Abandoned
    );
    let entry = ledger.get(&p).unwrap().unwrap();
    assert_eq!(entry.meta["state_since_ms"], t);
    let cache = ledger.get(&p.join("target")).unwrap().unwrap();
    assert_eq!(cache.meta["worktree_state"], "abandoned");
    assert_eq!(cache.meta["state_since_ms"], t);
    worktree::scan(&f.home, t + 1).unwrap();
    assert_eq!(ledger.get(&p).unwrap().unwrap().meta["state_since_ms"], t);
    let head = entry.meta["registered_head"].clone();
    f.register(&p);
    let entry = ledger.get(&p).unwrap().unwrap();
    assert_eq!(entry.meta["registered_head"], head);
    assert_eq!(entry.meta["worktree_state"], "abandoned");
    git(&f.repo, &["worktree", "remove", p.to_str().unwrap()]);
    assert_eq!(
        worktree::scan(&f.home, t + 2).unwrap()[0].state,
        State::Gone
    );
    assert!(matches!(
        worktree::remove(&f.home, &p, true, t + 3).unwrap(),
        Removal::Removed { .. }
    ));
}
