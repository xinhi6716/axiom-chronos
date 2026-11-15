use axiom::graph::{
    self,
    code::SyncOptions,
    measure::{self, ClaudeUsage, RunUsage},
    GraphStore,
};
use axiom::{memory::Memory, orch};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path};

fn fixtures() -> &'static Path {
    Path::new("tests/fixtures/graph/eval")
}
fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for e in fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dst.join(e.file_name()));
        } else {
            fs::copy(e.path(), dst.join(e.file_name())).unwrap();
        }
    }
}
fn usage(n: u64) -> ClaudeUsage {
    ClaudeUsage {
        input: Some(n),
        cache_write: Some(10),
        cache_read: Some(20),
        output: Some(7),
    }
}
#[test]
fn frozen_scenarios_seed_and_brief() {
    let m = measure::load_manifest(fixtures()).unwrap();
    assert_eq!(m.version, 1);
    assert_eq!(m.frozen, "2026-11-12");
    assert_eq!(m.scenarios.len(), 12);
    let t = &m.thresholds;
    assert_eq!(t.claude_processed_input_median_saving_min, 0.1);
    assert_eq!(t.required_evidence_regressions_max, 0);
    assert_eq!(t.condition_regressions_max, 0);
    assert_eq!(t.major_finding_regressions_max, 0);
    assert_eq!(t.wrong_certain_edges_max, 0);
    assert_eq!(t.budget_overruns_max, 0);
    assert_eq!(t.resolution_recall_min, 0.7);
    for s in &m.scenarios {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let home = temp.path().join("home");
        copy_dir(&fixtures().join(&s.repo), &repo);
        let store = GraphStore::open(&home).unwrap();
        graph::code::sync(&store, &repo, None, &SyncOptions::default()).unwrap();
        measure::seed_home(&home, &repo, fixtures(), Path::new(&s.seed)).unwrap();
        let brief = measure::graph_brief(&home, &repo, s).unwrap();
        let seed: Value =
            serde_json::from_slice(&fs::read(fixtures().join(&s.seed)).unwrap()).unwrap();
        let constraints = seed["constraints"].as_str().unwrap();
        for line in constraints.lines().filter(|l| !l.is_empty()) {
            assert!(brief.contains(line), "{} missing {line}", s.id);
        }
        let pos = brief.find("\n\n{").unwrap() + 2;
        let payload = &brief[pos..];
        let _: Value = serde_json::from_str(payload).unwrap();
        assert!(payload.len() <= s.budget, "{} {}", s.id, payload.len());
        let a = fs::read_to_string(fixtures().join(&s.a_brief)).unwrap();
        let p = measure::payload_check(s, &a, &brief, payload.len());
        assert!(!p.budget_overrun);
        let project = seed["project"].as_str().unwrap();
        let project_store = orch::wish_store(&home, project, None).unwrap();
        let wish_store =
            orch::wish_store(&home, project, Some("w-1700000000001-abcdef")).unwrap();
        let units = orch::read_units(&wish_store).unwrap();
        let entries = seed["entries"].as_array().unwrap();
        assert_eq!(units.len(), entries.len(), "{}", s.id);
        assert!(fs::read_to_string(project_store.join("aliases.csv"))
            .unwrap()
            .starts_with("canonical,alias\n"));
        let memory = Memory::open(&home).unwrap();
        let listed = memory
            .list(Some(project), entries.len() as i64 + 1)
            .unwrap();
        let records = listed["entries"].as_array().unwrap();
        assert_eq!(records.len(), entries.len(), "{}", s.id);
        for entry in entries {
            let unit = units
                .iter()
                .find(|unit| unit.id == entry["unit"].as_str().unwrap())
                .unwrap();
            assert_eq!(unit.state, "verified", "{}", s.id);
            let unit_brief = fs::read_to_string(&unit.brief).unwrap();
            let scope = unit_brief
                .lines()
                .find_map(|line| line.strip_prefix("SCOPE "))
                .unwrap();
            for file in entry["files"].as_array().unwrap() {
                assert!(scope.split_whitespace().any(|part| part == file.as_str().unwrap()));
            }
            assert!(
                records.iter().any(|record| {
                    let full = memory.get(record["id"].as_i64().unwrap()).unwrap();
                    let files = full["entry"]["files"].as_array().unwrap();
                    full["entry"]["unit"] == entry["unit"]
                        && full["entry"]["title"] == entry["title"]
                        && full["entry"]["summary"] == entry["summary"]
                        && full["entry"]["body"] == entry["body"]
                        && files.iter().map(|file| &file["path"]).collect::<Vec<_>>()
                            == entry["files"].as_array().unwrap().iter().collect::<Vec<_>>()
                        && full["entry"]["symbols"] == entry["symbols"]
                        && full["entry"]["wish"] == entry["wish"]
                }),
                "{} missing memory entry {}",
                s.id,
                entry["unit"]
            );
        }
        assert_eq!(
            graph::feedback::collect(&home, project)
                .unwrap()
                .iter()
                .filter(|e| e.kind == "correction")
                .count(),
            seed["amendments"].as_array().unwrap().len()
        );
        let scope = store.scope(&repo, None).unwrap().id;
        for research in seed["research"].as_array().unwrap() {
            let node = store
                .node(&graph::NodeRef {
                    scope,
                    key: graph::keys::artifact(research["path"].as_str().unwrap()),
                })
                .unwrap()
                .unwrap();
            let edges = store.edges(node.id, graph::Direction::Out, 100).unwrap();
            for link in research["links"].as_array().unwrap() {
                assert!(
                    edges
                        .iter()
                        .any(|edge| edge.kind == "cites"
                            && edge.dst.key == link["to"].as_str().unwrap()),
                    "{} research citation missing",
                    s.id
                );
            }
        }
    }
}
#[test]
fn payload_rules() {
    let mut s = measure::load_manifest(fixtures())
        .unwrap()
        .scenarios
        .remove(0);
    s.required_evidence = vec!["lost".into(), "kept".into(), "not in a".into()];
    s.conditions = vec!["condition".into()];
    s.major_findings = vec!["finding".into()];
    let p = measure::payload_check(&s, "lost kept condition finding", "kept", s.budget + 1);
    assert_eq!(p.evidence_regressions, vec!["lost"]);
    assert_eq!(p.condition_regressions, vec!["condition"]);
    assert_eq!(p.finding_regressions, vec!["finding"]);
    assert!(p.budget_overrun);
}
#[test]
fn measured_and_missing_usage() {
    let mut m = measure::load_manifest(fixtures()).unwrap();
    let mut runs = BTreeMap::new();
    // Processed A is 100; B varies 50..105, producing savings 0.50..-0.05.
    for (idx, s) in m.scenarios.iter().enumerate() {
        for i in 0..3 {
            runs.insert(
                (s.id.clone(), "A".into(), i),
                RunUsage {
                    claude: usage(70),
                    codex: Some(usage(1000)),
                },
            );
            runs.insert(
                (s.id.clone(), "B".into(), i),
                RunUsage {
                    claude: usage(20 + 5 * idx as u64),
                    codex: None,
                },
            );
        }
    }
    let r = measure::evaluate(&m, vec![], &runs, Some(0));
    assert_eq!(r.status, "measured");
    assert!((r.median_saving.unwrap() - 0.225).abs() < 1e-12);
    assert_eq!(r.thresholds_met, Some(true));
    assert_eq!(r.classes["A"]["claude"]["input"], json!(2520));
    assert_eq!(r.classes["A"]["claude"]["cache_write"], json!(360));
    assert_eq!(r.classes["A"]["claude"]["cache_read"], json!(720));
    assert_eq!(r.classes["A"]["claude"]["output"], json!(252));
    assert_eq!(r.classes["A"]["codex"]["input"], json!(36000));
    assert_eq!(r.classes["B"]["codex"]["unknown"]["input"], json!(36));
    let mut low_saving_runs = runs.clone();
    for ((_, arm, _), run) in &mut low_saving_runs {
        if arm == "B" {
            run.claude = usage(65);
        }
    }
    let low = measure::evaluate(&m, vec![], &low_saving_runs, Some(0));
    assert_eq!(low.status, "measured");
    assert!((low.median_saving.unwrap() - 0.05).abs() < 1e-12);
    assert_eq!(low.thresholds_met, Some(false));
    for kind in ["evidence", "condition", "finding", "budget"] {
        let mut check = measure::payload_check(&m.scenarios[0], "", "", 0);
        match kind {
            "evidence" => check.evidence_regressions.push("lost evidence".into()),
            "condition" => check.condition_regressions.push("lost condition".into()),
            "finding" => check.finding_regressions.push("lost finding".into()),
            "budget" => check.budget_overrun = true,
            _ => unreachable!(),
        }
        let failed = measure::evaluate(&m, vec![check], &runs, Some(0));
        assert_eq!(failed.status, "measured", "{kind}");
        assert_eq!(failed.thresholds_met, Some(false), "{kind}");
        assert_eq!(failed.budget_overruns, usize::from(kind == "budget"));
    }
    let wrong = measure::evaluate(&m, vec![], &runs, Some(1));
    assert_eq!(wrong.status, "measured");
    assert_eq!(wrong.thresholds_met, Some(false));
    let unknown = measure::evaluate(&m, vec![], &runs, None);
    assert_eq!(unknown.status, "measured");
    assert_eq!(unknown.thresholds_met, None);
    let text = measure::report_markdown(&r);
    assert!(text.contains("bytes, not tokens"));
    assert!(text.contains("codex"));
    assert!(text.contains("unknown"));
    let id = m.scenarios[0].id.clone();
    runs.get_mut(&(id, "B".into(), 1))
        .unwrap()
        .claude
        .cache_read = None;
    let mut regression = measure::payload_check(&m.scenarios[0], "", "", 0);
    regression.evidence_regressions.push("lost evidence".into());
    let partial = measure::evaluate(&m, vec![regression], &runs, Some(0));
    assert_eq!(partial.status, "incomplete");
    assert!(partial.scenarios[0]["saving"].is_null());
    assert!(partial.thresholds_met.is_none());
    let partial_text = measure::report_markdown(&partial);
    assert!(partial_text.contains("Status: incomplete"));
    assert!(partial_text.contains("evidence [\"lost evidence\"]"));
    assert_eq!(
        partial.classes["B"]["claude"]["unknown"]["cache_read"],
        json!(1)
    );
    // None-as-zero leaves the first scenario measured; its median remains 50.
    let wrong_median = (0.20 + 0.25) / 2.0;
    assert!((partial.median_saving.unwrap() - wrong_median).abs() > 1e-12);
    let none = measure::evaluate(&m, vec![], &BTreeMap::new(), None);
    assert_eq!(none.status, "not_measured");
    assert!(none.reason.is_some());
    let none_text = measure::report_markdown(&none);
    assert!(none_text.contains("Status: not_measured"));
    assert!(none_text.contains(none.reason.as_deref().unwrap()));
    for i in 0..3 {
        let mut u = usage(10);
        match i {
            0 => u.input = None,
            1 => u.cache_write = None,
            _ => u.cache_read = None,
        };
        assert_eq!(measure::processed_input(&u), None);
    }
    // A medians 100, 200, 300; B medians 50, 180, 120.
    // Savings are .5, .1, .6, so their median is .5 (not 1-120/200).
    m.scenarios.truncate(3);
    let mut small_runs = BTreeMap::new();
    for (s, (a, b)) in m.scenarios.iter().zip([(100, 50), (200, 180), (300, 120)]) {
        for i in 0..3 {
            for (arm, value) in [("A", a), ("B", b)] {
                small_runs.insert(
                    (s.id.clone(), arm.into(), i),
                    RunUsage {
                        claude: usage(value - 30),
                        codex: None,
                    },
                );
            }
        }
    }
    let small = measure::evaluate(&m, vec![], &small_runs, Some(0));
    assert!((small.median_saving.unwrap() - 0.5).abs() < 1e-12);
    assert!((small.median_saving.unwrap() - (1.0 - 120.0 / 200.0)).abs() > 1e-12);
}
