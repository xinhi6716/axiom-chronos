mod research_support;
use axiom::{
    desktop::{
        i18n::Lang,
        research::{LoadState, ResearchSeams, Workbench},
    },
    seams::SystemClock,
};
use gpui_kit::{component::ThemeMode, test::TestWindowExt as _, AppContext as _, TestAppContext};
use research_support::*;
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

fn seams(home: &Path) -> ResearchSeams {
    ResearchSeams {
        research_home: Some(home.to_owned()),
        ..Default::default()
    }
}
fn opened_workbench(
    cx: &mut TestAppContext,
    project: &Path,
    store: &Path,
) -> gpui_kit::Entity<Workbench> {
    let home = temp_dir();
    let wb = cx.new(|cx| Workbench::new(seams(&home), None, Arc::new(SystemClock), cx));
    wb.update(cx, |wb, cx| {
        wb.open_paths(project.to_owned(), Some(store.to_owned()), cx)
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        cx.run_until_parked();
        if wb.read_with(cx, |wb, _| match &wb.load {
            LoadState::Ready => wb.port.map().is_ok_and(|map| !map.stages.is_empty()),
            LoadState::Failed { kind, message } => panic!("{kind}: {message}"),
            _ => false,
        }) {
            return wb;
        }
        assert!(Instant::now() < deadline, "workbench timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[gpui_kit::test]
fn map_displays_truth_across_languages_themes_and_sizes(cx: &mut TestAppContext) {
    for lang in [Lang::En, Lang::ZhTw] {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            for size in [(1100., 700.), (1920., 1200.)] {
                let copy = fixture_copy("rram");
                let before = tree_hash(&copy);
                let wb = opened_workbench(cx, &copy.join("project"), &copy.join("store"));
                let (stage_index, progress, has_repro) = wb.read_with(cx, |wb, _| {
                    let m = wb.port.map().unwrap();
                    let (i, stage) = m
                        .stages
                        .iter()
                        .enumerate()
                        .find(|(_, s)| s.state.starts_with("Paused") && s.progress.is_some())
                        .expect("paused stage with progress");
                    assert!(m.engineering_en.is_some());
                    assert!(!m.funnel.is_empty());
                    (i, stage.progress.clone().unwrap(), m.repro_en.is_some())
                });
                let home = temp_dir();
                let (window, _) =
                    open_research(cx, lang, mode, size, &copy.join("project"), seams(&home));
                let deadline = Instant::now() + Duration::from_secs(30);
                let lines = loop {
                    cx.executor().advance_clock(Duration::from_millis(60));
                    let lines = capture(cx, window);
                    if visible(&lines, &format!("research-map-stage-{stage_index}")) {
                        break lines;
                    }
                    assert!(Instant::now() < deadline, "map feed timed out: {lines:?}");
                    std::thread::sleep(Duration::from_millis(2));
                };
                let (paused, rate, disclaimer) = match lang {
                    Lang::En => ("Paused", "rate limited", "not a research result"),
                    Lang::ZhTw => ("已暫停", "rate limited", "不是研究結果"),
                };
                let stage_id = format!("research-map-stage-{stage_index}");
                assert!(shows(&lines, "research-map-status", paused), "{lines:?}");
                assert!(
                    shows(&lines, &stage_id, paused)
                        && shows(&lines, &stage_id, rate)
                        && shows(&lines, &stage_id, &progress),
                    "{lines:?}"
                );
                assert!(
                    shows(&lines, "research-map-engineering", disclaimer),
                    "{lines:?}"
                );
                assert!(
                    has_repro && visible(&lines, "research-map-repro"),
                    "{lines:?}"
                );
                assert!(visible(&lines, "research-map-current"), "{lines:?}");
                assert_eq!(tree_hash(&copy), before);
            }
        }
    }
}
#[gpui_kit::test]
fn output_link_opens_role_tab_without_changing_fixture(cx: &mut TestAppContext) {
    let copy = fixture_copy("rram");
    let before = tree_hash(&copy);
    let wb = opened_workbench(cx, &copy.join("project"), &copy.join("store"));
    let (i, target) = wb.read_with(cx, |wb, _| {
        let map = wb.port.map().unwrap();
        let (i, _) = map
            .stages
            .iter()
            .enumerate()
            .find(|(_, stage)| stage.output == Some(axiom::desktop::research::Tab::Dataset))
            .expect("measurement output");
        (i, "research-dataset-panel")
    });
    let home = temp_dir();
    let (window, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1920., 1200.),
        &copy.join("project"),
        seams(&home),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        cx.executor().advance_clock(Duration::from_millis(60));
        if visible(&capture(cx, window), &format!("research-map-open-{i}")) {
            break;
        }
        assert!(Instant::now() < deadline, "map output timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
    click(cx, window, &format!("research-map-open-{i}"));
    cx.update_window(window, |_, window, _| {
        assert!(window.try_find(target).is_some(), "missing {target}");
        assert!(window.try_find("research-map-panel").is_none());
    })
    .unwrap();
    assert_eq!(tree_hash(&copy), before);
}

#[gpui_kit::test]
fn registered_research_targets_open_their_actual_tab(cx: &mut TestAppContext) {
    use axiom::desktop::{
        demo, i18n,
        targets::{self, CaptureState},
        theme, Shell, Store,
    };
    use gpui_kit::{px, size, Bounds, Point, WindowBounds, WindowOptions};
    use std::{cell::RefCell, rc::Rc};
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(ThemeMode::Dark, None, cx);
        i18n::set(Lang::En, cx);
        cx.set_global(Store::open(true));
    });
    let mut model = demo::state();
    model.ux.research = seams(&temp_dir());
    let state = cx.new(|_| model);
    let slot = Rc::new(RefCell::new(None));
    let saved = slot.clone();
    let shell_state = state.clone();
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Point::default(),
            size: size(px(1920.), px(1200.)),
        })),
        ..Default::default()
    };
    let (window, _) = cx
        .update(|cx| {
            gpui_kit::open_window(options, cx, |window, cx| {
                let shell = cx.new(|cx| Shell::new(shell_state, false, window, cx));
                *saved.borrow_mut() = Some(shell.clone());
                shell
            })
        })
        .unwrap();
    let shell = slot.borrow_mut().take().unwrap();
    for (id, panel) in [
        ("research", "research-map-panel"),
        ("research.map", "research-map-panel"),
        ("research.sources", "research-sources-panel"),
        ("research.evidence", "research-evidence-panel"),
        ("research.dataset", "research-dataset-panel"),
        ("research.exports", "research-exports-panel"),
        ("research.map", "research-map-panel"),
    ] {
        let target = targets::find(id).unwrap();
        cx.update_window(window, |_, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.open_target(&target, CaptureState::Live, window, cx)
            })
        })
        .unwrap()
        .unwrap();
        frame(cx, window);
        cx.update_window(window, |_, window, _| {
            assert!(
                window.try_find(panel).is_some(),
                "{id} did not open {panel}"
            );
            for element in target.panels.iter().chain(target.actions.iter()) {
                assert!(
                    window.try_find(*element).is_some(),
                    "{id} missing {element}"
                );
            }
        })
        .unwrap();
    }
    click(cx, window, "action:research.sources.open");
    frame(cx, window);
    cx.update_window(window, |_, window, _| {
        assert!(window.try_find("research.sources.panel").is_some());
        assert!(window.try_find("research.map.panel").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn map_feed_faults_do_not_disable_dataset(cx: &mut TestAppContext) {
    use axiom::desktop::data::faults::{self, Fault, FaultPlan};
    use std::collections::HashMap;
    for (fault, marker) in [
        (Fault::Corrupt, "retry:research.map.panel"),
        (Fault::Stale { age_ms: 60_000 }, "retry:research.map.panel"),
    ] {
        cx.update(|cx| {
            faults::install(
                FaultPlan(HashMap::from([("research.map.panel".into(), fault)])),
                cx,
            )
        });
        let copy = fixture_copy("rram");
        let before = tree_hash(&copy);
        let (window, _) = open_research(
            cx,
            Lang::En,
            ThemeMode::Dark,
            (1920., 1200.),
            &copy.join("project"),
            seams(&temp_dir()),
        );
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            cx.executor().advance_clock(Duration::from_millis(60));
            frame(cx, window);
            if cx
                .update_window(window, |_, w, _| w.try_find(marker.to_owned()).is_some())
                .unwrap()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "map fault did not produce {marker}"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        click(cx, window, "research-tab-dataset");
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if visible(&capture(cx, window), "research-dataset-row-0") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "dataset unavailable after map fault"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(tree_hash(&copy), before);
        cx.update_window(window, |_, window, _| window.remove_window())
            .unwrap();
    }
    cx.update(|cx| faults::install(FaultPlan::default(), cx));
}

#[gpui_kit::test]
fn map_split_drag_persists_and_restores_only_for_safe_home(cx: &mut TestAppContext) {
    use axiom::desktop::layout::persist;
    use gpui_kit::px;

    for safe in [true, false] {
        let copy = fixture_copy("rram");
        let before = tree_hash(&copy);
        let home = temp_dir();
        cx.update(|cx| persist::init(&home, cx));
        let configuration = if safe {
            seams(&home)
        } else {
            ResearchSeams::default()
        };
        let (window, _) = open_research(
            cx,
            Lang::En,
            ThemeMode::Dark,
            (1920., 1200.),
            &copy.join("project"),
            configuration.clone(),
        );
        let deadline = Instant::now() + Duration::from_secs(30);
        let original = loop {
            cx.executor().advance_clock(Duration::from_millis(60));
            frame(cx, window);
            if let Some(bounds) = cx
                .update_window(window, |_, window, _| {
                    window
                        .try_find("research-map-split-frame")
                        .map(|element| element.bounds())
                })
                .unwrap()
            {
                break bounds;
            }
            assert!(Instant::now() < deadline, "map split did not appear");
            std::thread::sleep(Duration::from_millis(2));
        };
        cx.update_window(window, |_, window, cx| {
            let start = original.origin + gpui_kit::point(original.size.width / 2., px(10.));
            let end = start + gpui_kit::point(px(4.), px(0.));
            window.drag(start, end, cx);
        })
        .unwrap();
        frame(cx, window);
        let moved = cx
            .update_window(window, |_, window, _| {
                window.find("research-map-split-frame").bounds()
            })
            .unwrap();
        assert!(
            moved.origin.x > original.origin.x,
            "drag did not move split: {original:?} -> {moved:?}"
        );
        let saved = cx.update(|cx| {
            persist::flush(cx);
            persist::sizes("research.map", cx)
        });
        if safe {
            let saved = saved.expect("safe drag was not persisted");
            assert_eq!(
                persist::load(&home).panels.get("research.map"),
                Some(&saved)
            );
            cx.update_window(window, |_, window, _| window.remove_window())
                .unwrap();
            let (reopened, _) = open_research(
                cx,
                Lang::En,
                ThemeMode::Dark,
                (1920., 1200.),
                &copy.join("project"),
                configuration,
            );
            let deadline = Instant::now() + Duration::from_secs(30);
            let restored = loop {
                cx.executor().advance_clock(Duration::from_millis(60));
                frame(cx, reopened);
                if let Some(bounds) = cx
                    .update_window(reopened, |_, window, _| {
                        window
                            .try_find("research-map-split-frame")
                            .map(|element| element.bounds())
                    })
                    .unwrap()
                {
                    break bounds;
                }
                assert!(
                    Instant::now() < deadline,
                    "restored map split did not appear"
                );
                std::thread::sleep(Duration::from_millis(2));
            };
            assert_eq!(restored.origin.x, moved.origin.x);
            cx.update_window(reopened, |_, window, _| window.remove_window())
                .unwrap();
        } else {
            assert_eq!(saved, None, "unsafe drag persisted sizes");
            assert!(!home.join("desktop/layout.json").exists());
            cx.update_window(window, |_, window, _| window.remove_window())
                .unwrap();
        }
        assert_eq!(tree_hash(&copy), before);
    }
}
