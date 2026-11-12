use std::{fs, path::Path};

fn within_cap(total: usize) -> bool {
    total <= 20813
}

#[test]
fn prompt_sizes_stay_within_caps() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let files = [
        ("engine/agents/dispatcher.md", 9809),
        ("engine/DISPATCH.md", 3120),
        ("engine/skills/wish/SKILL.md", 7884),
    ];
    let mut total = 0;
    for (path, before) in files {
        let after = fs::read_to_string(root.join(path))
            .unwrap()
            .replace("\r\n", "\n")
            .len();
        println!("{path}: before {before} bytes, after {after} bytes");
        total += after;
    }
    println!("capped total: before 20813 bytes, after {total} bytes");
    assert!(within_cap(total), "total is {total} LF bytes; cap is 20813");
}

#[test]
fn cap_check_rejects_one_byte_over() {
    assert!(within_cap(20813));
    assert!(!within_cap(20814));
}
