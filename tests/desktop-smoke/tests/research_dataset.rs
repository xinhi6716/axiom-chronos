mod research_support;
use axiom::{
    desktop::{i18n::Lang, research::ResearchSeams},
    research_workspace::{
        index::{Filter, Op},
        session::{self, SessionPrefs},
    },
};
use gpui_kit::{
    component::ThemeMode, test::TestWindowExt as _, AnyWindowHandle, AppContext as _,
    TestAppContext,
};
use research_support::*;
use std::time::{Duration, Instant};
fn wait_for(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    id: &str,
    text: &str,
) -> Vec<(String, String)> {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        let lines = capture(cx, window);
        if shows(&lines, id, text) {
            return lines;
        }
        assert!(Instant::now() < end, "waiting {id}: {lines:?}");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn scroll(cx: &mut TestAppContext, window: AnyWindowHandle, amount: f32) {
    cx.update_window(window, |_, window, cx| {
        window.scroll(
            "research-page",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(amount))),
            cx,
        )
    })
    .unwrap();
    frame(cx, window);
}
fn wait_prefs(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    home: &std::path::Path,
    f: impl Fn(&SessionPrefs) -> bool,
) -> SessionPrefs {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        let prefs = session::load(home);
        if f(&prefs) {
            return prefs;
        }
        assert!(Instant::now() < end, "session did not persist: {prefs:?}");
        frame(cx, window);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn run(cx: &mut TestAppContext, lang: Lang, theme: ThemeMode, dims: (f32, f32)) {
    let root = fixture_copy("rram");
    std::fs::remove_file(root.join("project/data/raw_measurements.csv")).unwrap();
    let before = tree_hash(&root);
    let home = temp_dir();
    let seams = ResearchSeams {
        research_home: Some(home.clone()),
        ..Default::default()
    };
    let (window, _) = open_research(cx, lang, theme, dims, &root.join("project"), seams.clone());
    click(cx, window, "research-tab-dataset");
    let lines = wait_for(cx, window, "research-dataset-row-0", "voltage");
    assert!(visible(&lines, "research-dataset-layer-extracted"));
    assert!(visible(&lines, "research-dataset-validation"));
    assert!(visible(&lines, "research-dataset-missing"));
    assert!(shows(&lines, "research-dataset-row-0", "0 V"));
    assert!(shows(
        &lines,
        "research-dataset-row-1",
        if lang == Lang::En { "empty" } else { "空值" }
    ));
    click(cx, window, "research-dataset-correct-0");
    scroll(cx, window, -300.);
    wait_for(
        cx,
        window,
        "research-correct-form",
        if lang == Lang::En {
            "There is no active session for this research."
        } else {
            "此研究沒有進行中的工作階段。"
        },
    );
    scroll(cx, window, 300.);
    click(cx, window, "research-dataset-evidence-0");
    let prefs = wait_prefs(cx, window, &home, |p| {
        p.views
            .get("rram_fixture")
            .is_some_and(|v| v.tab == "evidence")
    });
    let view = &prefs.views["rram_fixture"];
    assert_eq!(view.tab, "evidence");
    assert_eq!(view.tabs["evidence"].selection.as_deref(), Some("E1"));
    click(cx, window, "research-tab-dataset");
    click(cx, window, "research-dataset-filter-variable");
    type_text(cx, window, "absent-variable");
    let prefs = wait_prefs(cx, window, &home, |p| {
        p.views
            .get("rram_fixture")
            .and_then(|v| v.tabs.get("dataset"))
            .is_some_and(|v| {
                v.filters.iter().any(|f| {
                    f.column == "variable" && f.op == Op::Contains("absent-variable".into())
                })
            })
    });
    assert!(prefs.views["rram_fixture"].tabs["dataset"]
        .filters
        .contains(&Filter {
            column: "variable".into(),
            op: Op::Contains("absent-variable".into())
        }));
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let lines = capture(cx, window);
        if !visible(&lines, "research-dataset-row-0") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "filter did not remove rows: {lines:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    click(cx, window, "research-dataset-layer-raw");
    wait_for(cx, window, "research-dataset-error", "not_capable");
    assert_eq!(
        session::load(&home).views["rram_fixture"].tabs["dataset"]
            .node
            .as_deref(),
        Some("raw")
    );
    let (again, _) = open_shell(cx, lang, theme, dims, seams, true);
    wait_load(cx, again);
    click(cx, again, "research-tab-dataset");
    wait_for(cx, again, "research-dataset-error", "not_capable");
    let view = session::load(&home).views["rram_fixture"].tabs["dataset"].clone();
    assert_eq!(view.node.as_deref(), Some("raw"));
    assert!(view.filters.iter().any(|f| f.column == "variable"));
    assert_eq!(view.offset, 0);
    assert_eq!(tree_hash(&root), before);
}
#[gpui_kit::test]
fn dataset_layouts(cx: &mut TestAppContext) {
    run(cx, Lang::En, ThemeMode::Dark, (1100., 700.));
    run(cx, Lang::ZhTw, ThemeMode::Light, (1920., 1200.));
}
#[gpui_kit::test]
fn raw_and_missing(cx: &mut TestAppContext) {
    let root = fixture_copy("rawonly");
    let before = tree_hash(&root);
    let home = temp_dir();
    let seams = ResearchSeams {
        research_home: Some(home),
        ..Default::default()
    };
    let (window, _) = open_research(cx, Lang::ZhTw, ThemeMode::Dark, (1100., 700.), &root, seams);
    click(cx, window, "research-tab-dataset");
    wait_for(cx, window, "research-dataset-error", "not_capable");
    click(cx, window, "research-dataset-layer-raw");
    let lines = wait_for(cx, window, "research-dataset-row-0", "voltage");
    assert!(shows(&lines, "research-dataset-evidence-0", "RE1"));
    assert_eq!(tree_hash(&root), before);
    std::fs::remove_file(root.join("data/raw_measurements.csv")).unwrap();
    let home = temp_dir();
    let seams = ResearchSeams {
        research_home: Some(home.clone()),
        ..Default::default()
    };
    let mut prefs = SessionPrefs::default();
    prefs
        .views
        .entry("rawonly".into())
        .or_default()
        .tabs
        .entry("dataset".into())
        .or_default()
        .node = Some("raw".into());
    session::save(&home, &prefs).unwrap();
    let (window, _) = open_research(cx, Lang::En, ThemeMode::Light, (1920., 1200.), &root, seams);
    click(cx, window, "research-tab-dataset");
    wait_for(cx, window, "research-dataset-error", "missing_artifact");
}
#[gpui_kit::test]
fn restores_nonzero_offset(cx: &mut TestAppContext) {
    let root = fixture_copy("rram");
    let home = temp_dir();
    let mut prefs = SessionPrefs::default();
    let view = prefs
        .views
        .entry("rram_fixture".into())
        .or_default()
        .tabs
        .entry("dataset".into())
        .or_default();
    view.offset = 10;
    view.filters.push(Filter {
        column: "variable".into(),
        op: Op::Contains("voltage".into()),
    });
    session::save(&home, &prefs).unwrap();
    let seams = ResearchSeams {
        research_home: Some(home.clone()),
        ..Default::default()
    };
    let (window, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Light,
        (1920., 1200.),
        &root.join("project"),
        seams.clone(),
    );
    click(cx, window, "research-tab-dataset");
    wait_for(cx, window, "research-dataset-row-10", "voltage");
    let (again, _) = open_shell(cx, Lang::En, ThemeMode::Light, (1920., 1200.), seams, true);
    wait_load(cx, again);
    click(cx, again, "research-tab-dataset");
    wait_for(cx, again, "research-dataset-row-10", "voltage");
    assert_eq!(
        session::load(&home).views["rram_fixture"].tabs["dataset"].offset,
        10
    );
}
#[test]
fn tree_hash_negative_control() {
    use std::io::Write as _;
    let root = fixture_copy("rawonly");
    let before = tree_hash(&root);
    std::fs::OpenOptions::new()
        .append(true)
        .open(root.join("data/raw_measurements.csv"))
        .unwrap()
        .write_all(b"x")
        .unwrap();
    assert_ne!(tree_hash(&root), before);
}

#[gpui_kit::test]
fn split_drag_persists_and_restores(cx: &mut TestAppContext) {
    use axiom::desktop::layout::persist;
    use gpui_kit::px;
    let copy = fixture_copy("rram");
    let before = tree_hash(&copy);
    let home = temp_dir();
    cx.update(|cx| persist::init(&home, cx));
    let seams = ResearchSeams {
        research_home: Some(home.clone()),
        ..Default::default()
    };
    let (window, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1920., 1200.),
        &copy.join("project"),
        seams.clone(),
    );
    click(cx, window, "research-tab-dataset");
    wait_for(cx, window, "research-dataset-row-0", "voltage");
    let original = cx
        .update_window(window, |_, window, _| {
            window.find("research-dataset-split-frame").bounds()
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
            window.find("research-dataset-split-frame").bounds()
        })
        .unwrap();
    assert!(moved.origin.x > original.origin.x);
    let saved = cx
        .update(|cx| {
            persist::flush(cx);
            persist::sizes("research.dataset", cx)
        })
        .unwrap();
    assert_eq!(
        persist::load(&home).panels.get("research.dataset"),
        Some(&saved)
    );
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    let (again, _) = open_shell(cx, Lang::En, ThemeMode::Dark, (1920., 1200.), seams, true);
    wait_load(cx, again);
    click(cx, again, "research-tab-dataset");
    wait_for(cx, again, "research-dataset-row-0", "voltage");
    let restored = cx
        .update_window(again, |_, window, _| {
            window.find("research-dataset-split-frame").bounds()
        })
        .unwrap();
    assert_eq!(restored.origin.x, moved.origin.x);
    assert_eq!(tree_hash(&copy), before);
}
