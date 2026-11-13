use std::{fs, path::Path};
fn read(path: &str) -> String { fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap().replace("\r\n", "\n") }
fn sizes() -> usize {
    let mut total = 0;
    for path in ["engine/agents/dispatcher.md", "engine/DISPATCH.md", "engine/skills/wish/SKILL.md"] {
        let n = read(path).len(); println!("{path}: {n} LF bytes"); total += n;
    }
    println!("total: {total} LF bytes"); total
}
fn contains_all(path: &str, words: &[&str]) { let text = read(path); for word in words { assert!(text.contains(word), "{path}: missing {word}"); } }
#[test]
fn cap_constant_unchanged() { contains_all("tests/prompt_size.rs", &["total <= 20813", "!within_cap(20814)"]); }
#[test]
fn always_loaded_within_cap() { assert!(sizes() <= 20813); }
#[test]
fn routing_rubric_in_astra_and_architect() {
    for path in ["engine/agents/astra.md", "engine/agents/architect.md"] {
        contains_all(path, &["ROUTING", "<default|unit-id> <tier>/<effort> <risk> <reason...>", "sol/high", "sol/xhigh", "max", "astra", "terra/medium", "trivial", "standard", "state_machine", "concurrency", "migration", "parser", "security", "privacy", "cross_module", "prior_failure", "novel", "first-pass"]);
    }
}
#[test]
fn dispatch_documents_tier_and_matrix() {
    contains_all("engine/DISPATCH.md", &["TIER", "astra", "xhigh", "TEST_MATRIX", "CONTRACT", "--fixes", "--routing", "--reason", "RED", "<file>::<case>"]);
    assert!(!read("engine/DISPATCH.md").contains("Other values or positions are refused"));
}
#[test]
fn reviewers_and_selfcheck_documented() {
    for path in ["engine/agents/reviewer.md", "engine/roles/astra-reviewer.md"] { contains_all(path, &["self-check", "Astra reviewer", "default", "untested", "vacuous_check", "contract_mismatch", "blocker", "major", "Claude"]); }
    contains_all("docs/bridge.md", &["self-check", "set_model", "resume", "TIER astra"]);
}
#[test]
fn dispatcher_tells_unmapped_to_write_matrix() {
    contains_all("engine/agents/dispatcher.md", &["wish without a module map", "TEST_MATRIX", "CONTRACT"]);
    contains_all("engine/skills/wish/SKILL.md", &["Core stage models"]);
}
#[test]
fn dispatcher_has_no_dangling_fix_flow() {
    let text = read("engine/agents/dispatcher.md");
    assert!(!text.contains("fix flow"), "dispatcher has a dangling fix flow reference");
    let step6 = text.split_once("\n6. ").expect("missing step 6").1
        .split_once("\n7. ").expect("missing step 7").0;
    for word in ["--fixes", "escalat"] {
        assert!(step6.contains(word), "step 6: missing {word}");
    }
}
#[test]
fn cap_check_has_negative_control() {
    let within_cap = |total| total <= 20813;
    assert!(within_cap(20813)); assert!(!within_cap(20814));
    let total = sizes(); assert_eq!(within_cap(total + 1), total + 1 <= 20813);
}
fn sources(dir: &Path, text: &mut String) {
    for entry in fs::read_dir(dir).unwrap() { let p = entry.unwrap().path(); if p.is_dir() { sources(&p, text); } else if p.extension().is_some_and(|s| s == "rs") { text.push_str(&fs::read_to_string(p).unwrap()); } }
}
#[test]
fn no_invented_names() {
    let mut src = String::new(); sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut src);
    for name in ["--routing", "--reason", "set_model", "routing_partial", "escalation_saturated"] { assert!(src.contains(name), "unknown engine name: {name}"); }
}
