use crate::release_support::{fresh, Matrix};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Component, Path},
};

fn metrics(gate: &str) -> Option<&'static [(&'static str, &'static str)]> {
    match gate {
        "frame" => Some(&[
            ("p95", "frame_p95_ms"),
            ("p99", "frame_p99_ms"),
            ("max", "frame_max_ms"),
        ]),
        "event_to_pixel" => Some(&[("value", "event_to_pixel_ms")]),
        "idle_cpu" => Some(&[("value", "idle_cpu_pct_one_core")]),
        "cold_start" => Some(&[("p95", "cold_start_p95_s")]),
        "memory" => Some(&[("value", "idle_private_mib")]),
        "exe_size" => Some(&[("value", "exe_mib")]),
        "large_data" => Some(&[("value", "large_extra_peak_mib")]),
        _ => None,
    }
}

pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    assert!(!sorted.is_empty() && (0.0..=100.0).contains(&p));
    let rank = (p / 100.0 * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1)]
}

fn within_budget(m: &Value, gate: &str, budgets: &Value) -> bool {
    metrics(gate).is_some_and(|items| {
        items.iter().all(|(metric, key)| {
            match (
                m.get(metric).and_then(Value::as_f64),
                budgets.get(key).and_then(Value::as_f64),
            ) {
                (Some(value), Some(budget)) => {
                    value.is_finite() && budget.is_finite() && value <= budget
                }
                _ => false,
            }
        })
    })
}

pub fn measurement(
    gate: &str,
    view: &str,
    scenario: &str,
    samples_rel: &str,
    values: &[f64],
    value: Option<f64>,
    budgets: &Value,
) -> Value {
    assert!(!values.is_empty() && values.iter().all(|v| v.is_finite()));
    let keys = metrics(gate).expect("unknown performance gate");
    let budget = budgets
        .get(keys[0].1)
        .and_then(Value::as_f64)
        .expect("missing performance budget");
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mut doc = json!({"gate":gate,"view":view,"scenario":scenario,"samples":samples_rel,"n":values.len(),
        "p95":percentile(&sorted,95.0),"p99":percentile(&sorted,99.0),"max":sorted[sorted.len()-1],
        "value":value,"budget":budget});
    doc["result"] = json!(if within_budget(&doc, gate, budgets) {
        "pass"
    } else {
        "fail"
    });
    doc
}

pub fn write_raw(perf_dir: &Path, phase: &str, file: &str, values: &[Value]) -> String {
    assert!(matches!(phase, "before" | "final"));
    assert!(matches!(
        Path::new(file).components().collect::<Vec<_>>().as_slice(),
        [Component::Normal(_)]
    ));
    let rel = format!("raw/{phase}/{file}");
    let path = perf_dir.join(&rel);
    fs::create_dir_all(path.parent().unwrap()).expect("create raw directory");
    let mut bytes = Vec::new();
    for value in values {
        serde_json::to_writer(&mut bytes, value).expect("serialize raw sample");
        bytes.push(b'\n');
    }
    fs::write(path, bytes).expect("write raw samples");
    rel
}

pub fn write_results(perf_dir: &Path, doc: &Value) {
    fs::create_dir_all(perf_dir).expect("create perf directory");
    fs::write(
        perf_dir.join("results.json"),
        serde_json::to_vec_pretty(doc).expect("serialize results"),
    )
    .expect("write results");
}

pub fn validate(
    perf_dir: &Path,
    phase: &str,
    matrix: &Matrix,
    current_source_id: &str,
) -> Vec<String> {
    let mut problems = Vec::new();
    let doc: Value = match fs::read(perf_dir.join("results.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    {
        Some(doc) => doc,
        None => return vec!["missing or invalid results.json".into()],
    };
    if !matches!(phase, "before" | "final")
        || doc.get("phase").and_then(Value::as_str) != Some(phase)
    {
        problems.push("phase differs".into());
    }
    if !fresh(
        doc.get("source_id").and_then(Value::as_str).unwrap_or(""),
        current_source_id,
    ) {
        problems.push("source_id is missing or stale".into());
    }
    let Some(measurements) = doc.get("measurements").and_then(Value::as_array) else {
        problems.push("missing measurements".into());
        return problems;
    };
    let mut required = Vec::new();
    for view in &matrix.views {
        for gate in ["frame", "event_to_pixel"] {
            if view
                .gates
                .as_array()
                .is_some_and(|gates| gates.iter().any(|g| g.as_str() == Some(gate)))
            {
                required.push((gate, view.id.as_str()));
            }
        }
    }
    for gate in ["idle_cpu", "cold_start", "memory", "exe_size"] {
        if !matrix
            .global_gates
            .as_array()
            .is_some_and(|gates| gates.iter().any(|g| g.as_str() == Some(gate)))
        {
            problems.push(format!("missing global gate {gate} in matrix"));
        }
        required.push((gate, "*"));
    }
    required.push(("large_data", "*"));
    for (gate, view) in required {
        let found: Vec<_> = measurements
            .iter()
            .filter(|m| {
                m.get("gate").and_then(Value::as_str) == Some(gate)
                    && m.get("view").and_then(Value::as_str) == Some(view)
            })
            .collect();
        if found.is_empty() {
            problems.push(format!("missing {gate}/{view}"));
        }
        for m in found {
            let label = format!("{gate}/{view}");
            if m.get("result").and_then(Value::as_str) != Some("pass")
                || !within_budget(m, gate, &matrix.budgets)
            {
                problems.push(format!("{label} does not pass its budgets"));
            }
            let budget_key = metrics(gate).unwrap()[0].1;
            if m.get("budget")
                .and_then(Value::as_f64)
                .zip(matrix.budgets.get(budget_key).and_then(Value::as_f64))
                .is_none_or(|(a, b)| a != b)
            {
                problems.push(format!("{label} budget differs"));
            }
            let raw = m.get("samples").and_then(Value::as_str).and_then(|rel| {
                let parts: Vec<_> = Path::new(rel).components().collect();
                if !matches!(parts.as_slice(), [Component::Normal(a), Component::Normal(b), Component::Normal(_)] if *a == "raw" && *b == phase) { return None; }
                fs::read_to_string(perf_dir.join(rel)).ok()
            });
            match raw {
                Some(raw)
                    if m.get("n")
                        .and_then(Value::as_u64)
                        .is_some_and(|n| n > 0 && n == raw.lines().count() as u64)
                        && raw
                            .lines()
                            .all(|line| serde_json::from_str::<Value>(line).is_ok()) => {}
                _ => problems.push(format!("{label} missing or invalid raw samples/count")),
            }
        }
    }
    problems
}
