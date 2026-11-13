mod release_support;

use release_support::{fresh, GateResult, Matrix};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct Evidence {
    pub repo: PathBuf,
    pub source_id: String,
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

pub fn evaluate(matrix: &Matrix, view_id: &str, gate: &str, evidence: &Evidence) -> GateResult {
    let smoke = evidence.repo.join("tests/desktop-smoke");
    match gate {
        "png" | "diff" => {
            let Some(manifest) = read_json(&smoke.join("captures/png/manifest.json")) else {
                return GateResult::Unknown;
            };
            let Some(entries) = manifest.get("current").and_then(Value::as_array) else {
                return GateResult::Unknown;
            };
            let mut found = false;
            let mut missing_source = false;
            for entry in entries
                .iter()
                .filter(|e| e.get("view").and_then(Value::as_str) == Some(view_id))
            {
                found = true;
                let Some(id) = entry.get("source_id").and_then(Value::as_str) else {
                    missing_source = true;
                    continue;
                };
                if !fresh(id, &evidence.source_id) {
                    return GateResult::Fail(format!(
                        "stale png entry for {view_id}: source_id {id}"
                    ));
                }
            }
            if !found
                || missing_source
                || (gate == "diff"
                    && manifest
                        .get("thresholds")
                        .and_then(|v| v.get(view_id))
                        .and_then(Value::as_f64)
                        .is_none())
            {
                GateResult::Unknown
            } else {
                GateResult::Pass
            }
        }
        "layout" | "fault" | "tokens" | "deps" => {
            let Some(view) = matrix.views.iter().find(|v| v.id == view_id) else {
                return GateResult::Unknown;
            };
            let Some(method) = matrix
                .gates
                .get(gate)
                .and_then(|g| g.get("method"))
                .and_then(Value::as_str)
            else {
                return GateResult::Unknown;
            };
            let (expected, path) = match gate {
                "layout" => (
                    "layout_matrix <group>::",
                    smoke.join("tests/layout_matrix.rs"),
                ),
                "fault" => (
                    "fault_matrix <group>::",
                    smoke.join("tests/fault_matrix.rs"),
                ),
                "tokens" => (
                    "desktop_architecture tokens::<group>",
                    evidence.repo.join("tests/desktop_architecture.rs"),
                ),
                _ => (
                    "desktop_architecture deps::<group>",
                    evidence.repo.join("tests/desktop_architecture.rs"),
                ),
            };
            if method != expected {
                return GateResult::Unknown;
            }
            let Ok(contents) = fs::read_to_string(path) else {
                return GateResult::Unknown;
            };
            let declaration = format!("mod {}", view.group);
            if contents.lines().any(|line| {
                let line = line.trim_start();
                let line = line.strip_prefix("pub ").unwrap_or(line);
                line.strip_prefix(&declaration).is_some_and(|tail| {
                    tail.starts_with(|c: char| c.is_whitespace() || c == '{' || c == ';')
                })
            }) {
                GateResult::Pass
            } else {
                GateResult::Unknown
            }
        }
        "frame" | "event_to_pixel" | "idle_cpu" | "cold_start" | "memory" | "exe_size" => {
            let Some(perf) = read_json(&smoke.join("perf/results.json")) else {
                return GateResult::Unknown;
            };
            if perf.get("phase").and_then(Value::as_str) != Some("final") {
                return GateResult::Unknown;
            }
            let Some(id) = perf.get("source_id").and_then(Value::as_str) else {
                return GateResult::Unknown;
            };
            if !fresh(id, &evidence.source_id) {
                return GateResult::Fail(format!("stale perf source_id {id}"));
            }
            let Some(measurements) = perf.get("measurements").and_then(Value::as_array) else {
                return GateResult::Unknown;
            };
            let metrics: &[(&str, &str)] = match gate {
                "frame" => &[
                    ("p95", "frame_p95_ms"),
                    ("p99", "frame_p99_ms"),
                    ("max", "frame_max_ms"),
                ],
                "event_to_pixel" => &[("value", "event_to_pixel_ms")],
                "idle_cpu" => &[("value", "idle_cpu_pct_one_core")],
                "cold_start" => &[("p95", "cold_start_p95_s")],
                "memory" => &[("value", "idle_private_mib")],
                _ => &[("value", "exe_mib")],
            };
            let mut found = false;
            let mut unknown = false;
            for measurement in measurements.iter().filter(|m| {
                m.get("gate").and_then(Value::as_str) == Some(gate)
                    && m.get("view").and_then(Value::as_str) == Some(view_id)
            }) {
                found = true;
                match measurement.get("result").and_then(Value::as_str) {
                    Some("fail") => {
                        return GateResult::Fail(format!("{gate} measurement reports fail"))
                    }
                    Some("pass") => {}
                    _ => unknown = true,
                }
                for (metric, budget) in metrics {
                    match (
                        measurement.get(*metric).and_then(Value::as_f64),
                        matrix.budgets.get(*budget).and_then(Value::as_f64),
                    ) {
                        (Some(actual), Some(limit)) if actual > limit => {
                            return GateResult::Fail(format!(
                                "{gate} {metric} {actual} exceeds {budget} {limit}"
                            ))
                        }
                        (Some(_), Some(_)) => {}
                        _ => unknown = true,
                    }
                }
            }
            if found && !unknown {
                GateResult::Pass
            } else {
                GateResult::Unknown
            }
        }
        _ => GateResult::Unknown,
    }
}

mod evidence {
    use super::release_support::{counts_as_pass, load_matrix, repo, source_id};
    use super::{evaluate, Evidence};

    #[test]
    fn all_views_all_gates_pass() {
        let repo = repo();
        let source_id = source_id(&repo).expect("cannot compute current source_id");
        let evidence = Evidence { repo, source_id };
        let matrix = load_matrix();
        let mut problems = Vec::new();
        for view in &matrix.views {
            let gates = view.gates.as_array().expect("view gates must be an array");
            for gate in gates {
                let gate = gate.as_str().expect("gate must be a string");
                let result = evaluate(&matrix, &view.id, gate, &evidence);
                if !counts_as_pass(&result) {
                    problems.push(format!("{} / {gate}: {result:?}", view.id));
                }
            }
        }
        for gate in matrix
            .global_gates
            .as_array()
            .expect("global gates must be an array")
        {
            let gate = gate.as_str().expect("global gate must be a string");
            let result = evaluate(&matrix, "*", gate, &evidence);
            if !counts_as_pass(&result) {
                problems.push(format!("* / {gate}: {result:?}"));
            }
        }
        assert!(
            problems.is_empty(),
            "{} evidence problems (first 40):\n{}",
            problems.len(),
            problems
                .iter()
                .take(40)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

#[cfg(test)]
mod rules {
    use super::*;
    use release_support::counts_as_pass;
    use serde_json::json;
    use tempfile::TempDir;

    fn matrix() -> Matrix {
        Matrix::from_value(json!({
            "budgets":{"frame_p95_ms":8.3,"frame_p99_ms":16.7,"frame_max_ms":33,"event_to_pixel_ms":250,"idle_cpu_pct_one_core":0.5,"cold_start_p95_s":2,"idle_private_mib":512,"exe_mib":150},
            "capture":{}, "layout":{},
            "views":[{"id":"view","group":"example","opening":"page:view","panels":[],"actions":[],"states":[],"animated":true,"live":true,"gates":["png","diff","layout","fault","tokens","deps","frame","event_to_pixel"]}],
            "gates":{"layout":{"method":"layout_matrix <group>::"},"fault":{"method":"fault_matrix <group>::"},"tokens":{"method":"desktop_architecture tokens::<group>"},"deps":{"method":"desktop_architecture deps::<group>"}},
            "global_gates":["idle_cpu","cold_start","memory","exe_size"]
        })).unwrap()
    }
    fn write(root: &Path, name: &str, contents: &str) {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    fn setup() -> (TempDir, Evidence) {
        let dir = tempfile::tempdir().unwrap();
        let evidence = Evidence {
            repo: dir.path().to_owned(),
            source_id: "current".into(),
        };
        write(
            dir.path(),
            "tests/desktop-smoke/captures/png/manifest.json",
            &json!({"thresholds":{"view":0.01},"current":[{"view":"view","source_id":"current"}]})
                .to_string(),
        );
        write(
            dir.path(),
            "tests/desktop-smoke/perf/results.json",
            &json!({"phase":"final","source_id":"current","measurements":[
                {"gate":"frame","view":"view","result":"pass","p95":8,"p99":16,"max":32},
                {"gate":"event_to_pixel","view":"view","result":"pass","value":240},
                {"gate":"idle_cpu","view":"*","result":"pass","value":0.4},
                {"gate":"cold_start","view":"*","result":"pass","p95":1.9},
                {"gate":"memory","view":"*","result":"pass","value":500},
                {"gate":"exe_size","view":"*","result":"pass","value":140}
            ]})
            .to_string(),
        );
        for path in [
            "tests/desktop-smoke/tests/layout_matrix.rs",
            "tests/desktop-smoke/tests/fault_matrix.rs",
            "tests/desktop_architecture.rs",
        ] {
            write(dir.path(), path, "mod example {}\n");
        }
        (dir, evidence)
    }
    #[test]
    fn unknown_evidence_does_not_pass() {
        assert!(!counts_as_pass(&GateResult::Unknown));
        assert!(!counts_as_pass(&GateResult::Fail("failure".into())));
    }
    #[test]
    fn missing_file_is_unknown() {
        let (dir, evidence) = setup();
        fs::remove_file(
            dir.path()
                .join("tests/desktop-smoke/captures/png/manifest.json"),
        )
        .unwrap();
        assert_eq!(
            evaluate(&matrix(), "view", "png", &evidence),
            GateResult::Unknown
        );
        assert_eq!(
            evaluate(&matrix(), "view", "diff", &evidence),
            GateResult::Unknown
        );
        fs::remove_file(dir.path().join("tests/desktop-smoke/perf/results.json")).unwrap();
        assert_eq!(
            evaluate(&matrix(), "view", "frame", &evidence),
            GateResult::Unknown
        );
    }
    #[test]
    fn stale_source_is_failure() {
        let (dir, evidence) = setup();
        write(dir.path(), "tests/desktop-smoke/captures/png/manifest.json", &json!({"thresholds":{"view":0.01},"current":[{"view":"view"},{"view":"view","source_id":"old"}]}).to_string());
        assert!(matches!(
            evaluate(&matrix(), "view", "png", &evidence),
            GateResult::Fail(_)
        ));
        assert!(matches!(
            evaluate(&matrix(), "view", "diff", &evidence),
            GateResult::Fail(_)
        ));
        write(
            dir.path(),
            "tests/desktop-smoke/perf/results.json",
            &json!({"phase":"final","source_id":"old","measurements":[]}).to_string(),
        );
        assert!(matches!(
            evaluate(&matrix(), "view", "frame", &evidence),
            GateResult::Fail(_)
        ));
    }
    #[test]
    fn over_budget_p95_fails_despite_claimed_pass() {
        let (dir, evidence) = setup();
        write(dir.path(), "tests/desktop-smoke/perf/results.json", &json!({"phase":"final","source_id":"current","measurements":[{"gate":"frame","view":"view","result":"pass","p95":9,"p99":16,"max":32,"budget":999}]}).to_string());
        assert!(matches!(
            evaluate(&matrix(), "view", "frame", &evidence),
            GateResult::Fail(_)
        ));
    }
    #[test]
    fn missing_view_or_threshold_is_unknown() {
        let (dir, evidence) = setup();
        assert_eq!(
            evaluate(&matrix(), "other", "png", &evidence),
            GateResult::Unknown
        );
        write(
            dir.path(),
            "tests/desktop-smoke/captures/png/manifest.json",
            &json!({"thresholds":{},"current":[{"view":"view","source_id":"current"}]}).to_string(),
        );
        assert_eq!(
            evaluate(&matrix(), "view", "diff", &evidence),
            GateResult::Unknown
        );
    }
    #[test]
    fn missing_suite_is_unknown() {
        let (dir, evidence) = setup();
        fs::remove_file(
            dir.path()
                .join("tests/desktop-smoke/tests/layout_matrix.rs"),
        )
        .unwrap();
        assert_eq!(
            evaluate(&matrix(), "view", "layout", &evidence),
            GateResult::Unknown
        );
    }
    #[test]
    fn clean_evidence_passes_every_gate() {
        let (_dir, evidence) = setup();
        let matrix = matrix();
        for gate in matrix.views[0].gates.as_array().unwrap() {
            let gate = gate.as_str().unwrap();
            assert_eq!(
                evaluate(&matrix, "view", gate, &evidence),
                GateResult::Pass,
                "{gate}"
            );
        }
        for gate in matrix.global_gates.as_array().unwrap() {
            let gate = gate.as_str().unwrap();
            assert_eq!(
                evaluate(&matrix, "*", gate, &evidence),
                GateResult::Pass,
                "{gate}"
            );
        }
    }
}
