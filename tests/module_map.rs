use axiom::orch::{
    lint_map, module_context, parse_map, run, MAX_CONTEXT_BYTES,
};
use std::path::Path;

fn section(id: &str, paths: &str, deps: &str, extra: &str, command: &str) -> String {
    format!("## {id}\nPURPOSE purpose\nPATHS {paths}\nDEPENDS {deps}\nCONTRACT contract\nACCEPT test, `{command}`\n{extra}\n")
}
fn call(p: &Path, args: &[&str], json: bool) -> axiom::orch::Output {
    run(
        p,
        &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        json,
        false,
    )
}
fn init(p: &Path) {
    assert_eq!(call(p, &["init"], false).exit_code, 0);
    std::fs::create_dir_all(p.join("astra")).unwrap();
}
fn write(p: &Path, name: &str, text: &str) {
    std::fs::write(p.join(name), text).unwrap();
}

#[test]
fn all_problems_in_one_refusal_without_writes() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path();
    init(p);
    let map = format!(
        "{}{}{}OUT OF SCOPE X reason\n",
        section(
            "a",
            "missing/",
            "b absent",
            "NEW_PATHS outside\nSHARED_OWNER ghost\nSERVES Z",
            "export X=1"
        ),
        section("b", "missing/file", "a", "", "ok && set X=1"),
        "## bad\nPATHS other\nDEPENDS none\n"
    );
    write(p, "modules.md", &map);
    write(p, "astra/plan-final.md", "### P Plan\n### Q Other\n");
    let before = std::fs::read_dir(p).unwrap().count();
    let o = call(p, &["map", "--repo", p.to_str().unwrap()], false);
    assert_eq!(o.exit_code, 1, "{}", o.stderr);
    assert!(o.stdout.is_empty());
    assert_eq!(before, std::fs::read_dir(p).unwrap().count());
    for kind in [
        "parse",
        "paths_unmatched",
        "new_paths",
        "depends_unknown",
        "depends_order",
        "depends_cycle",
        "paths_overlap",
        "shared_owner",
        "accept_env",
        "plan",
    ] {
        assert!(
            o.stderr.contains(&format!("{kind}:")),
            "missing {kind}: {}",
            o.stderr
        );
    }
    assert!(o.stderr.contains("a -> b -> a"));
    assert!(o.stderr.contains("plan item P"));
    assert!(o.stderr.contains("plan item Q"));
}

#[test]
fn valid_new_shared_and_environment() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path();
    init(p);
    let map = format!(
        "{}{}",
        section(
            "a",
            "src/",
            "none",
            "NEW_PATHS src/",
            "cargo test --test set_x"
        ),
        section(
            "b",
            "src/thing",
            "a",
            "NEW_PATHS src/thing\nSHARED_OWNER a",
            "cargo test"
        )
    );
    assert!(parse_map(&map).is_ok());
    assert!(
        lint_map(&map, Some(""), Some(p)).is_empty(),
        "{:?}",
        lint_map(&map, Some(""), Some(p))
    );
    write(p, "modules.md", &map);
    write(p, "astra/plan-final.md", "");
    let json = call(p, &["map"], true);
    let json: serde_json::Value = serde_json::from_str(&json.stdout).unwrap();
    for m in json["modules"].as_array().unwrap() {
        assert!(m.get("new_paths").is_none());
        assert!(m.get("shared_owner").is_none());
    }
    let o = call(p, &["map"], false);
    assert_eq!(o.exit_code, 0, "{}", o.stderr);
    assert!(o.stderr.contains("paths not checked: pass --repo <dir>"));
    assert_eq!(
        call(p, &["map", "--repo", p.to_str().unwrap()], false).exit_code,
        0
    );
    for command in [
        "set X=1",
        "export X=1",
        "echo hi && SET X=1",
        "echo %VAR%",
        "(export X=1",
    ] {
        let m = section("x", "x", "none", "", command);
        assert!(
            lint_map(&m, None, None)
                .iter()
                .any(|p| p.kind == "accept_env"),
            "{command}"
        );
    }
    for command in [
        "cargo test --test set_x",
        "echo set X=1",
        "echo export X=1",
        "echo 100% done 50%",
    ] {
        assert!(
            !lint_map(&section("x", "x", "none", "", command), None, None)
                .iter()
                .any(|p| p.kind == "accept_env")
        );
    }
}

#[test]
fn lint_checks_real_repo_paths_and_new_path_coverage() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path();
    std::fs::create_dir_all(p.join("src")).unwrap();
    std::fs::create_dir_all(p.join("sub")).unwrap();
    write(p, "src/x.rs", "");
    write(p, "sub/f", "");
    let map = section(
        "a",
        r"src/x.rs src/ src/*.rs .\sub\f ./sub/f missing/",
        "none",
        "NEW_PATHS missing/child",
        "test",
    );
    let unmatched: Vec<_> = lint_map(&map, None, Some(p))
        .into_iter()
        .filter(|problem| problem.kind == "paths_unmatched")
        .map(|problem| problem.message)
        .collect();
    assert_eq!(
        unmatched,
        [
            "PATHS src/*.rs does not exist",
            "PATHS missing/ does not exist"
        ]
    );
}

#[test]
fn lint_reports_each_cycle_and_overlap_despite_third_party_owner() {
    let map = format!(
        "{}{}{}",
        section("a", "shared/", "a b c", "SHARED_OWNER c", "test"),
        section("b", "shared/file", "a", "", "test"),
        section("c", "shared/", "a", "SHARED_OWNER c", "test")
    );
    let problems = lint_map(&map, None, None);
    let cycles: Vec<_> = problems
        .iter()
        .filter(|p| p.kind == "depends_cycle")
        .map(|p| p.message.as_str())
        .collect();
    assert_eq!(cycles, ["a -> a", "a -> b -> a", "a -> c -> a"]);
    let overlaps: Vec<_> = problems
        .iter()
        .filter(|p| p.kind == "paths_overlap")
        .map(|p| (p.module.as_deref(), p.message.as_str()))
        .collect();
    assert_eq!(overlaps, [(Some("b"), "PATHS overlap module a")]);
}

#[test]
fn outside_paths_are_unmatched_and_do_not_exist_in_context() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let repo = root.join("repo");
    std::fs::create_dir(&repo).unwrap();
    write(root, "outside", "");
    write(&repo, "inside", "");
    init(&repo);
    let absolute = root.join("outside").to_string_lossy().replace('\\', "/");
    let paths = format!("inside ../outside sub/../../outside {absolute}");
    let map = section("a", &paths, "none", "", "test");
    write(&repo, "modules.md", &map);
    let problems = lint_map(&map, None, Some(&repo));
    let unmatched: Vec<_> = problems
        .iter()
        .filter(|p| p.kind == "paths_unmatched")
        .map(|p| p.message.clone())
        .collect();
    assert_eq!(
        unmatched,
        [
            "PATHS ../outside does not exist".to_string(),
            "PATHS sub/../../outside does not exist".to_string(),
            format!("PATHS {absolute} does not exist")
        ]
    );
    let context = module_context(&repo, "a", &repo).unwrap();
    assert_eq!(
        context.paths.iter().map(|p| p.exists).collect::<Vec<_>>(),
        [true, false, false, false]
    );
}

#[test]
fn context_budget_marks_exact_cut_sources_and_keeps_headers() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path();
    init(p);
    std::fs::create_dir_all(p.join("src")).unwrap();
    let map = section(
        "a",
        "src/",
        "none",
        "NEW_PATHS src/new\nSERVES X",
        "cargo test",
    );
    write(p, "modules.md", &map);
    let args = ["module-context", "a", "--repo", p.to_str().unwrap()];
    let read_context = || {
        let output = call(p, &args, true);
        assert_eq!(output.exit_code, 0, "{}", output.stderr);
        assert!(output.stdout.len() <= MAX_CONTEXT_BYTES);
        let v: serde_json::Value = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(v["module"], "a");
        assert_eq!(v["purpose"], "purpose");
        assert_eq!(
            v["paths"],
            serde_json::json!([{"path":"src/", "exists":true}])
        );
        assert_eq!(v["depends"], serde_json::json!([]));
        assert_eq!(v["accept_file"], "test");
        assert_eq!(v["accept_command"], "cargo test");
        assert_eq!(v["new_paths"], serde_json::json!(["src/new"]));
        assert_eq!(v["shared_owner"], serde_json::Value::Null);
        assert_eq!(v["serves"], serde_json::json!(["X"]));
        v
    };
    let expected = |sections: &[(&str, &str)]| {
        serde_json::Value::Array(sections.iter().map(|(section, source)|
        serde_json::json!({"section": section, "source": source.split('/').fold(p.to_path_buf(), |path, part| path.join(part)).display().to_string()})).collect())
    };

    write(
        p,
        "astra/plan-final.md",
        &format!("### X Long\n{}\n", "long ".repeat(5000)),
    );
    write(p, "decisions.md", "- short decision\n");
    let v = read_context();
    assert_eq!(
        v["incomplete"],
        expected(&[("plan", "astra/plan-final.md")])
    );
    assert!(v["plan"].as_array().unwrap().is_empty());
    assert_eq!(v["decisions"].as_array().unwrap().len(), 1);

    write(p, "astra/plan-final.md", "### X Short\nshort\n");
    write(
        p,
        "decisions.md",
        &format!("- {}\n", "decision ".repeat(3000)),
    );
    let v = read_context();
    assert_eq!(v["incomplete"], expected(&[("decisions", "decisions.md")]));
    assert_eq!(v["plan"].as_array().unwrap().len(), 1);

    let mut rows = String::from("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n");
    for i in 0..50 {
        rows.push_str(&format!(
            "unit-{i:04}-{}\ta\tpending\t\t\t\t\n",
            "x".repeat(45)
        ));
    }
    write(p, "units.tsv", &rows);
    write(
        p,
        "astra/plan-final.md",
        &format!("### X Long\n{}\n", "long ".repeat(5000)),
    );
    write(
        p,
        "decisions.md",
        &format!(
            "{}",
            "- a decision with enough detail to consume space\n".repeat(1000)
        ),
    );
    let v = read_context();
    assert_eq!(
        v["incomplete"],
        expected(&[
            ("plan", "astra/plan-final.md"),
            ("decisions", "decisions.md")
        ])
    );
    assert_eq!(v["units"].as_array().unwrap().len(), 50);
    assert!(v["decisions"].as_array().unwrap().len() < 1000);
}

#[test]
fn context_selection_bounds_and_missing() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path();
    init(p);
    // Checkout may already use CRLF; normalize before constructing CRLF inputs.
    let crlf = |text: &str| text.replace("\r\n", "\n").replace('\n', "\r\n");
    let map = crlf(include_str!("fixtures/module_map/modules.md"));
    let plan = crlf(include_str!("fixtures/module_map/plan.md"));
    let decisions = crlf(include_str!("fixtures/module_map/decisions.md"));
    write(p, "modules.md", &map);
    write(p, "astra/plan-final.md", &plan);
    write(p, "decisions.md", &decisions);
    let args = ["module-context", "a", "--repo", p.to_str().unwrap()];
    for json in [false, true] {
        let o = call(p, &args, json);
        assert_eq!(o.exit_code, 0, "{}", o.stderr);
        assert!(o.stdout.contains("alpha contract 中文"));
        assert!(o.stdout.contains("global decision"));
        assert!(!o.stdout.contains("beta secret"));
        assert!(!o.stdout.contains("Beta"));
        assert!(!o.stdout.contains("ab secret"));
        assert!(!o.stdout.contains("src/c/file"));
        assert!(!o.stdout.contains('\r'));
        if json {
            let v: serde_json::Value = serde_json::from_str(&o.stdout).unwrap();
            assert_eq!(v["units"].as_array().unwrap().len(), 0);
        }
    }
    let trailing = call(
        p,
        &[
            "module-context",
            "a",
            "--repo",
            p.to_str().unwrap(),
            "--json",
        ],
        false,
    );
    assert!(serde_json::from_str::<serde_json::Value>(&trailing.stdout).is_ok());
    assert!(trailing.stdout.len() <= MAX_CONTEXT_BYTES);
    let long = map.replace(
        "  alpha contract 中文",
        &format!("  {}", "中文".repeat(12000)),
    );
    write(p, "modules.md", &long);
    for json in [false, true] {
        let o = call(p, &args, json);
        assert_eq!(o.exit_code, 0, "{}", o.stderr);
        assert!(o.stdout.len() <= MAX_CONTEXT_BYTES, "{}", o.stdout.len());
        assert!(o.stdout.contains("modules.md"));
        assert!(o.stdout.to_lowercase().contains("incomplete"));
    }
    write(p, "modules.md", &map);
    std::fs::remove_file(p.join("astra/plan-final.md")).unwrap();
    std::fs::remove_file(p.join("decisions.md")).unwrap();
    let c = module_context(p, "a", p).unwrap();
    assert_eq!(c.missing.len(), 2);
    assert!(c.missing.iter().any(|s| s.ends_with("plan-final.md")));
    assert!(c.missing.iter().any(|s| s.ends_with("decisions.md")));
    let o = call(
        p,
        &["module-context", "unknown", "--repo", p.to_str().unwrap()],
        false,
    );
    assert_eq!(o.exit_code, 2);
    assert!(o.stderr.contains("modules.md"));
    assert_eq!(call(p, &["map"], false).exit_code, 0);
    std::fs::write(p.join("astra/plan-final.md"), [0xff]).unwrap();
    let o = call(p, &["map"], false);
    assert_eq!(o.exit_code, 1);
    assert!(o.stderr.contains("plan-final.md"));
}

#[test]
fn context_units_and_entry_boundaries() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path();
    init(p);
    write(
        p,
        "modules.md",
        include_str!("fixtures/module_map/modules.md"),
    );
    write(p, "astra/plan-final.md", "### X Unrelated\nsrc/ab/file\n### Y Related\nsrc/a/file\n## Coverage\n### Z Hidden\nsrc/a/file\n");
    write(p, "decisions.md", "- general choice\n  - src/ab/file secret continuation\n- src/a/file selected\n1. ab secret\nplain global\n");
    assert_eq!(
        call(p, &["unit", "add", "u-a", "--track", "a"], false).exit_code,
        0
    );
    assert_eq!(
        call(p, &["unit", "add", "u-ab", "--track", "ab"], false).exit_code,
        0
    );
    let c = module_context(p, "a", p).unwrap();
    assert_eq!(c.units.len(), 1);
    assert_eq!(c.units[0].unit, "u-a");
    assert_eq!(c.units[0].state, "pending");
    assert_eq!(c.units[0].next, "blocked");
    assert_eq!(
        c.plan.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(),
        ["Y"]
    );
    assert_eq!(c.decisions, ["- src/a/file selected", "plain global"]);
    std::fs::remove_file(p.join("modules.md")).unwrap();
    let o = call(
        p,
        &["module-context", "a", "--repo", p.to_str().unwrap()],
        false,
    );
    assert_eq!(o.exit_code, 1);
    assert!(o.stderr.contains("modules.md"));
    let o = call(
        p,
        &[
            "module-context",
            "a",
            "--repo",
            p.join("absent").to_str().unwrap(),
        ],
        false,
    );
    assert_eq!(o.exit_code, 1);
    assert!(o.stderr.contains("--repo"));
}

#[test]
fn long_header_that_fits_is_not_charged_for_unused_markers() {
    let t = tempfile::Builder::new()
        .prefix(&"d".repeat(110))
        .tempdir()
        .unwrap();
    let p = t.path();
    init(p);
    let write_purpose = |purpose: &str| {
        write(
            p,
            "modules.md",
            &section("m", "p", "none", "", "test")
                .replace("PURPOSE purpose", &format!("PURPOSE {purpose}")),
        );
    };
    write_purpose("x");
    let short = module_context(p, "m", p).unwrap();
    let overhead = serde_json::to_string_pretty(&short).unwrap().len() + 1 - short.purpose.len();
    let purpose = "x".repeat(MAX_CONTEXT_BYTES - overhead);
    write_purpose(&purpose);
    let c = module_context(p, "m", p).unwrap();
    assert_eq!(c.purpose, purpose);
    assert!(c.incomplete.is_empty());
    assert_eq!(
        serde_json::to_string_pretty(&c).unwrap().len() + 1,
        MAX_CONTEXT_BYTES
    );

    write_purpose(&format!("{purpose}x"));
    let error = module_context(p, "m", p).unwrap_err();
    assert!(error.message.contains("exceeds byte limit"), "{error:?}");
}

#[test]
fn command_list_keeps_legacy_prefix_and_documents_new_options() {
    let t = tempfile::tempdir().unwrap();
    let output = call(t.path(), &["help"], false);
    assert!(output
        .stderr
        .starts_with("error: unknown command help; commands: init, map, status,"));
    assert!(output.stderr.contains("map [--repo <dir>]"));
    assert!(output
        .stderr
        .contains("module-context <module> --repo <dir>"));
}

#[test]
fn directory_without_slash_is_a_plan_match_but_not_a_decision_path_tag() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path();
    init(p);
    let map = format!(
        "{}{}",
        section("alpha", "src/a/", "none", "", "test"),
        section("beta", "src/b/", "none", "", "test")
    );
    write(p, "modules.md", &map);
    write(
        p,
        "astra/plan-final.md",
        "### X Selected\nsrc/a\n### Y Other\nsrc/ab\n",
    );
    write(p, "decisions.md", "- src/a\n- src/a/file\n");
    let alpha = module_context(p, "alpha", p).unwrap();
    assert_eq!(
        alpha.plan.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        ["X"]
    );
    let beta = module_context(p, "beta", p).unwrap();
    assert_eq!(beta.decisions, ["- src/a"]);
}
