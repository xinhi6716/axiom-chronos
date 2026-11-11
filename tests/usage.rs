mod common;

use axiom::usage::{
    ledger,
    price::{cache_write_usd, usd, CallUsage},
    render, transcript_dir_name, AgentRow, Ledger, Locations, Warning, CACHE_GAP_MS, CONTEXT_WARN,
    LARGE_CACHE_WRITE, LARGE_READ_BYTES,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime},
};

fn near(actual: Option<f64>, expected: f64) {
    let actual = actual.expect("priced");
    assert!(
        (actual - expected).abs() < 1e-9,
        "{actual} is not {expected}"
    );
}
fn tokens(input: u64, write_5m: u64, write_1h: u64, read: u64, output: u64) -> CallUsage {
    CallUsage {
        input,
        cache_write: write_5m + write_1h,
        cache_write_1h: write_1h,
        cache_read: read,
        output,
        ..CallUsage::default()
    }
}
const M: u64 = 1_000_000;

#[test]
fn each_category_is_priced_at_its_own_rate() {
    for (model, input, write_5m, write_1h, read, output) in [
        ("claude-opus-5-5", 4.0, 5.0, 8.0, 0.20, 20.0),
        ("claude-sonnet-5-5", 2.0, 2.5, 4.0, 0.10, 10.0),
        ("claude-sonnet-5", 2.0, 2.5, 4.0, 0.20, 10.0),
        ("claude-opus-5", 5.0, 6.25, 10.0, 0.50, 25.0),
        ("claude-opus-4-1", 15.0, 18.75, 30.0, 1.50, 75.0),
        ("claude-haiku-4-5", 1.0, 1.25, 2.0, 0.10, 5.0),
        ("claude-fable-5-1", 10.0, 12.5, 20.0, 0.25, 50.0),
    ] {
        near(usd(model, &tokens(M, 0, 0, 0, 0)), input);
        near(usd(model, &tokens(0, M, 0, 0, 0)), write_5m);
        near(usd(model, &tokens(0, 0, M, 0, 0)), write_1h);
        near(usd(model, &tokens(0, 0, 0, M, 0)), read);
        near(usd(model, &tokens(0, 0, 0, 0, M)), output);
    }
}

#[test]
fn cache_writes_without_a_breakdown_are_five_minute_writes() {
    let u = CallUsage {
        cache_write: M,
        ..CallUsage::default()
    };
    near(usd("claude-opus-5-5", &u), 5.0);
    near(cache_write_usd("claude-opus-5-5", &u), 5.0);
}

#[test]
fn cache_write_cost_excludes_every_other_category() {
    let u = tokens(M, M, M, M, M);
    near(cache_write_usd("claude-sonnet-5-5", &u), 2.5 + 4.0);
}

#[test]
fn haiku_5_5_is_priced_per_request_by_prompt_length() {
    let at_limit = tokens(40_000, 30_000, 0, 30_000, 1_000);
    near(
        usd("claude-haiku-5-5", &at_limit),
        (40_000.0 * 0.10 + 30_000.0 * 0.125 + 30_000.0 * 0.01 + 1_000.0 * 0.50) / 1e6,
    );
    let over = tokens(40_001, 30_000, 0, 30_000, 1_000);
    near(
        usd("claude-haiku-5-5", &over),
        (40_001.0 * 0.50 + 30_000.0 * 0.625 + 30_000.0 * 0.05 + 1_000.0 * 2.50) / 1e6,
    );
}

#[test]
fn date_suffixes_resolve_to_their_family_and_nothing_else_does() {
    near(
        usd("claude-opus-4-1-20250805", &tokens(M, 0, 0, 0, 0)),
        15.0,
    );
    near(usd("claude-opus-4-20250514", &tokens(M, 0, 0, 0, 0)), 15.0);
    near(
        usd("claude-sonnet-4-5-20250929", &tokens(M, 0, 0, 0, 0)),
        3.0,
    );
    for unknown in [
        "claude-opus-5-6",
        "claude-opus-4-2",
        "claude-sonnet-6",
        "claude-opus-5-5-preview",
        "anthropic.claude-opus-5-5",
        "gpt-6.1-terra",
        "<synthetic>",
        "",
    ] {
        assert_eq!(usd(unknown, &tokens(M, M, M, M, M)), None, "{unknown}");
        assert_eq!(cache_write_usd(unknown, &tokens(0, M, 0, 0, 0)), None);
    }
}

#[test]
fn fast_mode_doubles_every_rate_where_it_exists_and_is_unpriced_elsewhere() {
    let u = CallUsage {
        fast: true,
        ..tokens(M, M, M, M, M)
    };
    near(
        usd("claude-opus-5-5", &u),
        2.0 * (4.0 + 5.0 + 8.0 + 0.20 + 20.0),
    );
    near(
        usd("claude-opus-4-8", &u),
        2.0 * (5.0 + 6.25 + 10.0 + 0.5 + 25.0),
    );
    assert_eq!(usd("claude-sonnet-5-5", &u), None);
}

#[test]
fn us_only_inference_costs_more_on_tokens_but_not_on_searches() {
    let u = CallUsage {
        us_only: true,
        web_searches: 100,
        ..tokens(M, 0, 0, 0, M)
    };
    near(usd("claude-opus-5-5", &u), 1.1 * (4.0 + 20.0) + 1.0);
}

#[test]
fn non_standard_service_tiers_are_not_guessed() {
    let u = CallUsage {
        non_standard_tier: true,
        ..tokens(M, 0, 0, 0, 0)
    };
    assert_eq!(usd("claude-opus-5-5", &u), None);
}

const SESSION: &str = "11111111-2222-3333-4444-555555555555";
const OPUS: &str = "claude-opus-5-5";
const SONNET: &str = "claude-sonnet-5-5";
const PROJECT_DIR: &str = "E--project";

fn at(minute: u32, second: u32, ms: u32) -> String {
    format!("2026-01-01T00:{minute:02}:{second:02}.{ms:03}Z")
}
fn counts(input: u64, write: u64, read: u64, output: u64) -> Value {
    json!({"input_tokens": input, "cache_creation_input_tokens": write,
           "cache_read_input_tokens": read, "output_tokens": output})
}
fn line_with(id: &str, model: &str, timestamp: &str, usage: Value, content: Value) -> String {
    json!({"type": "assistant", "timestamp": timestamp,
           "message": {"id": id, "model": model, "content": content, "usage": usage}})
    .to_string()
}
fn assistant(id: &str, model: &str, timestamp: &str, usage: Value) -> String {
    line_with(id, model, timestamp, usage, json!([]))
}
fn tool_use(id: &str, name: &str, input: Value) -> Value {
    json!({"type": "tool_use", "id": id, "name": name, "input": input})
}
fn tool_result(tool_use_id: &str, content: Value) -> String {
    json!({"type": "user", "message": {"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": tool_use_id, "content": content}]}})
    .to_string()
}
fn write_lines(path: &Path, lines: &[String]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, lines.join("\n") + "\n").unwrap();
}
fn age(path: &Path, seconds_ago: u64) {
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(seconds_ago))
        .unwrap();
}

struct Run {
    root: tempfile::TempDir,
}
impl Run {
    fn new() -> Self {
        Self {
            root: common::temp(),
        }
    }
    fn locations(&self) -> Locations {
        Locations {
            home: self.root.path().join("home"),
            projects: self.root.path().join("claude").join("projects"),
        }
    }
    fn transcript_at(&self, dir: &str, session: &str, lines: &[String]) -> PathBuf {
        let path = self
            .locations()
            .projects
            .join(dir)
            .join(format!("{session}.jsonl"));
        write_lines(&path, lines);
        path
    }
    fn transcript(&self, lines: &[String]) -> PathBuf {
        self.transcript_at(PROJECT_DIR, SESSION, lines)
    }
    fn subagent(&self, agent: &str, agent_type: Option<&str>, lines: &[String]) {
        let dir = self
            .locations()
            .projects
            .join(PROJECT_DIR)
            .join(SESSION)
            .join("subagents");
        write_lines(&dir.join(format!("agent-{agent}.jsonl")), lines);
        if let Some(agent_type) = agent_type {
            fs::write(
                dir.join(format!("agent-{agent}.meta.json")),
                json!({"agentType": agent_type, "description": format!("{agent} task")})
                    .to_string(),
            )
            .unwrap();
        }
    }
    fn log(&self, name: &str, lines: &[String]) -> PathBuf {
        let path = self.locations().home.join("logs").join(name);
        write_lines(&path, lines);
        path
    }
    fn goal(&self, session: &str, cwd: &str) -> PathBuf {
        let path = self
            .locations()
            .home
            .join("goals")
            .join(format!("{session}.json"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            json!({"wish": "w", "cwd": cwd, "status": "done"}).to_string(),
        )
        .unwrap();
        path
    }
    fn ledger(&self) -> Ledger {
        ledger(&self.locations(), Some(SESSION)).unwrap()
    }
}
fn init(session: &str, cwd: &str) -> String {
    json!({"type": "system", "subtype": "init", "session_id": session, "cwd": cwd}).to_string()
}
fn row<'a>(l: &'a Ledger, agent: &str, model: &str) -> &'a AgentRow {
    l.by_agent
        .iter()
        .find(|r| r.agent_type == agent && r.model == model)
        .unwrap_or_else(|| panic!("no row for {agent} {model}: {l:#?}"))
}
fn instance_peak(l: &Ledger, id: &str) -> u64 {
    l.instances
        .iter()
        .find(|i| i.id == id)
        .unwrap_or_else(|| panic!("no instance {id}"))
        .tally
        .peak_context
}

#[test]
fn repeated_lines_of_one_message_count_once_with_the_largest_values() {
    let run = Run::new();
    run.transcript(&[
        assistant("msg_1", OPUS, &at(0, 0, 0), counts(2, 10, 100, 4)),
        assistant("msg_1", OPUS, &at(0, 1, 0), counts(2, 10, 100, 340)),
        assistant("msg_1", OPUS, &at(0, 2, 0), counts(2, 10, 100, 340)),
        assistant("msg_2", OPUS, &at(0, 3, 0), counts(3, 20, 200, 50)),
        assistant("msg_2", OPUS, &at(0, 4, 0), counts(3, 20, 200, 4)),
    ]);
    let l = run.ledger();
    let t = &row(&l, "main", OPUS).tally;
    assert_eq!(
        (t.calls, t.input, t.cache_write, t.cache_read, t.output),
        (2, 5, 30, 300, 390)
    );
    assert_eq!(l.totals.calls, 2);
    assert_eq!(l.skipped_lines, 0);
}

#[test]
fn subagents_are_attributed_by_their_meta_type_and_shared_messages_count_once() {
    let run = Run::new();
    run.transcript(&[
        assistant("m1", OPUS, &at(0, 0, 0), counts(1, 100, 1000, 10)),
        assistant("shared", OPUS, &at(0, 1, 0), counts(7, 70, 700, 77)),
    ]);
    run.subagent(
        "aaa",
        Some("chronos:reviewer"),
        &[
            assistant("r1", SONNET, &at(0, 0, 0), counts(2, 200, 2000, 20)),
            assistant("r2", SONNET, &at(0, 1, 0), counts(3, 300, 3000, 30)),
        ],
    );
    run.subagent(
        "bbb",
        Some("chronos:reviewer"),
        &[
            assistant("r3", SONNET, &at(0, 0, 0), counts(4, 400, 4000, 40)),
            assistant("shared", OPUS, &at(0, 1, 0), counts(7, 70, 700, 77)),
        ],
    );
    run.subagent(
        "ccc",
        None,
        &[assistant(
            "x1",
            SONNET,
            &at(0, 0, 0),
            counts(5, 500, 5000, 50),
        )],
    );
    let l = run.ledger();
    assert_eq!(l.subagent_files, 3);
    assert_eq!(l.totals.calls, 6);
    let main = &row(&l, "main", OPUS).tally;
    assert_eq!(
        (
            main.calls,
            main.input,
            main.cache_write,
            main.cache_read,
            main.output
        ),
        (2, 8, 170, 1700, 87)
    );
    let reviewer = &row(&l, "chronos:reviewer", SONNET).tally;
    assert_eq!(
        (
            reviewer.calls,
            reviewer.input,
            reviewer.cache_write,
            reviewer.cache_read,
            reviewer.output
        ),
        (3, 9, 900, 9000, 90)
    );
    let unknown = &row(&l, "unknown", SONNET).tally;
    assert_eq!((unknown.calls, unknown.output), (1, 50));
    assert!(l
        .by_agent
        .iter()
        .all(|r| r.agent_type != "chronos:reviewer" || r.model != OPUS));
    assert_eq!(
        l.instances
            .iter()
            .find(|i| i.id == "aaa")
            .unwrap()
            .description
            .as_deref(),
        Some("aaa task")
    );
}

#[test]
fn peak_context_is_the_largest_single_call_not_a_sum() {
    let run = Run::new();
    run.transcript(&[
        assistant("a", OPUS, &at(0, 0, 0), counts(10, 20, 30, 999)),
        assistant("b", OPUS, &at(0, 1, 0), counts(1, 2, 300, 1)),
        assistant("c", OPUS, &at(0, 2, 0), counts(5, 5, 5, 1)),
    ]);
    run.subagent(
        "big",
        Some("chronos:dispatcher"),
        &[assistant(
            "d",
            SONNET,
            &at(0, 0, 0),
            counts(1000, 2000, 3000, 7),
        )],
    );
    let l = run.ledger();
    assert_eq!(row(&l, "main", OPUS).tally.peak_context, 303);
    assert_eq!(instance_peak(&l, "main"), 303);
    assert_eq!(
        row(&l, "chronos:dispatcher", SONNET).tally.peak_context,
        6000
    );
    assert_eq!(instance_peak(&l, "big"), 6000);
    assert_eq!(l.totals.peak_context, 6000);
    assert_eq!(l.instances[0].id, "big");
    assert_eq!(l.instances[0].peak_model, SONNET);
}

#[test]
fn only_large_cache_writes_after_a_gap_over_five_minutes_count() {
    assert_eq!(CACHE_GAP_MS, 300_000);
    let large = counts(1, LARGE_CACHE_WRITE, 10, 1);
    let run = Run::new();
    run.transcript(&[
        assistant("first", OPUS, &at(0, 0, 0), large.clone()),
        assistant("at_limit", OPUS, &at(5, 0, 0), large.clone()),
        assistant("just_over", OPUS, &at(10, 0, 1), large.clone()),
        assistant(
            "small",
            OPUS,
            &at(20, 0, 0),
            counts(1, LARGE_CACHE_WRITE - 1, 10, 1),
        ),
    ]);
    run.subagent(
        "busy",
        Some("chronos:reviewer"),
        &[
            assistant("s1", SONNET, &at(0, 0, 0), counts(1, 10, 10, 1)),
            assistant(
                "s2",
                SONNET,
                &at(10, 0, 0),
                counts(1, LARGE_CACHE_WRITE, 10, 1),
            ),
        ],
    );
    let l = run.ledger();
    let main = &row(&l, "main", OPUS).tally;
    assert_eq!(main.gap_rewrites, 1, "only just_over crosses both limits");
    assert_eq!(main.gap_rewrite_tokens, LARGE_CACHE_WRITE);
    assert_eq!(row(&l, "chronos:reviewer", SONNET).tally.gap_rewrites, 1);
    assert_eq!(l.totals.gap_rewrites, 2);
    assert_eq!(
        l.warnings,
        vec![
            Warning::CacheRewritesAfterGap {
                agent_type: "main".into(),
                instance: "main".into(),
                count: 1,
                tokens: LARGE_CACHE_WRITE,
                usd: 0.25,
            },
            Warning::CacheRewritesAfterGap {
                agent_type: "chronos:reviewer".into(),
                instance: "busy".into(),
                count: 1,
                tokens: LARGE_CACHE_WRITE,
                usd: 0.125,
            },
        ]
    );
}

#[test]
fn gaps_run_from_the_previous_calls_last_line_within_one_agent_across_dates() {
    let large = counts(1, LARGE_CACHE_WRITE, 10, 1);
    let run = Run::new();
    run.transcript(&[
        assistant("a", OPUS, "2024-02-29T23:50:00.000Z", large.clone()),
        assistant("a", OPUS, "2024-02-29T23:58:30.000Z", large.clone()),
        assistant("b", OPUS, "2024-03-01T00:03:00.000Z", large.clone()),
        assistant("c", OPUS, "2024-03-01T00:09:00.500Z", large.clone()),
    ]);
    let l = run.ledger();
    assert_eq!(row(&l, "main", OPUS).tally.gap_rewrites, 1, "{l:#?}");
    let run = Run::new();
    run.transcript(&[
        assistant("a", OPUS, &at(0, 0, 0), large.clone()),
        assistant("b", OPUS, "not a timestamp", large.clone()),
        assistant("c", OPUS, &at(4, 0, 0), large),
    ]);
    let l = run.ledger();
    assert_eq!(l.totals.calls, 3);
    assert_eq!(l.totals.gap_rewrites, 0);
}

#[test]
fn gaps_are_measured_correctly_across_every_month_boundary() {
    let large = counts(1, LARGE_CACHE_WRITE, 10, 1);
    for (last_day, first_day) in [
        ("2025-01-31", "2025-02-01"),
        ("2025-02-28", "2025-03-01"),
        ("2024-02-29", "2024-03-01"),
        ("2025-03-31", "2025-04-01"),
        ("2025-04-30", "2025-05-01"),
        ("2025-05-31", "2025-06-01"),
        ("2025-06-30", "2025-07-01"),
        ("2025-07-31", "2025-08-01"),
        ("2025-08-31", "2025-09-01"),
        ("2025-09-30", "2025-10-01"),
        ("2025-10-31", "2025-11-01"),
        ("2025-11-30", "2025-12-01"),
        ("2025-12-31", "2026-01-01"),
    ] {
        for (minute, flagged) in [("01", 0), ("03", 1)] {
            let run = Run::new();
            run.transcript(&[
                assistant(
                    "a",
                    OPUS,
                    &format!("{last_day}T23:57:00.000Z"),
                    large.clone(),
                ),
                assistant(
                    "b",
                    OPUS,
                    &format!("{first_day}T00:{minute}:00.000Z"),
                    large.clone(),
                ),
            ]);
            assert_eq!(
                run.ledger().totals.gap_rewrites,
                flagged,
                "{first_day} {minute}"
            );
        }
    }
}

#[test]
fn a_truncated_last_line_is_skipped_and_everything_before_it_counts() {
    let run = Run::new();
    let path = run.transcript(&[
        assistant("a", OPUS, &at(0, 0, 0), counts(1, 2, 3, 4)),
        "not json at all".to_owned(),
        assistant("b", OPUS, &at(0, 1, 0), counts(5, 6, 7, 8)),
    ]);
    let partial = assistant("c", OPUS, &at(0, 2, 0), counts(9, 9, 9, 9));
    let mut bytes = fs::read(&path).unwrap();
    bytes.extend_from_slice(&partial.as_bytes()[..partial.len() / 2]);
    fs::write(&path, bytes).unwrap();
    let l = run.ledger();
    assert_eq!(l.skipped_lines, 2);
    let t = &row(&l, "main", OPUS).tally;
    assert_eq!((t.calls, t.input, t.output), (2, 6, 12));
}

#[test]
fn unknown_models_report_tokens_with_no_usd_and_never_a_guess() {
    let run = Run::new();
    run.transcript(&[
        assistant(
            "a",
            "claude-opus-9",
            &at(0, 0, 0),
            counts(1000, 2000, 3000, 4000),
        ),
        assistant("b", OPUS, &at(0, 1, 0), counts(1_000_000, 0, 0, 0)),
    ]);
    let l = run.ledger();
    let unknown = &row(&l, "main", "claude-opus-9").tally;
    assert_eq!(
        (
            unknown.calls,
            unknown.input,
            unknown.cache_write,
            unknown.cache_read,
            unknown.output
        ),
        (1, 1000, 2000, 3000, 4000)
    );
    assert_eq!(unknown.usd, None);
    assert_eq!(unknown.unpriced_calls, 1);
    assert_eq!(
        l.totals.usd, None,
        "a total with an unpriced call is not known"
    );
    assert_eq!(l.totals.unpriced_calls, 1);
    assert!((l.totals.priced_usd - 4.0).abs() < 1e-9);
    assert_eq!(row(&l, "main", OPUS).tally.usd, Some(4.0));
    assert!(render(&l).contains("unpriced"));
}

#[test]
fn five_minute_and_one_hour_writes_are_priced_at_their_own_rates() {
    let usage = json!({"input_tokens": 0, "cache_creation_input_tokens": 3_000_000,
        "cache_read_input_tokens": 0, "output_tokens": 0,
        "cache_creation": {"ephemeral_5m_input_tokens": 2_000_000, "ephemeral_1h_input_tokens": 1_000_000}});
    let run = Run::new();
    run.transcript(&[
        assistant("a", OPUS, &at(0, 0, 0), usage),
        assistant("b", SONNET, &at(0, 1, 0), counts(0, 1_000_000, 0, 0)),
    ]);
    let l = run.ledger();
    let opus = &row(&l, "main", OPUS).tally;
    assert_eq!(
        (opus.cache_write, opus.cache_write_5m, opus.cache_write_1h),
        (3_000_000, 2_000_000, 1_000_000)
    );
    assert_eq!(opus.usd, Some(2.0 * 5.0 + 8.0));
    let sonnet = &row(&l, "main", SONNET).tally;
    assert_eq!(
        (sonnet.cache_write_5m, sonnet.cache_write_1h),
        (1_000_000, 0)
    );
    assert_eq!(sonnet.usd, Some(2.5));
}

#[test]
fn synthetic_messages_are_not_calls() {
    let run = Run::new();
    run.transcript(&[
        assistant("s", "<synthetic>", &at(0, 0, 0), counts(0, 0, 0, 0)),
        assistant("a", OPUS, &at(0, 1, 0), counts(1, 1, 1, 1)),
    ]);
    let l = run.ledger();
    assert_eq!(l.totals.calls, 1);
    assert_eq!(l.copied_calls, 0);
    assert_eq!(l.by_agent.len(), 1);
}

fn api_request(id: &str) -> String {
    json!({"type": "api-request", "id": id, "timestamp": at(0, 0, 0)}).to_string()
}
fn referenced(id: &str, request: &str, usage: Value) -> String {
    json!({"type": "assistant", "timestamp": at(30, 0, 0), "requestRef": request,
           "message": {"id": id, "model": OPUS, "content": [
               {"type": "tool_use", "id": format!("tool-{id}"), "name": "mcp__x__codex_run", "input": {}}
           ], "usage": usage}})
    .to_string()
}

#[test]
fn history_copied_from_an_earlier_session_is_excluded() {
    let compact = json!({"type": "system", "subtype": "compact_boundary"}).to_string();
    let run = Run::new();
    let copied_report = json!({"model": "gpt-terra", "usage": {"inputTokens": 999}}).to_string();
    run.transcript(&[
        compact.clone(),
        referenced("old1", "gone-1", counts(1000, 1000, 1000, 1000)),
        tool_result("tool-old1", json!(copied_report)),
        referenced("old2", "gone-2", counts(0, 0, 0, 0)),
        api_request("live-1"),
        referenced("new1", "live-1", counts(1, 2, 3, 4)),
        compact,
    ]);
    let l = run.ledger();
    assert_eq!(l.totals.calls, 1);
    assert_eq!((l.totals.input, l.totals.output), (1, 4));
    assert_eq!(l.copied_calls, 2);
    assert_eq!(
        l.compactions, 1,
        "only the one after this session's own first call"
    );
    assert!(
        l.codex.is_empty(),
        "codex runs of copied calls are excluded too"
    );
    assert_eq!(l.codex_runs_without_usage, 0);
    assert!(l
        .warnings
        .iter()
        .all(|w| !matches!(w, Warning::CopiedHistoryNotSeparable { .. })));
}

#[test]
fn a_format_without_request_records_warns_that_copies_may_be_included() {
    let zeroed = line_with("old", OPUS, &at(0, 0, 0), counts(0, 0, 0, 0), json!([]));
    let run = Run::new();
    run.transcript(&[
        zeroed.clone(),
        zeroed,
        json!({"type": "assistant", "timestamp": at(0, 1, 0), "requestRef": "unmatched",
               "message": {"id": "kept", "model": OPUS, "content": [], "usage": counts(1, 1, 1, 1)}})
        .to_string(),
    ]);
    let l = run.ledger();
    assert_eq!(
        l.totals.calls, 1,
        "without request records a reference proves nothing"
    );
    assert_eq!(l.copied_calls, 1);
    assert!(l.warnings.iter().any(|w| matches!(
        w,
        Warning::CopiedHistoryNotSeparable {
            zeroed_calls: 1,
            ..
        }
    )));
}

#[test]
fn tool_records_of_zeroed_copies_are_excluded_with_their_calls() {
    let report = json!({"model": "gpt-terra", "usage": {"inputTokens": 999, "outputTokens": 9}})
        .to_string();
    let big = "x".repeat(LARGE_READ_BYTES as usize);
    let run = Run::new();
    run.transcript(&[
        line_with(
            "old",
            OPUS,
            &at(0, 0, 0),
            counts(0, 0, 0, 0),
            json!([
                tool_use("old-codex", "mcp__plugin_chronos_bridge__codex_run", json!({})),
                tool_use("old-read", "Read", json!({"file_path": "big.txt"})),
            ]),
        ),
        tool_result("old-codex", json!(report)),
        tool_result("old-read", json!(big)),
        assistant("new", OPUS, &at(0, 1, 0), counts(1, 1, 1, 1)),
    ]);
    let l = run.ledger();
    assert_eq!(l.totals.calls, 1);
    assert_eq!(l.copied_calls, 1);
    assert!(l.codex.is_empty(), "{:?}", l.codex);
    assert_eq!(l.codex_runs_without_usage, 0);
    assert!(
        l.warnings
            .iter()
            .all(|w| !matches!(w, Warning::LargeFileRead { .. })),
        "{:?}",
        l.warnings
    );
}

#[test]
fn a_timestamp_with_a_multibyte_character_is_unreadable_not_fatal() {
    let run = Run::new();
    run.transcript(&[
        assistant("a", OPUS, "2026-01-01T00:00:\u{20ac}Z", counts(1, 1, 1, 1)),
        assistant("b", OPUS, "2026-01-01T00:00:0\u{20ac}Z", counts(1, 1, 1, 1)),
        assistant("c", OPUS, &at(0, 1, 0), counts(1, 1, 1, 1)),
    ]);
    assert_eq!(run.ledger().totals.calls, 3);
}

#[test]
fn a_single_agent_above_the_context_threshold_is_warned_about_once() {
    let run = Run::new();
    run.transcript(&[
        assistant("a", OPUS, &at(0, 0, 0), counts(0, 0, CONTEXT_WARN, 1)),
        assistant("b", OPUS, &at(0, 1, 0), counts(0, 0, CONTEXT_WARN + 100, 1)),
    ]);
    run.subagent(
        "under",
        Some("chronos:reviewer"),
        &[assistant(
            "c",
            SONNET,
            &at(0, 0, 0),
            counts(0, 0, CONTEXT_WARN, 1),
        )],
    );
    let l = run.ledger();
    assert_eq!(
        l.warnings,
        vec![Warning::ContextHigh {
            agent_type: "main".into(),
            instance: "main".into(),
            model: OPUS.into(),
            peak_context: CONTEXT_WARN + 100,
        }]
    );
}

#[test]
fn whole_reads_of_large_files_are_warned_about_and_bounded_ones_are_not() {
    let big = "x".repeat(LARGE_READ_BYTES as usize);
    let small = "x".repeat(LARGE_READ_BYTES as usize - 1);
    let read = |id: &str, path: &str, input: Value| {
        let mut input = input;
        input["file_path"] = json!(path);
        tool_use(id, "Read", input)
    };
    let run = Run::new();
    run.transcript(&[
        line_with(
            "a",
            OPUS,
            &at(0, 0, 0),
            counts(1, 1, 1, 1),
            json!([
                read("whole", "big.txt", json!({})),
                read("wide", "big.txt", json!({"limit": 501})),
                read("bounded", "big.txt", json!({"limit": 500})),
                read("short", "small.txt", json!({})),
                read("listed", "listed.txt", json!({})),
            ]),
        ),
        tool_result("whole", json!(big)),
        tool_result("wide", json!(big)),
        tool_result("bounded", json!(big)),
        tool_result("short", json!(small)),
        tool_result("listed", json!([{"type": "text", "text": big}])),
    ]);
    let l = run.ledger();
    let mut reads: Vec<_> = l
        .warnings
        .iter()
        .filter_map(|w| match w {
            Warning::LargeFileRead {
                path,
                reads,
                largest_bytes,
                ..
            } => Some((path.as_str(), *reads, *largest_bytes)),
            _ => None,
        })
        .collect();
    reads.sort();
    assert_eq!(
        reads,
        vec![
            ("big.txt", 2, LARGE_READ_BYTES),
            ("listed.txt", 1, LARGE_READ_BYTES),
        ]
    );
}

#[test]
fn codex_tokens_come_from_run_results_and_missing_usage_is_counted() {
    let usage = |input: u64| {
        json!({"status": "completed", "model": "gpt-terra", "usage": {
            "inputTokens": input, "cachedInputTokens": input / 2, "outputTokens": 30,
            "reasoningOutputTokens": 10, "totalTokens": input + 30}})
        .to_string()
    };
    let run = Run::new();
    run.subagent(
        "worker",
        Some("chronos:dispatcher"),
        &[
            line_with(
                "a",
                SONNET,
                &at(0, 0, 0),
                counts(1, 1, 1, 1),
                json!([
                    tool_use("c1", "mcp__plugin_chronos_bridge__codex_run", json!({})),
                    tool_use("c2", "mcp__plugin_chronos_bridge__codex_run", json!({})),
                    tool_use("c3", "mcp__plugin_chronos_bridge__codex_run", json!({})),
                    tool_use("c4", "mcp__plugin_chronos_bridge__codex_run", json!({})),
                    tool_use("other", "Bash", json!({"command": "ls"})),
                ]),
            ),
            tool_result("c1", json!(usage(100))),
            tool_result("c2", json!([{"type": "text", "text": usage(200)}])),
            tool_result(
                "c3",
                json!(
                    json!({"status": "bridge_error", "error": {"kind": "not_ready"}}).to_string()
                ),
            ),
            tool_result("c4", json!("{\"truncated\": \"before the use")),
            tool_result("other", json!(usage(9999))),
        ],
    );
    run.transcript(&[assistant("m", OPUS, &at(0, 0, 0), counts(1, 1, 1, 1))]);
    let l = run.ledger();
    assert_eq!(l.codex.len(), 1);
    let c = &l.codex[0];
    assert_eq!(
        (
            c.model.as_str(),
            c.runs,
            c.tokens.input,
            c.tokens.cached_input,
            c.tokens.output,
            c.tokens.reasoning_output,
            c.tokens.total
        ),
        ("gpt-terra", 2, 300, 150, 60, 20, 360)
    );
    assert_eq!(l.codex_runs_without_usage, 2);
}

fn stream(id: &str, model: &str, parent: Option<&str>, output: u64) -> String {
    json!({"type": "assistant", "timestamp": at(0, 0, 0), "parent_tool_use_id": parent,
           "message": {"id": id, "model": model, "content": [], "usage": counts(1, 2, 3, output)}})
    .to_string()
}

#[test]
fn a_stream_log_alone_gives_a_ledger_flagged_as_partial() {
    let spawn = json!({"type": "assistant", "timestamp": at(0, 0, 0), "parent_tool_use_id": null,
        "message": {"id": "m1", "model": OPUS, "usage": counts(1, 2, 3, 4), "content": [
            tool_use("agent-call", "Agent", json!({"subagent_type": "chronos:reviewer", "description": "Review it"}))
        ]}})
    .to_string();
    let run = Run::new();
    run.log(
        "20260101-000000-wish.jsonl",
        &[
            init(SESSION, "E:\\project"),
            spawn.clone(),
            spawn,
            stream("s1", SONNET, Some("agent-call"), 7),
            stream("s1", SONNET, Some("agent-call"), 7),
            stream("s2", SONNET, Some("not-seen"), 8),
            json!({"type": "result", "total_cost_usd": 1.5}).to_string(),
        ],
    );
    let l = run.ledger();
    assert_eq!(l.source, "log");
    assert!(l.output_tokens_partial);
    assert_eq!(l.cwd.as_deref(), Some("E:\\project"));
    assert_eq!(l.claude_code_usd, Some(1.5));
    assert_eq!(row(&l, "main", OPUS).tally.calls, 1);
    assert_eq!(row(&l, "chronos:reviewer", SONNET).tally.output, 7);
    assert_eq!(row(&l, "unknown", SONNET).tally.output, 8);
    let reviewer = l.instances.iter().find(|i| i.id == "agent-call").unwrap();
    assert_eq!(reviewer.description.as_deref(), Some("Review it"));
    assert!(render(&l).contains("lower bounds"));
}

#[test]
fn a_transcript_outranks_the_log_whose_output_tokens_are_partial() {
    let run = Run::new();
    run.log(
        "20260101-000000-wish.jsonl",
        &[
            init(SESSION, "E:\\project"),
            stream("m1", OPUS, None, 5),
            stream("only-in-log", OPUS, None, 5),
        ],
    );
    run.transcript(&[assistant("m1", OPUS, &at(0, 0, 0), counts(1, 2, 3, 146))]);
    let l = run.ledger();
    assert_eq!(l.source, "transcript");
    assert!(!l.output_tokens_partial);
    assert_eq!(l.totals.output, 146);
    assert_eq!(l.totals.calls, 1);
}

#[test]
fn claude_codes_own_total_is_the_last_one_saved_in_the_main_transcript() {
    let state = |usd: f64| json!({"type": "cost-state", "totalCostUSD": usd}).to_string();
    let run = Run::new();
    run.transcript(&[
        assistant("a", OPUS, &at(0, 0, 0), counts(1, 1, 1, 1)),
        state(1.0),
        state(2.5),
    ]);
    run.subagent(
        "worker",
        Some("general-purpose"),
        &[
            assistant("b", SONNET, &at(0, 0, 0), counts(1, 1, 1, 1)),
            state(9.0),
        ],
    );
    assert_eq!(run.ledger().claude_code_usd, Some(2.5));
}

#[test]
fn a_run_with_no_calls_costs_nothing() {
    let run = Run::new();
    run.transcript(&[
        json!({"type": "user", "message": {"role": "user", "content": "hi"}}).to_string(),
    ]);
    let l = run.ledger();
    assert_eq!(l.totals.calls, 0);
    assert_eq!(l.totals.usd, Some(0.0));
    assert!(l.by_agent.is_empty());
}

#[test]
fn real_working_directories_map_to_their_transcript_directories() {
    for (cwd, dir) in [
        (r"E:\codex\ML_HfO2", "E--codex-ML-HfO2"),
        (r"E:\codex\OPEN_DESIGN\axiom", "E--codex-OPEN-DESIGN-axiom"),
        (r"C:\Users\someone", "C--Users-someone"),
        (r"E:\codex\OPEN_DESIGN\TW\TaiwanStockRadar", "E--codex-OPEN-DESIGN-TW-TaiwanStockRadar"),
        (r"E:\codex\ML_SIC\OPEN_SOURCE_SKILLS", "E--codex-ML-SIC-OPEN-SOURCE-SKILLS"),
        (
            r"E:\codex\OPEN_DESIGN\axiom\experiments\bench\ledger-r3",
            "E--codex-OPEN-DESIGN-axiom-experiments-bench-ledger-r3",
        ),
        (
            r"C:\Users\someone\AppData\Local\Temp\claude\E--codex-OPEN-DESIGN-axiom\51327f98-867e-4182-ad6c-b4efb141f64a\scratchpad\toy-rust",
            "C--Users-someone-AppData-Local-Temp-claude-E--codex-OPEN-DESIGN-axiom-51327f98-867e-4182-ad6c-b4efb141f64a-scratchpad-toy-rust",
        ),
    ] {
        assert_eq!(transcript_dir_name(cwd), dir);
    }
}

#[test]
fn the_newest_run_is_the_one_most_recently_active_across_logs_goals_and_transcripts() {
    const HEADLESS: &str = "aaaaaaaa-0000-0000-0000-000000000000";
    const WISH: &str = "bbbbbbbb-0000-0000-0000-000000000000";
    const STALE: &str = "cccccccc-0000-0000-0000-000000000000";
    let run = Run::new();
    let log = run.log(
        "20260101-000000-wish.jsonl",
        &[init(HEADLESS, "E:\\one"), stream("h1", OPUS, None, 1)],
    );
    let headless = run.transcript_at(
        "E--one",
        HEADLESS,
        &[assistant("h1", OPUS, &at(0, 0, 0), counts(1, 1, 1, 1))],
    );
    let goal = run.goal(WISH, "E:\\two");
    let wish = run.transcript_at(
        "E--two",
        WISH,
        &[assistant("w1", OPUS, &at(0, 0, 0), counts(2, 2, 2, 2))],
    );
    let no_records = run.goal(STALE, "E:\\three");
    run.log(
        "20260102-000000-wish.jsonl",
        &["{\"hostEvent\":\"threadStarted\"}".to_owned()],
    );
    fs::write(
        run.locations()
            .home
            .join("logs")
            .join("20260103-000000-wish.jsonl"),
        "",
    )
    .unwrap();

    for path in [&log, &headless, &goal] {
        age(path, 500);
    }
    age(&wish, 100);
    assert_eq!(ledger(&run.locations(), None).unwrap().session, WISH);

    age(&wish, 900);
    age(&goal, 900);
    age(&no_records, 900);
    age(&log, 10);
    assert_eq!(ledger(&run.locations(), None).unwrap().session, HEADLESS);
}

#[test]
fn a_named_session_is_found_without_a_goal_or_log_and_its_cwd_read_from_the_transcript() {
    let run = Run::new();
    run.transcript_at(
        "E--somewhere-odd",
        SESSION,
        &[
            json!({"type": "user", "cwd": "E:\\somewhere\\odd", "message": {"role": "user", "content": "hi"}}).to_string(),
            assistant("a", OPUS, &at(0, 0, 0), counts(1, 1, 1, 1)),
        ],
    );
    let l = run.ledger();
    assert_eq!(l.totals.calls, 1);
    assert_eq!(l.cwd.as_deref(), Some("E:\\somewhere\\odd"));
}

#[test]
fn the_directory_for_the_recorded_cwd_wins_over_a_stray_copy() {
    let run = Run::new();
    run.log("20260101-000000-wish.jsonl", &[init(SESSION, "E:\\real")]);
    run.transcript_at(
        "E--aaa-stray",
        SESSION,
        &[assistant("x", OPUS, &at(0, 0, 0), counts(9, 9, 9, 9))],
    );
    run.transcript_at(
        "E--real",
        SESSION,
        &[assistant("y", OPUS, &at(0, 0, 0), counts(1, 1, 1, 1))],
    );
    assert_eq!(run.ledger().totals.input, 1);
}

#[test]
fn missing_and_invalid_sessions_fail_explicitly() {
    let run = Run::new();
    let e = ledger(&run.locations(), Some(SESSION)).unwrap_err();
    assert_eq!(e.kind, "not_found");
    let e = ledger(&run.locations(), None).unwrap_err();
    assert_eq!(e.kind, "not_found");
    let e = ledger(&run.locations(), Some("../../etc/passwd")).unwrap_err();
    assert_eq!(e.kind, "invalid_argument");
}

fn axiom(run: &Run) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_axiom"));
    c.env("AXIOM_HOME", run.root.path().join("home"))
        .env("CLAUDE_CONFIG_DIR", run.root.path().join("claude"));
    c
}

#[test]
fn the_cli_prints_a_ledger_as_text_or_json_and_reports_errors() {
    let run = Run::new();
    run.transcript(&[assistant(
        "a",
        OPUS,
        &at(0, 0, 0),
        counts(1_000_000, 0, 0, 2_000),
    )]);
    run.subagent(
        "worker",
        Some("chronos:reviewer"),
        &[assistant(
            "b",
            SONNET,
            &at(0, 0, 0),
            counts(0, 0, 1_234_567, 1),
        )],
    );

    let o = axiom(&run)
        .args(["usage", "--session", SESSION])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let text = String::from_utf8(o.stdout).unwrap();
    for expected in [
        SESSION,
        "chronos:reviewer",
        OPUS,
        "1,234,567",
        "$4.04",
        "total",
    ] {
        assert!(text.contains(expected), "{expected} missing from {text}");
    }

    let o = axiom(&run)
        .args(["usage", "--json", "--session", SESSION])
        .output()
        .unwrap();
    assert!(o.status.success());
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["session"], SESSION);
    assert_eq!(v["source"], "transcript");
    assert_eq!(v["totals"]["calls"], 2);
    assert_eq!(v["totals"]["peak_context"], 1_234_567);
    assert_eq!(v["by_agent"][0]["agent_type"], "main");
    assert_eq!(v["by_agent"][0]["cache_write_1h"], 0);
    assert_eq!(v["instances"][0]["id"], "worker");
    assert_eq!(v["prices"]["as_of"], "2026-11-09");
    assert_eq!(v["warnings"].as_array().unwrap().len(), 2);
    assert_eq!(v["warnings"][0]["kind"], "context_high");
    assert_eq!(v["warnings"][0]["peak_context"], 1_234_567);
    assert_eq!(v["warnings"][1]["peak_context"], 1_000_000);

    let o = axiom(&run)
        .args(["usage", "--session", "no-such-run"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("no records for session no-such-run"));

    for bad in [
        vec!["usage", "--session"],
        vec!["usage", "--what"],
        vec!["usage", "--session", "../x"],
    ] {
        let o = axiom(&run).args(bad).output().unwrap();
        assert_eq!(o.status.code(), Some(2));
    }
}

#[test]
fn the_cli_defaults_to_the_newest_run() {
    let run = Run::new();
    run.log(
        "20260101-000000-wish.jsonl",
        &[init(SESSION, "E:\\project"), stream("h1", OPUS, None, 3)],
    );
    let o = axiom(&run).args(["usage", "--json"]).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["session"], SESSION);
    assert_eq!(v["source"], "log");
    assert_eq!(v["totals"]["output"], 3);
}
