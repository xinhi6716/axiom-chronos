//! Lock order between a unit's claim and the store lock, under load in one process.
//!
//! The bridge runs every tool call on its own thread. `begin_unit` and `release_claim` take the
//! in-process claims table, then the store lock. `unit add --fixes` must never take them the other
//! way round (its readiness scan reads the claims table), or the two wait for each other forever.
use axiom::orch::*;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

const FIXES: usize = 50;
/// A run that finishes takes about a minute on a debug build; a deadlock never finishes.
const WATCHDOG: Duration = Duration::from_secs(300);

fn call(store: &Path, args: &[&str]) -> Output {
    run(
        store,
        &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        false,
        false,
    )
}

fn claim_rows(store: &Path) -> Vec<(String, String)> {
    match fs::read_to_string(store.join("claims.tsv")) {
        Ok(text) => text
            .lines()
            .skip(1)
            .filter(|l| !l.is_empty())
            .map(|l| {
                let c: Vec<_> = l.split('\t').collect();
                (c[0].to_string(), c.get(1).unwrap_or(&"").to_string())
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn setup(root: &Path) -> (PathBuf, PathBuf) {
    let repo = root.join("repo");
    let store = root.join("store");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::create_dir_all(&store).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    fs::write(repo.join("src/a"), "a").unwrap();
    assert_eq!(call(&store, &["init"]).exit_code, 0);
    fs::write(
        store.join("modules.md"),
        "## m\nPURPOSE x\nPATHS src/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\nROUTING\n  default terra/medium trivial reason\n",
    )
    .unwrap();
    let brief = |name: &str, scope: &str| {
        let path = store.join(format!("{name}.brief"));
        // A routed store runs only units with a test matrix.
        fs::write(
            &path,
            format!("GOAL g\nRISK trivial\nSCOPE {scope}\nCONTRACT c\nTEST_MATRIX\nx | behaviour | {scope}::c | ok | cargo test\nVERIFY echo ok\n"),
        )
        .unwrap();
        path.to_str().unwrap().to_owned()
    };
    let mut units = String::from("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n");
    units.push_str(&format!("a\tm\tpending\t\t\t\t{}\n", brief("a", "src/a")));
    for i in 0..FIXES {
        units.push_str(&format!(
            "origin{i}\tm\tpending\t\t\t\t{}\n",
            brief(&format!("origin{i}"), &format!("src/o{i}"))
        ));
        brief(&format!("fix{i}"), &format!("src/o{i}"));
    }
    fs::write(store.join("units.tsv"), units).unwrap();
    (store, repo)
}

#[test]
fn claims_and_fix_units_in_parallel_threads_neither_deadlock_nor_lose_writes() {
    let root = tempfile::tempdir().unwrap();
    let (store, repo) = setup(root.path());
    let fixes_done = Arc::new(AtomicUsize::new(0));
    let claims_done = Arc::new(AtomicUsize::new(0));
    let adding = Arc::new(AtomicBool::new(true));
    let (tx, rx) = mpsc::channel::<Result<(), String>>();

    // Begin and release a claim on `a`, for at least as long as the fix units are being added.
    std::thread::spawn({
        let (store, repo, tx) = (store.clone(), repo.clone(), tx.clone());
        let (claims_done, adding) = (claims_done.clone(), adding.clone());
        move || {
            let result = (|| -> Result<(), String> {
                while adding.load(Ordering::SeqCst) || claims_done.load(Ordering::SeqCst) < FIXES {
                    let n = claims_done.load(Ordering::SeqCst);
                    let attempt_id = match begin_unit(&store, "a", "worker", &repo, false) {
                        Ok(Begin::Run { attempt_id, .. }) => attempt_id,
                        Ok(Begin::Reused) => return Err(format!("begin {n}: reused")),
                        Err(e) => return Err(format!("begin {n}: {}: {}", e.kind, e.message)),
                    };
                    if !claim_rows(&store).contains(&("a".to_string(), attempt_id.clone())) {
                        return Err(format!("begin {n}: claim {attempt_id} is not on disk"));
                    }
                    release_claim(&store, "a", &attempt_id)
                        .map_err(|e| format!("release {n}: {}: {}", e.kind, e.message))?;
                    if claim_rows(&store).iter().any(|(unit, _)| unit == "a") {
                        return Err(format!("release {n}: claim {attempt_id} is still on disk"));
                    }
                    claims_done.fetch_add(1, Ordering::SeqCst);
                }
                Ok(())
            })();
            let _ = tx.send(result);
        }
    });

    // Add one fix unit to each origin.
    std::thread::spawn({
        let (store, tx) = (store.clone(), tx.clone());
        let (fixes_done, adding) = (fixes_done.clone(), adding.clone());
        move || {
            let result = (|| -> Result<(), String> {
                for i in 0..FIXES {
                    let brief = store.join(format!("fix{i}.brief"));
                    let out = call(
                        &store,
                        &[
                            "unit",
                            "add",
                            &format!("fix{i}"),
                            "--track",
                            "m",
                            "--brief",
                            brief.to_str().unwrap(),
                            "--fixes",
                            &format!("origin{i}"),
                        ],
                    );
                    if out.exit_code != 0 {
                        return Err(format!("unit add fix{i}: {}", out.stderr));
                    }
                    fixes_done.fetch_add(1, Ordering::SeqCst);
                }
                Ok(())
            })();
            adding.store(false, Ordering::SeqCst);
            let _ = tx.send(result);
        }
    });

    // Threads that deadlocked are left behind: the test fails instead of hanging.
    let deadline = Instant::now() + WATCHDOG;
    for _ in 0..2 {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => panic!("{e}"),
            Err(_) => panic!(
                "deadlock: after {WATCHDOG:?}, {} of {FIXES} fix units added and {} claims begun and released",
                fixes_done.load(Ordering::SeqCst),
                claims_done.load(Ordering::SeqCst)
            ),
        }
    }

    eprintln!(
        "{FIXES} fix units added while {} claims were begun and released",
        claims_done.load(Ordering::SeqCst)
    );
    assert!(claims_done.load(Ordering::SeqCst) >= FIXES);
    assert_eq!(claim_rows(&store), Vec::<(String, String)>::new());
    let units = read_units(&store).unwrap();
    assert_eq!(units.len(), 1 + 2 * FIXES);
    for i in 0..FIXES {
        let (origin, fix) = (format!("origin{i}"), format!("fix{i}"));
        let row = units.iter().find(|u| u.id == origin).unwrap();
        assert_eq!(row.superseded_by(), Some(fix.as_str()), "{origin}");
        assert_eq!(
            fixed_unit(&store, &fix).unwrap().as_deref(),
            Some(origin.as_str()),
            "{fix} lost its fixes evidence"
        );
    }
    assert!(!store.join(".orch.lock").exists());
}
