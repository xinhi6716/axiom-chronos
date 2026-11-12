use std::{fs, path::Path};

#[test]
fn prompt_sizes_stay_within_caps() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let files = [
        ("engine/agents/dispatcher.md", 9274, Some(9274)),
        ("engine/DISPATCH.md", 4179, Some(4179)),
        ("engine/skills/wish/SKILL.md", 7755, Some(7755)),
        ("engine/agents/architect.md", 2083, None),
        ("docs/bridge.md", 10298, None),
    ];
    let mut total = 0;
    for (path, before, cap) in files {
        let after = fs::read_to_string(root.join(path)).unwrap().replace("\r\n", "\n").len();
        println!("{path}: before {before} bytes ({} bytes / 4), after {after} bytes ({} bytes / 4)", before as f64 / 4.0, after as f64 / 4.0);
        if let Some(cap) = cap {
            total += after;
            assert!(after <= cap, "{path} is {after} LF bytes; cap is {cap}");
        }
    }
    println!("capped total: before 21208 bytes (5302 bytes / 4), after {total} bytes ({} bytes / 4)", total as f64 / 4.0);
    assert!(total <= 21208, "total is {total} LF bytes; cap is 21208");
}
