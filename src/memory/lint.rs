//! Deterministic checks of written claims: no model involved, no tokens spent.
//! A quote line has the form `> "exact text" - <relative path>`; the text must
//! appear in that file after whitespace and line-ending normalisation.
use regex::Regex;
use std::path::{Component, Path};

pub struct Problem {
    pub kind: &'static str,
    pub message: String,
}

/// Collapses every run of whitespace (CRLF included) to one space.
pub fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Checks every quote line of `text` against its source under `root`.
/// `label` names the text in messages, e.g. `body` gives `body line 3`.
pub fn check_quotes(text: &str, root: &Path, label: &str) -> Vec<Problem> {
    let re = Regex::new(r#"^\s*>\s*["\u{201c}](.+)["\u{201d}]\s+[-\u{2013}\u{2014}]\s+(.+?)\s*$"#)
        .expect("constant regex");
    let mut problems = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let Some(c) = re.captures(line) else { continue };
        let (quote, source) = (&c[1], &c[2]);
        let place = format!("{label} line {}", n + 1);
        let rel = Path::new(source);
        if rel.is_absolute()
            || rel
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_) | Component::RootDir))
        {
            problems.push(Problem {
                kind: "invalid_source",
                message: format!("{place}: quote source must be a relative path inside the project: {source}"),
            });
            continue;
        }
        let path = root.join(rel);
        let Ok(real) = std::fs::canonicalize(&path) else {
            problems.push(Problem {
                kind: "missing_source",
                message: format!("{place}: quote source not found: {source}"),
            });
            continue;
        };
        // A link inside the project may lead out of it: the real file must still be inside.
        if !std::fs::canonicalize(root).is_ok_and(|root| real.starts_with(root)) {
            problems.push(Problem {
                kind: "invalid_source",
                message: format!("{place}: quote source must be a relative path inside the project: {source}"),
            });
            continue;
        }
        let Ok(bytes) = std::fs::read(&real) else {
            problems.push(Problem {
                kind: "missing_source",
                message: format!("{place}: quote source not found: {source}"),
            });
            continue;
        };
        let file = normalize(&String::from_utf8_lossy(&bytes));
        if !file.contains(&normalize(quote)) {
            problems.push(Problem {
                kind: "fabricated_quote",
                message: format!("{place}: quote not found in {source}: \"{quote}\""),
            });
        }
    }
    problems
}
