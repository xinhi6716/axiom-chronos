use std::{
    fs,
    path::{Path, PathBuf},
};

const PROMPTS: [&str; 3] = [
    "engine/agents/dispatcher.md",
    "engine/DISPATCH.md",
    "engine/skills/wish/SKILL.md",
];

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}
fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .replace("\r\n", "\n")
}
fn files(dir: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.unwrap().path();
        if path.is_dir() {
            result.extend(files(&path));
        } else if path.is_file() {
            result.push(path);
        }
    }
    result.sort();
    result
}
fn within_limit(total: usize) -> bool {
    total <= 20758
}
fn references_paper(text: &str) -> bool {
    text.contains("engine/paper") || text.contains("engine\\paper")
}

#[test]
fn always_loaded_prompt_bytes_not_grown() {
    let total = PROMPTS.iter().map(|p| read(&root().join(p)).len()).sum();
    assert!(
        within_limit(total),
        "always-loaded prompts: {total} bytes, limit 20758"
    );
}
#[test]
fn limit_rejects_one_byte_over() {
    assert!(within_limit(20758));
    assert!(!within_limit(20759));
}
#[test]
fn always_loaded_prompts_do_not_reference_engine_paper() {
    for path in PROMPTS {
        assert!(
            !references_paper(&read(&root().join(path))),
            "{path} references engine/paper"
        );
    }
}
#[test]
fn reference_negative_control() {
    assert!(references_paper("engine/paper/prompts/x.md"));
}

#[test]
fn every_engine_paper_asset_embedded_and_loaded_on_demand() {
    let assets = files(&root().join("engine/paper"));
    assert!(!assets.is_empty(), "no paper assets found");
    let prompts: Vec<_> = PROMPTS.iter().map(|p| (p, read(&root().join(p)))).collect();
    for path in assets {
        let name = path
            .strip_prefix(root().join("engine"))
            .unwrap()
            .to_str()
            .unwrap()
            .replace('\\', "/");
        let content = read(&path);
        let embedded = axiom::engine::FILES
            .iter()
            .find(|(key, _)| *key == name)
            .unwrap_or_else(|| panic!("{name} is not embedded"));
        assert_eq!(
            embedded.1.replace("\r\n", "\n"),
            content,
            "embedded content differs for {name}"
        );
        for (prompt, text) in &prompts {
            assert!(
                !text.replace('\\', "/").contains(&name),
                "{prompt} names {name}"
            );
        }
    }
}

fn detector_evasion(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            if lower.contains("not") || lower.contains("never") || lower.contains("no ") {
                return false;
            }
            let phrases = [
                "evade detect",
                "evading detect",
                "bypass detect",
                "avoid detection",
                "defeat detector",
                "fool detector",
                "undetectable",
                "pass ai detection",
                "beat gptzero",
                "turnitin",
            ];
            if phrases.iter().any(|p| lower.contains(p)) {
                return true;
            }
            let words: Vec<_> = lower
                .split(|c: char| !c.is_ascii_alphanumeric())
                .filter(|w| !w.is_empty())
                .collect();
            words.iter().enumerate().any(|(i, w)| {
                w.starts_with("evad")
                    && words
                        .iter()
                        .skip(i + 1)
                        .take(2)
                        .any(|w| w.starts_with("detect"))
            })
        })
        .collect()
}

// Find literal include_str! targets in every paper source file, including non-paper paths.
fn include_targets(source_path: &Path, source: &str) -> Vec<PathBuf> {
    let mut rest = source;
    let mut targets = Vec::new();
    while let Some(index) = rest.find("include_str!") {
        rest = rest[index + "include_str!".len()..].trim_start();
        rest = rest
            .strip_prefix('(')
            .expect("include_str! must use a literal target")
            .trim_start();
        rest = rest
            .strip_prefix('"')
            .expect("include_str! must use a literal target");
        let end = rest.find('"').expect("unterminated include_str! target");
        let target = &rest[..end];
        assert!(
            !target.contains('\\'),
            "escaped include_str! target requires handling"
        );
        targets.push(source_path.parent().unwrap().join(target));
        rest = &rest[end + 1..];
    }
    targets
}
#[test]
fn no_paper_prompt_mentions_detector_evasion() {
    let mut paths = files(&root().join("engine/paper"));
    for source in files(&root().join("src/paper")) {
        if source.extension().is_some_and(|ext| ext == "rs") {
            paths.extend(include_targets(&source, &read(&source)));
        }
    }
    for path in paths {
        let text = read(&path);
        let hits = detector_evasion(&text);
        assert!(hits.is_empty(), "{}: {hits:?}", path.display());
    }
}
#[test]
fn detector_evasion_negative_control() {
    assert!(!detector_evasion("rewrite so it evades AI detectors").is_empty());
}

// Mask comments and literals so braces and identifiers in them do not affect item scanning.
fn code_only(source: &str) -> Vec<u8> {
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        if bytes[i..].starts_with(b"//") {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if bytes[i..].starts_with(b"/*") {
            i += 2;
            let mut depth = 1;
            while i < bytes.len() && depth > 0 {
                if bytes[i..].starts_with(b"/*") {
                    depth += 1;
                    i += 2;
                } else if bytes[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            assert_eq!(depth, 0, "unterminated comment");
        } else if bytes[i] == b'r' && (i == 0 || !ident(bytes[i - 1])) {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] == b'#' {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'"' {
                let hashes = j - i - 1;
                i = j + 1;
                while i < bytes.len() {
                    if bytes[i] == b'"'
                        && i + hashes < bytes.len()
                        && bytes[i + 1..i + 1 + hashes].iter().all(|b| *b == b'#')
                    {
                        i += 1 + hashes;
                        break;
                    }
                    i += 1;
                }
            } else {
                i += 1;
                continue;
            }
        } else if bytes[i] == b'"'
            || (bytes[i] == b'\''
                && i + 2 < bytes.len()
                && (bytes[i + 2] == b'\''
                    || (bytes[i + 1] == b'\\'
                        && bytes[i + 2..]
                            .iter()
                            .position(|b| *b == b'\'')
                            .is_some_and(|n| n <= 5))))
        {
            let quote = bytes[i];
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                } else if bytes[i] == quote {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
        } else {
            i += 1;
            continue;
        }
        for b in &mut out[start..i] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
    }
    out
}
fn ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
fn skip_space(code: &[u8], mut i: usize) -> usize {
    while i < code.len() && code[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

// Skip a whole item, including nested braces, or a semicolon-terminated item.
fn item_end(code: &[u8], mut i: usize) -> usize {
    let mut parens = 0;
    let mut brackets = 0;
    while i < code.len() {
        match code[i] {
            b'(' => parens += 1,
            b')' => parens -= 1,
            b'[' => brackets += 1,
            b']' => brackets -= 1,
            b';' if parens == 0 && brackets == 0 => return i + 1,
            b'{' if parens == 0 && brackets == 0 => {
                let mut depth = 1;
                i += 1;
                while i < code.len() && depth > 0 {
                    if code[i] == b'{' {
                        depth += 1;
                    } else if code[i] == b'}' {
                        depth -= 1;
                    }
                    i += 1;
                }
                assert_eq!(depth, 0, "unbalanced item braces");
                return i;
            }
            _ => {}
        }
        i += 1;
    }
    panic!("unterminated item")
}
fn strip_test_items(source: &str) -> Vec<u8> {
    let mut code = code_only(source);
    let mut i = 0;
    while i < code.len() {
        if code[i] == b'#' {
            let open = skip_space(&code, i + 1);
            if code.get(open) == Some(&b'[') {
                let mut end = open + 1;
                let mut depth = 1;
                while end < code.len() && depth > 0 {
                    if code[end] == b'[' {
                        depth += 1;
                    } else if code[end] == b']' {
                        depth -= 1;
                    }
                    end += 1;
                }
                assert_eq!(depth, 0, "unbalanced attribute");
                let attr: Vec<_> = code[open + 1..end - 1]
                    .iter()
                    .copied()
                    .filter(|b| !b.is_ascii_whitespace())
                    .collect();
                if attr == b"cfg(test)" {
                    let finish = item_end(&code, end);
                    code[i..finish].fill(b' ');
                    i = finish;
                    continue;
                }
            }
        }
        if code[i..].starts_with(b"fn")
            && (i == 0 || !ident(code[i - 1]))
            && (i + 2 == code.len() || !ident(code[i + 2]))
        {
            let name = skip_space(&code, i + 2);
            if code[name..].starts_with(b"test_") {
                let finish = item_end(&code, name);
                code[i..finish].fill(b' ');
                i = finish;
                continue;
            }
        }
        i += 1;
    }
    code
}
fn fake_identifiers(path: &str, source: &str) -> Vec<String> {
    let code = strip_test_items(source);
    let mut hits = Vec::new();
    let mut i = 0;
    while i < code.len() {
        if ident(code[i]) {
            let start = i;
            while i < code.len() && ident(code[i]) {
                i += 1;
            }
            let name = std::str::from_utf8(&code[start..i]).unwrap();
            if (name.starts_with("Fake") || name.starts_with("Canned"))
                && !(path == "src/paper/http.rs"
                    && ["FakeClock", "Canned", "CannedTransport"].contains(&name))
            {
                hits.push(name.to_owned());
            }
        } else {
            i += 1;
        }
    }
    hits
}
#[test]
fn production_paper_code_has_no_fake() {
    let mut paths = files(&root().join("src/paper"));
    paths.push(root().join("src/bridge/paper_tool.rs"));
    for path in paths {
        let name = path
            .strip_prefix(root())
            .unwrap()
            .to_str()
            .unwrap()
            .replace('\\', "/");
        let hits = fake_identifiers(&name, &read(&path));
        assert!(hits.is_empty(), "{name}: {hits:?}");
    }
}
#[test]
fn fake_identifier_negative_controls() {
    assert_eq!(
        fake_identifiers("src/paper/x.rs", "struct FakeThing;"),
        ["FakeThing"]
    );
    assert!(fake_identifiers(
        "src/paper/x.rs",
        "#[cfg(test)] mod tests { struct FakeThing; }"
    )
    .is_empty());
    assert_eq!(
        fake_identifiers("src/paper/x.rs", "struct CannedTransport;"),
        ["CannedTransport"]
    );
}

fn missing_terms(text: &str, terms: &[&str]) -> Vec<String> {
    terms
        .iter()
        .filter(|term| !text.contains(**term))
        .map(|term| (*term).to_owned())
        .collect()
}
fn check_record(terms: &[&str]) {
    let text = read(&root().join("docs/paper-3.0.md"));
    let missing = missing_terms(&text, terms);
    assert!(missing.is_empty(), "missing record terms: {missing:?}");
}
#[test]
fn missing_terms_negative_control() {
    assert_eq!(missing_terms("abc", &["abc", "zzz"]), ["zzz"]);
}
#[test]
fn paper_record_has_acceptance_items() {
    check_record(&[
        "Acceptance",
        "Journal matching",
        "Templates",
        "Pipeline",
        "Integrity gates",
        "Style",
        "Disclosure",
        "tests/paper_journals.rs",
        "tests/paper_templates.rs",
        "tests/paper_runner.rs",
        "tests/paper_integrity.rs",
        "tests/paper_style.rs",
    ]);
}
#[test]
fn paper_record_has_prompt_bytes() {
    check_record(&["20,813", "20,758", "before", "after"]);
}
#[test]
fn paper_record_has_figure_route() {
    check_record(&[
        "Tectonic",
        "TikZ",
        "Plotters",
        "line",
        "scatter",
        "error_bar",
        "bar",
        "log axes",
        "multi-panel",
    ]);
}
#[test]
fn paper_record_has_crates() {
    check_record(&["ttf-parser", "windows", "No new crates"]);
}
#[test]
fn paper_record_has_template_evidence() {
    check_record(&[
        "elsarticle",
        "IEEEtran",
        "sn-jnl",
        "achemso",
        "verified",
        "unconfirmed",
    ]);
}
#[test]
fn paper_record_has_limitations() {
    check_record(&[
        "Springer",
        "ACS",
        "DPI",
        "Arial",
        "DejaVu",
        "returns early",
        "agy",
        "stdin",
    ]);
}
#[test]
fn paper_record_has_ruling_20_skip() {
    check_record(&["ruling 20", "removed"]);
}
#[test]
fn paper_record_has_diff_stat() {
    check_record(&["git diff --stat 7a5d43a9 -- tests/", "Cargo.lock"]);
}
