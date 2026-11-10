use axiom::usage::{period, Locations, Warning, DAY_MS, LARGE_READ_BYTES};
use serde_json::{json, Value};
use std::{fs, path::Path};

const NOW: i64 = 1_767_830_400_000; // 2026-01-08T00:00:00Z
const MODEL: &str = "claude-opus-5-5";

fn locations(root: &Path) -> Locations {
    Locations {
        home: root.join("home"),
        projects: root.join("projects"),
    }
}
fn write(path: &Path, lines: &[String]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, lines.join("\n") + "\n").unwrap();
}
fn call(id: &str, model: &str, timestamp: Option<&str>, write: u64, content: Value) -> String {
    json!({"type":"assistant", "timestamp":timestamp, "message": {
        "id":id, "model":model, "usage":{"input_tokens":10, "cache_creation_input_tokens":write, "output_tokens":2}, "content":content
    }}).to_string()
}
fn init(session: &str) -> String {
    json!({"type":"system", "subtype":"init", "session_id":session, "cwd":"E:\\project"})
        .to_string()
}
fn result(id: &str, timestamp: Option<&str>, content: Value) -> String {
    json!({"type":"user", "timestamp":timestamp, "message":{"content":[{"type":"tool_result", "tool_use_id":id, "content":content}]}}).to_string()
}

#[test]
fn period_deduplicates_and_marks_cross_boundary_rewrites_and_history() {
    let temp = tempfile::tempdir().unwrap();
    let loc = locations(temp.path());
    let before = call(
        "before",
        MODEL,
        Some("2025-12-31T23:59:59.999Z"),
        0,
        json!([]),
    );
    let rewrite = call(
        "rewrite",
        MODEL,
        Some("2026-01-01T00:06:00.001Z"),
        50_000,
        json!([]),
    );
    let inside = call(
        "inside",
        MODEL,
        Some("2026-01-01T00:00:00.001Z"),
        0,
        json!([]),
    );
    let undated = call("undated", MODEL, None, 0, json!([]));
    write(
        &loc.home.join("logs/20260101-000000-wish-1.jsonl"),
        &[init("a")],
    );
    write(
        &loc.projects.join("E--project/a.jsonl"),
        &[before.clone(), rewrite.clone(), rewrite.clone(), undated],
    );
    write(
        &loc.projects.join("E--project/a/subagents/agent-sub.jsonl"),
        &[rewrite, inside.clone(), inside],
    );
    write(
        &loc.projects.join("another/b.jsonl"),
        &[call(
            "old",
            MODEL,
            Some("2025-12-20T00:00:00Z"),
            0,
            json!([]),
        )],
    );
    let p = period(&loc, NOW, 7).unwrap();
    assert_eq!((p.start_ms, p.end_ms), (NOW - 7 * DAY_MS, NOW));
    assert_eq!(p.runs.len(), 1);
    assert_eq!(p.runs[0].session, "a");
    assert_eq!(p.runs[0].skill.as_deref(), Some("wish"));
    assert_eq!(p.runs[0].ledger.totals.calls, 2);
    assert_eq!(p.runs[0].first_ms, Some(p.start_ms + 1));
    assert_eq!(p.runs[0].ledger.totals.gap_rewrites, 1);
    assert_eq!(p.undated_calls, 1);
    assert_eq!(
        p.history
            .iter()
            .map(|h| h.session.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(p.history[0].calls, 4);
    assert_eq!(p.history[1].calls, 1);
    assert_eq!(p.history[1].skill, None);
}

#[test]
fn log_only_unknown_and_boundaries() {
    let temp = tempfile::tempdir().unwrap();
    let loc = locations(temp.path());
    write(
        &loc.home.join("logs/20260101-000000-research.jsonl"),
        &[
            init("log"),
            call(
                "start",
                "future-model",
                Some("2026-01-01T00:00:00.000Z"),
                0,
                json!([]),
            ),
            call("end", MODEL, Some("2026-01-08T00:00:00.000Z"), 0, json!([])),
            call(
                "after",
                MODEL,
                Some("2026-01-08T00:00:00.001Z"),
                0,
                json!([]),
            ),
            "not json".into(),
        ],
    );
    let p = period(&loc, NOW, 7).unwrap();
    let run = &p.runs[0];
    assert_eq!(run.skill.as_deref(), Some("research"));
    assert!(run.ledger.output_tokens_partial);
    assert_eq!(run.ledger.totals.calls, 2);
    assert_eq!(run.ledger.totals.usd, None);
    assert_eq!(run.ledger.totals.unpriced_calls, 1);
    assert!(run.ledger.totals.priced_usd > 0.0);
    assert_eq!(p.history[0].calls, 3);
    assert_eq!(p.history[0].usd, None);
    assert_eq!(p.history[0].unpriced_calls, 1);
    assert_eq!(p.skipped_lines, 1);
}

#[test]
fn tool_result_times_control_reads_and_codex_and_deduplicate() {
    let temp = tempfile::tempdir().unwrap();
    let loc = locations(temp.path());
    let tools = json!([
        {"type":"tool_use", "id":"inside", "name":"Read", "input":{"file_path":"in.txt"}},
        {"type":"tool_use", "id":"outside", "name":"Read", "input":{"file_path":"out.txt"}},
        {"type":"tool_use", "id":"undated", "name":"Read", "input":{"file_path":"no-date.txt"}},
        {"type":"tool_use", "id":"c1", "name":"codex_run", "input":{}},
        {"type":"tool_use", "id":"c2", "name":"codex_run", "input":{}}
    ]);
    let request = call("request", MODEL, Some("2026-01-02T00:00:00Z"), 0, tools);
    let big = json!("x".repeat(LARGE_READ_BYTES as usize));
    let report = json!({"model":"gpt-terra", "usage":{"inputTokens":100}}).to_string();
    let lines = vec![
        request,
        result("inside", Some("2026-01-01T00:00:00.001Z"), big.clone()),
        result("outside", Some("2025-12-31T23:59:59.999Z"), big.clone()),
        result("undated", None, big),
        result("c1", Some("2026-01-02T00:00:00Z"), json!(report)),
        result("c2", Some("2025-12-31T23:59:59.999Z"), json!(report)),
    ];
    write(&loc.projects.join("project/s.jsonl"), &lines);
    write(
        &loc.projects.join("project/s/subagents/agent-copy.jsonl"),
        &lines,
    );
    let p = period(&loc, NOW, 7).unwrap();
    let l = &p.runs[0].ledger;
    assert_eq!(l.totals.calls, 1);
    let reads: Vec<_> = l
        .warnings
        .iter()
        .filter_map(|w| match w {
            Warning::LargeFileRead { path, reads, .. } => Some((path.as_str(), *reads)),
            _ => None,
        })
        .collect();
    assert_eq!(reads, [("in.txt", 1)]);
    assert_eq!(l.codex.len(), 1);
    assert_eq!(l.codex[0].runs, 1);
    assert_eq!(l.codex[0].tokens.input, 100);
}

#[test]
fn missing_locations_and_invalid_days() {
    let temp = tempfile::tempdir().unwrap();
    let loc = locations(temp.path());
    let p = period(&loc, NOW, 7).unwrap();
    assert!(p.runs.is_empty() && p.history.is_empty());
    assert_eq!((p.undated_calls, p.skipped_lines), (0, 0));
    assert_eq!(period(&loc, NOW, 0).unwrap_err().kind, "invalid_argument");
    assert!(!loc.home.exists() && !loc.projects.exists());
}
