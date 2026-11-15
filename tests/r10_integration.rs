use std::{fs, path::Path};

const HEADINGS: [&str; 7] = [
    "## Merges",
    "## Prompt sizes",
    "## Offline graph evaluation",
    "## Engine fixes",
    "## Seams",
    "## Suites",
    "## Limitations",
];

const SEAMS: [&str; 17] = [
    "dual_core", "turn_policy", "interjections", "approvals", "storage", "background",
    "modes", "sessions", "unit_actions", "optimizer", "github", "DecisionRouter",
    "WishLauncher", "Launcher", "Provider", "SessionStream", "DecisionObserver",
];

const PROMPTS: [&str; 3] = [
    "engine/agents/dispatcher.md",
    "engine/DISPATCH.md",
    "engine/skills/wish/SKILL.md",
];

fn read(path: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
        .unwrap_or_else(|err| panic!("could not read {path}: {err}"))
        .replace("\r\n", "\n")
}

fn check_sections(text: &str, headings: &[&str]) -> bool {
    let mut lines = text.lines();
    headings.iter().all(|heading| lines.any(|line| line == *heading))
}

fn section<'a>(text: &'a str, heading: &str) -> Option<&'a str> {
    let start = text.lines().position(|line| line == heading)?;
    let offset = text.lines().take(start + 1).map(|line| line.len() + 1).sum();
    let body = text.get(offset..)?;
    Some(body.split("\n## ").next().unwrap_or(body))
}

fn seam_in_doc(text: &str, name: &str) -> bool {
    text.lines().any(|line| {
        line.contains(name)
            && line
                .split(|c: char| !c.is_ascii_alphabetic())
                .any(|word| word == "wired" || word == "unwired")
    })
}

fn check_seams(first: &str, second: &str, names: &[&str]) -> bool {
    names.iter().all(|name| seam_in_doc(first, name) && seam_in_doc(second, name))
}

fn check_total(recorded: usize, measured: usize) -> bool {
    recorded == measured && recorded <= 20813
}

fn recorded_bytes(text: &str, label: &str) -> Option<usize> {
    let prefix = format!("- {label}: ");
    let mut matches = text.lines().filter_map(|line| line.strip_prefix(&prefix));
    let value = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    let (number, rest) = value.split_once(" bytes")?;
    if !rest.is_empty() && !(label == "total" && rest == " (cap 20813)") {
        return None;
    }
    number.parse().ok()
}

#[test]
fn all_sections_present() {
    assert!(check_sections(&read("docs/integration-3.0.md"), &HEADINGS));
}

#[test]
fn merges_named() {
    let doc = read("docs/integration-3.0.md");
    let merges = section(&doc, "## Merges").expect("Merges section");
    for name in ["r9/p2b", "r9/p2a", "r9/p4"] {
        assert!(merges.contains(name), "missing merge {name}");
    }
}

#[test]
fn seam_names_in_both_docs() {
    assert!(check_seams(&read("docs/seams.md"), &read("docs/integration-3.0.md"), &SEAMS));
}

#[test]
fn prompt_total_matches_files() {
    let doc = read("docs/integration-3.0.md");
    let sizes = section(&doc, "## Prompt sizes").expect("Prompt sizes section");
    let mut sum = 0;
    for path in PROMPTS {
        let measured = read(path).len();
        assert_eq!(recorded_bytes(sizes, path), Some(measured), "{path}");
        sum += measured;
    }
    let total = recorded_bytes(sizes, "total").expect("recorded total");
    assert!(check_total(total, sum), "recorded {total}, measured {sum}, cap 20813");
}

#[test]
fn negative_controls() {
    assert!(!check_total(1001, 1000));
    assert!(!check_total(20814, 20814));
    let missing_one = HEADINGS
        .iter()
        .filter(|heading| **heading != "## Engine fixes")
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!check_sections(&missing_one, &HEADINGS));
    assert!(!check_seams("dual_core wired\n", "other wired\n", &["dual_core"]));
}
