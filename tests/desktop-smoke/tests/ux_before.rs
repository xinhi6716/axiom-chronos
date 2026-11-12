mod ux_support;
use axiom::desktop::i18n::Lang;
use gpui_kit::TestAppContext;
use std::path::PathBuf;
use ux_support::{CaptureLine, capture, checklist, newest_log_is_raw_json, open, read_capture, scenarios, shows, write_capture};

fn before_dir() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("captures/before") }

#[gpui_kit::test]
#[ignore]
fn a0_record_before(cx: &mut TestAppContext) {
    for scenario in scenarios() {
        for lang in [Lang::En, Lang::ZhTw] {
            let mut app = cx.new_app();
            let (window, state) = open(&mut app, lang, (scenario.build)(), scenario.screen);
            if let Some(action) = scenario.action { action(&mut app, window, &state); }
            let lines = capture(&mut app, window);
            assert!(!lines.is_empty(), "{} {lang:?} captured no text", scenario.id);
            write_capture(&before_dir(), scenario.id, lang, &lines);
        }
    }
}

#[test]
fn checklist_is_complete_and_ordered() {
    let rows = checklist();
    assert_eq!(rows.len(), 12);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row.id, format!("UX{:02}", index + 1));
        assert!([&row.title, &row.scenario, &row.element, &row.after_en, &row.after_zh_tw, &row.before_lacks].iter().all(|value| !value.trim().is_empty()));
    }
    assert_eq!(scenarios().iter().map(|s| s.id).collect::<Vec<_>>(), rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>());
}

#[test]
fn before_files_show_the_gaps() {
    for row in checklist() {
        for (lang, marker) in [(Lang::En, &row.after_en), (Lang::ZhTw, &row.after_zh_tw)] {
            let lines = read_capture(&before_dir(), &row.id, lang).unwrap_or_else(|| panic!("missing {} {lang:?}", row.id));
            assert!(!lines.is_empty(), "empty {} {lang:?}", row.id);
            if row.id != "UX07" {
                assert!(!shows(&lines, &row.element, marker), "{} {lang:?}: after marker already visible", row.id);
            }
        }
    }
}

#[test]
fn raw_events_are_visible_before() {
    let scenario = scenarios().into_iter().find(|scenario| scenario.id == "UX07").expect("UX07 scenario");
    let mut state = (scenario.build)();
    assert!(newest_log_is_raw_json(&state), "UX07: newest log line is not raw JSON");
    state.runs[state.current.unwrap()].log.last_mut().unwrap().text = "not JSON".into();
    assert!(!newest_log_is_raw_json(&state), "non-JSON log line must not count as raw JSON");

    let row = checklist().into_iter().find(|row| row.id == "UX07").expect("UX07 checklist row");
    for (lang, marker) in [(Lang::En, &row.after_en), (Lang::ZhTw, &row.after_zh_tw)] {
        let lines = read_capture(&before_dir(), "UX07", lang).expect("UX07 capture");
        assert!(!shows(&lines, &row.element, marker), "UX07 {lang:?}: after marker already visible");
        assert!(!shows(&lines, &row.element, ""), "UX07 {lang:?}: event-summary element exists");
    }
}

#[test]
fn shows_requires_both_element_and_marker() {
    let lines = vec![CaptureLine { path: vec!["root".into(), "target".into()], text: "the marker is here".into() }];
    assert!(shows(&lines, "target", "marker"));
    assert!(!shows(&lines, "other", "marker"));
    assert!(!shows(&lines, "target", "missing"));
}
