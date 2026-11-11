use axiom::orch::*;
use std::{collections::BTreeMap, fs, path::Path};

fn call(store: &Path, args: &[&str], json: bool) -> Output {
    run(
        store,
        &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        json,
        false,
    )
}

fn ok(store: &Path, args: &[&str], json: bool) -> Output {
    let out = call(store, args, json);
    assert_eq!(out.exit_code, 0, "{args:?}: {}", out.stderr);
    out
}

fn setup() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    ok(dir.path(), &["init"], false);
    let brief = dir.path().join("brief.md");
    fs::write(&brief, "GOAL g\nSCOPE src/a\nVERIFY echo ok\n").unwrap();
    (dir, brief.to_str().unwrap().to_owned())
}

fn add(store: &Path, id: &str, brief: &str, old: Option<&str>, json: bool) -> Output {
    let mut args = vec!["unit", "add", id, "--track", "core", "--brief", brief];
    if let Some(old) = old {
        args.extend(["--fixes", old]);
    }
    call(store, &args, json)
}

fn snapshot(store: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    files.insert(
        "units.tsv".into(),
        fs::read(store.join("units.tsv")).unwrap(),
    );
    let reports = store.join("reports");
    if reports.exists() {
        for entry in fs::read_dir(reports).unwrap() {
            let entry = entry.unwrap();
            if entry
                .file_name()
                .to_string_lossy()
                .ends_with(".evidence.tsv")
            {
                files.insert(
                    entry.file_name().to_string_lossy().into_owned(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    files
}

#[test]
fn two_fixes_succeed_and_third_is_refused_without_writes() {
    let (dir, brief) = setup();
    let p = dir.path();
    assert_eq!(add(p, "origin", &brief, None, false).exit_code, 0);
    let first = add(p, "fix1", &brief, Some("origin"), false);
    assert_eq!(first.exit_code, 0, "{}", first.stderr);
    assert!(first
        .stdout
        .starts_with(&format!("fix1\tcore\tpending\t\t\t\t{brief}\n")));
    let second = add(p, "fix2", &brief, Some("fix1"), true);
    assert_eq!(second.exit_code, 0, "{}", second.stderr);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&second.stdout).unwrap()["fixes"],
        "fix1"
    );
    assert_eq!(fixed_unit(p, "origin").unwrap(), None);
    assert_eq!(fixed_unit(p, "fix1").unwrap().as_deref(), Some("origin"));
    assert_eq!(fixed_unit(p, "fix2").unwrap().as_deref(), Some("fix1"));
    let evidence = evidence(p, "fix2").unwrap();
    assert_eq!(evidence.len(), 1);
    assert_eq!(
        (
            &*evidence[0].kind,
            &*evidence[0].command,
            evidence[0].exit_code,
            &*evidence[0].commit
        ),
        (FIXES_KIND, "fix1", 0, "")
    );
    assert!(!evidence[0].ts.is_empty());
    let units = read_units(p).unwrap();
    assert_eq!(
        units.iter().map(|u| u.state.as_str()).collect::<Vec<_>>(),
        ["superseded:fix1", "superseded:fix2", "pending"]
    );
    let before = snapshot(p);
    let third = add(p, "fix3", &brief, Some("fix2"), false);
    assert_eq!(third.exit_code, 1);
    assert!(
        third.stderr.starts_with("error: fix_rounds:"),
        "{}",
        third.stderr
    );
    for text in ["fix1", "fix2", "core", "waive", "blocked"] {
        assert!(third.stderr.contains(text), "{}", third.stderr);
    }
    assert_eq!(snapshot(p), before);
}

#[test]
fn missing_old_and_missing_brief_refuse_without_writes() {
    let (dir, brief) = setup();
    let p = dir.path();
    let before = snapshot(p);
    let missing = add(p, "fix", &brief, Some("missing"), false);
    assert_eq!(missing.exit_code, 2);
    assert!(missing.stderr.contains("unit missing not found"));
    assert_eq!(snapshot(p), before);
    let no_brief = call(
        p,
        &[
            "unit", "add", "fix", "--track", "core", "--fixes", "missing",
        ],
        false,
    );
    assert_eq!(no_brief.exit_code, 1);
    assert!(no_brief.stderr.contains("--fixes requires --brief"));
    assert_eq!(snapshot(p), before);
}

#[test]
fn ordinary_supersede_and_branched_fix_chains() {
    let (dir, brief) = setup();
    let p = dir.path();
    for id in ["origin", "other"] {
        assert_eq!(add(p, id, &brief, None, false).exit_code, 0);
    }
    assert_eq!(add(p, "fix1", &brief, Some("origin"), false).exit_code, 0);
    ok(p, &["unit", "supersede", "other", "--by", "fix1"], false);
    assert_eq!(add(p, "fix2", &brief, Some("other"), false).exit_code, 1);
    assert_eq!(add(p, "fix2", &brief, Some("fix1"), false).exit_code, 0);
    let before = snapshot(p);
    let out = add(p, "fix3", &brief, Some("fix2"), false);
    assert!(out.stderr.contains("fix_rounds:"), "{}", out.stderr);
    assert_eq!(snapshot(p), before);
}

#[test]
fn add_fix_reuses_supersede_checks_before_writing() {
    let (dir, brief) = setup();
    let p = dir.path();
    assert_eq!(add(p, "origin", &brief, None, false).exit_code, 0);
    let narrow = p.join("narrow.md");
    fs::write(&narrow, "GOAL g\nSCOPE src/b\nVERIFY echo ok\n").unwrap();
    let before = snapshot(p);
    let scope = add(p, "fix", narrow.to_str().unwrap(), Some("origin"), false);
    assert!(
        scope.stderr.contains("does not cover src/a"),
        "{}",
        scope.stderr
    );
    assert_eq!(snapshot(p), before);
    let track = call(
        p,
        &[
            "unit", "add", "fix", "--track", "other", "--brief", &brief, "--fixes", "origin",
        ],
        false,
    );
    assert!(
        track.stderr.contains("belongs to other, not core"),
        "{}",
        track.stderr
    );
    assert_eq!(snapshot(p), before);
    let duplicate = add(p, "origin", &brief, Some("origin"), false);
    assert!(
        duplicate.stderr.contains("unit origin already exists"),
        "{}",
        duplicate.stderr
    );
    assert_eq!(snapshot(p), before);
}

#[test]
fn fix_units_in_separate_branches_count_after_they_merge() {
    let (dir, brief) = setup();
    let p = dir.path();
    for id in ["left", "right", "merge"] {
        assert_eq!(add(p, id, &brief, None, false).exit_code, 0);
    }
    assert_eq!(add(p, "fix1", &brief, Some("left"), false).exit_code, 0);
    assert_eq!(add(p, "fix2", &brief, Some("right"), false).exit_code, 0);
    for id in ["fix1", "fix2"] {
        ok(p, &["unit", "supersede", id, "--by", "merge"], false);
    }
    let before = snapshot(p);
    let out = add(p, "fix3", &brief, Some("merge"), false);
    assert!(out.stderr.contains("fix_rounds:"), "{}", out.stderr);
    assert!(out.stderr.contains("fix1") && out.stderr.contains("fix2"));
    assert_eq!(snapshot(p), before);
}

#[test]
fn concurrent_fixes_cannot_both_supersede_the_same_unit() {
    let (dir, brief) = setup();
    let p = dir.path();
    assert_eq!(add(p, "origin", &brief, None, false).exit_code, 0);
    let results = std::thread::scope(|scope| {
        let a = scope.spawn(|| add(p, "fix1", &brief, Some("origin"), false));
        let b = scope.spawn(|| add(p, "fix2", &brief, Some("origin"), false));
        [a.join().unwrap(), b.join().unwrap()]
    });
    assert_eq!(results.iter().filter(|out| out.exit_code == 0).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|out| out.stderr.contains("already superseded"))
            .count(),
        1
    );
    let units = read_units(p).unwrap();
    assert_eq!(units.len(), 2);
    let winner = units[0].superseded_by().unwrap();
    assert_eq!(fixed_unit(p, winner).unwrap().as_deref(), Some("origin"));
}

#[test]
fn invalid_existing_readiness_data_refuses_before_writing() {
    let (dir, brief) = setup();
    let p = dir.path();
    assert_eq!(add(p, "origin", &brief, None, false).exit_code, 0);
    let other = p.join("other.md");
    fs::write(&other, "GOAL g\nSCOPE src/b\nVERIFY echo ok\n").unwrap();
    assert_eq!(
        add(p, "other", other.to_str().unwrap(), None, false).exit_code,
        0
    );
    fs::write(&other, "GOAL invalid\n").unwrap();
    let before = snapshot(p);
    let out = add(p, "fix", &brief, Some("origin"), false);
    assert_ne!(out.exit_code, 0);
    assert_eq!(snapshot(p), before);
}
