mod research_support;
use axiom::{
    desktop::{i18n::Lang, research::ResearchSeams},
    research_workspace::{
        seams::FakeRenderer,
        session::{self, SessionPrefs},
    },
};
use gpui_kit::{component::ThemeMode, test::TestWindowExt as _, AppContext as _, TestAppContext};
use research_support::*;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn wait_for(
    cx: &mut TestAppContext,
    w: gpui_kit::AnyWindowHandle,
    id: &str,
    text: &str,
) -> Vec<(String, String)> {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        let lines = capture(cx, w);
        if shows(&lines, id, text) {
            return lines;
        }
        assert!(Instant::now() < end, "waiting for {id}: {lines:?}");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn prefs(home: &std::path::Path, research: &str, raw: bool, selection: &str) {
    let mut prefs = SessionPrefs::default();
    let view = prefs
        .views
        .entry(research.into())
        .or_default()
        .tabs
        .entry("evidence".into())
        .or_default();
    view.node = raw.then(|| "raw".into());
    view.selection = Some(selection.into());
    session::save(home, &prefs).unwrap();
}
fn wait_prefs(home: &std::path::Path, predicate: impl Fn(&SessionPrefs) -> bool) -> SessionPrefs {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        let prefs = session::load(home);
        if predicate(&prefs) {
            return prefs;
        }
        assert!(Instant::now() < end, "session preferences were not saved");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn seams(home: &std::path::Path, renderer: Arc<FakeRenderer>) -> ResearchSeams {
    ResearchSeams {
        research_home: Some(home.into()),
        renderer: Some(renderer),
        page_cache_bytes: 1024 * 1024,
        ..Default::default()
    }
}
#[gpui_kit::test]
fn evidence_views_and_rendering(cx: &mut TestAppContext) {
    for lang in [Lang::En, Lang::ZhTw] {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            for size in [(1100., 700.), (1920., 1200.)] {
                let copy = fixture_copy("rram");
                let home = temp_dir();
                let before = tree_hash(&copy);
                let renderer = Arc::new(FakeRenderer::new());
                prefs(&home, "rram_fixture", false, "E6");
                let (w, _) = open_research(
                    cx,
                    lang,
                    mode,
                    size,
                    &copy.join("project"),
                    seams(&home, renderer.clone()),
                );
                click(cx, w, "research-tab-evidence");
                let text = if lang == Lang::En {
                    "No region recorded; showing the whole page"
                } else {
                    "未記錄區域；顯示整頁"
                };
                let lines = wait_for(cx, w, "research-evidence-locator", text);
                assert!(visible(&lines, "research-evidence-layer-literature"));
                assert!(visible(&lines, "research-evidence-layer-raw"));
                assert!(visible(&lines, "research-evidence-condition-0"));
                assert!(visible(&lines, "research-evidence-contradiction-0"));
                assert!(!lines.iter().any(|(p, _)| p
                    .split('/')
                    .any(|x| x.starts_with("research-evidence-highlight"))));
                cx.update_window(w, |_, window, _| {
                    assert!(window.try_find("research-evidence-page").is_some())
                })
                .unwrap();
                assert!(!renderer.calls().is_empty());
                assert!(!visible(&lines, "research-evidence-degraded"));
                assert!(std::fs::read_dir(home.join("cache/pages"))
                    .unwrap()
                    .any(|entry| entry
                        .unwrap()
                        .path()
                        .extension()
                        .is_some_and(|ext| ext == "png")));
                assert_eq!(tree_hash(&copy), before);
            }
        }
    }
}
#[gpui_kit::test]
fn renderer_failures_are_visible(cx: &mut TestAppContext) {
    for kind in ["renderer_missing", "corrupt_pdf", "hash_mismatch", "429"] {
        let copy = fixture_copy("rram");
        let home = temp_dir();
        let before = tree_hash(&copy);
        let renderer = Arc::new(FakeRenderer::new());
        renderer.fail_next(kind);
        prefs(&home, "rram_fixture", false, "E1");
        let (w, _) = open_research(
            cx,
            Lang::En,
            ThemeMode::Dark,
            (1100., 700.),
            &copy.join("project"),
            seams(&home, renderer),
        );
        click(cx, w, "research-tab-evidence");
        wait_for(cx, w, "research-evidence-degraded", kind);
        assert_eq!(tree_hash(&copy), before);
    }
}
#[gpui_kit::test]
fn missing_artifact_is_visible(cx: &mut TestAppContext) {
    let copy = fixture_copy("rram");
    std::fs::remove_file(copy.join("project/data/evidence.csv")).unwrap();
    let before = tree_hash(&copy);
    let home = temp_dir();
    prefs(&home, "rram_fixture", false, "E1");
    let (w, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1100., 700.),
        &copy.join("project"),
        seams(&home, Arc::new(FakeRenderer::new())),
    );
    click(cx, w, "research-tab-evidence");
    wait_for(cx, w, "research-evidence-degraded", "missing or corrupt");
    assert_eq!(tree_hash(&copy), before);
}
#[gpui_kit::test]
fn raw_only_and_restored_selection(cx: &mut TestAppContext) {
    let copy = fixture_copy("rawonly");
    let home = temp_dir();
    let before = tree_hash(&copy);
    let (w, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1100., 700.),
        &copy,
        seams(&home, Arc::new(FakeRenderer::new())),
    );
    click(cx, w, "research-tab-evidence");
    wait_for(cx, w, "research-evidence-layer-raw", "Raw");
    click(cx, w, "research-evidence-layer-raw");
    let lines = wait_for(
        cx,
        w,
        "research-evidence-locator",
        "runs/device_A.csv, row 1, column value",
    );
    assert!(shows(
        &lines,
        "research-evidence-capability",
        "PDF pages are not available"
    ));
    click(cx, w, "research-evidence-item-RE2");
    wait_for(cx, w, "research-evidence-locator", "runs/device_B.jsonl");
    let saved = wait_prefs(&home, |p| {
        p.views
            .get("rawonly")
            .and_then(|v| v.tabs.get("evidence"))
            .is_some_and(|v| {
                v.node.as_deref() == Some("raw") && v.selection.as_deref() == Some("RE2")
            })
    });
    let view = &saved.views["rawonly"].tabs["evidence"];
    assert_eq!(view.node.as_deref(), Some("raw"));
    assert_eq!(view.selection.as_deref(), Some("RE2"));
    let (again, _) = open_shell(
        cx,
        Lang::En,
        ThemeMode::Light,
        (1100., 700.),
        seams(&home, Arc::new(FakeRenderer::new())),
        true,
    );
    wait_load(cx, again);
    click(cx, again, "research-tab-evidence");
    wait_for(
        cx,
        again,
        "research-evidence-locator",
        "runs/device_B.jsonl",
    );
    click(cx, again, "research-evidence-correct");
    cx.update_window(again, |_, window, _| {
        assert!(window.try_find("research-correct-after").is_some())
    })
    .unwrap();
    click(cx, again, "research-evidence-open-dataset");
    let expected = vec![axiom::research_workspace::dataset::evidence_filter("RE2")];
    let saved = wait_prefs(&home, |p| {
        p.views
            .get("rawonly")
            .and_then(|v| v.tabs.get("dataset"))
            .is_some_and(|v| v.filters == expected)
    });
    assert_eq!(saved.views["rawonly"].tabs["dataset"].filters, expected);
    assert_eq!(tree_hash(&copy), before);
}
#[gpui_kit::test]
fn unopened_research_is_a_plain_state(cx: &mut TestAppContext) {
    let (window, _) = open_shell(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1100., 700.),
        ResearchSeams::default(),
        true,
    );
    click(cx, window, "research-tab-evidence");
    assert!(shows(
        &capture(cx, window),
        "research-evidence-panel",
        "No research open or indexing"
    ));
}

#[gpui_kit::test]
fn split_drag_persists_and_restores(cx: &mut TestAppContext) {
    use axiom::desktop::layout::persist;
    use gpui_kit::px;
    let copy = fixture_copy("rram");
    let before = tree_hash(&copy);
    let home = temp_dir();
    cx.update(|cx| persist::init(&home, cx));
    let configuration = seams(&home, Arc::new(FakeRenderer::new()));
    let (window, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1920., 1200.),
        &copy.join("project"),
        configuration.clone(),
    );
    click(cx, window, "research-tab-evidence");
    wait_for(cx, window, "research-evidence-locator", "");
    let original = cx
        .update_window(window, |_, window, _| {
            window.find("research-evidence-split-frame").bounds()
        })
        .unwrap();
    cx.update_window(window, |_, window, cx| {
        let start = original.origin + gpui_kit::point(original.size.width / 2., px(10.));
        window.drag(start, start + gpui_kit::point(px(4.), px(0.)), cx);
    })
    .unwrap();
    frame(cx, window);
    let moved = cx
        .update_window(window, |_, window, _| {
            window.find("research-evidence-split-frame").bounds()
        })
        .unwrap();
    assert!(moved.origin.x > original.origin.x);
    let saved = cx
        .update(|cx| {
            persist::flush(cx);
            persist::sizes("research.evidence", cx)
        })
        .unwrap();
    assert_eq!(
        persist::load(&home).panels.get("research.evidence"),
        Some(&saved)
    );
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    let (again, _) = open_shell(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1920., 1200.),
        configuration,
        true,
    );
    wait_load(cx, again);
    click(cx, again, "research-tab-evidence");
    wait_for(cx, again, "research-evidence-locator", "");
    let restored = cx
        .update_window(again, |_, window, _| {
            window.find("research-evidence-split-frame").bounds()
        })
        .unwrap();
    assert_eq!(restored.origin.x, moved.origin.x);
    assert_eq!(tree_hash(&copy), before);
}

#[gpui_kit::test]
fn feed_failure_is_local_to_tab(cx: &mut TestAppContext) {
    use axiom::desktop::data::faults::{self, Fault, FaultPlan};
    use std::collections::HashMap;
    let copy = fixture_copy("rram");
    let before = tree_hash(&copy);
    let home = temp_dir();
    let configuration = seams(&home, Arc::new(FakeRenderer::new()));
    cx.update(|cx| {
        faults::install(
            FaultPlan(HashMap::from([(
                "research.evidence.panel".into(),
                Fault::Corrupt,
            )])),
            cx,
        )
    });
    let (window, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1920., 1200.),
        &copy.join("project"),
        configuration,
    );
    click(cx, window, "research-tab-evidence");
    wait_retry(cx, window, "retry:research.evidence.panel");
    click(cx, window, "research-tab-map");
    wait_for(cx, window, "research-map-status", "");
    cx.update(|cx| faults::install(FaultPlan::default(), cx));
    assert_eq!(tree_hash(&copy), before);
}

#[gpui_kit::test]
fn reopening_same_selection_locates_in_new_generation(cx: &mut TestAppContext) {
    let copy = fixture_copy("rram");
    let before = tree_hash(&copy);
    let home = temp_dir();
    prefs(&home, "rram_fixture", false, "E1");
    let (window, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1920., 1200.),
        &copy.join("project"),
        seams(&home, Arc::new(FakeRenderer::new())),
    );
    click(cx, window, "research-tab-evidence");
    wait_for(cx, window, "research-evidence-locator", "Page 1");
    click(cx, window, "research-tab-map");
    click(cx, window, "research-open-button");
    wait_load(cx, window);
    click(cx, window, "research-tab-evidence");
    wait_for(cx, window, "research-evidence-locator", "Page 1");
    cx.update_window(window, |_, window, _| {
        assert!(window.try_find("research-evidence-split-frame").is_some());
    })
    .unwrap();
    assert_eq!(tree_hash(&copy), before);
}
