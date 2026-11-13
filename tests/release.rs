// The shipped-set scan requires git on PATH and a git checkout.
use std::fs;
use std::path::Path;
use std::path::{Component, PathBuf};
use std::collections::BTreeMap;
use std::process::Command;

use regex::{bytes::Regex as BytesRegex, Regex};
use serde::Deserialize;

fn no_python(path: &Path) {
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            no_python(&path);
        } else {
            assert_ne!(
                path.extension().and_then(|s| s.to_str()),
                Some("py"),
                "{}",
                path.display()
            );
        }
    }
}

#[test]
fn rust_release_contract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    no_python(&root.join("src"));
    assert!(!root.join("pyproject.toml").exists());
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    for required in [
        "ubuntu-latest",
        "windows-latest",
        "setup-python",
        "cargo test --locked",
    ] {
        assert!(ci.contains(required), "CI missing {required}");
    }
    for name in ["README.md", "README.zh-TW.md"] {
        let readme = fs::read_to_string(root.join(name)).unwrap();
        let lower = readme.to_lowercase();
        for forbidden in ["pipx", "bun", "axiom_python", "--model", "`60`", "beta", "測試版"] {
            assert!(!lower.contains(forbidden), "{name} contains {forbidden}");
        }
        for required in ["2.0.0", "interjection"] {
            assert!(lower.contains(required), "{name} missing {required}");
        }
    }
    let bridge = fs::read_to_string(root.join("docs/bridge.md")).unwrap();
    assert!(bridge.contains("axiom bridge"));
    assert!(!bridge.contains("orch.ts"));
}

// The boolean selects printable binary strings and byte offsets instead of text lines.
type ScanFiles = BTreeMap<PathBuf, (bool, Option<&'static str>)>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Allow {
    path: String,
    pattern: String,
    reason: String,
}

struct Hit {
    path: String,
    pattern: &'static str,
    report: String,
}

fn patterns() -> Vec<(&'static str, BytesRegex)> {
    [
        ("gppup", r"gppup"),
        ("user-path", r"(?:file:///C:/Users/|C:/Users/|C:\\\\Users\\\\|C:\\Users\\|/c/Users/|/mnt/c/Users/)"),
        ("sk", r"sk-[A-Za-z0-9_-]{20,}"),
        ("sk-ant", r"sk-ant-"),
        ("ghp", r"ghp_"),
        ("github-pat", r"github_pat_"),
        ("akia", r"AKIA[0-9A-Z]{16}"),
        ("aiza", r"AIza[0-9A-Za-z_-]{35}"),
        ("slack", r"xox[abprs]-"),
        ("private-key", r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
        ("bearer", r"bearer\s+[A-Za-z0-9._~+/-]{20,}=*"),
    ]
    .into_iter()
    .map(|(id, pattern)| (id, BytesRegex::new(&format!("(?i:{pattern})")).unwrap()))
    .collect()
}

fn normalized(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => (),
            Component::ParentDir => assert!(result.pop(), "path escapes repository"),
            _ => result.push(part.as_os_str()),
        }
    }
    result
}

fn relative_name(path: &Path) -> String {
    path.to_str().expect("non-UTF-8 file path").replace('\\', "/")
}

fn collect_sources(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_sources(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

fn embedded_files(root: &Path) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    collect_sources(&root.join("src"), &mut sources);
    let macros = Regex::new(r"\binclude_(?:str|bytes)!\s*\(\s*([^)]*)\)").unwrap();
    let mut result = Vec::new();
    for source in sources {
        let text = fs::read_to_string(&source).unwrap();
        for found in macros.captures_iter(&text) {
            let argument = found[1].trim().trim_end_matches(',').trim();
            let target: String = serde_json::from_str(argument)
                .unwrap_or_else(|_| panic!("non-literal include target in {}", source.display()));
            let parent = source.parent().unwrap().strip_prefix(root).unwrap();
            result.push(normalized(&parent.join(target)));
        }
    }
    result
}

fn payload_files(root: &Path) -> Vec<PathBuf> {
    let source = fs::read_to_string(root.join("tools/package_release.py")).unwrap();
    let literal = Regex::new(r"(?s)\bPAYLOAD_FILES\s*=\s*(\[[^\]]*\])").unwrap();
    let capture = literal.captures(&source).expect("PAYLOAD_FILES literal missing");
    let files: Vec<String> = serde_json::from_str(&capture[1]).expect("invalid PAYLOAD_FILES literal");
    assert!(!files.is_empty(), "empty PAYLOAD_FILES");
    files.into_iter().map(|file| {
        let path = PathBuf::from(file);
        assert!(!path.is_absolute());
        normalized(&path)
    }).collect()
}

fn shipped_files(root: &Path) -> ScanFiles {
    let output = Command::new("git").current_dir(root).args([
        "ls-files", "-z", "--", "docs/", "engine/", "assets/", "examples/", "licenses/",
        "README*", "LICENSE", "CHANGELOG.md",
    ]).output().expect("git ls-files failed");
    assert!(output.status.success(), "git ls-files failed");
    let mut files = ScanFiles::new();
    for name in output.stdout.split(|byte| *byte == 0).filter(|name| !name.is_empty()) {
        files.insert(PathBuf::from(std::str::from_utf8(name).unwrap()), (false, None));
    }
    for file in embedded_files(root).into_iter().chain(payload_files(root)) {
        files.entry(file).or_insert((false, None));
    }
    files
}

fn scan(root: &Path, files: &ScanFiles) -> Vec<Hit> {
    let patterns = patterns();
    let mut hits = Vec::new();
    for (path, (binary, label)) in files {
        let data = fs::read(root.join(path)).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let name = label.map(str::to_owned).unwrap_or_else(|| relative_name(path));
        // Non-text assets are also scanned as printable ASCII strings.
        let binary = *binary || std::str::from_utf8(&data).is_err() || data.contains(&0);
        let spans: Vec<(usize, &[u8])> = if binary {
            let mut spans = Vec::new();
            let mut start = 0;
            while start < data.len() {
                if !(0x20..=0x7e).contains(&data[start]) { start += 1; continue; }
                let mut end = start + 1;
                while end < data.len() && (0x20..=0x7e).contains(&data[end]) { end += 1; }
                if end - start >= 6 { spans.push((start, &data[start..end])); }
                start = end;
            }
            spans
        } else {
            vec![(0, data.as_slice())]
        };
        for (offset, span) in spans {
            for (id, pattern) in &patterns {
                for found in pattern.find_iter(span) {
                    let location = if binary { offset + found.start() } else {
                        1 + span[..found.start()].iter().filter(|byte| **byte == b'\n').count()
                    };
                    let prefix = &found.as_bytes()[..found.len().min(4)];
                    let masked = format!("{}***", String::from_utf8_lossy(prefix));
                    hits.push(Hit { path: name.clone(), pattern: id,
                        report: format!("{name}:{location} {id} {masked}") });
                }
            }
        }
    }
    hits
}

fn assert_clean(root: &Path, files: &ScanFiles) {
    let allow: Vec<Allow> = serde_json::from_slice(&fs::read(root.join("tests/fixtures/release_scan/allow.json")).unwrap()).unwrap();
    for entry in &allow {
        assert!(!entry.path.trim().is_empty() && !entry.pattern.trim().is_empty() && !entry.reason.trim().is_empty(), "invalid allow entry");
    }
    let unhandled: Vec<String> = scan(root, files).into_iter()
        .filter(|hit| !allow.iter().any(|entry| entry.path == hit.path && entry.pattern == hit.pattern))
        .map(|hit| hit.report).collect();
    assert!(unhandled.is_empty(), "unhandled release scan hits:\n{}", unhandled.join("\n"));
}

#[test]
fn shipped_set_is_clean() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = shipped_files(root);
    let binary = Path::new(env!("CARGO_BIN_EXE_axiom"));
    files.insert(binary.to_path_buf(), (true, Some("bin/axiom")));
    assert_clean(root, &files);
}

#[test]
#[ignore]
fn release_binaries_clean() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = ScanFiles::new();
    for path in payload_files(root) { files.insert(path, (false, None)); }
    for (name, label) in [("target/release/chronos.exe", "bin/chronos"), ("target/release/axiom.exe", "bin/axiom")] {
        files.insert(PathBuf::from(name), (true, Some(label)));
    }
    assert_clean(root, &files);
}

#[test]
fn scan_negative_controls() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), "const A: &str = include_str!(\"asset.txt\");").unwrap();
    let cases = [
        ("gppup", format!("{}{}", "gp", "pup-control")),
        ("user-path", format!("{}{}", "C:/Us", "ers/alice")),
        ("user-path", format!("{}{}", "C:\\Us", "ers\\alice")),
        ("user-path", format!("{}{}", "C:\\\\Us", "ers\\\\alice")),
        ("user-path", format!("{}{}", "/c/Us", "ers/alice")),
        ("user-path", format!("{}{}", "/mnt/c/Us", "ers/alice")),
        ("user-path", format!("{}{}", "file:///C:/Us", "ers/alice")),
        ("sk", format!("{}{}", "sk-", "a".repeat(20))),
        ("sk-ant", ["sk-", "ant-"].concat()),
        ("ghp", ["gh", "p_control"].concat()),
        ("github-pat", ["github", "_pat_"].concat()),
        ("akia", format!("{}{}", "AK", "IA1234567890123456")),
        ("aiza", format!("{}{}", "AI", format!("za{}", "a".repeat(35)))),
        ("slack", ["xo", "xb-"].concat()),
        ("private-key", ["-----BE", "GIN RSA PRIVATE KEY-----"].concat()),
        ("bearer", format!("{}{}", "bear", format!("er {}", "a".repeat(20)))),
    ];
    let cases: Vec<_> = cases.into_iter().flat_map(|(id, value)| {
        let upper = value.to_ascii_uppercase();
        [(id, value), (id, upper)]
    }).collect();
    fs::write(root.join("doc.md"), cases.iter().map(|(_, value)| value.as_str()).collect::<Vec<_>>().join("\n")).unwrap();
    fs::write(root.join("src/asset.txt"), &cases[14].1).unwrap();
    fs::write(root.join("fake.bin"), format!("\0{}\0", cases[2].1)).unwrap();
    fs::write(root.join("short.bin"), b"\0ghp_\0").unwrap();
    fs::create_dir_all(root.join("tools")).unwrap();
    fs::write(root.join("tools/package_release.py"), "PAYLOAD_FILES = [\"payload.txt\"]\n").unwrap();
    fs::write(root.join("payload.txt"), &cases[1].1).unwrap();
    let mut files = ScanFiles::new();
    files.insert(PathBuf::from("doc.md"), (false, None));
    files.insert(PathBuf::from("fake.bin"), (true, None));
    files.insert(PathBuf::from("short.bin"), (true, None));
    for path in embedded_files(root).into_iter().chain(payload_files(root)) { files.insert(path, (false, None)); }
    let hits = scan(root, &files);
    let patterns = patterns();
    let check = |path: &str, location: usize, id: &str, value: &str| {
        let pattern = &patterns.iter().find(|(name, _)| *name == id).unwrap().1;
        let found = pattern.find(value.as_bytes()).expect("control does not match");
        let masked = format!("{}***", std::str::from_utf8(&found.as_bytes()[..found.len().min(4)]).unwrap());
        let hit = hits.iter().find(|hit| hit.path == path && hit.pattern == id &&
            hit.report.starts_with(&format!("{path}:{location} {id} "))).expect("missing control hit");
        assert_eq!(hit.report.rsplit_once(' ').unwrap().1, masked);
        assert!(!hit.report.contains(value), "unmasked control in report");
    };
    for (index, (id, value)) in cases.iter().enumerate() {
        check("doc.md", index + 1, id, value);
    }
    check("src/asset.txt", 1, cases[14].0, &cases[14].1);
    check("fake.bin", 1, cases[2].0, &cases[2].1);
    check("payload.txt", 1, cases[1].0, &cases[1].1);
    assert!(!hits.iter().any(|hit| hit.path == "short.bin"));
}

#[test]
fn binary_labels_and_allow_entries_are_independent_of_location() {
    let temp = tempfile::tempdir().unwrap();
    let binaries = tempfile::tempdir().unwrap();
    let root = temp.path();
    let allow_path = "tests/fixtures/release_scan/allow.json";
    fs::create_dir_all(root.join("tests/fixtures/release_scan")).unwrap();
    fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join(allow_path), root.join(allow_path)).unwrap();
    let mut files = ScanFiles::new();
    for (name, label) in [("arbitrary-profile/axiom", "bin/axiom"), ("other/chronos.exe", "bin/chronos")] {
        let path = binaries.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"\0ghp_detector github_pat_detector\0").unwrap();
        files.insert(path, (true, Some(label)));
    }
    let hits = scan(root, &files);
    for label in ["bin/axiom", "bin/chronos"] {
        for pattern in ["ghp", "github-pat"] {
            assert!(hits.iter().any(|hit| hit.path == label && hit.pattern == pattern && hit.report.starts_with(&format!("{label}:"))));
        }
    }
    assert_clean(root, &files);
}
