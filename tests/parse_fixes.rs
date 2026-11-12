use axiom::orch::{parse_brief, parse_verdict};

fn verdict(finding: &str) -> String {
    format!(r#"{{"unit":"u","tree":"t","verdict":"fail","findings":[{finding}]}}"#)
}

#[test]
fn verdict_null_needs_matches_omitted() {
    let omitted = parse_verdict(&verdict(r#"{"severity":"major","check":"untested"}"#)).unwrap();
    let null = parse_verdict(&verdict(r#"{"severity":"major","check":"untested","needs":null}"#))
        .unwrap();
    assert_eq!(null.findings[0].needs, None);
    assert_eq!(null, omitted);
    assert_eq!(null.findings[0].needs, omitted.findings[0].needs);
    assert_eq!(null.findings[0].fails(), omitted.findings[0].fails());
    // Negative control: other values stay rejected.
    for bad in [r#""Smoke""#, "42", "true", r#""other""#, "[]"] {
        let error = parse_verdict(&verdict(&format!(r#"{{}},{{"needs":{bad}}}"#))).unwrap_err();
        assert_eq!(error.kind, "invalid_argument");
        assert_eq!(error.message, format!("finding 2: invalid needs {bad}"));
    }
}

#[test]
fn brief_multiline_scope_is_complete_or_explicitly_rejected() {
    // Unindented continuation lines.
    let b = parse_brief("GOAL g\nSCOPE src/a.rs, src/b.rs\nsrc/c.rs\ntests/d.rs\nVERIFY v\n").unwrap();
    assert_eq!(b.scope, ["src/a.rs", "src/b.rs", "src/c.rs", "tests/d.rs"]);
    // Bullet list, blank lines, CRLF, EFFORT and TIER end the section.
    let b = parse_brief(
        "GOAL g\r\nSCOPE\r\n- src/a.rs\r\n\r\n* src/b/\r\n+ tests/c.rs, tests/d.rs\r\nEFFORT low\r\nVERIFY v\r\nCONTEXT c\r\n",
    )
    .unwrap();
    assert_eq!(b.scope, ["src/a.rs", "src/b/", "tests/c.rs", "tests/d.rs"]);
    // One-line SCOPE is unchanged and the section stops at the next field.
    let b = parse_brief("GOAL g\nSCOPE a\nVERIFY v\nnot/a/scope\n").unwrap();
    assert_eq!(b.scope, ["a"]);
    // Every field ends the section, including CONTEXT and settings.
    for field in ["GOAL another", "VERIFY v", "CONTEXT c", "EFFORT low", "TIER luna"] {
        let text = format!("GOAL g\nSCOPE a\n{field}\nnot/a/scope\nVERIFY v\n");
        assert_eq!(parse_brief(&text).unwrap().scope, ["a"], "{field}");
    }
    // Duplicate SCOPE is rejected with both line numbers.
    assert_eq!(
        parse_brief("GOAL g\nSCOPE a\nVERIFY v\nSCOPE b\n").unwrap_err(),
        "has SCOPE twice: line 4 repeats line 2"
    );
    assert_eq!(
        parse_brief("note\nGOAL g\nSCOPE a\n- b\nSCOPE c\nVERIFY v\n").unwrap_err(),
        "has SCOPE twice: line 5 repeats line 3"
    );
    assert_eq!(
        parse_brief("GOAL g\nSCOPE a\nVERIFY v\nCONTEXT c\nSCOPE b\n").unwrap_err(),
        "has SCOPE twice: line 5 repeats line 2"
    );
    // Notes inside SCOPE are rejected, never silently kept or dropped.
    for (text, token, line) in [
        ("GOAL g\nSCOPE a.rs\n(new file)\nVERIFY v\n", "\"(new\"", 3),
        ("note\nGOAL g\nSCOPE a.rs\nfile)\nVERIFY v\n", "\"file)\"", 4),
        ("GOAL g\nSCOPE a.rs\n- `b.rs`\nVERIFY v\n", "\"`b.rs`\"", 3),
        ("GOAL g\nSCOPE a.rs\nNotes:\nVERIFY v\n", "\"Notes:\"", 3),
        ("GOAL g\nSCOPE \"a.rs\"\nVERIFY v\n", "\"\\\"a.rs\\\"\"", 2),
    ] {
        assert_eq!(
            parse_brief(text).unwrap_err(),
            format!("has SCOPE entry {token} on line {line} that is not a path; put notes in CONTEXT")
        );
    }
    assert_eq!(parse_brief("GOAL g\nSCOPE\nVERIFY v\n").unwrap_err(), "is missing SCOPE");
}
