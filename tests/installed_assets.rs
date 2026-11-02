mod common;

use regex::Regex;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn relative_path(base: &str, reference: &str) -> Option<String> {
    if reference.contains('\\')
        || reference.starts_with('/')
        || reference
            .split('/')
            .next()
            .is_some_and(|part| part.contains(':'))
    {
        return None;
    }
    let mut parts: Vec<&str> = base.split('/').filter(|part| !part.is_empty()).collect();
    for part in reference.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.pop().is_some() => {}
            ".." => return None,
            _ => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

#[derive(Default)]
struct Scan {
    missing: BTreeSet<String>,
    malformed: BTreeSet<String>,
    referenced: BTreeSet<String>,
    plugin_references: usize,
    links: usize,
    md_files: usize,
    json_files: usize,
}

fn string_values<'a>(value: &'a serde_json::Value, strings: &mut Vec<&'a str>) {
    match value {
        serde_json::Value::String(value) => strings.push(value),
        serde_json::Value::Array(values) => {
            for value in values {
                string_values(value, strings);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                string_values(value, strings);
            }
        }
        _ => {}
    }
}

// Resolve lexically against the exact-case names of installed files.
fn scan(root: &Path) -> Scan {
    let plugin = Regex::new(r#"\$\{CLAUDE_PLUGIN_ROOT\}/([^\s`"'\)\]>]+)"#).unwrap();
    let links = Regex::new(r"\]\(([^)]+)\)").unwrap();
    let mut paths = Vec::new();
    files(root, &mut paths);
    let installed: BTreeSet<String> = paths
        .iter()
        .map(|path| {
            path.strip_prefix(root)
                .unwrap()
                .components()
                .map(|part| part.as_os_str().to_str().unwrap())
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect();
    let mut scan = Scan::default();
    for path in paths {
        let is_json = match path.extension().and_then(|s| s.to_str()) {
            Some("md") => {
                scan.md_files += 1;
                false
            }
            Some("json") => {
                scan.json_files += 1;
                true
            }
            _ => continue,
        };
        let text = fs::read_to_string(&path).unwrap();
        let json = if is_json {
            match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(value) => Some(value),
                Err(_) => {
                    scan.malformed.insert(
                        path.strip_prefix(root)
                            .unwrap()
                            .components()
                            .map(|part| part.as_os_str().to_str().unwrap())
                            .collect::<Vec<_>>()
                            .join("/"),
                    );
                    continue;
                }
            }
        } else {
            None
        };
        let mut strings = Vec::new();
        if let Some(value) = &json {
            string_values(value, &mut strings);
        } else {
            strings.push(text.as_str());
        }
        let parent = path
            .parent()
            .unwrap()
            .strip_prefix(root)
            .unwrap()
            .components()
            .map(|part| part.as_os_str().to_str().unwrap())
            .collect::<Vec<_>>()
            .join("/");
        let mut references = Vec::new();
        for text in strings {
            for capture in plugin.captures_iter(text) {
                scan.plugin_references += 1;
                references.push(("", capture[1].to_owned()));
            }
            for capture in links.captures_iter(text) {
                let link = capture[1]
                    .trim()
                    .trim_start_matches('<')
                    .trim_end_matches('>');
                if link.starts_with("http://")
                    || link.starts_with("https://")
                    || link.starts_with('#')
                {
                    continue;
                }
                scan.links += 1;
                references.push((parent.as_str(), link.split('#').next().unwrap().to_owned()));
            }
        }
        for (base, reference) in references {
            match relative_path(base, &reference) {
                Some(relative) if installed.contains(&relative) => {
                    scan.referenced.insert(relative);
                }
                Some(relative) => {
                    scan.missing.insert(relative);
                }
                None => {
                    scan.missing.insert(format!("{base}/{reference}"));
                }
            }
        }
    }
    scan
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let destination = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            fs::copy(entry.path(), destination).unwrap();
        }
    }
}

#[test]
fn binary_installs_all_referenced_files() {
    let temp = common::temp();
    let home = temp.path().join("home");
    let data = temp.path().join("data");
    let output = Command::new(env!("CARGO_BIN_EXE_axiom"))
        .args(["wish", "check installed assets"])
        .env("AXIOM_HOME", &home)
        .env("AXIOM_DATA_HOME", &data)
        .env("FAKE_CLAUDE_RECORD", temp.path().join("claude.json"))
        .env(
            "AXIOM_CLAUDE_CMD",
            serde_json::to_string(&common::fake("fake_claude")).unwrap(),
        )
        .env_remove("ORCH_STORE")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let installed = home.join("engine").join(env!("CARGO_PKG_VERSION"));
    let result = scan(&installed);
    assert_eq!(result.missing, BTreeSet::new());
    assert_eq!(result.malformed, BTreeSet::new());
    assert!(result.plugin_references > 0);
    assert!(result.md_files > 0);
    assert!(result.json_files > 0);
    let removed = result
        .referenced
        .iter()
        .next()
        .expect("no referenced installed files");

    let copy = temp.path().join("copy");
    copy_tree(&installed, &copy);
    fs::remove_file(copy.join(removed)).unwrap();
    let result = scan(&copy);
    assert_eq!(result.missing, BTreeSet::from([removed.clone()]));
    assert_eq!(result.malformed, BTreeSet::new());
}

#[test]
fn checker_decodes_json_strings_and_reports_malformed_json() {
    let temp = common::temp();
    fs::create_dir(temp.path().join("sub")).unwrap();
    fs::write(temp.path().join("sub/present.md"), "").unwrap();
    fs::write(
        temp.path().join("references.json"),
        r#"{"nested":[{"present":"${CLAUDE_PLUGIN_ROOT}\/sub\/present.md","absent":"${CLAUDE_PLUGIN_ROOT}\/sub\/absent.md","backslash":"[x](sub\\present.md)"}]}"#,
    ).unwrap();
    fs::write(temp.path().join("malformed.json"), r#"{"a":"#).unwrap();
    let result = scan(temp.path());
    assert_eq!(
        result.referenced,
        BTreeSet::from(["sub/present.md".to_owned()])
    );
    assert_eq!(
        result.missing,
        BTreeSet::from(["sub/absent.md".to_owned(), "/sub\\present.md".to_owned(),])
    );
    assert_eq!(
        result.malformed,
        BTreeSet::from(["malformed.json".to_owned()])
    );
}

#[test]
fn checker_reports_broken_references_in_both_file_types() {
    let temp = common::temp();
    fs::write(
        temp.path().join("broken.md"),
        "${CLAUDE_PLUGIN_ROOT}/missing-md-plugin [link](missing-md-link)",
    )
    .unwrap();
    fs::write(temp.path().join("broken.json"),
        r#"{"plugin":"${CLAUDE_PLUGIN_ROOT}/missing-json-plugin","link":"[link](missing-json-link)"}"#).unwrap();
    assert_eq!(
        scan(temp.path()).missing,
        BTreeSet::from([
            "missing-md-plugin".to_owned(),
            "missing-md-link".to_owned(),
            "missing-json-plugin".to_owned(),
            "missing-json-link".to_owned(),
        ])
    );
}

#[test]
fn checker_uses_exact_case_and_rejects_escaping_paths() {
    let temp = common::temp();
    fs::write(temp.path().join("Target.txt"), "").unwrap();
    fs::write(
        temp.path().join("references.md"),
        "[case](target.txt) [escape](../outside) [absolute](/outside) [backslash](dir\\outside)",
    )
    .unwrap();
    assert_eq!(
        scan(temp.path()).missing,
        BTreeSet::from([
            "target.txt".to_owned(),
            "/../outside".to_owned(),
            "//outside".to_owned(),
            "/dir\\outside".to_owned(),
        ])
    );
}
