use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[path = "common/captures.rs"]
mod captures;

fn validate(doc: &str) -> Result<(), String> {
    let headings: Vec<_> = doc.lines().filter(|line| line.starts_with("## ")).collect();
    if headings != ["## Captures", "## Weaknesses", "## Inspiration"] {
        return Err("expected exactly the three baseline sections in order".into());
    }
    let captures = doc.split_once("## Captures\n").ok_or("missing captures")?.1
        .split_once("## Weaknesses\n").ok_or("missing weaknesses")?.0;
    let weaknesses = doc.split_once("## Weaknesses\n").ok_or("missing weaknesses")?.1
        .split_once("## Inspiration\n").ok_or("missing inspiration")?.0;
    let mut lines = captures.lines().filter(|line| !line.trim().is_empty());
    if lines.next() != Some("| view | theme | size | file |") {
        return Err("incorrect captures header".into());
    }
    let separator = lines.next().ok_or("missing table separator")?;
    if !separator.starts_with('|') || separator.split('|').filter(|s| !s.is_empty()).count() != 4
        || separator.split('|').filter(|s| !s.is_empty()).any(|s| {
            let t = s.trim().trim_matches(':');
            t.len() < 3 || !t.chars().all(|c| c == '-')
        }) {
        return Err("invalid table separator".into());
    }
    let views = ["run", "wishes", "memory", "usage", "settings"];
    let themes = ["dark", "light"];
    let sizes = ["1100x700", "1920x1080"];
    let mut combinations = HashSet::new();
    let mut files = HashSet::new();
    for line in lines {
        if !line.starts_with('|') { continue; }
        if !line.starts_with('|') || !line.ends_with('|') { return Err("invalid table row".into()); }
        let cells: Vec<_> = line[1..line.len()-1].split('|').map(str::trim).collect();
        if cells.len() != 4 || !views.contains(&cells[0]) || !themes.contains(&cells[1]) || !sizes.contains(&cells[2]) {
            return Err("invalid capture combination".into());
        }
        if !combinations.insert((cells[0], cells[1], cells[2])) || !files.insert(cells[3]) {
            return Err("duplicate combination or file".into());
        }
        let file = cells[3];
        let expected = format!("{}-{}-{}.png", cells[0], cells[1], cells[2]);
        if file.contains('\\') || !(file.starts_with('/') || file.starts_with("~/") || file.as_bytes().get(1..3) == Some(b":/"))
            || file.rsplit('/').next() != Some(expected.as_str()) {
            return Err("capture path or name invalid".into());
        }

    }
    if combinations.len() != 20 { return Err("captures are not the full cross product".into()); }

    let item = Regex::new(r"^- \*\*W([1-9][0-9]*)\*\* `(run|wishes|memory|usage|settings|all)` `(layout|hierarchy|spacing|effects|usability|interaction)` — (.+)\. PASS IF (.+)\. COST (.+)\.(?: DROPPED (.+))?$").unwrap();
    let mut ids = HashSet::new();
    let mut counts = HashMap::new();
    let mut needs_you = false;
    for line in weaknesses.lines().filter(|line| !line.trim().is_empty()) {
        let m = item.captures(line).ok_or_else(|| format!("invalid weakness: {line}"))?;
        if !ids.insert(m[1].to_owned()) { return Err("duplicate weakness id".into()); }
        *counts.entry(m[2].to_owned()).or_insert(0) += 1;
        if &m[2] == "run" && &m[3] == "layout"
            && m[4].contains("Run: needs-you card and checklist visible without scrolling")
            && &m[5] == "at 1100x700 both are fully visible with no scrolling" {
            needs_you = true;
        }
    }
    if views.iter().any(|v| counts.get(*v).copied().unwrap_or(0) < 2) || !needs_you {
        return Err("missing per-view weaknesses or needs-you criterion".into());
    }
    Ok(())
}

fn document() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/ui-baseline.md")).unwrap().replace("\r\n", "\n")
}

#[test]
fn baseline_is_valid() {
    let doc = document();
    validate(&doc).unwrap();
    let files: Vec<_> = doc.lines().filter(|line| line.starts_with("| ") && line.ends_with(" |"))
        .filter_map(|line| {
            let cells: Vec<_> = line[1..line.len()-1].split('|').map(str::trim).collect();
            (cells.len() == 4 && ["run", "wishes", "memory", "usage", "settings"].contains(&cells[0]))
                .then(|| cells[3])
        }).filter(|file| !file.split('/').any(|part| part == ".axiom-tmp")).collect();
    let folders: Vec<_> = files.iter().map(|file| captures::expand(file).and_then(|p| p.parent().map(Path::to_path_buf))).collect();
    if !captures::folders_exist(&folders) { return; }
    for file in files {
        assert!(captures::expand(file).unwrap().is_file(), "capture missing: {file}");
    }
}

#[test]
fn rejects_invalid_baselines() {
    let doc = document();
    let row = doc.lines().find(|l| l.starts_with("| run | dark | 1100x700 | ")).unwrap();
    assert!(validate(&doc.replacen(row, &format!("{row}\n{row}"), 1)).is_err());
    let no_memory = doc.lines().filter(|l| !l.starts_with("- **W") || !l.contains(" `memory` ")).collect::<Vec<_>>().join("\n") + "\n";
    assert!(validate(&no_memory).is_err());
    assert!(validate(&doc.replacen(". PASS IF ", ". ", 1)).is_err());
    assert!(validate(&(doc + "\n## Extra\n")).is_err());
}
