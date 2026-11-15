mod perf_support;
mod release_support;

#[cfg(test)]
mod rules {
    use super::{perf_support::results, release_support::Matrix};
    use serde_json::{json, Value};
    use tempfile::TempDir;

    fn fixture() -> (TempDir, Matrix, Value) {
        let matrix = Matrix::from_value(json!({
            "budgets":{"frame_p95_ms":8.3,"frame_p99_ms":16.7,"frame_max_ms":33,"event_to_pixel_ms":250,
                "idle_cpu_pct_one_core":0.5,"cold_start_p95_s":2,"idle_private_mib":512,"exe_mib":150,"large_extra_peak_mib":512},
            "capture":{},"layout":{},"gates":{},"global_gates":["idle_cpu","cold_start","memory","exe_size"],
            "views":[{"id":"test","group":"test","opening":"test","panels":[],"actions":[],"states":[],"animated":true,"live":true,"gates":["frame","event_to_pixel"]}]
        })).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut measurements = Vec::new();
        for (gate, view, v) in [
            ("frame", "test", 1.0),
            ("event_to_pixel", "test", 1.0),
            ("idle_cpu", "*", 0.1),
            ("cold_start", "*", 1.0),
            ("memory", "*", 1.0),
            ("exe_size", "*", 1.0),
            ("large_data", "*", 1.0),
        ] {
            let raw = results::write_raw(
                dir.path(),
                "final",
                &format!("{gate}.jsonl"),
                &[json!(v), json!(v)],
            );
            measurements.push(results::measurement(
                gate,
                view,
                "synthetic",
                &raw,
                &[v, v],
                Some(v),
                &matrix.budgets,
            ));
        }
        let doc = json!({"source_id":"current","commit":"commit","phase":"final","machine":{"gpu":"test","refresh_hz":60,"dpi":100},"measurements":measurements,"workload":{"rows":300,"seed":1,"sha256":"test"}});
        (dir, matrix, doc)
    }

    fn check(dir: &TempDir, matrix: &Matrix, doc: &Value) -> Vec<String> {
        results::write_results(dir.path(), doc);
        results::validate(dir.path(), "final", matrix, "current")
    }

    #[test]
    fn clean_results_accepted() {
        let (dir, matrix, doc) = fixture();
        assert!(check(&dir, &matrix, &doc).is_empty());
    }
    #[test]
    fn missing_measurement_fails() {
        let (dir, matrix, mut doc) = fixture();
        doc["measurements"].as_array_mut().unwrap().remove(0);
        assert!(!check(&dir, &matrix, &doc).is_empty());
    }
    #[test]
    fn over_budget_value_fails() {
        let (dir, matrix, mut doc) = fixture();
        doc["measurements"][1]["value"] = json!(251);
        assert!(!check(&dir, &matrix, &doc).is_empty());
    }
    #[test]
    fn dropped_sample_fails() {
        let (dir, matrix, mut doc) = fixture();
        doc["measurements"][0]["n"] = json!(3);
        assert!(!check(&dir, &matrix, &doc).is_empty());
    }
    #[test]
    fn changed_budget_fails() {
        let (dir, matrix, mut doc) = fixture();
        doc["measurements"][0]["budget"] = json!(1000);
        assert!(!check(&dir, &matrix, &doc).is_empty());
    }
    #[test]
    fn old_source_id_fails() {
        let (dir, matrix, mut doc) = fixture();
        doc["source_id"] = json!("old");
        assert!(!check(&dir, &matrix, &doc).is_empty());
    }
}

#[cfg(test)]
mod evidence {
    use super::{perf_support::results, release_support};

    #[test]
    #[ignore = "3.0.0-beta.1: the final performance run (perf-final) has not run; the official 3.0.0 re-enables this test"]
    fn final_perf_passes_for_every_gate() {
        let repo = release_support::repo();
        let source_id = release_support::source_id(&repo).expect("current source_id");
        let matrix = release_support::load_matrix();
        let problems = results::validate(
            &repo.join("tests/desktop-smoke/perf"),
            "final",
            &matrix,
            &source_id,
        );
        assert!(
            problems.is_empty(),
            "performance evidence problems: {problems:#?}"
        );
    }
}
