use axiom::orch::{covers, overlaps, parse_brief, parse_map};

fn section(id: &str, paths: &str, deps: &str, accept: &str) -> String {
    format!("## {id}\nPURPOSE purpose\nPATHS {paths}\nDEPENDS {deps}\nCONTRACT\n  contract\nACCEPT {accept}\n")
}

#[test]
fn brief_fields_and_crlf() {
    let text = "VERIFY ignored\r\nSCOPE ignored\r\nGOAL first\r\n  second\r\nSCOPE src/a/, tests/a.rs\r\n  src/b\r\nVERIFY cargo test\r\nCONTEXT 背景\r\n";
    let b = parse_brief(text).unwrap();
    assert_eq!(b.goal, "first\nsecond");
    assert_eq!(b.scope, ["src/a/", "tests/a.rs", "src/b"]);
    assert_eq!(b.verify, "cargo test");
    assert_eq!(b.context.as_deref(), Some("背景"));
    assert_eq!(b.text, text);
    assert!(parse_brief("GOAL x\nSCOPE x\n")
        .unwrap_err()
        .contains("VERIFY"));
    assert!(parse_brief("GOAL x\nSCOPE ,,,\nVERIFY x\n")
        .unwrap_err()
        .contains("SCOPE"));
}

#[test]
fn delivery_declaration_is_parseable_and_history_is_readable() {
    let legacy = "GOAL x\nSCOPE x\nVERIFY check\n";
    assert_eq!(parse_brief(legacy).unwrap().delivery, None);
    assert_eq!(parse_brief(&format!("DELIVERY invalid prose\n{legacy}")).unwrap_err(),"DELIVERY");
    assert_eq!(parse_brief(&format!("OUTPUTS src/main.rs\n{legacy}")).unwrap_err(),"OUTPUTS");
    for kind in ["implementation", "verification", "measurement"] {
        let text = format!("{legacy}DELIVERY {kind}\nCONTEXT note\n");
        let brief = parse_brief(&text).unwrap();
        assert_eq!(brief.delivery.as_deref(), Some(kind));
        assert_eq!(brief.text, text);
        assert_eq!(brief.verify, "check");
    }
    for suffix in ["DELIVERY\n", "DELIVERY unknown\n", "DELIVERY implementation\nDELIVERY verification\n"] {
        assert_eq!(parse_brief(&format!("{legacy}{suffix}")).unwrap_err(), "DELIVERY");
    }
}

#[test]
fn explicit_outputs_are_literal_scoped_and_normalized() {
    let base = "GOAL x\nSCOPE src/ tests/\nVERIFY check\nDELIVERY implementation\n";
    assert_eq!(parse_brief(base).unwrap().outputs, None);
    assert_eq!(parse_brief(&format!("{base}OUTPUTS ./src/a.rs, tests/\n  src/b.rs\n")).unwrap().outputs.unwrap(), ["src/a.rs", "tests/", "src/b.rs"]);
    for value in ["none", "none src/a", "", ".", "../outside", "src/../outside", "/absolute", "C:\\outside", "//host/share", "src/*.rs", "src/?.rs", "src/[ab].rs", "src/a.rs ./src/a.rs", "src/a src/a/", "other/file", "target/"] {
        assert_eq!(parse_brief(&format!("{base}OUTPUTS {value}\n")).unwrap_err(), "OUTPUTS", "{value}");
    }
    assert_eq!(parse_brief(&format!("{base}OUTPUTS src/a\nOUTPUTS src/b\n")).unwrap_err(), "OUTPUTS");
    for kind in ["verification", "measurement"] {
        let brief = parse_brief(&format!("GOAL x\nSCOPE src/\nVERIFY check\nDELIVERY {kind}\nOUTPUTS none\n")).unwrap();
        assert_eq!(brief.outputs, Some(vec![]));
    }
    assert!(parse_brief("GOAL x\nSCOPE src/\nVERIFY check\nOUTPUTS none\n").is_err());
    assert_eq!(parse_brief("GOAL x\nSCOPE .\nVERIFY check\nDELIVERY implementation\nOUTPUTS target/artifact\n").unwrap().outputs.unwrap(), ["target/artifact"]);
    #[cfg(windows)]
    assert!(parse_brief(&format!("{base}OUTPUTS src/A.rs src/a.rs\n")).is_err());
    let legacy = serde_json::json!({"goal":"x","scope":["src/"],"verify":"check","context":null,"text":"legacy"});
    let decoded: axiom::orch::Brief = serde_json::from_value(legacy).unwrap();
    assert_eq!(decoded.outputs, None);
}

#[test]
fn map_accept_and_full_test() {
    let input = format!(
        "global\n{}{}FULL TEST `cargo test --all`\n",
        section("a", "src/a/", "none", "tests/a.rs, `cargo test --test a`"),
        section("b", "src/b/", "a", "tests/b.rs, cargo test --test b")
    );
    let map = parse_map(&input.replace('\n', "\r\n")).unwrap();
    assert_eq!(map.preamble, "global\n");
    assert_eq!(map.modules[0].accept_file, "tests/a.rs");
    assert_eq!(map.modules[0].accept_command, "cargo test --test a");
    assert_eq!(map.modules[1].accept_command, "cargo test --test b");
    assert_eq!(map.full_test.as_deref(), Some("cargo test --all"));
    let plain = format!(
        "{}FULL TEST cargo test\n",
        section("a", "a", "none", "a, test")
    );
    assert_eq!(
        parse_map(&plain).unwrap().full_test.as_deref(),
        Some("cargo test")
    );
}

#[test]
fn map_rejections_name_module() {
    let a = section("a", "src/a/", "none", "a, test");
    for (input, reason) in [
        (
            section("missing", "m", "none", ""),
            "module missing: missing ACCEPT",
        ),
        (
            section("unknown", "u", "absent", "u, test"),
            "module unknown: unknown or later dependency absent",
        ),
        (
            format!(
                "{}{}",
                section("cycle_a", "a", "cycle_b", "a, test"),
                section("cycle_b", "b", "cycle_a", "b, test")
            ),
            "module cycle_a: unknown or later dependency cycle_b",
        ),
        (
            format!("{a}{}", section("overlap", "./src/a/file", "a", "x, test")),
            "module overlap: PATHS overlap module a",
        ),
        (
            format!("{a}{}", section("a", "b", "none", "b, test")),
            "module a: duplicate id",
        ),
    ] {
        assert_eq!(parse_map(&input).unwrap_err(), reason);
    }
}

#[test]
fn fixture_map_in_dependency_order() {
    let map = parse_map(include_str!("../fixtures/orch/modules.md")).unwrap();
    for (i, module) in map.modules.iter().enumerate() {
        for dependency in &module.depends {
            assert!(map.modules[..i].iter().any(|m| &m.id == dependency));
        }
    }
    assert!(map.full_test.is_some());
}

#[test]
fn path_coverage_and_overlap() {
    let paths = vec![r".\src\a\".to_string(), "./tests/a.rs".to_string()];
    assert!(covers(&paths, "./src/a/file.rs"));
    assert!(covers(&paths, r"tests\a.rs"));
    assert!(!covers(&paths, "src/ab/file.rs"));
    assert!(overlaps(&paths, &[r"src\a\nested\".into()]));
    assert!(overlaps(&["./tests/a.rs".into()], &[r"tests\a.rs".into()]));
    assert!(!overlaps(&paths, &["src/b/".into()]));
}

#[test]
fn scope_rejects_traversal_and_platform_absolute_paths() {
    for path in [
        "../outside",
        "src/../../outside",
        "/absolute",
        "C:\\outside",
        "//host/share",
        "src//file",
    ] {
        assert!(
            parse_brief(&format!("GOAL x\nSCOPE {path}\nVERIFY test\n")).is_err(),
            "{path}"
        );
        assert!(!covers(&["src/".into()], path));
    }
    assert!(covers(&["./src/".into()], "src/file"));
    assert!(covers(&[".".into()], "src/file"));
}

#[test]
fn product_journal_wire_schema_and_cli_are_explicit() {
    use axiom::orch::{run, ProductRun, WaitState};
    use serde_json::json;
    let run_json = json!({"id":"product","rounds":[{"round":1,"event_start":0,"transition":null}]});
    let parsed: ProductRun = serde_json::from_value(run_json.clone()).unwrap();
    assert_eq!(parsed.rounds[0].round, 1);
    for bad in [
        json!({"rounds":[]}),
        json!({"id":"product","rounds":[],"task_round":5}),
        json!({"id":"product","rounds":[{"round":1,"event_start":0,"task":"unit","transition":null}]}),
    ] {
        assert!(serde_json::from_value::<ProductRun>(bad).is_err());
    }
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let start: Vec<String> = ["wait", "product-checkpoint", "--run", "product"]
        .map(str::to_owned)
        .into();
    let result = run(p, &start, true, false);
    assert_eq!(result.exit_code, 0, "{}", result.stderr);
    let state: WaitState = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(state.schema_version, 2);
    assert_eq!(
        serde_json::to_value(&state.product_runs[0]).unwrap(),
        run_json
    );
    assert!(state.checkpoints.is_empty());
    assert_ne!(run(p, &start, true, false).exit_code, 0);
    for round in ["0", "invalid", "4294967296"] {
        let args: Vec<String> = [
            "wait",
            "product-transition",
            "--run",
            "product",
            "--round",
            round,
            "--verifications",
            "unit-verification",
            "--assessment",
            "{}",
            "--action",
            "finish",
        ]
        .map(str::to_owned)
        .into();
        assert_ne!(run(p, &args, true, false).exit_code, 0);
    }
}
