use regex::Regex;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

// Preserve byte offsets and newlines so that matches still point into the original source.
fn blank(bytes: &mut [u8], start: usize, end: usize) {
    for byte in &mut bytes[start..end] {
        if *byte != b'\n' && *byte != b'\r' {
            *byte = b' ';
        }
    }
}

fn code_without_comments_and_literals(source: &str) -> String {
    let input = source.as_bytes();
    let mut output = input.to_vec();
    let mut i = 0;
    while i < input.len() {
        let start = i;
        if input[i..].starts_with(b"//") {
            i += 2;
            while i < input.len() && input[i] != b'\n' { i += 1; }
        } else if input[i..].starts_with(b"/*") {
            i += 2;
            let mut depth = 1;
            while i < input.len() && depth > 0 {
                if input[i..].starts_with(b"/*") { depth += 1; i += 2; }
                else if input[i..].starts_with(b"*/") { depth -= 1; i += 2; }
                else { i += 1; }
            }
        } else {
            // Raw strings may have any number of # delimiters (and a b/c prefix).
            let raw = if input[i] == b'r' { Some(i) }
                else if matches!(input[i], b'b' | b'c') && input.get(i + 1) == Some(&b'r') { Some(i + 1) }
                else { None };
            let mut raw_end = None;
            if let Some(r) = raw {
                let mut quote = r + 1;
                while input.get(quote) == Some(&b'#') { quote += 1; }
                if input.get(quote) == Some(&b'"') {
                    let hashes = quote - r - 1;
                    let mut j = quote + 1;
                    while j < input.len() {
                        if input[j] == b'"' && j + 1 + hashes <= input.len()
                            && input[j + 1..j + 1 + hashes].iter().all(|b| *b == b'#') {
                            j += 1 + hashes;
                            break;
                        }
                        j += 1;
                    }
                    raw_end = Some(j);
                }
            }
            if let Some(end) = raw_end {
                i = end;
            } else if input[i] == b'"' || (matches!(input[i], b'b' | b'c') && input.get(i + 1) == Some(&b'"')) {
                if input[i] != b'"' { i += 1; }
                i += 1;
                while i < input.len() {
                    if input[i] == b'\\' { i = (i + 2).min(input.len()); }
                    else if input[i] == b'"' { i += 1; break; }
                    else { i += 1; }
                }
            } else if input[i] == b'\'' || (input[i] == b'b' && input.get(i + 1) == Some(&b'\'')) {
                // Only consume a valid single character/escape, not a lifetime such as 'a.
                let quote = if input[i] == b'b' { i + 1 } else { i };
                let rest = &source[quote + 1..];
                let char_end = if rest.starts_with('\\') {
                    if rest.starts_with("\\u{") {
                        rest.find('}').map(|p| quote + 1 + p + 1)
                    } else if rest.starts_with("\\x") {
                        Some(quote + 5)
                    } else { Some(quote + 3) }
                } else { rest.chars().next().map(|c| quote + 1 + c.len_utf8()) };
                if let Some(end) = char_end.filter(|end| input.get(*end) == Some(&b'\'')) {
                    i = end + 1;
                } else { i += 1; continue; }
            } else {
                i += 1;
                continue;
            }
        }
        blank(&mut output, start, i);
    }
    String::from_utf8(output).unwrap()
}

fn strip_test_items(code: &str) -> String {
    let cfg = Regex::new(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]").unwrap();
    let mut bytes = code.as_bytes().to_vec();
    let mut search = 0;
    while let Some(attribute) = cfg.find(&code[search..]) {
        let start = search + attribute.start();
        let mut i = search + attribute.end();
        let (mut parens, mut brackets, mut braces) = (0usize, 0usize, 0usize);
        while i < bytes.len() {
            match bytes[i] {
                b'(' => parens += 1,
                b')' => parens = parens.saturating_sub(1),
                b'[' => brackets += 1,
                b']' => brackets = brackets.saturating_sub(1),
                b'{' if parens == 0 && brackets == 0 => braces += 1,
                b'}' if braces > 0 && parens == 0 && brackets == 0 => {
                    braces -= 1;
                    if braces == 0 { i += 1; break; }
                }
                b';' | b',' if parens == 0 && brackets == 0 && braces == 0 => { i += 1; break; }
                _ => {}
            }
            i += 1;
        }
        blank(&mut bytes, start, i);
        search = i;
    }
    String::from_utf8(bytes).unwrap()
}

fn violations(path: &str, source: &str) -> Vec<String> {
    let path = path.replace('\\', "/");
    if path == "src/desktop/theme.rs" || path.rsplit('/').next() == Some("accept.rs") {
        return Vec::new();
    }
    let code = strip_test_items(&code_without_comments_and_literals(source));
    let named = Regex::new(r"\b(?:text_size\s*\(\s*px\s*\(|rounded_(?:md|lg|xl|2xl|3xl)\b)").unwrap();
    let rounded = Regex::new(r"\brounded(?:_[tblr])?\s*\(\s*px\s*\(\s*([0-9][0-9_]*(?:\.[0-9_]*)?(?:[eE][+-]?[0-9_]+)?)(?:f32|f64)?\s*\)\s*\)").unwrap();
    let mut positions: Vec<usize> = named.find_iter(&code).map(|m| m.start()).collect();
    for capture in rounded.captures_iter(&code) {
        if capture[1].replace('_', "").parse::<f64>().is_ok_and(|n| n > 4.0) {
            positions.push(capture.get(0).unwrap().start());
        }
    }
    positions.sort_unstable();
    positions.into_iter().map(|pos| {
        let line = code.as_bytes()[..pos].iter().filter(|b| **b == b'\n').count() + 1;
        format!("{path}:{line}")
    }).collect()
}

fn scan_tree(dir: &Path, root: &Path, found: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            scan_tree(&path, root, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let relative = path.strip_prefix(root).unwrap().to_string_lossy();
            found.extend(violations(&relative, &fs::read_to_string(&path).unwrap()));
        }
    }
}

#[test]
fn controls_forbidden_patterns() {
    for pattern in ["text_size(px(12.))", "rounded_md()", "rounded_lg()", "rounded_xl()",
        "rounded_2xl()", "rounded_3xl()", "rounded(px(5))", "rounded_t(px(6.5))",
        "rounded_b(px(5.))", "rounded_l(px(4.1))", "rounded_r(px(5.0))"] {
        assert_eq!(violations("src/desktop/example.rs", pattern), ["src/desktop/example.rs:1"], "{pattern}");
    }
}

#[test]
fn controls_allowed_and_excluded_code() {
    let source = r###"
rounded(px(4.)); rounded(px(2.)); rounded_full(); rounded(px(radius));
rounded(px(6.0 + radius)); rounded(px(6.) + offset); padding(px(20.)); gap(px(20.));
// rounded_xl() }
/* rounded_lg() /* rounded_md() */ } */
let a = "rounded_2xl() }";
let b = r#"rounded_3xl() }"#;
let c = '}';
#[cfg(test)] mod tests { fn nested() { let x = "}"; let y = '{'; /* } */ rounded_xl(); } }
#[cfg(test)] mod accept;
#[cfg(test)] const VALUE: i32 = rounded_lg();
#[cfg(test)] field: Option<i32>,
"###;
    assert!(violations("src/desktop/example.rs", source).is_empty());
    assert!(violations("src/desktop/theme.rs", "rounded_xl()").is_empty());
    assert!(violations("src/desktop/theme/other.rs", "rounded_xl()").len() == 1);
    assert!(violations("src/desktop/example/accept.rs", "rounded_xl()").is_empty());
    assert_eq!(violations("src/desktop/example.rs", "#[cfg(test)] mod tests { rounded_xl(); }\nrounded_lg();"), ["src/desktop/example.rs:2"]);
}

#[test]
fn src_desktop_follows_decision_19() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    scan_tree(&root.join("src/desktop"), root, &mut found);
    found.sort();
    assert!(found.is_empty(), "decision 19 violations:\n{}", found.join("\n"));
}

const NOT_VERIFIED: &str = "## Not verified\n\n\
- Native minimise and restore of the main window when Mini opens.\n\
- Mini always-on-top on and off (no capture or test signal for a PopUp window).\n\
- Delete-confirmation title text (plain text not in snapshots).\n\
- Selection glow and focus-ring painting (no shadow in snapshots).\n\
- 60 s quota repaint.\n\
- Startup with a saved Mini.\n\
- Tooltip rendering.\n\
- Real-engine and native-window behaviour.";

const OLD_W4: &str = "W4 FAIL wishes-dark-1100x700.png - running wish title still ends in an ellipsis at 1100x700";
const OLD_W10: &str = "W10 FAIL usage-dark-1100x700.png - model-cost table not visible at 1100x700, below the warning panel";

fn section(doc: &str, heading: &str) -> String {
    let mut lines = doc.lines().skip_while(|line| *line != heading);
    let first = lines.next().expect("section heading missing");
    std::iter::once(first)
        .chain(lines.take_while(|line| !line.starts_with("## ")))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

fn not_verified_section(doc: &str) -> String {
    section(doc, "## Not verified")
}

fn capture_files(doc: &str) -> Vec<String> {
    let captures = section(doc, "## Captures");
    let mut lines = captures.lines().skip(2);
    assert_eq!(lines.next().map(str::trim), Some("| name | file |"));
    assert_eq!(lines.next().map(str::trim), Some("| --- | --- |"));
    let files: Vec<_> = lines.filter(|line| !line.trim().is_empty()).map(|line| {
        let cells: Vec<_> = line.split('|').map(str::trim).collect();
        assert!(cells.len() == 4 && cells[0].is_empty() && cells[3].is_empty(), "invalid capture row: {line}");
        let path = Path::new(cells[2]);
        assert!(path.is_absolute() && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("png")), "invalid capture path: {line}");
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), cells[1], "capture name mismatch: {line}");
        cells[2].to_owned()
    }).collect();
    assert!(!files.is_empty(), "capture table is empty");
    files
}

fn differs_from_old(line: &str, old: &str) -> bool {
    line != old
}

fn newer_than(candidate: SystemTime, baseline: SystemTime) -> bool {
    candidate > baseline
}

fn ui_after_doc() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/ui-after.md")).unwrap()
}

#[test]
fn not_verified_block_is_verbatim() {
    let doc = ui_after_doc();
    assert_eq!(not_verified_section(&doc), NOT_VERIFIED);
    let altered = doc.replace("- Tooltip rendering.", "- Tooltip painting.");
    assert_ne!(not_verified_section(&altered), NOT_VERIFIED);
}

#[test]
fn w4_and_w10_verdicts_are_rewritten_and_reference_captures() {
    let doc = ui_after_doc();
    let files = capture_files(&doc);
    let format = Regex::new(r"^(W4|W10) (PASS|FAIL) ([^ /\\]+\.png) - (\S.*)$").unwrap();
    let mut unchanged = Vec::new();
    for (id, old) in [("W4", OLD_W4), ("W10", OLD_W10)] {
        assert!(!differs_from_old(old, old), "negative control for {id}");
        let prefix = format!("{id} ");
        let lines: Vec<_> = doc.lines().filter(|line| line.starts_with(&prefix)).collect();
        assert_eq!(lines.len(), 1, "expected exactly one {id} line");
        let line = lines[0];
        let verdict = format.captures(line).unwrap_or_else(|| panic!("invalid verdict: {line}"));
        assert!(files.iter().any(|file| Path::new(file).file_name().unwrap() == &verdict[3]), "capture not in table: {line}");
        if !differs_from_old(line, old) { unchanged.push(id); }
    }
    assert!(unchanged.is_empty(), "unchanged verdicts: {}", unchanged.join(", "));
}

#[test]
fn after_captures_are_newer_than_all_before_captures() {
    let before = Path::new("C:/Users/gppup/.axiom/orchestrate/r6-app/captures/before/");
    let newest_before = fs::read_dir(before).expect("cannot read before captures")
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("png")))
        .map(|path| fs::metadata(path).unwrap().modified().unwrap())
        .max().expect("no before PNG captures");
    assert!(!newer_than(SystemTime::UNIX_EPOCH, SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1)));
    for file in capture_files(&ui_after_doc()) {
        let modified = fs::metadata(&file).unwrap_or_else(|err| panic!("cannot read {file}: {err}"))
            .modified().unwrap();
        assert!(newer_than(modified, newest_before), "capture is not newer than all before captures: {file}");
    }
}
