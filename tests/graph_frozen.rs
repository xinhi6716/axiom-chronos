use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

const FROZEN_HASH: &str = "28333a0dafc0dcf6e2e01498634ee15f0d2d7f35aa21da603a9d45dc90195e3f";
const LANGUAGES: [&str; 4] = ["rust", "python", "ts", "js"];

type Check = Result<(), String>;

fn normalized(bytes: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            i += 1;
        }
        result.push(bytes[i]);
        i += 1;
    }
    result
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(normalized(bytes)))
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn read_json(path: &Path) -> Result<Value, String> {
    serde_json::from_slice(&read(path)?).map_err(|e| format!("{}: {e}", path.display()))
}

fn safe_path(base: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if relative.is_empty()
        || relative.contains('\\')
        || !path.components().all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(format!("invalid relative path: {relative}"));
    }
    Ok(base.join(path))
}

fn collect_files(base: &Path, dir: &Path, result: &mut BTreeMap<String, Vec<u8>>) -> Check {
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            collect_files(base, &path, result)?;
        } else if kind.is_file() {
            let relative = path
                .strip_prefix(base)
                .map_err(|e| e.to_string())?
                .to_str()
                .ok_or("non-UTF-8 path")?
                .replace('\\', "/");
            result.insert(relative, read(&path)?);
        } else {
            return Err(format!("unexpected filesystem entry: {}", path.display()));
        }
    }
    Ok(())
}

fn check_hashes(frozen: &[u8], expected_hash: &str, files: &BTreeMap<String, Vec<u8>>) -> Check {
    if hash(frozen) != expected_hash {
        return Err("FROZEN.sha256 digest differs".into());
    }
    let bytes = normalized(frozen);
    let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
    if !text.ends_with('\n') {
        return Err("hash list has no final newline".into());
    }
    let mut listed = BTreeMap::new();
    let mut previous: Option<&str> = None;
    for line in text.lines() {
        let (digest, path) = line.split_once("  ").ok_or("invalid hash list line")?;
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !(path.starts_with("eval/") || path.starts_with("holdout/"))
        {
            return Err(format!("invalid hash entry: {line}"));
        }
        safe_path(Path::new(""), path)?;
        if previous.is_some_and(|p| p >= path) {
            return Err("hash list not strictly sorted".into());
        }
        previous = Some(path);
        listed.insert(path, digest);
    }
    if listed.len() != files.len() {
        return Err("hash list and fixture files differ".into());
    }
    for (path, bytes) in files {
        let expected = listed
            .get(path.as_str())
            .ok_or_else(|| format!("unlisted file: {path}"))?;
        if &hash(bytes) != expected {
            return Err(format!("hash mismatch: {path}"));
        }
    }
    Ok(())
}

fn check_manifest(manifest: &Value) -> Check {
    if manifest["version"] != 1 || manifest["frozen"] != "2026-11-12" {
        return Err("manifest version/date differs".into());
    }
    let design = json!({"arms":["A","B"],"runs_per_arm":3,"A":"today's brief: the recorded briefs/<id>.a.md","B":"graph brief: constraints block + graph_explore mixed packet","fixed":["model","prompt","snapshot","tool permissions","cache conditions"]});
    let metric = json!({"claude_processed_input":["input","cache_write","cache_read"],"aggregate":"median over scenarios of (1 - median(B runs)/median(A runs))"});
    let thresholds = json!({"claude_processed_input_median_saving_min":0.10,"required_evidence_regressions_max":0,"condition_regressions_max":0,"major_finding_regressions_max":0,"wrong_certain_edges_max":0,"budget_overruns_max":0,"resolution_recall_min":0.70});
    if manifest["design"] != design
        || manifest["metric"] != metric
        || manifest["thresholds"] != thresholds
    {
        return Err("design, metric, or thresholds differ".into());
    }
    let scenarios = manifest["scenarios"]
        .as_array()
        .ok_or("scenarios is not an array")?;
    if scenarios.len() != 12 {
        return Err("expected 12 scenarios".into());
    }
    let mut counts = BTreeMap::new();
    let mut research = 0;
    let mut chinese_queries = 0;
    for (index, scenario) in scenarios.iter().enumerate() {
        if scenario["id"] != format!("s{:02}", index + 1) {
            return Err("unexpected scenario id/order".into());
        }
        let language = scenario["language"].as_str().ok_or("missing language")?;
        if !LANGUAGES.contains(&language) {
            return Err("unknown scenario language".into());
        }
        *counts.entry(language).or_insert(0) += 1;
        match scenario["family"].as_str() {
            Some("research") => research += 1,
            Some("software") => (),
            _ => return Err("unknown scenario family".into()),
        }
        let query = scenario["query"].as_str().ok_or("missing query")?;
        if query.chars().count() == 2
            && query
                .chars()
                .all(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
        {
            chinese_queries += 1;
        }
    }
    if LANGUAGES
        .iter()
        .any(|l| counts.get(l).copied().unwrap_or(0) < 3)
        || research < 4
        || chinese_queries < 2
    {
        return Err("scenario coverage insufficient".into());
    }
    Ok(())
}

fn check_scenario_files(eval: &Path, manifest: &Value) -> Check {
    for s in manifest["scenarios"]
        .as_array()
        .ok_or("missing scenarios")?
    {
        let get = |key: &str| {
            s[key]
                .as_str()
                .ok_or_else(|| format!("missing scenario {key}"))
        };
        let repo = safe_path(eval, get("repo")?)?;
        if !repo.is_dir() {
            return Err(format!("missing repo: {}", repo.display()));
        }
        read(&safe_path(eval, get("a_brief")?)?)?;
        let seed = read_json(&safe_path(eval, get("seed")?)?)?;
        if s["family"] == "research" {
            let artifacts = seed["research"].as_array().ok_or("missing research")?;
            let linked = artifacts.iter().any(|artifact| {
                let Some(text) = artifact["text"].as_str() else {
                    return false;
                };
                artifact["links"].as_array().is_some_and(|links| {
                    links.iter().any(|link| {
                        link["to"]
                            .as_str()
                            .is_some_and(|to| to.starts_with("sym:") && to.len() > 4)
                            && link["quote"]
                                .as_str()
                                .is_some_and(|quote| !quote.is_empty() && text.contains(quote))
                            && link["line"].as_u64().is_some_and(|line| {
                                line > 0
                                    && text.lines().nth(line as usize - 1).is_some_and(|s| {
                                        s.contains(link["quote"].as_str().unwrap_or(""))
                                    })
                            })
                    })
                })
            });
            if !linked {
                return Err("research artifact lacks a quote-linked symbol".into());
            }
        }
    }
    Ok(())
}

fn check_counts(counts: &BTreeMap<String, usize>) -> Check {
    if LANGUAGES
        .iter()
        .any(|lang| counts.get(*lang).copied().unwrap_or(0) < 30)
    {
        return Err("holdout coverage insufficient".into());
    }
    Ok(())
}

fn check_case(dir: &Path, language: &str) -> Check {
    let case = read_json(&dir.join("case.json"))?;
    let id = dir
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("invalid case directory")?;
    if case["id"] != id
        || case["language"] != language
        || case["feature"].as_str().is_none_or(str::is_empty)
    {
        return Err(format!("bad case metadata: {id}"));
    }
    let refs = case["refs"]
        .as_array()
        .filter(|r| !r.is_empty())
        .ok_or("empty refs")?;
    for reference in refs {
        let base = dir.join("files");
        let filename = reference["file"].as_str().ok_or("missing ref file")?;
        let text =
            String::from_utf8(read(&safe_path(&base, filename)?)?).map_err(|e| e.to_string())?;
        let line = reference["line"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or("invalid ref line")?;
        let name = reference["name"]
            .as_str()
            .filter(|n| !n.is_empty())
            .ok_or("missing ref name")?;
        let nth = reference["nth"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or("invalid nth")?;
        let source = text
            .lines()
            .nth(line as usize - 1)
            .ok_or("ref line does not exist")?;
        if source.match_indices(name).count() < nth as usize {
            return Err(format!("ref {name} missing at {id}/{filename}:{line}"));
        }
        match reference["expect"]["resolution"].as_str() {
            Some("resolved") => {
                let target = reference["expect"]["target"]
                    .as_str()
                    .ok_or("missing target")?;
                let (path, symbol) = target.split_once('#').ok_or("bad target")?;
                let (kind, qualname) = symbol.split_once(':').ok_or("bad target symbol")?;
                if ![
                    "function",
                    "method",
                    "class",
                    "struct",
                    "enum",
                    "trait",
                    "interface",
                    "type",
                    "const",
                    "static",
                    "variable",
                    "module",
                    "macro",
                    "namespace",
                ]
                .contains(&kind)
                    || qualname.is_empty()
                {
                    return Err("bad target kind or name".into());
                }
                if !safe_path(&base, path)?.is_file() {
                    return Err(format!("missing target file: {target}"));
                }
            }
            Some("ambiguous" | "unresolved") => {
                if reference["expect"].get("target").is_some() {
                    return Err("unexpected target".into());
                }
            }
            _ => return Err("unknown resolution".into()),
        }
    }
    Ok(())
}

fn check_holdout(root: &Path) -> Check {
    let mut counts = BTreeMap::new();
    for language in LANGUAGES {
        let mut count = 0;
        for entry in fs::read_dir(root.join(language)).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                return Err("unexpected holdout entry".into());
            }
            check_case(&entry.path(), language)?;
            count += 1;
        }
        counts.insert(language.to_string(), count);
    }
    check_counts(&counts)
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/graph")
}

#[test]
fn frozen_contract() -> Check {
    let root = fixtures();
    let mut files = BTreeMap::new();
    for dir in ["eval", "holdout"] {
        collect_files(&root, &root.join(dir), &mut files)?;
    }
    check_hashes(&read(&root.join("FROZEN.sha256"))?, FROZEN_HASH, &files)?;
    let manifest = read_json(&root.join("eval/manifest.json"))?;
    check_manifest(&manifest)?;
    check_scenario_files(&root.join("eval"), &manifest)?;
    check_holdout(&root.join("holdout/resolution"))
}

#[test]
fn rejects_changed_hash() {
    let mut files = BTreeMap::new();
    files.insert("eval/a".to_string(), b"modified".to_vec());
    let list = format!("{}  eval/a\n", hash(b"original"));
    assert!(check_hashes(list.as_bytes(), &hash(list.as_bytes()), &files).is_err());
}

#[test]
fn rejects_changed_threshold() {
    let mut manifest = read_json(&fixtures().join("eval/manifest.json")).unwrap();
    manifest["thresholds"]["resolution_recall_min"] = json!(0.69);
    assert!(check_manifest(&manifest).is_err());
}

#[test]
fn rejects_eleven_scenarios() {
    let mut manifest = read_json(&fixtures().join("eval/manifest.json")).unwrap();
    manifest["scenarios"].as_array_mut().unwrap().pop();
    assert!(check_manifest(&manifest).is_err());
}

#[test]
fn rejects_twenty_nine_cases() {
    let counts = LANGUAGES
        .iter()
        .map(|l| (l.to_string(), if *l == "rust" { 29 } else { 30 }))
        .collect();
    assert!(check_counts(&counts).is_err());
}

#[test]
fn rejects_unlisted_file() {
    let mut files = BTreeMap::new();
    files.insert("eval/a".to_string(), b"a".to_vec());
    files.insert("eval/b".to_string(), b"b".to_vec());
    let list = format!("{}  eval/a\n", hash(b"a"));
    assert!(check_hashes(list.as_bytes(), &hash(list.as_bytes()), &files).is_err());
}
