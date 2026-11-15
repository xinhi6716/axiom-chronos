mod common;
#[path = "phase7/mod.rs"]
mod phase7;

use axiom::orch::{self, events};
use phase7::{Call, Metrics, Provider};

use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

fn seeds() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/phase7/acceptance")
}
fn fixture() -> phase7::Workload {
    phase7::run_workload_scripted(&seeds())
}
fn metrics(w: &phase7::Workload) -> Metrics {
    phase7::collect(w.home.path(), &w.project)
}
fn frozen_usage_mismatch<'a>(before: &Metrics, current: &'a Metrics) -> Option<&'a str> {
    current.by_model_role.iter().find_map(|(key, totals)| {
        if key.split('|').next().unwrap().starts_with("claude")
            && before
                .by_model_role
                .get(key)
                .map(|frozen| frozen.tokens.is_some())
                != Some(totals.tokens.is_some())
        {
            Some(key.as_str())
        } else {
            None
        }
    })
}
fn call(id: &str, input: Option<u64>) -> Call {
    Call {
        provider: Provider::Claude,
        role: "dispatcher".into(),
        model: "claude-opus-5-5".into(),
        unit: None,
        attempt: None,
        input_tokens: input,
        output_tokens: Some(5),
        id: id.into(),
    }
}
fn add_call(w: &phase7::Workload, c: &Call) {
    events::append_event(
        &w.store,
        "phase7_call",
        None,
        None,
        serde_json::to_value(c).unwrap(),
    )
    .unwrap();
}

#[test]
fn collect_counts_claude_per_unit() {
    let w = fixture();
    let m = metrics(&w);
    assert_eq!(m.verified_units, 2);
    assert_eq!(
        m.verified_units,
        orch::receipts::compute(&w.store, 1).unwrap().verified_units
    );
    assert_eq!(m.claude_calls_per_unit, Some(3.0));
    assert_eq!(m.claude_tokens_per_unit, Some(45.0));
}
#[test]
fn duplicate_cost_events_counted_once() {
    let w = fixture();
    let c = call("duplicate", Some(10));
    add_call(&w, &c);
    let once = metrics(&w);
    add_call(&w, &c);
    assert_eq!(metrics(&w), once);
    let spend = json!({"id":"ledger-duplicate", "role":"reviewer", "model":"claude-opus-5-5", "usd":0.1, "input_tokens":10, "output_tokens":5, "call":"review"});
    events::append_event(
        &w.store,
        axiom::engine::kinds::ROLE_SPEND,
        None,
        None,
        spend.clone(),
    )
    .unwrap();
    let once = metrics(&w);
    events::append_event(
        &w.store,
        axiom::engine::kinds::ROLE_SPEND,
        None,
        None,
        spend,
    )
    .unwrap();
    assert_eq!(metrics(&w), once);
}
#[test]
fn missing_usage_is_unknown() {
    let w = fixture();
    assert!(metrics(&w).claude_tokens_per_unit.is_some());
    add_call(&w, &call("missing", None));
    let m = metrics(&w);
    assert_eq!(m.claude_tokens_per_unit, None);
    assert_eq!(m.by_model_role["claude-opus-5-5|dispatcher"].tokens, None);
    assert_eq!(m.fixture_usd, None);
    let mut codex = call("missing-codex", None);
    codex.provider = Provider::Codex;
    codex.model = "gpt-6.1-terra".into();
    let other = fixture();
    add_call(&other, &codex);
    assert_eq!(metrics(&other).claude_tokens_per_unit, None);
}
#[test]
fn zero_verified_units_is_none() {
    let tmp = common::temp();
    let project = "empty";
    let store = orch::store_for(tmp.path(), project).unwrap();
    fs::create_dir_all(&store).unwrap();
    fs::write(
        store.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n",
    )
    .unwrap();
    let m = phase7::collect(tmp.path(), project);
    assert_eq!(m.verified_units, 0);
    assert_eq!(m.claude_calls_per_unit, None);
    assert_eq!(m.claude_tokens_per_unit, None);
    assert_eq!(m.rework_rate, None);
}
#[test]
fn repeated_resume_not_double_counted() {
    let w = fixture();
    let before = metrics(&w);
    w.resume();
    w.resume();
    assert_eq!(metrics(&w), before);
}
#[test]
fn supersede_without_review_is_rework() {
    let w = fixture();
    let before = metrics(&w);
    w.supersede_without_review();
    let after = metrics(&w);
    assert_eq!(after.rework.get("supersede_no_failure"), Some(&1));
    assert!(after.rework_rate.unwrap() > before.rework_rate.unwrap());
}
#[test]
fn passes_requires_strict_improvement() {
    let w = fixture();
    let before = metrics(&w);
    let mut after = before.clone();
    after.claude_calls_per_unit = Some(before.claude_calls_per_unit.unwrap() - 0.5);
    after.claude_tokens_per_unit = Some(before.claude_tokens_per_unit.unwrap() - 1.0);
    assert!(phase7::passes(&before, &after).is_ok());
    let mut bad = after.clone();
    bad.claude_calls_per_unit = before.claude_calls_per_unit;
    assert!(phase7::passes(&before, &bad).is_err());
    bad = after.clone();
    bad.claude_tokens_per_unit = before.claude_tokens_per_unit;
    assert!(phase7::passes(&before, &bad).is_err());
    bad = after.clone();
    bad.rework_rate = Some(before.rework_rate.unwrap() + 1.0);
    assert!(phase7::passes(&before, &bad).is_err());
    bad = after.clone();
    bad.claude_tokens_per_unit = None;
    assert!(phase7::passes(&before, &bad).is_err());
    bad = before.clone();
    bad.rework_rate = None;
    assert!(phase7::passes(&bad, &after).is_err());
}
#[test]
fn before_json_frozen() {
    let path = seeds().parent().unwrap().join("before.json");
    let before: Metrics = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert!(before.verified_units > 0);
    let current = metrics(&fixture());
    assert_eq!(frozen_usage_mismatch(&before, &current), None);
    let label = phase7::fixture_cost_label(&before);
    assert!(label.contains("fixture-only"));
    println!("{label}");
}
#[test]
fn frozen_usage_mismatch_detected() {
    let path = seeds().parent().unwrap().join("before.json");
    let mut before: Metrics = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let current = metrics(&fixture());
    let key = "claude-opus-5-5|dispatcher";
    let tokens = &mut before.by_model_role.get_mut(key).unwrap().tokens;
    *tokens = if tokens.is_some() { None } else { Some(0) };
    assert_eq!(frozen_usage_mismatch(&before, &current), Some(key));
}
#[test]
fn acceptance_seeds_frozen() {
    let dir = seeds();
    let mut files: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    assert_eq!(files.len(), 2);
    let hashes = fs::read_to_string(dir.parent().unwrap().join("acceptance.sha256")).unwrap();
    for p in files {
        let bytes = fs::read(&p).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            text.contains("CONTRACT") && text.contains("TEST_MATRIX") && text.contains("FAKE_")
        );
        let line = format!(
            "{:x}  {}",
            Sha256::digest(&bytes),
            p.file_name().unwrap().to_str().unwrap()
        );
        assert!(hashes.lines().any(|s| s == line));
    }
    assert_eq!(hashes.lines().count(), 2);
}
#[test]
fn workload_runs_isolated() {
    let a = fixture();
    let b = fixture();
    assert_ne!(a.home.path(), b.home.path());
    let original = metrics(&b);
    add_call(&a, &call("only-a", Some(10)));
    assert_eq!(metrics(&b), original);
    assert_ne!(metrics(&a), original);
}

// TEST_MATRIX (observable regression coverage):
// r1 behaviour routed_map_parses_with_astra_unit: every module routes, including Astra.
// r2 behaviour briefs_carry_tier_effort_matrix: parsed briefs and actual RPC routing agree.
// r3 behaviour selfcheck_and_astra_review_run_in_engine: findings precede Astra review.
// r4 behaviour core_stage_switches_to_sonnet: stage log and control request agree.
// r5 behaviour claude_calls_equal_fake_log: usage and result counts come from the fake.
// r6 failure scripted_constant_fails_log_crosscheck: an invented id is rejected.
// r7 edge frozen_inputs_untouched: frozen hashes and baseline metrics remain intact.
// r8 behaviour after_beats_before: unchanged improvement criterion passes.
fn routed_fixture() -> phase7::Workload { phase7::run_workload(&seeds()) }
fn fake_results(w: &phase7::Workload) -> Vec<serde_json::Value> {
    fs::read_dir(w.home.path()).unwrap().filter_map(|e| {
        let p = e.unwrap().path();
        if !p.file_name()?.to_str()?.starts_with("claude-") || p.extension()? != "json" { return None; }
        let v: serde_json::Value = serde_json::from_slice(&fs::read(p).unwrap()).unwrap();
        Some(v["results"].as_array().cloned().unwrap_or_default())
    }).flatten().collect()
}
fn log_crosscheck(w: &phase7::Workload) -> Result<(), String> {
    phase7::verify_claude_log(w)
}

#[test]
fn routed_map_parses_with_astra_unit() {
    let w = routed_fixture();
    let text = fs::read_to_string(w.store.join("modules.md")).unwrap();
    let map = orch::parse::parse_map(&text).unwrap();
    assert!(map.modules.iter().all(|m| m.routing.iter().any(|r| r.key == "default")));
    assert!(map.modules.iter().any(|m| m.routing.iter().any(|r| r.key != "default" && r.tier == "astra")));
}
#[test]
fn briefs_carry_tier_effort_matrix() {
    let w = routed_fixture();
    let map = orch::parse::parse_map(&fs::read_to_string(w.store.join("modules.md")).unwrap()).unwrap();
    let log: Vec<serde_json::Value> = fs::read_to_string(w.home.path().join("codex.log")).unwrap().lines().map(|s| serde_json::from_str(s).unwrap()).collect();
    for m in map.modules {
        let r = m.routing.iter().find(|r| r.key == m.id).unwrap();
        let brief = fs::read_to_string(w.store.join(format!("{}.md", m.id))).unwrap();
        for line in [format!("TIER {}", r.tier), format!("EFFORT {}", r.effort)] { assert!(brief.lines().any(|s| s == line)); }
        let seed = fs::read_to_string(seeds().join(format!("{}.md", m.id))).unwrap();
        assert!(seed.lines().filter(|s| s.starts_with("TEST_MATRIX")).all(|s| brief.lines().any(|b| b == s)));
        let parsed=orch::parse_brief(&brief).unwrap();
        assert_eq!(parsed.tier.as_deref(),Some(r.tier.as_str()));
        assert_eq!(parsed.effort.as_deref(),Some(r.effort.as_str()));
        assert!(!parsed.test_matrix.is_empty());
        let turn=log.iter().find(|v| v["method"] == "turn/start" && v["params"]["input"][0]["text"].as_str().unwrap().contains(&parsed.goal)).unwrap();
        let thread=log.iter().find(|v| v["method"] == "thread/start" && v["params"]["model"].as_str().is_some_and(|s| s.ends_with(&r.tier)));
        assert!(thread.is_some());
        assert_eq!(turn["params"]["effort"],r.effort);
    }
}
#[test]
fn selfcheck_and_astra_review_run_in_engine() {
    let w = routed_fixture();
    for (u, kind) in [("vacuous","vacuous_check"),("missing","untested")] {
        let rows = orch::evidence(&w.store,u).unwrap();
        assert!(rows.iter().position(|r| r.kind == "selfcheck").unwrap() < rows.iter().position(|r| r.kind == "review").unwrap());
        let v: serde_json::Value = serde_json::from_slice(&fs::read(w.store.join(format!("reports/{u}.selfcheck.json"))).unwrap()).unwrap();
        assert!(v["findings"].as_array().unwrap().iter().any(|f| f["kind"] == kind));
        // Engine-owned reviews are namespaced engine-astra:<resolved model>.
        assert!(events::read_events(&w.store,0).unwrap().iter().any(|e| e.kind == "review_recorded" && e.unit.as_deref() == Some(u) && e.data["reviewer"].as_str().is_some_and(|r| r.strip_prefix("engine-").unwrap_or(r).starts_with("astra"))));
    }
    assert!(!events::read_events(&w.store,0).unwrap().iter().any(|e| e.kind == "phase7_call" && e.data["provider"] == "claude" && e.data["role"] == "reviewer"));
}
#[test]
fn core_stage_switches_to_sonnet() {
    let w = routed_fixture();
    let rows: Vec<_> = fs::read_dir(w.home.path().join("goals")).unwrap().filter_map(|e| {
        let p=e.unwrap().path(); if !p.to_string_lossy().ends_with(".turns.jsonl") {return None;}
        Some(fs::read_to_string(p).unwrap().lines().map(|s| serde_json::from_str::<serde_json::Value>(s).unwrap()).collect::<Vec<_>>())
    }).flatten().collect();
    assert!(rows.iter().any(|r| r["stage"] == "build" && r["model"].as_str().unwrap().contains("sonnet") && r["ok"] == true));
    assert!(rows.iter().any(|r| (r["stage"] == "planning" || r["stage"] == "close") && r["model"].as_str().unwrap().contains("opus")));
    let log: serde_json::Value=serde_json::from_slice(&fs::read(w.home.path().join("claude-core.json")).unwrap()).unwrap();
    assert!(log["control_requests"].as_array().unwrap().iter().any(|r| r["request"]["subtype"] == "set_model" && r["request"]["model"] == "sonnet"));
}
#[test]
fn claude_calls_equal_fake_log() {
    let w = routed_fixture(); let m=metrics(&w);
    let n: u64=m.by_model_role.iter().filter(|(k,_)| k.starts_with("claude") || k.starts_with("opus|") || k.starts_with("sonnet|")).map(|(_,v)| v.calls).sum();
    assert_eq!(n as usize,fake_results(&w).len()); assert_ne!(n as usize,3*m.verified_units);
    assert!(log_crosscheck(&w).is_ok());
    let tokens:u64=fake_results(&w).iter().flat_map(|r|r["modelUsage"].as_object().unwrap().values()).map(|u|u["inputTokens"].as_u64().unwrap()+u["outputTokens"].as_u64().unwrap()).sum();
    assert_eq!(m.claude_tokens_per_unit,Some(tokens as f64/m.verified_units as f64));
}
#[test]
fn scripted_constant_fails_log_crosscheck() {
    let w=routed_fixture(); assert!(log_crosscheck(&w).is_ok());
    add_call(&w,&call("not-in-fake-log",Some(10)));
    assert!(log_crosscheck(&w).is_err());
}
#[test]
fn frozen_inputs_untouched() {
    let _w=routed_fixture(); acceptance_seeds_frozen(); before_json_frozen();
    let b: Metrics=serde_json::from_slice(&fs::read(seeds().parent().unwrap().join("before.json")).unwrap()).unwrap();
    assert_eq!((b.claude_calls_per_unit,b.claude_tokens_per_unit,b.rework_rate),(Some(3.0),Some(45.0),Some(0.0)));
}
#[test]
fn after_beats_before() {
    let w=routed_fixture();
    let b: Metrics=serde_json::from_slice(&fs::read(seeds().parent().unwrap().join("before.json")).unwrap()).unwrap();
    phase7::passes(&b,&metrics(&w)).unwrap();
}


