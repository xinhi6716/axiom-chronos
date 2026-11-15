use std::{
    fs,
    path::{Path, PathBuf},
};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .replace("\r\n", "\n")
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

fn contains(bytes: &[u8], needle: &str) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle.as_bytes())
}

fn hits(needle: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    files(&root().join("src"), &mut paths);
    paths.sort();
    paths
        .into_iter()
        .filter(|p| contains(&fs::read(p).unwrap(), needle))
        .collect()
}

// The single occurrence is the retained enum variant, not a routing rule produced by code.
fn routing_ok(text: &str) -> bool {
    text.matches("LegacyRole").count() == 1
}

fn for_attempt_body(text: &str) -> &str {
    let parsed = syn::parse_file(text).expect("parse routing source");
    let functions: Vec<_> = parsed.items.iter().filter_map(|item| match item {
        syn::Item::Fn(f) if f.sig.ident == "for_attempt" => Some(f),
        _ => None,
    }).collect();
    assert_eq!(functions.len(), 1, "expected exactly one for_attempt");
    let braces = functions[0].block.brace_token.span;
    let start = braces.open().start();
    let end = braces.close().end();
    let offset = |line: usize, column: usize| {
        text.split_inclusive('\n').take(line - 1).map(str::len).sum::<usize>() + column
    };
    &text[offset(start.line, start.column)..offset(end.line, end.column)]
}

fn worker_read(body: &str) -> bool {
    regex::Regex::new(r"\.\s*worker\b").unwrap().is_match(body)
}

fn prompt_total() -> usize {
    [
        "engine/agents/dispatcher.md",
        "engine/DISPATCH.md",
        "engine/skills/wish/SKILL.md",
    ]
    .iter()
    .map(|p| read(&root().join(p)).len())
    .sum()
}

fn prompt_ok(total: usize) -> bool {
    total <= 20_758
}

fn version_ok(cargo: &str) -> bool {
    let package = cargo.lines().skip_while(|line| line.trim() != "[package]").skip(1)
        .take_while(|line| !line.trim_start().starts_with('['));
    let versions: Vec<_> = package.filter_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "version").then_some(value.trim())
    }).collect();
    versions == ["\"3.0.0-beta.1\""]
}

#[test]
fn no_sol_gate_in_src() {
    let found = hits("sol_not_allowed");
    assert!(found.is_empty(), "{found:?}");
    assert!(contains(b"let error = sol_not_allowed;", "sol_not_allowed"));
}

#[test]
fn background_tasks_never_disabled_in_src() {
    let needle = "\"CLAUDE_CODE_DISABLE_BACKGROUND_TASKS\", \"1\"";
    let found = hits(needle);
    assert!(found.is_empty(), "{found:?}");
    assert!(contains(b".env(\"CLAUDE_CODE_DISABLE_BACKGROUND_TASKS\", \"1\")", needle));
}

#[test]
fn routing_never_produces_legacy_role() {
    let routing = read(&root().join("src/codex/routing.rs"));
    assert!(routing_ok(&routing), "LegacyRole must occur only as the retained enum variant");
    assert!(!routing_ok("enum Rule { LegacyRole }\nlet r = Rule::LegacyRole;"));
    let found = hits("Rule::LegacyRole");
    assert!(found.is_empty(), "{found:?}");
    assert!(contains(b"let r = Rule::LegacyRole;", "Rule::LegacyRole"));
}

#[test]
fn for_attempt_ignores_worker_setting() {
    let routing = read(&root().join("src/codex/routing.rs"));
    assert!(!worker_read(for_attempt_body(&routing)), "worker setting read");
    let bad = "pub fn for_attempt() { let m = settings.models\n .worker.get(x); }\nfn other() {}";
    assert!(worker_read(for_attempt_body(bad)));
    let outside = "pub fn for_attempt() {}\nfn other() { settings.models.worker; }";
    assert!(!worker_read(for_attempt_body(outside)));
}

#[test]
fn always_loaded_prompts_within_20758() {
    let total = prompt_total();
    assert!(prompt_ok(total), "always-loaded prompts: {total} LF bytes, limit 20758");
    assert!(prompt_ok(20_758));
    assert!(!prompt_ok(20_759));
}

#[test]
fn package_version_is_the_release_version() {
    let cargo = read(&root().join("Cargo.toml"));
    assert!(version_ok(&cargo), "package version must be 3.0.0-beta.1");
    assert!(!version_ok("[package]\nversion = \"2.0.1\"\n[dependencies]\nversion = \"3.0.0-beta.1\""));
}
