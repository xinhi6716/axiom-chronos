#[test]
fn wish_prompt_stays_within_size_cap() {
    let skill = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("engine/skills/wish/SKILL.md"),
    )
    .unwrap()
    .replace("\r\n", "\n");
    let before = 9546;
    let after = skill.len();
    println!(
        "wish SKILL.md: before {before} bytes ({} bytes / 4), after {after} bytes ({} bytes / 4)",
        before as f64 / 4.0,
        after as f64 / 4.0
    );
    assert!(after <= 8114, "wish SKILL.md is {after} LF bytes; cap is 8114");
}
