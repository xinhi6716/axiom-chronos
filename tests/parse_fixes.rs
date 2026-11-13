use axiom::orch::{parse_brief, parse_verdict};

fn verdict(finding: &str) -> String {
    format!(r#"{{"unit":"u","tree":"t","verdict":"fail","findings":[{finding}]}}"#)
}

#[test]
fn verdict_null_needs_matches_omitted() {
    let omitted = parse_verdict(&verdict(r#"{"severity":"major","check":"untested"}"#)).unwrap();
    let null = parse_verdict(&verdict(
        r#"{"severity":"major","check":"untested","needs":null}"#,
    ))
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
    let b =
        parse_brief("GOAL g\nSCOPE src/a.rs, src/b.rs\nsrc/c.rs\ntests/d.rs\nVERIFY v\n").unwrap();
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
    for field in [
        "GOAL another",
        "VERIFY v",
        "CONTEXT c",
        "EFFORT low",
        "TIER luna",
    ] {
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
        (
            "note\nGOAL g\nSCOPE a.rs\nfile)\nVERIFY v\n",
            "\"file)\"",
            4,
        ),
        ("GOAL g\nSCOPE a.rs\n- `b.rs`\nVERIFY v\n", "\"`b.rs`\"", 3),
        ("GOAL g\nSCOPE a.rs\nNotes:\nVERIFY v\n", "\"Notes:\"", 3),
        ("GOAL g\nSCOPE \"a.rs\"\nVERIFY v\n", "\"\\\"a.rs\\\"\"", 2),
    ] {
        assert_eq!(
            parse_brief(text).unwrap_err(),
            format!(
                "has SCOPE entry {token} on line {line} that is not a path; put notes in CONTEXT"
            )
        );
    }
    assert_eq!(
        parse_brief("GOAL g\nSCOPE\nVERIFY v\n").unwrap_err(),
        "is missing SCOPE"
    );
}

fn brief(extra: &str) -> String {
    format!("GOAL g\nSCOPE tests/parse_fixes.rs\n{extra}VERIFY v\n")
}
fn row(id: &str, kind: &str, target: &str, assertion: &str) -> String {
    format!("{id} | {kind} | {target} | {assertion} | cargo test")
}
fn both(rows: &str) -> String {
    brief(&format!("CONTRACT first\n- second\nTEST_MATRIX\n{rows}\n"))
}
fn err(text: &str, expected: &str) {
    assert!(parse_brief(text).unwrap_err().contains(expected));
}

#[test]
fn matrix_parsed() {
    let b = parse_brief(&both("x1 | behaviour | tests/parse_fixes.rs::one | works | cargo test | more\nx2|edge|tests/parse_fixes.rs::two|ok|run")).unwrap();
    assert_eq!(b.contract, ["first", "second"]);
    assert_eq!(b.test_matrix.len(), 2);
    let a = &b.test_matrix[0];
    assert_eq!(
        (
            &*a.id,
            &*a.kind,
            &*a.file,
            &*a.case,
            &*a.assertion,
            &*a.command
        ),
        (
            "x1",
            "behaviour",
            "tests/parse_fixes.rs",
            "one",
            "works",
            "cargo test | more"
        )
    );
    let a = &b.test_matrix[1];
    assert_eq!(
        (
            &*a.id,
            &*a.kind,
            &*a.file,
            &*a.case,
            &*a.assertion,
            &*a.command
        ),
        ("x2", "edge", "tests/parse_fixes.rs", "two", "ok", "run")
    );
}
#[test]
fn sections_stay_in_worker_text() {
    let s = both("x | edge | tests/parse_fixes.rs::c | ok | run");
    let b = parse_brief(&s).unwrap();
    assert_eq!(b.text, s);
}
#[test]
fn no_sections_unchanged() {
    let s = brief("");
    let b = parse_brief(&s).unwrap();
    assert!(b.contract.is_empty());
    assert!(b.test_matrix.is_empty());
    assert_eq!(b.text, s);
}
#[test]
fn one_section_without_other() {
    err(&brief("CONTRACT x\n"), "has CONTRACT without TEST_MATRIX");
    err(&brief("TEST_MATRIX\n"), "has TEST_MATRIX without CONTRACT");
}
#[test]
fn incomplete_rows() {
    for r in [
        "x | edge | tests/parse_fixes.rs::c | ok",
        "x | edge | tests/parse_fixes.rs::c | | run",
        "x | edge | tests/parse_fixes.rs:: | ok | run",
    ] {
        err(&both(r), &format!("has an incomplete TEST_MATRIX row: {r}"));
    }
}
#[test]
fn bad_and_duplicate_ids() {
    for id in ["1x", "a b"] {
        err(
            &both(&row(id, "edge", "tests/parse_fixes.rs::c", "ok")),
            &format!("has an invalid TEST_MATRIX id {id}"),
        );
    }
    let r = row("x", "edge", "tests/parse_fixes.rs::c", "ok");
    err(
        &both(&format!("{r}\n{r}")),
        "has a duplicate TEST_MATRIX id x",
    );
}
#[test]
fn unknown_kind() {
    err(
        &both(&row("x", "happy", "tests/parse_fixes.rs::c", "ok")),
        "has an unknown TEST_MATRIX kind happy",
    );
}
#[test]
fn test_outside_scope() {
    let r = row("x", "edge", "other::c", "ok");
    err(&both(&r), "has a TEST_MATRIX test outside SCOPE: other");
    assert!(parse_brief(&format!(
        "GOAL g\nSCOPE tests/parse_fixes.rs other\nCONTRACT x\nTEST_MATRIX\n{r}\nVERIFY v\n"
    ))
    .is_ok());
}
#[test]
fn duplicate_section() {
    err(
        &brief("CONTRACT x\nTEST_MATRIX\nTEST_MATRIX\n"),
        "has TEST_MATRIX twice",
    );
}
#[test]
fn sections_in_context_ignored() {
    let b = parse_brief(&brief(
        "CONTEXT pasted\nCONTRACT\nTEST_MATRIX\nx | edge | f::c | a | run\n",
    ))
    .unwrap();
    assert!(b.contract.is_empty());
    assert!(b.test_matrix.is_empty());
}
#[test]
fn scope_stops_at_sections() {
    let b =
        parse_brief("GOAL g\nSCOPE a, b\nTEST_MATRIX\nx|edge|a::c|ok|run\nCONTRACT x\nVERIFY v\n")
            .unwrap();
    assert_eq!(b.scope, ["a", "b"]);
}
#[test]
fn effort_with_sections() {
    let s = brief("CONTRACT x\nTEST_MATRIX\nx|edge|tests/parse_fixes.rs::c|ok|run\nEFFORT high\nTIER sol\nCONTEXT c\n");
    let b = parse_brief(&s).unwrap();
    assert_eq!(b.effort.as_deref(), Some("high"));
    assert_eq!(b.tier.as_deref(), Some("sol"));
    assert!(!b.text.contains("EFFORT high"));
    assert!(!b.text.contains("TIER sol"));
    assert_eq!(b.test_matrix.len(), 1);
}
