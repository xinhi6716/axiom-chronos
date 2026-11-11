use regex::Regex;
use std::collections::HashSet;
use std::path::Path;

#[path = "common/captures.rs"]
mod captures;

fn validate(baseline: &str, doc: &str) -> Result<(), String> {
    let weaknesses = baseline
        .split_once("## Weaknesses\n").ok_or("missing weaknesses")?.1
        .split_once("## Inspiration\n").ok_or("missing inspiration")?.0;
    let weakness = Regex::new(r"^- \*\*(W[1-9][0-9]*)\*\* ").unwrap();
    let mut ids = HashSet::new();
    for line in weaknesses.lines().filter(|line| !line.trim().is_empty()) {
        let id = weakness.captures(line).ok_or("invalid weakness")?[1].to_owned();
        if !ids.insert(id) { return Err("duplicate weakness".into()); }
    }
    if ids.is_empty() { return Err("no weaknesses".into()); }

    let headings: Vec<_> = doc.lines().filter(|line| line.starts_with("## ")).collect();
    if !doc.starts_with("# UI after\n")
        || headings != ["## Captures", "## Verdicts", "## Not verified", "## Record keeping"] {
        return Err("invalid heading order".into());
    }
    let captures = doc.split_once("## Captures\n").ok_or("missing captures")?.1
        .split_once("## Verdicts\n").ok_or("missing verdicts")?.0;
    let verdicts = doc.split_once("## Verdicts\n").ok_or("missing verdicts")?.1
        .split_once("## Not verified\n").ok_or("missing not verified")?.0;
    let not_verified = doc.split_once("## Not verified\n").ok_or("missing not verified")?.1
        .split_once("## Record keeping\n").ok_or("missing record keeping")?.0;
    let record = doc.split_once("## Record keeping\n").ok_or("missing record keeping")?.1;

    let mut expected = HashSet::new();
    for view in ["run", "wishes", "memory", "usage", "settings"] {
        for theme in ["dark", "light"] {
            for size in ["1100x700", "1920x1080"] {
                expected.insert(format!("{view}-{theme}-{size}.png"));
            }
        }
    }
    for name in ["blocked-1100x700", "focus-graph", "focus-checklist", "mini-top-off",
        "dense", "at-rest", "feed-error", "stale", "quota-low", "quota-out", "quota-unknown"] {
        expected.insert(format!("{name}.png"));
    }
    let mut lines = captures.lines().filter(|line| !line.trim().is_empty());
    if lines.next() != Some("| name | file |") || lines.next() != Some("| --- | --- |") {
        return Err("invalid captures header".into());
    }
    let mut files = HashSet::new();
    for line in lines {
        let row = line.strip_prefix('|').and_then(|s| s.strip_suffix('|')).ok_or("invalid capture row")?;
        let cells: Vec<_> = row.split('|').map(str::trim).collect();
        if cells.len() != 2 || cells[1] != format!("~/.axiom/orchestrate/r6-ui2/captures/after/{}", cells[0])
            || !files.insert(cells[0].to_owned()) {
            return Err("invalid or duplicate capture".into());
        }
    }
    if files != expected { return Err("incorrect captures".into()); }

    let verdict = Regex::new(r"^(W[1-9][0-9]*) (PASS|FAIL|DROPPED) ([^ /]+\.png) - \S.*$").unwrap();
    let idle = Regex::new(r"^IDLE CPU ([0-9]+(?:\.[0-9]+)?)% over 30 s$").unwrap();
    let mut seen = HashSet::new();
    let mut idle_count = 0;
    for line in verdicts.lines().filter(|line| !line.trim().is_empty()) {
        if line.starts_with('W') {
            let m = verdict.captures(line).ok_or("invalid verdict")?;
            if !ids.contains(&m[1]) || !seen.insert(m[1].to_owned()) || !files.contains(&m[3]) {
                return Err("unknown or duplicate verdict, or unknown capture".into());
            }
        } else if line.starts_with("IDLE CPU") {
            let m = idle.captures(line).ok_or("invalid idle CPU line")?;
            let value: f64 = m[1].parse().map_err(|_| "invalid CPU number")?;
            if value >= 1.0 { return Err("idle CPU too high".into()); }
            idle_count += 1;
        }
    }
    if seen != ids || idle_count != 1 { return Err("missing verdict or idle CPU line".into()); }

    let bullets: Vec<_> = not_verified.lines().filter_map(|line| line.strip_prefix("- ")).map(str::to_lowercase).collect();
    if ["tooltip", "always-on-top", "glow", "60 s"].iter().any(|term| !bullets.iter().any(|line| line.contains(term)))
        || !record.contains("findings") || !record.contains("condensed") {
        return Err("missing not-verified item or record-keeping note".into());
    }
    Ok(())
}

fn document(name: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs").join(name))
        .unwrap().replace("\r\n", "\n")
}

#[test]
fn after_is_valid() {
    let doc = document("ui-after.md");
    validate(&document("ui-baseline.md"), &doc).unwrap();
    let folder = captures::expand("~/.axiom/orchestrate/r6-ui2/captures/after/");
    if !captures::folders_exist(&[folder]) { return; }
    let table = doc.split_once("## Captures\n").unwrap().1.split_once("## Verdicts\n").unwrap().0;
    for line in table.lines().filter(|line| line.contains("~/.axiom/")) {
        let file = line.split('|').nth(2).unwrap().trim();
        assert!(captures::expand(file).unwrap().is_file(), "capture missing: {file}");
    }
}

#[test]
fn rejects_broken_after_documents() {
    let baseline = document("ui-baseline.md");
    let doc = document("ui-after.md");
    let line = doc.lines().find(|line| line.starts_with("W1 ")).unwrap();
    assert!(validate(&baseline, &doc.replacen(line, "", 1)).is_err());
    assert!(validate(&baseline, &doc.replacen("IDLE CPU 0.21%", "IDLE CPU 1.5%", 1)).is_err());
    assert!(validate(&baseline, &doc.replacen(line, &line.replace("blocked-1100x700.png", "missing.png"), 1)).is_err());
}
