//! Run from the repository root after building the release binary:
//! `cargo build --locked --offline --release --features app --bin chronos`
//! `cargo test --locked --offline --manifest-path tests/desktop-smoke/Cargo.toml --test perf_run -- pilot::running_app_emits_interactive_and_event_records --ignored --exact`
//! `cargo test --locked --offline --manifest-path tests/desktop-smoke/Cargo.toml --test perf_run -- measure_before --ignored --exact`
//! `cargo test --locked --offline --manifest-path tests/desktop-smoke/Cargo.toml --test perf_run -- measure_final --ignored --exact`
#![cfg(windows)]
#![allow(dead_code)]
mod perf_support;
mod release_support;
#[path = "perf_support/scenarios.rs"]
mod scenarios;
#[path = "perf_support/win.rs"]
mod win;

#[test]
#[ignore]
fn measure_before() {
    scenarios::measure("before");
}
#[test]
#[ignore]
fn measure_final() {
    scenarios::measure("final");
}

mod pilot {
    #[test]
    #[ignore]
    fn running_app_emits_interactive_and_event_records() {
        super::scenarios::pilot();
    }
}

mod harness {
    use super::{perf_support::results, scenarios, win};
    use serde_json::json;
    use std::time::{Duration, Instant};
    #[test]
    fn parses_perf_records_and_pairs_by_sequence() {
        let text = concat!(
            "{\"t\":\"interactive\",\"since_start_us\":42}\n",
            "{\"t\":\"frame\",\"start_us\":2,\"cost_us\":3,\"interval_us\":4}\n",
            "{\"t\":\"event\",\"seq\":7,\"at_us\":8}\n",
            "{\"t\":\"pixel\",\"seq\":7,\"latency_us\":9}\n"
        );
        let log = scenarios::parse(text, true).unwrap();
        assert_eq!(log.interactive, Some(42));
        assert_eq!(log.frames.len(), 1);
        assert_eq!(log.events.len(), 1);
        assert_eq!(log.pixels.len(), 1);
        assert_eq!(scenarios::pairs(&log).unwrap()[0]["latency_us"], 9);
    }
    #[test]
    fn missing_interactive_is_an_error() {
        assert!(scenarios::parse("{\"t\":\"first_frame\"}\n", true).is_err());
    }
    #[test]
    fn unpaired_event_is_an_error() {
        let log = scenarios::parse("{\"t\":\"event\",\"seq\":1,\"at_us\":2}\n", false).unwrap();
        assert!(scenarios::pairs(&log).is_err());
    }
    #[test]
    fn busy_process_cpu_percent_is_sane() {
        let h = unsafe { windows_sys::Win32::System::Threading::GetCurrentProcess() };
        let start = win::cpu_ticks(h);
        let wall = Instant::now();
        let mut n = 0u64;
        while wall.elapsed() < Duration::from_secs(1) {
            n = std::hint::black_box(n.wrapping_add(1));
        }
        std::hint::black_box(n);
        let pct = win::cpu_percent(start, win::cpu_ticks(h), wall.elapsed());
        assert!(pct > 20.0 && pct < 400.0, "busy spin cpu: {pct}");
    }
    #[test]
    fn measurement_uses_nearest_rank_percentiles() {
        let m = results::measurement(
            "frame",
            "run",
            "known",
            "raw",
            &[4.0, 1.0, 3.0, 2.0],
            None,
            &json!({"frame_p95_ms":4,"frame_p99_ms":4,"frame_max_ms":4}),
        );
        assert_eq!(m["p95"], 4.0);
        assert_eq!(m["p99"], 4.0);
        assert_eq!(m["n"], 4);
        assert_eq!(m["result"], "pass");
    }
    #[test]
    fn live_fixture_is_discovered_at_the_event_store() {
        let home = tempfile::tempdir().unwrap();
        let store = scenarios::seed_run(home.path());
        let snapshot = axiom::app::snapshot(home.path());
        assert_eq!(snapshot.wishes.len(), 1);
        assert_eq!(snapshot.wishes[0].store.as_deref(), Some(store.as_path()));
        assert_eq!(snapshot.wishes[0].runs.len(), 1);
        let event =
            axiom::orch::events::append_event(&store, "heartbeat", None, None, json!({})).unwrap();
        assert_eq!(event.seq, 1);
    }
}
