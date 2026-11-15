mod release_support;

mod rules {
    use super::release_support::{counts_as_pass, fresh, validate, GateResult, Matrix};
    use axiom::desktop::targets::{CaptureState, Opening, Target};
    use serde_json::json;

    fn registry() -> Vec<Target> {
        vec![Target {
            id: "chat",
            group: "synthetic",
            opening: Opening::Page("chat"),
            panels: &["panel"],
            actions: &["action"],
            states: &[CaptureState::Live],
        }]
    }
    fn clean() -> Matrix {
        Matrix::from_value(json!({
            "version":1,
            "budgets":{"frame_p95_ms":8.3,"frame_p99_ms":16.7,"frame_max_ms":33,"event_to_pixel_ms":250,"idle_cpu_pct_one_core":0.5,"idle_window_s":60,"cold_start_p95_s":2.0,"idle_private_mib":512,"large_extra_peak_mib":512,"exe_mib":150,"large_rows":60000},
            "capture":{"langs":["zh-TW","en"],"themes":["dark","light"],"sizes":["1100x700","1920x1200"],"states":["live","empty","error","action"]},
            "layout":{"sizes":["800x600","1100x700","1920x1200","3440x1440"],"dpi":[100,150,200]},
            "views":[{"id":"chat","group":"synthetic","opening":"page:chat","panels":["panel"],"actions":["action"],"states":["live"],"animated":true,"live":true,"gates":["png","diff","layout","fault","tokens","deps","frame","event_to_pixel"]}],
            "gates":{"png":{"method":"captures/png/manifest.json"},"diff":{"method":"captures/png/manifest.json"},"layout":{"method":"layout_matrix <group>::"},"fault":{"method":"fault_matrix <group>::"},"tokens":{"method":"desktop_architecture tokens::<group>"},"deps":{"method":"desktop_architecture deps::<group>"},"frame":{"method":"perf/results.json"},"event_to_pixel":{"method":"perf/results.json"},"idle_cpu":{"method":"perf/results.json"},"cold_start":{"method":"perf/results.json"},"memory":{"method":"perf/results.json"},"exe_size":{"method":"perf/results.json"}},
            "global_gates":["idle_cpu","cold_start","memory","exe_size"]
        })).unwrap()
    }
    #[test]
    fn clean_matrix_is_accepted() {
        assert_eq!(validate(&clean(), &registry()), Vec::<String>::new());
    }
    #[test]
    fn missing_page_is_refused() {
        let mut matrix = clean();
        matrix.views.clear();
        assert!(validate(&matrix, &registry())
            .iter()
            .any(|s| s.contains("chat missing")));
    }
    #[test]
    fn missing_gate_is_refused() {
        let mut matrix = clean();
        matrix.views[0]
            .gates
            .as_array_mut()
            .unwrap()
            .retain(|v| v != "frame");
        assert!(validate(&matrix, &registry())
            .iter()
            .any(|s| s.contains("missing gate frame")));
    }
    #[test]
    fn unknown_and_failed_results_are_refused() {
        assert!(counts_as_pass(&GateResult::Pass));
        assert!(!counts_as_pass(&GateResult::Unknown));
        assert!(!counts_as_pass(&GateResult::Fail("failure".into())));
    }
    #[test]
    fn old_source_id_is_refused() {
        assert!(fresh("current", "current"));
        assert!(!fresh("old", "current"));
        assert!(!fresh("", ""));
    }
    #[test]
    fn changed_budget_is_refused() {
        let mut matrix = clean();
        matrix.budgets["frame_p95_ms"] = json!(8.4);
        assert!(validate(&matrix, &registry())
            .iter()
            .any(|s| s.contains("budget frame_p95_ms")));
    }
}

mod matrix {
    use super::release_support::{load_matrix, opening_name, validate};
    use axiom::desktop::{
        model::Screen,
        research::Tab,
        targets::{self, Opening},
        ux::Page,
    };
    use serde_json::json;
    use std::collections::HashSet;

    #[test]
    fn matches_registry() {
        let problems = validate(&load_matrix(), &targets::all());
        assert!(
            problems.is_empty(),
            "matrix problems:\n{}",
            problems.join("\n")
        );
    }
    #[test]
    fn all_openings_are_present() {
        let matrix = load_matrix();
        let openings: HashSet<_> = matrix.views.iter().map(|v| v.opening.as_str()).collect();
        let pages = [
            Page::Research,
            Page::Storage,
            Page::Chat,
            Page::QuickTask,
            Page::Plaza,
            Page::Session("test".into()),
            Page::Paper("test".into()),
            Page::Routine,
        ];
        let screens = [
            Screen::Run,
            Screen::Wishes,
            Screen::Memory,
            Screen::Usage,
            Screen::Doctor,
        ];
        let tabs = [
            Tab::Map,
            Tab::Sources,
            Tab::Evidence,
            Tab::Dataset,
            Tab::Exports,
        ];
        let mut expected: Vec<String> =
            pages.iter().map(|p| format!("page:{}", p.kind())).collect();
        expected.extend(
            screens
                .into_iter()
                .map(|s| opening_name(&Opening::Screen(s))),
        );
        expected.extend(
            tabs.into_iter()
                .map(|t| format!("research_tab:{}", t.key())),
        );
        for (kind, names) in [
            (
                "paper_step",
                &[
                    "form",
                    "shortlist",
                    "estimate",
                    "progress",
                    "changelog",
                    "preview",
                    "readiness",
                ][..],
            ),
            ("window", &["mini", "focus.graph", "focus.checklist"][..]),
            ("overlay", &["palette", "composer"][..]),
            (
                "dialog",
                &["close", "control", "interjection", "composer", "evolve"][..],
            ),
        ] {
            expected.extend(names.iter().map(|name| format!("{kind}:{name}")));
        }
        for opening in expected {
            assert!(
                openings.contains(opening.as_str()),
                "missing opening {opening}"
            );
        }
    }
    #[test]
    fn budgets_are_fixed() {
        assert_eq!(
            load_matrix().budgets,
            json!({"frame_p95_ms":8.3,"frame_p99_ms":16.7,"frame_max_ms":33,"event_to_pixel_ms":250,"idle_cpu_pct_one_core":0.5,"idle_window_s":60,"cold_start_p95_s":2.0,"idle_private_mib":512,"large_extra_peak_mib":512,"exe_mib":150,"large_rows":60000})
        );
    }
}
