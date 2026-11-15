mod research_support;
use axiom::{
    desktop::{
        i18n::Lang,
        research::{LoadState, ResearchRef as UiResearchRef, ResearchSeams, Tab, Workbench},
        ux::Page,
    },
    research_workspace::{
        index::{Filter, Op},
        seams::{FakeRenderer, PageImage, PageRenderer},
        session::{self, ResearchRef},
    },
    seams::SystemClock,
};
use gpui_kit::{component::ThemeMode, test::TestWindowExt as _, AppContext as _, TestAppContext};
use research_support::*;
use std::{path::Path, sync::Arc, time::Duration};

fn seams(home: &Path) -> ResearchSeams {
    ResearchSeams {
        research_home: Some(home.to_owned()),
        ..Default::default()
    }
}
fn wait_saved_tab(home: &Path, tab: &str) -> session::SessionPrefs {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let prefs = session::load(home);
        if prefs.views.values().any(|view| view.tab == tab) {
            return prefs;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "session did not save tab {tab}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
const PALETTE: &str = if cfg!(target_os = "macos") {
    "cmd-k"
} else {
    "ctrl-k"
};

#[gpui_kit::test]
fn palette_is_gated_and_opens_research(cx: &mut TestAppContext) {
    let home = temp_dir();
    let (window, state) = open_shell(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1440., 900.),
        ResearchSeams::default(),
        false,
    );
    press(cx, window, PALETTE);
    type_text(cx, window, "Research workbench");
    press(cx, window, "enter");
    assert_ne!(
        state.read_with(cx, |s, _| s.ux.page.clone()),
        Some(Page::Research)
    );
    press(cx, window, "escape");
    state.update(cx, |s, cx| {
        s.ux.research = seams(&home);
        cx.notify();
    });
    press(cx, window, PALETTE);
    type_text(cx, window, "Research workbench");
    press(cx, window, "enter");
    assert_eq!(
        state.read_with(cx, |s, _| s.ux.page.clone()),
        Some(Page::Research)
    );
    let lines = capture(cx, window);
    assert!(
        shows(&lines, "research-load", "No research open"),
        "{lines:?}"
    );
}

#[gpui_kit::test]
fn rram_blocker_is_visible_and_primary_returns_to_map(cx: &mut TestAppContext) {
    let copy = fixture_copy("rram");
    let before = tree_hash(&copy);
    let home = temp_dir();
    let (window, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1100., 700.),
        &copy.join("project"),
        seams(&home),
    );
    let lines = capture(cx, window);
    assert!(shows(&lines, "research-load", "Ready"), "{lines:?}");
    assert!(
        shows(&lines, "research-blocker", "Paused")
            && shows(&lines, "research-blocker", "rate limited"),
        "{lines:?}"
    );
    assert!(
        shows(&lines, "research-primary", "Show what is blocking"),
        "{lines:?}"
    );
    assert!(visible(&lines, "research-blocker") && visible(&lines, "research-primary"));
    cx.update_window(window, |_, window, _| {
        assert!(window.try_find("research-map-panel").is_some());
        assert!(window.try_find("research-dataset-panel").is_none());
    })
    .unwrap();
    click(cx, window, "research-tab-dataset");
    cx.update_window(window, |_, window, _| {
        assert!(window.try_find("research-map-panel").is_none());
        assert!(window.try_find("research-dataset-panel").is_some());
    })
    .unwrap();
    wait_saved_tab(&home, "dataset");
    click(cx, window, "research-primary");
    wait_saved_tab(&home, "map");
    assert_eq!(tree_hash(&copy), before);
}

#[gpui_kit::test]
fn corrupt_manifest_reports_kind_and_retry(cx: &mut TestAppContext) {
    let copy = fixture_copy("rram");
    let home = temp_dir();
    std::fs::write(copy.join("project/research-manifest.json"), "{").unwrap();
    let before = tree_hash(&copy);
    let (window, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1100., 700.),
        &copy.join("project"),
        seams(&home),
    );
    let lines = capture(cx, window);
    assert!(
        shows(&lines, "research-load", "Failed: invalid_manifest"),
        "{lines:?}"
    );
    assert!(
        shows(&lines, "research-primary", "Retry opening"),
        "{lines:?}"
    );
    click(cx, window, "research-primary");
    wait_load(cx, window);
    assert!(shows(
        &capture(cx, window),
        "research-load",
        "Failed: invalid_manifest"
    ));
    assert_eq!(tree_hash(&copy), before);
}

#[gpui_kit::test]
fn headers_are_localized(cx: &mut TestAppContext) {
    for (lang, ready, dataset, blocker) in [
        (Lang::En, "Ready", "Dataset", "Paused"),
        (Lang::ZhTw, "已就緒", "資料集", "已暫停"),
    ] {
        let copy = fixture_copy("rram");
        let home = temp_dir();
        let hash = tree_hash(&copy);
        let (window, _) = open_research(
            cx,
            lang,
            ThemeMode::Dark,
            (1440., 900.),
            &copy.join("project"),
            seams(&home),
        );
        let lines = capture(cx, window);
        assert!(
            shows(&lines, "research-load", ready)
                && shows(&lines, "research-tab-dataset", dataset)
                && shows(&lines, "research-blocker", blocker),
            "{lines:?}"
        );
        if lang == Lang::ZhTw {
            assert!(
                !lines
                    .iter()
                    .any(|(_, t)| t.contains("Nothing is blocking this research")
                        || t.contains("Show what is blocking")),
                "{lines:?}"
            );
        }
        assert_eq!(tree_hash(&copy), hash);
    }
}

#[test]
fn production_paths() {
    let home = Path::new("test-home");
    let s = ResearchSeams::production(home);
    assert_eq!(
        s.research_home.as_deref(),
        Some(home.join("research").as_path())
    );
    assert_eq!(
        s.orch_home.as_deref(),
        Some(home.join("orchestrate").as_path())
    );
    assert_eq!(s.goals_home.as_deref(), Some(home));
    assert!(s.renderer.is_some());
    assert_eq!(s.page_cache_bytes, 256 * 1024 * 1024);
}

struct SlowRenderer(FakeRenderer);
impl PageRenderer for SlowRenderer {
    fn render(
        &self,
        pdf: &Path,
        sha: &str,
        page: u32,
        scale: f32,
        out: &Path,
    ) -> axiom::base::Res<PageImage> {
        std::thread::sleep(Duration::from_millis(100));
        self.0.render(pdf, sha, page, scale, out)
    }
}

#[gpui_kit::test]
fn frames_render_during_indexing(cx: &mut TestAppContext) {
    let copy = fixture_copy("rawonly");
    let home = temp_dir();
    let mut data = String::from("row_id,device,value\n");
    for i in 0..200_000 {
        data.push_str(&format!("{i},device{i},{i}\n"));
    }
    std::fs::write(copy.join("data/raw_measurements.csv"), data).unwrap();
    let before = tree_hash(&copy);
    let mut s = seams(&home);
    s.renderer = Some(Arc::new(SlowRenderer(FakeRenderer::new())));
    let (window, _) = open_shell(cx, Lang::En, ThemeMode::Dark, (1100., 700.), s, true);
    start_open_unparked(cx, window, &copy);
    // Advance the deterministic executor one task at a time so its queued
    // progress events cannot all be drained before an intervening frame.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut frames = 0;
    loop {
        let (load, blocker, primary) = cx.update_window(window, |_, window, cx| {
            window.refresh();
            window.render_frame(cx);
            let texts = gpui_kit::base::test_support::texts(window);
            let select = |name: &str| texts.iter()
                .filter(|t| t.visible() && t.path().iter().any(|id| matches!(id, gpui_kit::ElementId::Name(n) if n.as_ref() == name)))
                .map(|t| t.text().to_owned()).collect::<Vec<_>>();
            (select("research-load"), select("research-blocker"), window.try_find("research-primary").and_then(|b| b.label().map(|s| s.to_string())))
        }).unwrap();
        assert!(
            load.iter()
                .any(|t| t.contains("Opening") || t.contains("Indexing")),
            "unexpected load: {load:?}"
        );
        assert!(
            blocker
                .iter()
                .any(|t| t.contains("Reading the research...")),
            "{blocker:?}"
        );
        assert_eq!(primary.as_deref(), Some("Open a research folder"));
        if load.iter().any(|t| t.contains("Indexing")) {
            frames += 1;
            break;
        }
        assert!(
            !load
                .iter()
                .any(|t| t.contains("Ready") || t.contains("Failed")),
            "no indexing frame: {load:?}"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "indexing timed out: {load:?}"
        );
        cx.dispatcher.tick(false);
    }
    assert!(frames >= 1, "no frame rendered during indexing");
    wait_load(cx, window);
    let lines = capture(cx, window);
    assert!(shows(&lines, "research-load", "Ready"), "{lines:?}");
    assert_eq!(tree_hash(&copy), before);
}

fn wait_workbench(cx: &mut TestAppContext, wb: &gpui_kit::Entity<Workbench>) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        cx.run_until_parked();
        let status = wb.read_with(cx, |wb, _| match &wb.load {
            LoadState::Ready => 1,
            LoadState::Failed { kind, message } => panic!("{kind}: {message}"),
            _ => 0,
        });
        if status == 1 {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "workbench timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[gpui_kit::test]
fn workbench_reopens_saved_tab_filters_and_offset(cx: &mut TestAppContext) {
    let copy = fixture_copy("rram");
    let home = temp_dir();
    let reference = UiResearchRef {
        root: copy.join("project"),
        store: Some(copy.join("store")),
    };
    let wb = cx.new(|cx| Workbench::new(seams(&home), None, Arc::new(SystemClock), cx));
    wb.update(cx, |wb, cx| wb.open(reference.clone(), cx));
    assert!(wb.read_with(cx, |wb, _| matches!(
        wb.load,
        LoadState::Opening | LoadState::Indexing { .. }
    )));
    wait_workbench(cx, &wb);
    let filter = axiom::desktop::ux::seams::research::Filter {
        column: "device".into(),
        op: axiom::desktop::ux::seams::research::FilterOp::Eq("RRAM".into()),
    };
    wb.update(cx, |wb, cx| {
        wb.set_tab(Tab::Dataset, cx);
        wb.set_view(
            Tab::Dataset,
            |view| {
                view.filters = vec![filter.clone()];
                view.offset = 42;
            },
            cx,
        );
    });
    let reopened = cx.new(|cx| Workbench::new(seams(&home), None, Arc::new(SystemClock), cx));
    wait_workbench(cx, &reopened);
    reopened.read_with(cx, |wb, _| {
        assert_eq!(wb.tab, Tab::Dataset);
        assert_eq!(wb.view(Tab::Dataset).filters, vec![filter]);
        assert_eq!(wb.view(Tab::Dataset).offset, 42);
    });
}

#[gpui_kit::test]
fn recent_click_opens_research(cx: &mut TestAppContext) {
    let copy = fixture_copy("rram");
    let home = temp_dir();
    let mut prefs = session::SessionPrefs::default();
    session::remember(
        &mut prefs,
        ResearchRef {
            root: copy.join("project"),
            store: Some(copy.join("store")),
        },
    );
    prefs.last = None;
    session::save(&home, &prefs).unwrap();
    let (window, _) = open_shell(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1440., 900.),
        seams(&home),
        true,
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let lines = capture(cx, window);
        if visible(&lines, "research-recent-0") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "recent item missing: {lines:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(shows(
        &capture(cx, window),
        "research-load",
        "No research open"
    ));
    click(cx, window, "research-recent-0");
    wait_load(cx, window);
    let lines = capture(cx, window);
    assert!(
        shows(&lines, "research-load", "Ready") && shows(&lines, "research-blocker", "Paused"),
        "{lines:?}"
    );
}

#[gpui_kit::test]
fn restores_dataset_tab_filters_and_offset(cx: &mut TestAppContext) {
    let copy = fixture_copy("rram");
    let home = temp_dir();
    let hash = tree_hash(&copy);
    let (window, state) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1440., 900.),
        &copy.join("project"),
        seams(&home),
    );
    cx.update_window(window, |_, window, _| {
        assert!(window.try_find("research-view-state").is_none());
    })
    .unwrap();
    click(cx, window, "research-tab-dataset");
    cx.update_window(window, |_, window, _| {
        assert!(window.try_find("research-view-state").is_none());
    })
    .unwrap();
    let mut prefs = wait_saved_tab(&home, "dataset");
    let view = prefs
        .views
        .values_mut()
        .find(|v| v.tab == "dataset")
        .unwrap()
        .tabs
        .entry("dataset".into())
        .or_default();
    view.filters = vec![Filter {
        column: "device".into(),
        op: Op::Eq("RRAM".into()),
    }];
    view.offset = 42;
    click(cx, window, "research-back");
    session::save(&home, &prefs).unwrap();
    state.update(cx, |s, cx| {
        s.ux.page = Some(Page::Research);
        cx.notify();
    });
    frame(cx, window);
    wait_load(cx, window);
    let lines = capture(cx, window);
    assert!(
        shows(&lines, "research-tab-dataset", "Dataset"),
        "{lines:?}"
    );
    assert!(
        shows(&lines, "research-view-state", "device")
            && shows(&lines, "research-view-state", "RRAM")
            && shows(&lines, "research-view-state", "43"),
        "dataset filters and offset are not visible: {lines:?}"
    );
    assert_eq!(tree_hash(&copy), hash);
}
