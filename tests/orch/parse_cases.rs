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
    assert_eq!((b.effort, b.tier), (None, None));
    assert_eq!(
        parse_brief("GOAL x\nSCOPE x\n").unwrap_err(),
        "is missing VERIFY"
    );
    assert_eq!(
        parse_brief("GOAL x\nSCOPE ,,,\nVERIFY x\n").unwrap_err(),
        "is missing SCOPE"
    );
}

#[test]
fn brief_settings_are_read_and_kept_from_the_worker() {
    let text = "note\r\nGOAL g\r\nEFFORT  low \r\nSCOPE a\r\nTIER luna\r\nVERIFY v\r\nCONTEXT c\r\n";
    let b = parse_brief(text).unwrap();
    assert_eq!(b.effort.as_deref(), Some("low"));
    assert_eq!(b.tier.as_deref(), Some("luna"));
    assert_eq!(b.text, "note\r\nGOAL g\r\nSCOPE a\r\nVERIFY v\r\nCONTEXT c\r\n");
    assert_eq!((b.goal.as_str(), b.verify.as_str()), ("g", "v"));
    assert_eq!(b.scope, ["a"]);
    for effort in ["low", "medium", "high"] {
        let b = parse_brief(&format!("GOAL g\nSCOPE a\nVERIFY v\nEFFORT {effort}")).unwrap();
        assert_eq!(b.effort.as_deref(), Some(effort));
        assert_eq!(b.tier, None);
        assert_eq!(b.text, "GOAL g\nSCOPE a\nVERIFY v\n");
    }
}

#[test]
fn brief_settings_outside_the_allowed_set_are_refused() {
    let efforts = "as one of low, medium, high";
    let tiers = "as one of luna";
    for (line, reason) in [
        ("EFFORT xhigh", format!("has invalid EFFORT \"xhigh\": give it once, {efforts}")),
        ("EFFORT Low", format!("has invalid EFFORT \"Low\": give it once, {efforts}")),
        ("EFFORT low please", format!("has invalid EFFORT \"low please\": give it once, {efforts}")),
        ("EFFORT", format!("has invalid EFFORT \"\": give it once, {efforts}")),
        ("EFFORT low\nEFFORT high", format!("has invalid EFFORT \"high\": give it once, {efforts}")),
        ("TIER terra", format!("has invalid TIER \"terra\": give it once, {tiers}")),
        ("TIER sol", format!("has invalid TIER \"sol\": give it once, {tiers}")),
        ("TIER gpt-6-luna", format!("has invalid TIER \"gpt-6-luna\": give it once, {tiers}")),
        ("TIER", format!("has invalid TIER \"\": give it once, {tiers}")),
    ] {
        let err = parse_brief(&format!("GOAL g\nSCOPE a\nVERIFY v\n{line}\n")).unwrap_err();
        assert_eq!(err, reason, "{line}");
    }
}

#[test]
fn old_briefs_cannot_trip_the_settings() {
    for text in [
        "EFFORT was spent here\nTIER 1 sources\nGOAL g\nSCOPE a\nVERIFY v\n",
        "GOAL g\nSCOPE a\nVERIFY v\nCONTEXT\nEFFORT was spent here\nTIER 1 sources\nEFFORT low\n",
        "GOAL g\nSCOPE a\nVERIFY v\nCONTEXT\n  EFFORT was spent here\n",
        "GOAL g\n  EFFORT was spent here\nSCOPE a\nVERIFY v\n",
        "GOAL g\nEFFORTS low\nTIERED luna\nSCOPE a\nVERIFY v\n",
    ] {
        let b = parse_brief(text).unwrap();
        assert_eq!((b.effort, b.tier), (None, None), "{text}");
        assert_eq!(b.text, text);
    }
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
