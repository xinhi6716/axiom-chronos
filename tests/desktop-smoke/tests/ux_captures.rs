mod ux_support;

use axiom::desktop::{
    i18n::Lang,
    model::{AppState, UnitState},
    ux::{
        activity::{OpenActivity, Projection},
        events::{Notice, NoticeKind},
        CouncilView, Seams,
    },
};
use gpui_kit::{test::TestQueryExt as _, AppContext as _, ElementId, TestAppContext};
use std::{collections::BTreeSet, path::PathBuf};
use ux_support::{
    capture, checklist, newest_log_is_raw_json, open, read_capture, scenarios, shows,
    write_capture, CaptureLine,
};

const NOW: i64 = 1_000_000;
fn capture_dir(which: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("captures")
        .join(which)
}

fn seed(id: &str, state: &mut AppState) {
    state.ux.clock_ms = Some(NOW);
    state.ux.seams = if matches!(id, "UX11" | "UX12") {
        Seams::demo()
    } else {
        Seams::default()
    };
    state.ux.activity.clear();
    state.ux.notices.clear();
    state.ux.council.clear();
    let run = &state.runs[state.current.unwrap()];
    let key = run.session.clone();
    let running_units = run
        .units
        .iter()
        .filter(|u| u.state == UnitState::Running)
        .map(|u| u.id.clone())
        .collect();
    let mut projection = Projection {
        last_seen_ms: Some(NOW - 1_000),
        last_activity_ms: Some(NOW - 12_000),
        running_units,
        ..Default::default()
    };
    if id == "UX02" {
        projection.open.push(OpenActivity {
            id: "review-1".into(),
            module: "core".into(),
            actor: "core".into(),
            phase: "review".into(),
            since_ms: NOW - 12_000,
        });
    }
    if matches!(id, "UX01" | "UX02" | "UX03" | "UX10") {
        state.ux.activity.insert(key.clone(), projection);
    }
    if id == "UX03" {
        state.ux.council.insert(
            key.clone(),
            CouncilView {
                active: true,
                round: 1,
                ..Default::default()
            },
        );
    }
    let notice = match id {
        "UX04" => Some(NoticeKind::MaxTurns {
            turn_limit: Some(40),
        }),
        "UX05" => Some(NoticeKind::DegradedCouncil {
            member: "Fable".into(),
            phase: String::new(),
            round: 0,
            status: String::new(),
            reason: String::new(),
        }),
        "UX06" => Some(NoticeKind::ScopeFailure {
            unit: String::new(),
            allowed: vec![],
            outside: vec!["src/outside.rs".into()],
            outside_count: 1,
            ignored: vec![],
        }),
        _ => None,
    };
    if let Some(kind) = notice {
        state.ux.notices.insert(
            key,
            vec![Notice {
                key: id.into(),
                kind,
                at_ms: None,
            }],
        );
    }
    // The event panel must not be pushed out of view by unrelated demo needs.
    if id == "UX07" {
        state.runs[state.current.unwrap()].needs.clear();
    }
    // Give each demo session the same stable wish identity that the feed supplies.
    state.ux.wish_ids.clear();
    for run in &state.runs {
        state
            .ux
            .wish_ids
            .insert(run.session.clone(), run.session.clone());
    }
    for usage in &mut state.ux.usage_sessions {
        usage.wish = usage.session.clone();
    }
}

fn raw_event(lines: &[CaptureLine]) -> bool {
    lines
        .iter()
        .any(|line| line.path.iter().any(|id| id == "event") && line.text.starts_with("{\""))
}

#[gpui_kit::test]
fn record_and_check_after_captures(cx: &mut TestAppContext) {
    let after_dir = capture_dir("after");
    std::fs::create_dir_all(&after_dir).expect("create after directory");
    for entry in std::fs::read_dir(&after_dir).expect("read after directory") {
        let entry = entry.expect("after directory entry");
        if entry.file_type().expect("entry type").is_file() {
            std::fs::remove_file(entry.path()).expect("remove stray capture");
        }
    }
    let rows = checklist();
    let scenarios = scenarios();
    assert_eq!(rows.len(), 12);
    assert_eq!(scenarios.len(), rows.len());
    let mut failures = Vec::new();
    let mut expected = BTreeSet::new();
    for (scenario, row) in scenarios.into_iter().zip(&rows) {
        assert_eq!(scenario.id, row.id);
        for (lang, code, marker) in [
            (Lang::En, "en", &row.after_en),
            (Lang::ZhTw, "zh-TW", &row.after_zh_tw),
        ] {
            expected.insert(format!("{}.{code}.txt", row.id));
            let mut initial = (scenario.build)();
            if row.id == "UX07" && !newest_log_is_raw_json(&initial) {
                failures.push(format!("{} {code}: scenario has no raw JSON", row.id));
            }
            seed(scenario.id, &mut initial);
            let mut app = cx.new_app();
            let (window, state) = open(&mut app, lang, initial, scenario.screen);
            if let Some(action) = scenario.action {
                action(&mut app, window, &state);
            }
            let mut lines = capture(&mut app, window);
            app.update_window(window, |_, window, _| {
                for element in window.elements() {
                    if let Some(label) = element.label().filter(|label| !label.is_empty()) {
                        let line = CaptureLine {
                            path: element
                                .path()
                                .iter()
                                .filter_map(|id| match id {
                                    ElementId::Name(n) | ElementId::NamedInteger(n, _) => {
                                        Some(n.to_string())
                                    }
                                    _ => None,
                                })
                                .collect(),
                            text: label.replace(['\t', '\r', '\n'], " "),
                        };
                        if !lines.contains(&line) {
                            lines.push(line);
                        }
                    }
                }
            })
            .expect("capture element labels");
            write_capture(&after_dir, scenario.id, lang, &lines);
            if lines.is_empty() {
                failures.push(format!("{} {code}: empty after capture", row.id));
            }
            let after_marker = if row.id == "UX07" {
                ""
            } else {
                marker.as_str()
            };
            if !shows(&lines, &row.element, after_marker) {
                failures.push(format!(
                    "{} {code}: missing {} with marker {after_marker:?}",
                    row.id, row.element
                ));
            }
            if row.id == "UX07" && raw_event(&lines) {
                failures.push(format!("{} {code}: raw JSON visible after", row.id));
            }
            for line in &lines {
                let text = line.text.to_lowercase();
                if ["lorem", "todo", "tbd", "placeholder"]
                    .iter()
                    .any(|word| text.contains(word))
                {
                    failures.push(format!(
                        "{} {code}: forbidden visible text {:?}",
                        row.id, line.text
                    ));
                }
            }
            match read_capture(&capture_dir("before"), scenario.id, lang) {
                None => failures.push(format!("{} {code}: missing before capture", row.id)),
                Some(before) => {
                    if before.is_empty() {
                        failures.push(format!("{} {code}: empty before capture", row.id));
                    }
                    if shows(&before, &row.element, after_marker) {
                        failures.push(format!(
                            "{} {code}: after element and marker already visible before",
                            row.id
                        ));
                    }
                }
            }
        }
    }
    let actual: BTreeSet<_> = std::fs::read_dir(&after_dir)
        .expect("read after directory")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .into_string()
                .expect("UTF-8 filename")
        })
        .collect();
    assert_eq!(actual, expected, "after directory contents");
    assert!(
        failures.is_empty(),
        "capture checks failed:\n{}",
        failures.join("\n")
    );
}
