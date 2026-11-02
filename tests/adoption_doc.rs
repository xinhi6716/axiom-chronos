use std::path::Path;

const SECTIONS: [(&str, &str, Option<&str>); 12] = [
    ("DeepSeek", "claim", None),
    ("Orca", "claim", None),
    ("Stop That Shit", "claim", None),
    ("Pstack", "verdict", Some("ae5650f")),
    ("Codebase Memory", "memory", None),
    ("Trail-Code", "memory", None),
    ("Agency Agents", "claim", None),
    ("HERO", "wish-flow", Some("92686ca")),
    ("ARIS", "research", Some("c485faa")),
    ("ECC", "install", None),
    ("Axiom Atropos", "install", Some("1f39b07")),
    ("Orkas", "progress", Some("dce78ad")),
];
const LABELS: [&str; 6] = ["來源：", "閱讀：", "整合：", "落點：", "測試：", "排除："];

fn check(text: &str) -> Result<(), String> {
    let mut sections: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("## ") {
            sections.push((name, Vec::new()));
        } else if let Some((_, lines)) = sections.last_mut() {
            lines.push(line);
        }
    }
    if sections.iter().map(|(name, _)| *name).collect::<Vec<_>>()
        != SECTIONS
            .iter()
            .map(|(name, _, _)| *name)
            .collect::<Vec<_>>()
    {
        return Err("section names or order differ".into());
    }
    for ((name, lines), (_, module, head)) in sections.into_iter().zip(SECTIONS) {
        let mut values = Vec::new();
        for label in LABELS {
            let matches: Vec<_> = lines
                .iter()
                .filter_map(|line| line.strip_prefix(label))
                .collect();
            if matches.len() != 1 || matches[0].trim().is_empty() {
                return Err(format!("{name}: missing, duplicate or empty {label}"));
            }
            values.push(matches[0].trim());
        }
        let source = values[0];
        let source_path = source
            .split(|c: char| c.is_whitespace() || c == '，' || c == ',')
            .next()
            .unwrap();
        let head_value = source
            .split_once("HEAD")
            .map(|(_, value)| value.trim_start_matches(|c: char| c.is_whitespace() || c == '='));
        let head_word =
            head_value.and_then(|value| value.split(|c: char| !c.is_ascii_alphanumeric()).next());
        if !source_path.contains('/')
            || head_word.is_none_or(str::is_empty)
            || head.is_some_and(|expected| head_word != Some(expected))
        {
            return Err(format!("{name}: malformed source or wrong HEAD"));
        }
        let reading = values[1];
        if !(reading.starts_with("本輪實讀") || reading.starts_with("承襲"))
            || !reading.contains("codex-rebuild.zh-TW.md")
        {
            return Err(format!("{name}: malformed reading"));
        }
        let mut tokens = values[3]
            .split(|c: char| c.is_whitespace() || c == ',' || c == '，')
            .filter(|s| !s.is_empty());
        if tokens.next() != Some(module) {
            return Err(format!("{name}: wrong module"));
        }
        let paths: Vec<_> = tokens.collect();
        if paths.is_empty() {
            return Err(format!("{name}: missing paths"));
        }
        for path in paths {
            if path.starts_with('/')
                || path.contains(['\\', ':'])
                || path
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
                || !Path::new(env!("CARGO_MANIFEST_DIR")).join(path).is_file()
            {
                return Err(format!("{name}: invalid file {path}"));
            }
        }
    }
    Ok(())
}

#[test]
fn adoption_document_and_negative_controls() {
    let text = include_str!("../docs/round4-adoption.zh-TW.md");
    assert!(check(text).is_ok(), "{:?}", check(text));
    assert!(check(&text.replacen("## HERO", "## renamed", 1)).is_err());
    assert!(check(&text.replacen("## HERO\n", "", 1)).is_err());
    let swapped = text
        .replacen("92686ca", "TEMP_HEAD", 1)
        .replacen("c485faa", "92686ca", 1)
        .replacen("TEMP_HEAD", "c485faa", 1);
    assert!(check(&swapped).is_err());
    assert!(check(&text.replacen("落點：claim", "落點：wrong", 1)).is_err());
    assert!(check(&text.replacen("閱讀：承襲", "閱讀：錯誤", 1)).is_err());
    assert!(check(&text.replacen("落點：claim", "落點：claim ../Cargo.toml", 1)).is_err());
    let first_placement = text
        .lines()
        .find(|line| line.starts_with("落點："))
        .unwrap();
    let with_nonexistent = format!("{first_placement} nonexistent-adoption-file.rs");
    assert!(check(&text.replacen(first_placement, &with_nonexistent, 1)).is_err());
}
