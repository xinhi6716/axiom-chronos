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
    fn collect_receipts(dir: &Path, store: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect_receipts(&path, store, files);
            } else {
                files.insert(
                    path.strip_prefix(store).unwrap().to_string_lossy().replace('\\', "/"),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    files.insert(
        "units.tsv".into(),
        fs::read(store.join("units.tsv")).unwrap(),
    );
    for entry in fs::read_dir(store).unwrap() {
        let entry = entry.unwrap();
        if entry.path().is_file() && entry.path().extension().is_some_and(|ext| ext == "md") {
            files.insert(
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path()).unwrap(),
            );
        }
    }
    if store.join("receipts").exists() {
        collect_receipts(&store.join("receipts"), store, &mut files);
    }
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
    let routing = store.join("routing");
    if routing.exists() {
        for entry in fs::read_dir(routing).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                files.insert(
                    format!("routing/{}", entry.file_name().to_string_lossy()),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    files
}

#[test]
#[cfg(debug_assertions)]
fn final_brief_write_failure_rolls_back_all_files() {
    for existing in [false, true] {
        let (dir, brief) = setup();
        let p = dir.path();
        assert_eq!(add(p, "origin", &brief, None, false).exit_code, 0);
        let evidence_path = p.join("reports/fix.evidence.tsv");
        let fix_path = p.join("routing/fix.fix.json");
        if existing {
            append_evidence(p, "fix", &Evidence {
                kind: "verify".into(),
                command: "echo previous".into(),
                exit_code: 0,
                commit: String::new(),
                ts: "2026-01-01T00:00:00Z".into(),
            }).unwrap();
            fs::create_dir_all(fix_path.parent().unwrap()).unwrap();
            fs::write(&fix_path, b"previous fix record\n").unwrap();
        }
        fs::create_dir_all(p.join("receipts/nested")).unwrap();
        fs::write(p.join("receipts/nested/sentinel.json"), b"{}\n").unwrap();
        let before = snapshot(p);
        let original_evidence = fs::read(&evidence_path).ok();
        let original_fix = fs::read(&fix_path).ok();
        let original_brief = fs::read(&brief).unwrap();

        // A child-local hook fails only after units, evidence and routing were written.
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_axiom"))
            .args(["orch", "unit", "add", "fix", "--track", "core", "--brief", &brief,
                "--fixes", "origin"])
            .env("ORCH_STORE", p)
            .env("AXIOM_TEST_FAIL_FIX_BRIEF_WRITE", "1")
            .output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{stderr}");
        assert!(stderr.contains("injected fix brief write failure"), "{stderr}");
        assert_eq!(fs::read(p.join("units.tsv")).unwrap(), before["units.tsv"]);
        assert_eq!(fs::read(&evidence_path).ok(), original_evidence);
        assert_eq!(fs::read(&fix_path).ok(), original_fix);
        assert_eq!(fs::read(&brief).unwrap(), original_brief);
        assert_eq!(snapshot(p), before, "existing artifacts: {existing}");
    }
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

#[test]
fn legacy_parent_defaults() {
    let (dir, brief) = setup();
    let p = dir.path();
    assert_eq!(add(p, "origin", &brief, None, false).exit_code, 0);
    let out = add(p, "fix", &brief, Some("origin"), true);
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    let value: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(value["routing"]["tier"], "sol");
    assert_eq!(value["routing"]["effort"], "high");
}

#[test]
fn ladder_every_step() {
    use axiom::codex::routing::{write_snapshot, Effort, Risk, RoutingSnapshot, Rule, Tier};
    let cases = [
        (Tier::Terra, Some(Effort::Medium), "sol", "high"),
        (Tier::Luna, None, "sol", "high"),
        (Tier::Sol, Some(Effort::Low), "sol", "high"),
        (Tier::Sol, Some(Effort::Medium), "sol", "high"),
        (Tier::Sol, Some(Effort::High), "sol", "xhigh"),
        (Tier::Sol, Some(Effort::Xhigh), "astra", "xhigh"),
        (Tier::Sol, Some(Effort::Max), "astra", "max"),
        (Tier::Astra, Some(Effort::High), "astra", "xhigh"),
        (Tier::Astra, Some(Effort::Xhigh), "astra", "max"),
    ];
    for (tier, effort, expected_tier, expected_effort) in cases {
        let (dir, brief) = setup();
        let p = dir.path();
        assert_eq!(add(p, "origin", &brief, None, false).exit_code, 0);
        write_snapshot(
            p,
            &RoutingSnapshot {
                unit: "origin".into(),
                attempt_id: "a".into(),
                role: "worker".into(),
                tier: Some(tier),
                effort,
                risk: Some(Risk::Standard),
                model: String::new(),
                rule: Rule::Brief,
                reason: String::new(),
                settings_version: None,
            },
        )
        .unwrap();
        let out = add(p, "fix", &brief, Some("origin"), true);
        assert_eq!(out.exit_code, 0, "{}", out.stderr);
        let value: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(
            value["routing"]["tier"], expected_tier,
            "{tier:?}/{effort:?}"
        );
        assert_eq!(
            value["routing"]["effort"], expected_effort,
            "{tier:?}/{effort:?}"
        );
    }
}

#[test]
fn escalates_sol_high_to_xhigh() {
    use axiom::codex::routing::{
        read_fix, write_snapshot, Effort, Risk, RoutingSnapshot, Rule, Tier,
    };
    let (dir, brief) = setup();
    let p = dir.path();
    assert_eq!(add(p, "origin", &brief, None, false).exit_code, 0);
    write_snapshot(
        p,
        &RoutingSnapshot {
            unit: "origin".into(),
            attempt_id: "a".into(),
            role: "worker".into(),
            tier: Some(Tier::Sol),
            effort: Some(Effort::High),
            risk: Some(Risk::Standard),
            model: String::new(),
            rule: Rule::Brief,
            reason: String::new(),
            settings_version: None,
        },
    )
    .unwrap();
    let out = add(p, "fix", &brief, Some("origin"), true);
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["routing"]["tier"], "sol");
    assert_eq!(v["routing"]["effort"], "xhigh");
    assert_eq!(v["routing"]["rule"], "fix_escalation");
    let record = read_fix(p, "fix").unwrap().unwrap();
    assert_eq!(record.parent_tier, Tier::Sol);
    assert_eq!(record.parent_effort, Some(Effort::High));
    assert!(fs::read_to_string(&brief)
        .unwrap()
        .starts_with("GOAL g\nTIER sol\nEFFORT xhigh\n"));
}

#[test]
fn exception_needs_both_flags() {
    let (dir, brief) = setup();
    let p = dir.path();
    assert_eq!(add(p, "origin", &brief, None, false).exit_code, 0);
    let before = snapshot(p);
    for extra in [
        vec!["--routing", "sol/high"],
        vec!["--reason", "x"],
        vec!["--routing", "sol/high", "--reason", " "],
        vec!["--routing", "sol/ultra", "--reason", "x"],
    ] {
        let mut args = vec![
            "unit", "add", "fix", "--track", "core", "--brief", &brief, "--fixes", "origin",
        ];
        args.extend(extra);
        let out = call(p, &args, false);
        assert!(out.stderr.contains("invalid_argument"), "{}", out.stderr);
        assert_eq!(snapshot(p), before);
        assert!(!p.join("routing/fix.fix.json").exists());
    }
    let out = call(
        p,
        &[
            "unit",
            "add",
            "other",
            "--track",
            "core",
            "--brief",
            &brief,
            "--routing",
            "sol/high",
            "--reason",
            "x",
        ],
        false,
    );
    assert!(out.stderr.contains("invalid_argument"), "{}", out.stderr);
    assert_eq!(snapshot(p), before);
}

use axiom::codex::routing::{self, Effort, Risk, RoutingSnapshot, Rule, Tier};
fn parent(p: &Path, tier: Tier, effort: Effort, risk: Risk) {
    routing::write_snapshot(
        p,
        &RoutingSnapshot {
            unit: "origin".into(),
            attempt_id: "a".into(),
            role: "worker".into(),
            tier: Some(tier),
            effort: Some(effort),
            risk: Some(risk),
            model: String::new(),
            rule: Rule::Brief,
            reason: String::new(),
            settings_version: None,
        },
    )
    .unwrap();
}
fn exception(p: &Path, brief: &str, route: &str, reason: &str) -> Output {
    call(
        p,
        &[
            "unit",
            "add",
            "fix",
            "--track",
            "core",
            "--brief",
            brief,
            "--fixes",
            "origin",
            "--routing",
            route,
            "--reason",
            reason,
        ],
        true,
    )
}
#[test]
fn escalates_sol_xhigh_to_astra() {
    let (d, b) = setup();
    let p = d.path();
    assert_eq!(add(p, "origin", &b, None, false).exit_code, 0);
    parent(p, Tier::Sol, Effort::Xhigh, Risk::Standard);
    let o = add(p, "fix", &b, Some("origin"), true);
    assert_eq!(o.exit_code, 0, "{}", o.stderr);
    let v: serde_json::Value = serde_json::from_str(&o.stdout).unwrap();
    assert_eq!(v["routing"]["tier"], "astra");
    assert_eq!(v["routing"]["effort"], "xhigh");
    assert!(fs::read_to_string(b)
        .unwrap()
        .starts_with("GOAL g\nTIER astra\nEFFORT xhigh\n"));
}
#[test]
fn parent_from_map_when_no_snapshot() {
    for (lines, snap, expected) in [
        (
            "  default sol/high standard reason\n  origin terra/medium trivial reason",
            false,
            "high",
        ),
        ("  default sol/high standard reason", false, "xhigh"),
        ("  default sol/high standard reason", true, "high"),
    ] {
        let (d, b) = setup();
        let p = d.path();
        assert_eq!(add(p, "origin", &b, None, false).exit_code, 0);
        fs::write(p.join("modules.md"),format!("## core\nPURPOSE x\nPATHS src/a\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\nROUTING\n{lines}\n")).unwrap();
        if snap {
            parent(p, Tier::Terra, Effort::Medium, Risk::Trivial);
        }
        let o = add(p, "fix", &b, Some("origin"), true);
        assert_eq!(o.exit_code, 0, "{}", o.stderr);
        let v: serde_json::Value = serde_json::from_str(&o.stdout).unwrap();
        assert_eq!(v["routing"]["tier"], "sol");
        assert_eq!(v["routing"]["effort"], expected);
    }
}
#[test]
fn explicit_exception() {
    let (d, b) = setup();
    let p = d.path();
    assert_eq!(add(p, "origin", &b, None, false).exit_code, 0);
    parent(p, Tier::Sol, Effort::Xhigh, Risk::Standard);
    let o = exception(p, &b, "astra/high", "needs astra");
    assert_eq!(o.exit_code, 0, "{}", o.stderr);
    let v: serde_json::Value = serde_json::from_str(&o.stdout).unwrap();
    assert_eq!(v["routing"]["rule"], "plan_exception");
    assert_eq!(v["routing"]["reason"], "needs astra");
    assert_eq!(v["routing"]["tier"], "astra");
    assert_eq!(v["routing"]["effort"], "high");
    let r = routing::read_fix(p, "fix").unwrap().unwrap();
    assert_eq!(
        (r.tier, r.effort, r.rule, r.reason.as_str()),
        (
            Tier::Astra,
            Effort::High,
            Rule::PlanException,
            "needs astra"
        )
    );
    assert!(fs::read_to_string(b)
        .unwrap()
        .starts_with("GOAL g\nTIER astra\nEFFORT high\n"));
}
#[test]
fn exception_rubric_violation() {
    let (d, b) = setup();
    let p = d.path();
    assert_eq!(add(p, "origin", &b, None, false).exit_code, 0);
    parent(p, Tier::Sol, Effort::Xhigh, Risk::Parser);
    let before = snapshot(p);
    let o = exception(p, &b, "terra/medium", "x");
    assert!(o.stderr.contains("routing_rubric"), "{}", o.stderr);
    assert_eq!(snapshot(p), before);
}
#[test]
fn saturated_parent() {
    let (d, b) = setup();
    let p = d.path();
    assert_eq!(add(p, "origin", &b, None, false).exit_code, 0);
    parent(p, Tier::Astra, Effort::Max, Risk::Standard);
    let before = snapshot(p);
    let o = add(p, "fix", &b, Some("origin"), false);
    assert!(o.stderr.contains("escalation_saturated"), "{}", o.stderr);
    assert_eq!(snapshot(p), before);
    assert!(routing::read_fix(p, "fix").unwrap().is_none());
    let o = exception(p, &b, "astra/max", "x");
    assert_eq!(o.exit_code, 0, "{}", o.stderr);
}
#[test]
fn brief_line_mismatch() {
    let (d, b) = setup();
    let p = d.path();
    assert_eq!(add(p, "origin", &b, None, false).exit_code, 0);
    parent(p, Tier::Sol, Effort::Xhigh, Risk::Standard);
    fs::write(&b, "GOAL g\nTIER sol\nSCOPE src/a\nVERIFY echo ok\n").unwrap();
    let before = snapshot(p);
    let o = add(p, "fix", &b, Some("origin"), false);
    assert!(o.stderr.contains("routing_mismatch"), "{}", o.stderr);
    assert_eq!(snapshot(p), before);
    fs::write(
        &b,
        "GOAL g\nTIER astra\nEFFORT xhigh\nSCOPE src/a\nVERIFY echo ok\n",
    )
    .unwrap();
    let o = add(p, "fix", &b, Some("origin"), false);
    assert_eq!(o.exit_code, 0, "{}", o.stderr);
    let t = fs::read_to_string(b).unwrap();
    assert_eq!(t.matches("TIER ").count(), 1);
    assert_eq!(t.matches("EFFORT ").count(), 1);
}
#[test]
fn duplicate_add_writes_no_record() {
    let (d, b) = setup();
    let p = d.path();
    assert_eq!(add(p, "origin", &b, None, false).exit_code, 0);
    let before = snapshot(p);
    assert_ne!(add(p, "origin", &b, Some("origin"), false).exit_code, 0);
    assert_eq!(snapshot(p), before);
    assert!(routing::read_fix(p, "origin").unwrap().is_none());
}
#[test]
fn concurrent_adds_one_record() {
    let (d, b) = setup();
    let p = d.path();
    assert_eq!(add(p, "origin", &b, None, false).exit_code, 0);
    let r = std::thread::scope(|s| {
        let a = s.spawn(|| add(p, "one", &b, Some("origin"), false));
        let c = s.spawn(|| add(p, "two", &b, Some("origin"), false));
        [a.join().unwrap(), c.join().unwrap()]
    });
    assert_eq!(r.iter().filter(|o| o.exit_code == 0).count(), 1);
    assert_eq!(
        ["one", "two"]
            .iter()
            .filter(|id| routing::read_fix(p, id).unwrap().is_some())
            .count(),
        1
    );
}
#[test]
fn third_round_unchanged() {
    let (d, b) = setup();
    let p = d.path();
    assert_eq!(add(p, "origin", &b, None, false).exit_code, 0);
    assert_eq!(add(p, "one", &b, Some("origin"), false).exit_code, 0);
    assert_eq!(add(p, "two", &b, Some("one"), false).exit_code, 0);
    let before = snapshot(p);
    let o = add(p, "three", &b, Some("two"), false);
    assert!(o.stderr.contains("fix_rounds"), "{}", o.stderr);
    assert_eq!(snapshot(p), before);
    assert!(routing::read_fix(p, "three").unwrap().is_none());
    assert_eq!(fixed_unit(p, "two").unwrap().as_deref(), Some("one"));
}

mod common;
#[test]
fn fix_dispatch_applies_model_and_effort() {
    use std::process::Command;
    let d = tempfile::tempdir().unwrap();
    let repo = d.path().join("repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(repo.join("src/mod.py"), "fixture").unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["add", "src/mod.py"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            "fixture",
        ],
    ] {
        assert!(Command::new("git")
            .args(args)
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
    }
    let p = axiom::orch::store_for(d.path(), "demo").unwrap();
    fs::create_dir_all(&p).unwrap();
    ok(&p, &["init"], false);
    let b = p.join("brief.md");
    fs::write(&b, "GOAL g\nSCOPE src/mod.py\nVERIFY echo ok\n").unwrap();
    let b = b.to_str().unwrap();
    fs::write(
        p.join("modules.md"),
        "## core\nPURPOSE x\nPATHS src/mod.py\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
    )
    .unwrap();
    assert_eq!(add(&p, "origin", b, None, false).exit_code, 0);
    parent(&p, Tier::Sol, Effort::High, Risk::Standard);
    let o = add(&p, "fix", b, Some("origin"), false);
    assert_eq!(o.exit_code, 0, "{}", o.stderr);
    let log = d.path().join("codex.log");
    let wrapper = d.path().join("fake.py");
    fs::write(&wrapper,"import os,runpy,sys\nos.environ['FAKE_CODEX_LOG']=sys.argv[1]\nos.environ['FAKE_CODEX_MODELS']='gpt-6.1-sol,gpt-6-astra'\nsys.argv=sys.argv[2:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n").unwrap();
    let mut cfg = axiom::bridge::Config::from_env();
    cfg.data_home = Some(d.path().into());
    cfg.codex = Ok(vec![
        common::python(),
        wrapper.to_string_lossy().into_owned(),
        log.to_string_lossy().into_owned(),
        common::repo_root()
            .join("tests/fakes/fake_codex.py")
            .to_string_lossy()
            .into_owned(),
    ]);
    cfg.wsl = None;
    cfg.on_windows = false;
    let bridge = axiom::bridge::Bridge::new(cfg);
    let result = axiom::bridge::call(
        &bridge,
        "codex_run",
        &serde_json::json!({"project":"demo","unit":"fix","role":"terra","cwd":repo}),
    )["structuredContent"]
        .clone();
    assert_eq!(result["status"], "completed", "{result}");
    let entries: Vec<serde_json::Value> = fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(
        entries
            .iter()
            .find(|v| v["method"] == "thread/start")
            .unwrap()["params"]["model"],
        "gpt-6.1-sol"
    );
    assert_eq!(
        entries
            .iter()
            .find(|v| v["method"] == "turn/start")
            .unwrap()["params"]["effort"],
        "xhigh"
    );
}
