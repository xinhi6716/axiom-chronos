use crate::base::{Failure, Res};
use crate::orch::{derive_state, evidence, needs_accept, read_units, Severity, Verdict};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub const MAX_REPLY_BYTES: usize = 8192;
pub const MAX_FAILING_LINES: usize = 20;
pub const MAX_FAILING_BYTES: usize = 2048;
pub const MAX_PATH_SAMPLE: usize = 20;
pub const MAX_REASON_CHARS: usize = 240;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TestCounts {
    pub passed: u64,
    pub failed: u64,
    pub ignored: u64,
}

pub fn test_counts(output: &str) -> Option<TestCounts> {
    static CARGO: OnceLock<Regex> = OnceLock::new();
    static PYTEST: OnceLock<Regex> = OnceLock::new();
    static COUNTS: OnceLock<Regex> = OnceLock::new();
    let cargo = CARGO.get_or_init(|| {
        Regex::new(r"test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;")
            .unwrap()
    });
    let pytest =
        PYTEST.get_or_init(|| Regex::new(r"^\s*=*\s*.*\bin\s+\d+(?:\.\d+)?s\s*=*\s*$").unwrap());
    let counts =
        COUNTS.get_or_init(|| Regex::new(r"\b(\d+)\s+(passed|failed|errors?|skipped)\b").unwrap());
    let mut total = TestCounts::default();
    let mut found = false;
    let mut last_pytest = None;
    for line in output.lines() {
        for c in cargo.captures_iter(line) {
            if let (Ok(passed), Ok(failed), Ok(ignored)) = (
                c[1].parse::<u64>(),
                c[2].parse::<u64>(),
                c[3].parse::<u64>(),
            ) {
                total.passed = total.passed.saturating_add(passed);
                total.failed = total.failed.saturating_add(failed);
                total.ignored = total.ignored.saturating_add(ignored);
                found = true;
            }
        }
        if !line.trim_start().starts_with("test result:") && pytest.is_match(line) {
            let mut current = TestCounts::default();
            let mut any = false;
            for c in counts.captures_iter(line) {
                if let Ok(n) = c[1].parse::<u64>() {
                    match &c[2] {
                        "passed" => current.passed = current.passed.saturating_add(n),
                        "failed" | "error" | "errors" => {
                            current.failed = current.failed.saturating_add(n)
                        }
                        "skipped" => current.ignored = current.ignored.saturating_add(n),
                        _ => unreachable!(),
                    }
                    any = true;
                }
            }
            if any {
                last_pytest = Some(current);
            }
        }
    }
    if let Some(p) = last_pytest {
        total.passed = total.passed.saturating_add(p.passed);
        total.failed = total.failed.saturating_add(p.failed);
        total.ignored = total.ignored.saturating_add(p.ignored);
        found = true;
    }
    found.then_some(total)
}

pub fn failing_lines(output: &str, exit_code: i32) -> Vec<String> {
    if exit_code == 0 {
        return Vec::new();
    }
    static FAIL: OnceLock<Regex> = OnceLock::new();
    let re = FAIL.get_or_init(|| Regex::new(r"(?i)\b(error|errors|fail|failed|failure|failures|panicked|traceback|exception)\b|assert").unwrap());
    let mut matches = Vec::new();
    let mut tail = VecDeque::new();
    for line in output.lines().map(|line| line.trim_end_matches('\r')) {
        if re.is_match(line) && matches.len() < MAX_FAILING_LINES {
            matches.push(line);
        }
        if !line.trim().is_empty() {
            if tail.len() == MAX_FAILING_LINES {
                tail.pop_front();
            }
            tail.push_back(line);
        }
    }
    let lines: Vec<&str> = if matches.is_empty() {
        tail.into_iter().collect()
    } else {
        matches
    };
    let mut result = Vec::new();
    let mut remaining = MAX_FAILING_BYTES;
    for line in lines {
        if line.len() <= remaining {
            result.push(line.to_owned());
            remaining -= line.len();
        } else {
            if remaining >= '…'.len_utf8() {
                let mut end = remaining - '…'.len_utf8();
                while !line.is_char_boundary(end) {
                    end -= 1;
                }
                result.push(format!("{}…", &line[..end]));
            }
            break;
        }
    }
    result
}

pub fn path_sample(paths: &[String]) -> Value {
    json!({"count": paths.len(), "sample": paths.iter().take(MAX_PATH_SAMPLE).collect::<Vec<_>>()})
}

pub fn one_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    let mut chars = line.chars();
    let mut result: String = chars.by_ref().take(MAX_REASON_CHARS).collect();
    if chars.next().is_some() {
        result.pop();
        result.push('…');
    }
    result
}

pub fn summary(
    exit_code: Option<i32>,
    output: &str,
    output_file: Option<&Path>,
    changed: Option<&[String]>,
) -> Value {
    json!({"exit_code": exit_code, "tests": test_counts(output), "failing": failing_lines(output, exit_code.unwrap_or(0)), "output_file": output_file, "changed": changed.map(path_sample)})
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Next {
    Review,
    Fix,
    Wait,
    Done,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct VerdictCounts {
    pub blocker: usize,
    pub major: usize,
    pub minor: usize,
}

pub fn verdict_counts(v: &Verdict) -> VerdictCounts {
    let mut counts = VerdictCounts::default();
    for finding in &v.findings {
        match finding.severity {
            Severity::Blocker => counts.blocker += 1,
            Severity::Major => counts.major += 1,
            Severity::Minor => counts.minor += 1,
        }
    }
    counts
}

pub fn next_for(store: &Path, unit: &str) -> Res<Next> {
    let units = read_units(store)?;
    let u = units
        .iter()
        .find(|u| u.id == unit)
        .ok_or_else(|| Failure::new("not_found", format!("unit {unit} not found")))?;
    if u.superseded_by().is_some() {
        return Ok(Next::Done);
    }
    let rows = evidence(store, unit)?;
    let needs = needs_accept(store, unit)?;
    if !matches!(
        super::selfcheck_state(store, unit)?,
        super::SelfCheckState::NotRequired | super::SelfCheckState::Passed
    ) {
        return Ok(Next::Fix);
    }
    Ok(match derive_state(&rows, needs).as_str() {
        "verified" => Next::Done,
        "built" => {
            let verify = rows.iter().rposition(|r| r.kind == "verify");
            if needs && verify.is_some_and(|v| !rows[v + 1..].iter().any(|r| r.kind == "accept")) {
                Next::Wait
            } else {
                Next::Review
            }
        }
        _ => Next::Fix,
    })
}

#[derive(Serialize, Clone, Debug)]
pub struct ReviewItem {
    pub unit: String,
    pub pack: Option<PathBuf>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Outcome {
    pub unit: String,
    pub state: String,
    pub next: Next,
    pub reason: String,
    pub verdict: Option<VerdictCounts>,
    pub review: Vec<ReviewItem>,
    pub paths: BTreeMap<String, PathBuf>,
    pub module_done: Option<bool>,
}

impl Outcome {
    pub fn line(&self) -> String {
        let next = match self.next {
            Next::Review => "review",
            Next::Fix => "fix",
            Next::Wait => "wait",
            Next::Done => "done",
        };
        let verdict = self
            .verdict
            .as_ref()
            .map(|v| {
                format!(
                    "\tverdict blocker={} major={} minor={}",
                    v.blocker, v.major, v.minor
                )
            })
            .unwrap_or_default();
        format!(
            "{}\t{}\tnext={}{}\t{}",
            self.unit,
            self.state,
            next,
            verdict,
            one_line(&self.reason)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orch::{append_evidence, Evidence};

    #[test]
    fn reply_byte_cap_measures_whole_bridge_value() {
        // Vary both string and numeric fields because the bridge duplicates the payload.
        let mut sizes = [false; 2];
        for n in 3800..4200 {
            for extra in ["x", "\n"] {
                let value =
                    crate::bridge::call_body(|| Ok(json!({"text": "x".repeat(n), "extra": extra})));
                let size = serde_json::to_string(&value).unwrap().len();
                if size == MAX_REPLY_BYTES {
                    sizes[0] = true;
                }
                if size == MAX_REPLY_BYTES + 1 {
                    sizes[1] = true;
                }
            }
        }
        assert_eq!(sizes, [true, true]);
    }

    #[test]
    fn counts_cargo_and_last_pytest_only() {
        let cargo_output = "test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s\n\
test result: FAILED. 3 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n\
test result: ok. 5 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 0.01s";
        assert_eq!(
            test_counts(cargo_output),
            Some(TestCounts {
                passed: 10,
                failed: 2,
                ignored: 3,
            })
        );
        let output = format!(
            "=== 99 passed in 0.10s ===\n=== 4 passed, 1 failed, 2 errors, 3 skipped in 1.23s ===\n{cargo_output}"
        );
        assert_eq!(
            test_counts(&output),
            Some(TestCounts {
                passed: 14,
                failed: 5,
                ignored: 6
            })
        );
        assert_eq!(
            test_counts("=== 1 error in 0.01s ==="),
            Some(TestCounts {
                failed: 1,
                ..Default::default()
            })
        );
        assert_eq!(test_counts("no tests ran"), None);
        assert_eq!(
            test_counts("test result: ok. 0 passed; 0 failed; 0 ignored;"),
            Some(TestCounts::default())
        );
    }

    #[test]
    fn failing_line_count_cap_and_fallback() {
        for n in [MAX_FAILING_LINES, MAX_FAILING_LINES + 1] {
            let text = (0..n).map(|i| format!("error {i}\r\n")).collect::<String>();
            let lines = failing_lines(&text, 1);
            assert_eq!(lines.len(), n.min(MAX_FAILING_LINES));
            assert_eq!(lines[0], "error 0");
            assert!(lines.iter().all(|l| !l.contains('\r')));
            assert!(failing_lines(&text, 0).is_empty());
            let ordinary = (0..n).map(|i| format!("line {i}\n")).collect::<String>();
            let fallback = failing_lines(&ordinary, 1);
            assert_eq!(fallback.len(), n.min(MAX_FAILING_LINES));
            assert_eq!(
                fallback[0],
                format!("line {}", n.saturating_sub(MAX_FAILING_LINES))
            );
        }
        let text = format!("early failure\n{}", "ordinary\n".repeat(5000));
        assert_eq!(failing_lines(&text, 1), vec!["early failure"]);
    }

    #[test]
    fn failing_byte_cap_and_utf8_boundaries() {
        for n in [MAX_FAILING_BYTES, MAX_FAILING_BYTES + 1] {
            let text = format!("error{}", "x".repeat(n - 5));
            let lines = failing_lines(&text, 1);
            assert_eq!(lines[0].len(), MAX_FAILING_BYTES);
            assert_eq!(lines[0].ends_with('…'), n > MAX_FAILING_BYTES);
        }
        let text = format!("error{}界suffix", "x".repeat(MAX_FAILING_BYTES - 5 - 4));
        let lines = failing_lines(&text, 1);
        assert!(lines[0].len() <= MAX_FAILING_BYTES);
        assert!(lines[0].ends_with('…'));
        assert!(!lines[0].contains('界'));
        let multiple = format!("error {}\nerror {}", "a".repeat(1000), "b".repeat(2000));
        let lines = failing_lines(&multiple, 1);
        assert_eq!(
            lines.iter().map(String::len).sum::<usize>(),
            MAX_FAILING_BYTES
        );
        assert!(lines[1].ends_with('…'));
        let huge = format!("failure {}", "x".repeat(1_000_000));
        assert_eq!(failing_lines(&huge, 1)[0].len(), MAX_FAILING_BYTES);
    }

    #[test]
    fn path_sample_cap() {
        for n in [MAX_PATH_SAMPLE, MAX_PATH_SAMPLE + 1, 10_000] {
            let paths: Vec<_> = (0..n).map(|i| format!("path-{i}")).collect();
            let sample = path_sample(&paths);
            assert_eq!(sample["count"], n);
            assert_eq!(sample["sample"], json!(&paths[..n.min(MAX_PATH_SAMPLE)]));
        }
    }

    #[test]
    fn reason_char_cap() {
        for n in [MAX_REASON_CHARS, MAX_REASON_CHARS + 1] {
            let input = format!("\n  \n  {}  \nignored", "界".repeat(n));
            let result = one_line(&input);
            assert_eq!(result.chars().count(), MAX_REASON_CHARS);
            assert_eq!(result.ends_with('…'), n > MAX_REASON_CHARS);
        }
        assert_eq!(one_line("\n \r\n"), "");
    }

    #[test]
    fn summary_shape_and_outcome_line() {
        let result = summary(
            Some(1),
            "private raw log\nerror broke",
            Some(Path::new("log.txt")),
            Some(&["file".into()]),
        );
        assert_eq!(
            result,
            json!({"exit_code": 1, "tests": null, "failing": ["error broke"], "output_file": "log.txt", "changed": {"count": 1, "sample": ["file"]}})
        );
        assert!(!result.to_string().contains("private raw log"));
        assert_eq!(
            summary(None, "", None, None),
            json!({"exit_code": null, "tests": null, "failing": [], "output_file": null, "changed": null})
        );
        let verdict = crate::orch::parse_verdict(r#"{"unit":"u","tree":"t","verdict":"fail","findings":[{"severity":"blocker"},{"severity":"major"},{"severity":"minor"}]}"#).unwrap();
        let counts = verdict_counts(&verdict);
        assert_eq!(
            counts,
            VerdictCounts {
                blocker: 1,
                major: 1,
                minor: 1
            }
        );
        let mut outcome = Outcome {
            unit: "u".into(),
            state: "built".into(),
            next: Next::Review,
            reason: "\n reason\nsecond".into(),
            verdict: Some(counts),
            review: vec![],
            paths: BTreeMap::new(),
            module_done: None,
        };
        assert_eq!(
            outcome.line(),
            "u\tbuilt\tnext=review\tverdict blocker=1 major=1 minor=1\treason"
        );
        outcome.verdict = None;
        assert_eq!(outcome.line(), "u\tbuilt\tnext=review\treason");
        assert_eq!(serde_json::to_string(&Next::Wait).unwrap(), "\"wait\"");
    }

    fn add(store: &Path, unit: &str, kind: &str, exit_code: i32, command: &str) {
        append_evidence(
            store,
            unit,
            &Evidence {
                kind: kind.into(),
                command: command.into(),
                exit_code,
                commit: "tree:abc".into(),
                ts: "time".into(),
            },
        )
        .unwrap();
    }

    #[test]
    fn next_uses_derived_state_and_accept_after_last_verify() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path();
        let brief = store.join("unit.brief");
        std::fs::write(&brief, "GOAL test\nSCOPE src/\nVERIFY cargo test\n").unwrap();
        let mut units = "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n".to_owned();
        for row in [
            "verified\tother\tpending", "failed\tother\tpending",
            "waiting\tmodule\tpending", "built\tother\tpending",
            "old\tother\tsuperseded:new", "pending\tother\tpending",
            "acceptfailed\tmodule\tpending", "reviewfailed\tother\tpending",
        ] {
            units.push_str(&format!("{row}\t\t\t\t{}\n", brief.display()));
        }
        std::fs::write(store.join("units.tsv"), units).unwrap();
        std::fs::write(store.join("modules.md"), "## module\nPURPOSE test\nPATHS src/\nDEPENDS none\nCONTRACT test\nACCEPT test `cargo test`\n").unwrap();
        add(store, "verified", "run", 0, "codex-test");
        add(store, "verified", "verify", 0, "test");
        add(store, "verified", "review", 0, "claude-test");
        add(store, "failed", "run", 1, "codex");
        for unit in ["waiting", "built", "acceptfailed", "reviewfailed"] {
            add(store, unit, "run", 0, "codex");
            add(store, unit, "verify", 0, "test");
        }
        add(store, "waiting", "accept", 0, "test");
        add(store, "waiting", "verify", 0, "test");
        add(store, "acceptfailed", "accept", 1, "test");
        add(store, "reviewfailed", "review", 1, "claude");
        for (unit, expected) in [
            ("verified", Next::Done),
            ("failed", Next::Fix),
            ("waiting", Next::Wait),
            ("built", Next::Review),
            ("old", Next::Done),
            ("pending", Next::Fix),
            ("acceptfailed", Next::Fix),
            ("reviewfailed", Next::Fix),
        ] {
            assert_eq!(next_for(store, unit).unwrap(), expected, "{unit}");
        }
        add(store, "waiting", "accept", 0, "test");
        assert_eq!(next_for(store, "waiting").unwrap(), Next::Review);
        assert_eq!(next_for(store, "unknown").unwrap_err().kind, "not_found");
    }

    #[test]
    fn next_without_brief_reports_no_brief() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path();
        std::fs::write(store.join("units.tsv"), "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tother\tpending\t\t\t\t\n").unwrap();
        let error = next_for(store, "u").unwrap_err();
        assert_eq!(error.kind, "no_brief");
        assert_eq!(error.message, "unit u has no brief");
    }
}


/// Snapshot dispatchable units and their modules without claiming or dispatching them.
pub(crate) fn cycle_ready(root: &Path) -> Res<Vec<String>> {
    let mut ready = crate::orch::ready_units(root)?;
    let units = crate::orch::read_units(root)?;
    let tracks: Vec<_> = units.iter().filter(|u| ready.contains(&u.id))
        .map(|u| u.track.clone()).collect();
    ready.extend(tracks);
    if root.join("modules.md").is_file() {
        let text = std::fs::read_to_string(root.join("modules.md"))
            .map_err(|e| Failure::new("io", e.to_string()))?;
        let map = crate::orch::parse_map(&text).map_err(|e| Failure::new("invalid_argument", e))?;
        let claimed = crate::orch::lifecycle::scheduler::claimed_modules(root)?;
        for module in map.modules {
            if claimed.contains(&module.id) || crate::orch::lifecycle::scheduler::module_verified(root, &module.id)? {
                continue;
            }
            let mut unblocked = true;
            for dep in &module.depends {
                if !crate::orch::lifecycle::scheduler::module_verified(root, dep)? { unblocked = false; break; }
            }
            if unblocked { ready.push(module.id); }
        }
    }
    ready.sort();
    ready.dedup();
    Ok(ready)
}

pub(crate) fn cycle_core(reply: &Value) -> Value {
    let error = reply["error"]["kind"].as_str().unwrap_or("");
    let kind = match error {
        "fix_limit" => "fix_rounds",
        "escalation_saturated" | "escalation_required" => "escalation",
        "not_ready" | "blocked" | "unknown_track" | "no_brief" | "superseded" => "blocked",
        "selfcheck_required" | "verify_failed" | "red_invalid" => "fix_decision",
        _ if reply["next"] == "fix" => "fix_decision",
        _ => return Value::Null,
    };
    let reason = reply["error"]["message"].as_str().filter(|s| !s.is_empty())
        .or_else(|| reply["reason"].as_str().filter(|s| !s.is_empty()))
        .unwrap_or("unit requires a fix decision");
    json!({"kind":kind,"reason":one_line(reason)})
}

pub(crate) fn cycle_handoff(reply: &Value) -> String {
    let fields = [
        ("RESULT", format!("{} {}", reply["state"].as_str().unwrap_or(""), reply["next"].as_str().unwrap_or(""))),
        ("PATHS", reply["changed"]["sample"].as_array().into_iter().flatten()
            .filter_map(Value::as_str).collect::<Vec<_>>().join(", ")),
        ("COMMANDS", format!("verify exit {}", reply["verify_exit"])),
        ("OPEN", reply["core"]["reason"].as_str().or_else(|| reply["reason"].as_str()).unwrap_or("").to_owned()),
        ("EVIDENCE", format!("verify={} accept={}", reply["verify_exit"], reply["accept"]["exit_code"])),
    ];
    let mut handoff = String::new();
    // Five 220-byte values plus labels and separators always fit the 1200-byte cap.
    for (key, value) in fields {
        if !handoff.is_empty() { handoff.push('\n'); }
        handoff.push_str(key);
        handoff.push(' ');
        let clean = value.replace(['\r', '\n'], " ");
        let mut end = clean.len().min(220);
        while !clean.is_char_boundary(end) { end -= 1; }
        handoff.push_str(&clean[..end]);
    }
    handoff
}
