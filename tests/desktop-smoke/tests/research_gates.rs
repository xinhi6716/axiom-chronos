mod research_support;

use axiom::{
    desktop::{
        i18n::Lang,
        research::{ResearchSeams, Tab},
    },
    research_workspace::{
        index::Op,
        seams::FakeRenderer,
        session::{self, SessionPrefs},
    },
};
use gpui_kit::{
    component::ThemeMode, test::TestWindowExt as _, AnyWindowHandle, AppContext as _, ElementId,
    TestAppContext,
};
use research_support::*;
use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

static DESKTOP: Mutex<()> = Mutex::new(());

fn seams(home: &Path) -> ResearchSeams {
    ResearchSeams {
        research_home: Some(home.into()),
        renderer: Some(Arc::new(FakeRenderer::new())),
        page_cache_bytes: 1024 * 1024,
        ..Default::default()
    }
}

fn wait_for(cx: &mut TestAppContext, window: AnyWindowHandle, id: &str) -> Vec<(String, String)> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let lines = capture(cx, window);
        if visible(&lines, id) {
            return lines;
        }
        assert!(Instant::now() < deadline, "waiting for {id}: {lines:?}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

// Drive one scheduled task at a time: draining the test executor also drains
// the blocking index-event receiver through Ready and skips the preview.
fn preview_frame(cx: &mut TestAppContext, window: AnyWindowHandle) -> (Duration, Duration) {
    let tick = Instant::now();
    cx.dispatcher.tick(false);
    let tick_time = tick.elapsed();
    let render = Instant::now();
    cx.update_window(window, |_, w, cx| w.render_frame(cx))
        .unwrap();
    (tick_time, render.elapsed())
}

// Poll rendered element visibility without collecting every text in the window.
fn wait_visible(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    id: &str,
) -> (Duration, Duration) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut tick_time = Duration::ZERO;
    let mut harness = Duration::ZERO;
    loop {
        let (tick, render) = preview_frame(cx, window);
        tick_time += tick;
        let poll = Instant::now();
        let found = cx
            .update_window(window, |_, w, _| {
                w.try_find(id.to_owned())
                    .is_some_and(|element| element.visible())
            })
            .unwrap();
        harness += render + poll.elapsed();
        if found {
            return (tick_time, harness);
        }
        assert!(
            Instant::now() < deadline,
            "waiting for visible {id}: {:?}",
            capture(cx, window)
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn wait_preview_end(cx: &mut TestAppContext, window: AnyWindowHandle) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        preview_frame(cx, window);
        let preview = cx
            .update_window(window, |_, w, _| {
                w.try_find("research-dataset-preview-notice")
                    .is_some_and(|element| element.visible())
            })
            .unwrap();
        if !preview {
            return;
        }
        assert!(Instant::now() < deadline, "waiting for indexing to finish");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn preview_capture(cx: &mut TestAppContext, window: AnyWindowHandle) -> Vec<(String, String)> {
    cx.update_window(window, |_, w, _| {
        gpui_kit::base::test_support::texts(w)
            .into_iter()
            .filter(|t| t.visible())
            .map(|t| {
                let path = t
                    .path()
                    .iter()
                    .filter_map(|id| match id {
                        ElementId::Name(n) | ElementId::NamedInteger(n, _) => Some(n.to_string()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("/");
                (path, t.text().to_owned())
            })
            .collect()
    })
    .unwrap()
}

fn click_preview_tab(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.update_window(window, |_, w, cx| {
        assert!(w
            .try_find("research-tab-dataset")
            .is_some_and(|tab| tab.visible()));
        w.click("research-tab-dataset", cx)
    })
    .unwrap();
}

fn indexing_preview(lines: &[(String, String)], lang: Lang) -> bool {
    shows(
        lines,
        "research-load",
        if lang == Lang::En {
            "Indexing"
        } else {
            "正在建立索引"
        },
    ) && visible(lines, "research-dataset-row-0")
        && visible(lines, "research-dataset-preview-notice")
}

// UI phrases observed in English panel captures. Fixture values (including reason
// strings, column names and identifiers) are intentionally not translated.
const ENGLISH_UI: &[&str] = &[
    "Total:",
    "Deduplicated:",
    "Duplicates:",
    "Screened in:",
    "Screened out:",
    "Pending:",
    "Unprocessed:",
    "Read:",
    "Read failed:",
    "Group:",
    "Included",
    "Excluded:",
    "Failed:",
    "Duplicate:",
    "This research does not have literature sources.",
    "PDF pages are not available for this research",
    "Literature",
    "Extracted",
    "Raw",
    "No region recorded; showing the whole page",
    "No evidence",
    "Loading evidence",
    "Not run",
    "No missing-rate columns",
    "Original:",
    "Canonical:",
    "Grade:",
    "no evidence recorded",
    "Correct",
    "Open outputs",
    "Destination",
    "Export CSV",
    "Export evidence bundle",
    "Freeze snapshot",
    "Loading sources…",
    "Build work:",
    "Repro guard:",
    "Gate not declared",
    "Gate not run:",
    "Gate passed",
    "gate artifact ",
    "Done",
    "Paused:",
    "printed",
    "Repro:",
    "Stale:",
    "Flags:",
    "Confirm",
    "Dispute",
    "Open in dataset",
    "No PDF page for this evidence",
    "The page or evidence could not be loaded",
    "Page unavailable",
];

fn normalize_capture(body: &str, temp_paths: &[&Path]) -> String {
    temp_paths.iter().fold(body.to_owned(), |body, path| {
        body.replace(
            path.to_str().expect("temporary path must be UTF-8"),
            "<tmp>",
        )
    })
}

#[test]
fn capture_normalizes_temp_paths_without_hiding_content_changes() {
    let first = "recent\t/tmp/run-one/rram/project\nhome\t/tmp/home-one/session\nstatus\tReady\n";
    let second = "recent\t/tmp/run-two/rram/project\nhome\t/tmp/home-two/session\nstatus\tReady\n";
    let first = normalize_capture(
        first,
        &[Path::new("/tmp/run-one"), Path::new("/tmp/home-one")],
    );
    let second = normalize_capture(
        second,
        &[Path::new("/tmp/run-two"), Path::new("/tmp/home-two")],
    );
    assert_eq!(
        first,
        "recent\t<tmp>/rram/project\nhome\t<tmp>/session\nstatus\tReady\n"
    );
    assert_eq!(first, second);
    assert_ne!(first, second.replace("Ready", "Loading"));
}

fn save_capture(
    view: &str,
    lang: Lang,
    mode: ThemeMode,
    size: (f32, f32),
    lines: &[(String, String)],
    temp_paths: &[&Path],
) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("captures/research");
    fs::create_dir_all(&dir).unwrap();
    let lang = if lang == Lang::En { "en" } else { "zh-TW" };
    let theme = if mode == ThemeMode::Dark {
        "dark"
    } else {
        "light"
    };
    let body = lines
        .iter()
        .map(|(p, t)| format!("{p}\t{t}\n"))
        .collect::<String>();
    fs::write(
        dir.join(format!(
            "{view}.{theme}.{}x{}.{lang}.txt",
            size.0 as u32, size.1 as u32
        )),
        normalize_capture(&body, temp_paths),
    )
    .unwrap();
}

#[gpui_kit::test]
fn capture_walk(cx: &mut TestAppContext) {
    let _guard = DESKTOP.lock().unwrap_or_else(|e| e.into_inner());
    let rram = fixture_copy("rram");
    let raw = fixture_copy("rawonly");
    let before = (tree_hash(&rram), tree_hash(&raw));
    let mut count = 0;
    let mut problems = Vec::new();
    for (fixture, root, tabs) in [
        (
            "rram",
            rram.join("project"),
            vec![
                Tab::Map,
                Tab::Sources,
                Tab::Evidence,
                Tab::Dataset,
                Tab::Exports,
            ],
        ),
        ("rawonly", raw.clone(), vec![Tab::Sources, Tab::Evidence]),
    ] {
        for lang in [Lang::En, Lang::ZhTw] {
            for mode in [ThemeMode::Dark, ThemeMode::Light] {
                for size in [(1100., 700.), (1920., 1200.)] {
                    let home = temp_dir();
                    let mut prefs = SessionPrefs::default();
                    let id = if fixture == "rram" {
                        "rram_fixture"
                    } else {
                        "rawonly"
                    };
                    let view = prefs
                        .views
                        .entry(id.into())
                        .or_default()
                        .tabs
                        .entry("evidence".into())
                        .or_default();
                    view.selection = Some(if fixture == "rram" { "E6" } else { "RE1" }.into());
                    if fixture == "rawonly" {
                        view.node = Some("raw".into());
                    }
                    session::save(&home, &prefs).unwrap();
                    let (window, _) = open_research(cx, lang, mode, size, &root, seams(&home));
                    let lines = capture(cx, window);
                    assert!(
                        shows(
                            &lines,
                            "research-load",
                            if lang == Lang::En {
                                "Ready"
                            } else {
                                "已就緒"
                            }
                        ),
                        "{fixture}: {lines:?}"
                    );
                    for &tab in &tabs {
                        click(cx, window, &format!("research-tab-{}", tab.key()));
                        let ready = match (fixture, tab) {
                            (_, Tab::Map) => "research-map-status",
                            ("rram", Tab::Sources) => "research-sources-node-total",
                            (_, Tab::Sources) => "research-sources-error",
                            ("rram", Tab::Evidence) => "research-evidence-locator",
                            (_, Tab::Evidence) => "research-evidence-locator",
                            (_, Tab::Dataset) => "research-dataset-row-0",
                            (_, Tab::Exports) => "research-exports-repro",
                        };
                        let mut lines = wait_for(cx, window, ready);
                        if fixture == "rram" && tab == Tab::Evidence {
                            let deadline = Instant::now() + Duration::from_secs(30);
                            loop {
                                let page = cx
                                    .update_window(window, |_, w, _| {
                                        w.try_find("research-evidence-page").is_some()
                                    })
                                    .unwrap();
                                if page && !visible(&lines, "research-evidence-degraded") {
                                    break;
                                }
                                assert!(
                                    Instant::now() < deadline,
                                    "evidence page did not render: {lines:?}"
                                );
                                lines = capture(cx, window);
                            }
                        }
                        let panel = format!("research-{}-panel", tab.key());
                        let view_name = if fixture == "rram" {
                            tab.key().to_owned()
                        } else {
                            format!("rawonly-{}", tab.key())
                        };
                        save_capture(
                            &view_name,
                            lang,
                            mode,
                            size,
                            &lines,
                            &[rram.parent().unwrap(), raw.parent().unwrap(), &home],
                        );
                        count += 1;
                        cx.update_window(window, |_, w, _| {
                            if w.try_find(panel.clone()).is_none() {
                                problems.push(format!("{view_name}: missing {panel}"));
                            }
                            if w.try_find("research-evidence-highlight").is_some() {
                                problems.push(format!("{view_name}: highlight element"));
                            }
                        })
                        .unwrap();
                        if !lines.iter().any(|(path, text)| {
                            path.split('/').any(|part| part == panel) && !text.trim().is_empty()
                        }) {
                            problems.push(format!("{view_name} {lang:?} {size:?}: empty panel"));
                        }
                        if lines.iter().any(|(p, _)| {
                            p.split('/')
                                .any(|part| part.starts_with("research-evidence-highlight"))
                        }) {
                            problems.push(format!("{view_name}: highlight text path"));
                        }
                        if !visible(&lines, "research-blocker")
                            || !visible(&lines, "research-primary")
                        {
                            problems.push(format!(
                                "{view_name} {lang:?}: blocker or primary not visible"
                            ));
                        }
                        if lang == Lang::ZhTw {
                            for (p, t) in &lines {
                                if p.split('/').any(|part| part == panel) {
                                    for phrase in ENGLISH_UI {
                                        if t.contains(phrase) {
                                            problems.push(format!(
                                                "{view_name}: untranslated {phrase:?} in {t:?}"
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    cx.update_window(window, |_, w, _| w.remove_window())
                        .unwrap();
                }
            }
        }
    }
    assert_eq!(count, 56);
    assert_eq!(
        (tree_hash(&rram), tree_hash(&raw)),
        before,
        "fixture modified during walk"
    );
    assert!(problems.is_empty(), "capture problems: {problems:#?}");
}

fn synthetic(root: &Path) {
    fs::create_dir_all(root.join("data")).unwrap();
    fs::write(root.join("research-manifest.json"), r#"{"manifest_version":1,"research_id":"perf","research_version":"1","title":"Performance","capabilities":["evidence","dataset"],"settings":{},"versions":{},"artifacts":[{"id":"evidence","role":"evidence","path":"data/evidence.csv","format":"csv"},{"id":"measurements","role":"measurements","path":"data/measurements.csv","format":"csv"}]}"#).unwrap();
    let mut evidence = BufWriter::new(File::create(root.join("data/evidence.csv")).unwrap());
    writeln!(evidence, "evidence_id,paper_id,sha256,physical_page,quote").unwrap();
    for n in 0..40_000 {
        writeln!(
            evidence,
            "E{n},P{},hash{:064x},{},experimental evidence group {}",
            n % 2000,
            n,
            n % 30 + 1,
            n % 20
        )
        .unwrap();
    }
    drop(evidence);
    let mut measurements =
        BufWriter::new(File::create(root.join("data/measurements.csv")).unwrap());
    writeln!(
        measurements,
        "row_id,paper_id,variable,value_original,unit_original,value_canonical"
    )
    .unwrap();
    for n in 0..10_000 {
        let canonical = if n % 7 == 0 {
            String::new()
        } else {
            (n % 347).to_string()
        };
        writeln!(
            measurements,
            "M{n},P{},variable{},{},mV,{canonical}",
            n % 2000,
            n % 12,
            n % 347
        )
        .unwrap();
    }
}

#[gpui_kit::test]
fn first_screen_50k(cx: &mut TestAppContext) {
    if std::env::var_os("CI").is_some() {
        // A timing gate with an indexing-window race: it is measured on the reference
        // machine (the owner's measurement rule), not on shared CI runners.
        eprintln!("first_screen_50k: timing gate runs on the reference machine, skipped on CI");
        return;
    }
    let _guard = DESKTOP.lock().unwrap_or_else(|e| e.into_inner());
    let root = temp_dir();
    synthetic(&root);
    let mut times = Vec::new();
    for _ in 0..10 {
        let home = temp_dir();
        let (window, _) = open_shell(
            cx,
            Lang::En,
            ThemeMode::Dark,
            (1100., 700.),
            seams(&home),
            true,
        );
        let typing_start = Instant::now();
        click(cx, window, "research-open-path");
        type_text(cx, window, root.to_str().unwrap());
        let store = root.parent().unwrap().join("store");
        if store.is_dir() {
            click(cx, window, "research-open-store");
            type_text(cx, window, store.to_str().unwrap());
        }
        let start = cx
            .update_window(window, |_, window, cx| {
                let start = Instant::now();
                window.click("research-open-button", cx);
                start
            })
            .expect("open research");
        let started = Instant::now();
        // The tab bar is already present during Opening; wait for Indexing so
        // the Located event cannot subsequently restore the tab back to Map.
        let open_poll = wait_visible(cx, window, "research-map-indexing");
        let clicked = Instant::now();
        click_preview_tab(cx, window);
        let row_poll = wait_visible(cx, window, "research-dataset-row-0");
        let row_visible = Instant::now();
        let preview = preview_capture(cx, window);
        assert!(indexing_preview(&preview, Lang::En), "{preview:?}");
        // This is observed separately, after the gate endpoint, and is not part
        // of the first-screen measurement.
        wait_preview_end(cx, window);
        let ready = Instant::now();
        let capture_start = Instant::now();
        let lines = capture(cx, window);
        let capture_ms = capture_start.elapsed().as_secs_f64() * 1000.0;
        assert!(shows(&lines, "research-load", "Ready"), "{lines:?}");
        assert!(
            shows(&lines, "research-dataset-row-0", "variable0"),
            "{lines:?}"
        );
        let open_ms = ready.duration_since(start).as_secs_f64() * 1000.0;
        let open_to_click_ms = clicked.duration_since(start).as_secs_f64() * 1000.0;
        let row_ms = row_visible.duration_since(clicked).as_secs_f64() * 1000.0;
        println!("GATE phases typing_ms={:.3} open_to_ready_ms={open_ms:.3} click_to_row_ms={row_ms:.3} open_to_click_ms={open_to_click_ms:.3} start_open_ms={:.3} poll_render_lookup_ms={:.3} poll_tick_ms={:.3} capture_ms={capture_ms:.3}", start.duration_since(typing_start).as_secs_f64() * 1000.0, started.duration_since(start).as_secs_f64() * 1000.0, (open_poll.1 + row_poll.1).as_secs_f64() * 1000.0, (open_poll.0 + row_poll.0).as_secs_f64() * 1000.0);
        times.push(row_visible.duration_since(start).as_secs_f64() * 1000.0);
        cx.update_window(window, |_, w, _| w.remove_window())
            .unwrap();
    }
    times.sort_by(|a, b| a.total_cmp(b));
    let p95 = times[9]; // nearest-rank ceil(0.95 * 10)
    println!("GATE first_screen_p95_ms={p95:.3} runs=10");
    assert!(p95 <= 2000.0, "first-screen times: {times:?}");
}

#[gpui_kit::test]
fn dataset_preview_while_indexing(cx: &mut TestAppContext) {
    let _guard = DESKTOP.lock().unwrap_or_else(|e| e.into_inner());
    let root = temp_dir();
    synthetic(&root);
    for lang in [Lang::En, Lang::ZhTw] {
        let home = temp_dir();
        let (window, _) = open_shell(cx, lang, ThemeMode::Dark, (1100., 700.), seams(&home), true);
        start_open_unparked(cx, window, &root);
        wait_visible(cx, window, "research-map-indexing");
        click_preview_tab(cx, window);
        wait_visible(cx, window, "research-dataset-row-0");
        let lines = preview_capture(cx, window);
        assert!(indexing_preview(&lines, lang), "{lang:?}: {lines:?}");
        let without_row: Vec<_> = lines
            .iter()
            .filter(|(p, _)| !p.split('/').any(|part| part == "research-dataset-row-0"))
            .cloned()
            .collect();
        assert!(
            !indexing_preview(&without_row, lang),
            "negative control accepted missing row"
        );
        if lang == Lang::ZhTw {
            let notice = lines
                .iter()
                .find(|(p, _)| {
                    p.split('/')
                        .any(|part| part == "research-dataset-preview-notice")
                })
                .unwrap();
            assert!(
                !notice.1.chars().any(|c| c.is_ascii_alphabetic()),
                "untranslated notice: {notice:?}"
            );
        }
        wait_preview_end(cx, window);
        let lines = capture(cx, window);
        assert!(
            shows(
                &lines,
                "research-load",
                if lang == Lang::En {
                    "Ready"
                } else {
                    "已就緒"
                }
            ),
            "{lang:?}: {lines:?}"
        );
        assert!(
            !visible(&lines, "research-dataset-preview-notice"),
            "{lines:?}"
        );
        assert!(visible(&lines, "research-dataset-row-0"), "{lines:?}");
        cx.update_window(window, |_, w, _| w.remove_window())
            .unwrap();
    }
}

#[gpui_kit::test]
fn reopen_restores_page_and_filter(cx: &mut TestAppContext) {
    let _guard = DESKTOP.lock().unwrap_or_else(|e| e.into_inner());
    let root = temp_dir();
    synthetic(&root);
    let home = temp_dir();
    let (window, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1100., 700.),
        &root,
        seams(&home),
    );
    click(cx, window, "research-tab-dataset");
    wait_for(cx, window, "research-dataset-row-0");
    click(cx, window, "research-dataset-filter-variable");
    type_text(cx, window, "variable0");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let prefs = session::load(&home);
        if prefs
            .views
            .get("perf")
            .and_then(|v| v.tabs.get("dataset"))
            .is_some_and(|v| {
                v.filters
                    .iter()
                    .any(|f| f.column == "variable" && f.op == Op::Contains("variable0".into()))
            })
        {
            break;
        }
        assert!(Instant::now() < deadline, "filter not persisted: {prefs:?}");
        frame(cx, window);
    }
    cx.update_window(window, |_, w, cx| {
        w.scroll(
            "research-dataset-row-0",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-840.))),
            cx,
        )
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let offset = loop {
        let lines = capture(cx, window);
        let prefs = session::load(&home);
        if let Some(v) = prefs.views.get("perf").and_then(|v| v.tabs.get("dataset")) {
            if v.offset > 0 && visible(&lines, &format!("research-dataset-row-{}", v.offset)) {
                break v.offset;
            }
        }
        assert!(
            Instant::now() < deadline,
            "page not persisted: {prefs:?} {lines:?}"
        );
    };
    cx.update_window(window, |_, w, _| w.remove_window())
        .unwrap();
    let (again, _) = open_shell(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1100., 700.),
        seams(&home),
        true,
    );
    wait_load(cx, again);
    let lines = wait_for(cx, again, &format!("research-dataset-row-{offset}"));
    assert!(
        shows(
            &lines,
            &format!("research-dataset-row-{offset}"),
            "variable0"
        ),
        "{lines:?}"
    );
    let prefs = session::load(&home);
    let view = &prefs.views["perf"];
    assert_eq!(view.tab, "dataset");
    assert_eq!(view.tabs["dataset"].offset, offset);
    assert!(view.tabs["dataset"]
        .filters
        .iter()
        .any(|f| f.column == "variable" && f.op == Op::Contains("variable0".into())));
}

#[test]
fn tree_hash_negative_control() {
    let root = fixture_copy("rawonly");
    let before = tree_hash(&root);
    OpenOptions::new()
        .append(true)
        .open(root.join("data/raw_measurements.csv"))
        .unwrap()
        .write_all(b"x")
        .unwrap();
    assert_ne!(tree_hash(&root), before);
}

// Kept under `research` so the release-gate test filter includes these tests.
mod research {
    use super::*;
    use axiom::{
        desktop::research::{LoadState, Workbench},
        seams::SystemClock,
    };
    use gpui_kit::Entity;

    fn open(
        cx: &mut TestAppContext,
        root: &Path,
        store: Option<&Path>,
        home: &Path,
    ) -> Entity<Workbench> {
        let wb = cx.new(|cx| Workbench::new(seams(home), None, Arc::new(SystemClock), cx));
        wb.update(cx, |wb, cx| {
            wb.open_paths(root.to_owned(), store.map(Path::to_owned), cx)
        });
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            cx.run_until_parked();
            match wb.read_with(cx, |wb, _| wb.load.clone()) {
                LoadState::Ready => return wb,
                LoadState::Failed { kind, message } => panic!("open failed: {kind}: {message}"),
                _ => {}
            }
            assert!(Instant::now() < deadline, "open timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    // The port has one ordered worker. A view update after a command is a
    // barrier proving that command has finished before hashing the tree.
    fn barrier(cx: &mut TestAppContext, wb: &Entity<Workbench>, marker: usize) {
        let port = wb.read_with(cx, |wb, _| wb.port.clone());
        let mut view = port.view(Tab::Map);
        view.offset = marker;
        port.set_view(Tab::Map, view);
        let deadline = Instant::now() + Duration::from_secs(30);
        while port.view(Tab::Map).offset != marker {
            assert!(
                Instant::now() < deadline,
                "worker barrier {marker} timed out"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[gpui_kit::test]
    fn hash_unchanged_by_every_action(cx: &mut TestAppContext) {
        let copy = fixture_copy("rram");
        let home = temp_dir();
        let destination = temp_dir();
        let before = tree_hash(&copy);
        let wb = open(cx, &copy.join("project"), Some(&copy.join("store")), &home);
        let port = wb.read_with(cx, |wb, _| wb.port.clone());
        barrier(cx, &wb, 1);
        assert_eq!(tree_hash(&copy), before, "open");
        macro_rules! action {
            ($name:literal, $marker:expr, $call:expr) => {{
                $call;
                barrier(cx, &wb, $marker);
                assert_eq!(tree_hash(&copy), before, $name);
            }};
        }
        action!("sample", 2, port.sample("total".into(), 2, 7));
        assert!(!port.sources().unwrap().sample.is_empty());
        action!("locate_page", 3, port.locate_page("E1".into()));
        assert!(port
            .evidence()
            .unwrap()
            .items
            .iter()
            .any(|item| item.id == "E1" && item.target.is_some()));
        action!("render_page", 4, port.render_page("E1".into()));
        assert!(port
            .evidence()
            .unwrap()
            .items
            .iter()
            .any(|item| item.id == "E1" && item.image.is_some()));
        action!("export_csv", 5, port.export_csv(destination.clone()));
        action!("export_bundle", 6, port.export_bundle(destination.clone()));
        action!(
            "export_snapshot",
            7,
            port.export_snapshot(destination.clone())
        );
        let exports = port.exports().unwrap();
        for name in ["csv", "bundle", "snapshot"] {
            assert!(
                exports
                    .results
                    .iter()
                    .any(|item| item.action == name && item.path.is_some() && !item.busy),
                "{name}: {exports:?}"
            );
        }
        action!("refresh", 8, port.refresh());
        assert_eq!(port.load_state(), LoadState::Ready);

        // The public research module does not re-export the correction DTO,
        // so this exercises the rejected default request without relying on
        // an engine or seam implementation type.
        assert!(port.evidence().is_ok());
        action!("correct", 9, port.correct(Default::default()));
        assert_eq!(port.evidence().unwrap_err().kind, "invalid_argument");
        action!("select_entity", 11, port.select_entity("device_A".into()));
        action!(
            "set_filters",
            12,
            port.set_filters(Tab::Dataset, Vec::new())
        );

        let raw = fixture_copy("rawonly");
        let raw_home = temp_dir();
        let raw_before = tree_hash(&raw);
        let raw_wb = open(cx, &raw, None, &raw_home);
        let raw_port = raw_wb.read_with(cx, |wb, _| wb.port.clone());
        raw_port.correct(axiom::desktop::ux::seams::research::Correction {
            target: "evidence:RE1".into(),
            field: "raw_file".into(),
            after: "runs/device_A.csv".into(),
            who: "tester".into(),
            why: "review".into(),
        });
        barrier(cx, &raw_wb, 10);
        let store =
            axiom::research_workspace::annotations::AnnotationStore::open(&raw_home, "rawonly")
                .unwrap();
        assert_eq!(store.corrections(Some("evidence:RE1")).len(), 1);
        assert_eq!(tree_hash(&raw), raw_before, "successful correction");
    }

    #[test]
    fn hash_detects_a_changed_source() {
        let copy = fixture_copy("rawonly");
        let before = tree_hash(&copy);
        OpenOptions::new()
            .append(true)
            .open(copy.join("data/raw_measurements.csv"))
            .unwrap()
            .write_all(b"x")
            .unwrap();
        assert_ne!(tree_hash(&copy), before);
    }

    #[gpui_kit::test]
    fn fault_in_one_tab_does_not_mask_another(cx: &mut TestAppContext) {
        let copy = fixture_copy("rram");
        let wb = open(
            cx,
            &copy.join("project"),
            Some(&copy.join("store")),
            &temp_dir(),
        );
        let port = wb.read_with(cx, |wb, _| wb.port.clone());
        barrier(cx, &wb, 1);
        let mut view = port.view(Tab::Evidence);
        view.node = Some("raw".into());
        port.set_view(Tab::Evidence, view);
        port.locate_page("E101".into());
        barrier(cx, &wb, 2);
        assert!(
            port.evidence()
                .unwrap()
                .items
                .iter()
                .any(|item| item.id == "E101" && item.target.is_some() && item.notice.is_none()),
            "E101 must resolve before the source is removed"
        );
        fs::remove_file(copy.join("project/external/hf02/pulse_run1.csv")).unwrap();
        let before = tree_hash(&copy);
        port.locate_page("E101".into());
        barrier(cx, &wb, 3);
        let evidence = port.evidence().unwrap();
        assert!(
            evidence
                .items
                .iter()
                .any(|item| item.id == "E101" && item.notice.is_some()),
            "missing-source error not shown for E101"
        );
        assert_eq!(port.load_state(), LoadState::Ready);
        assert!(port.map().is_ok());
        assert!(port.sources().is_ok());
        assert!(port.dataset().is_ok());
        assert!(port.exports().is_ok());
        assert_eq!(tree_hash(&copy), before);

        let broken = fixture_copy("rram");
        fs::write(
            broken.join("project/data/literature_inventory.csv"),
            b"\xff",
        )
        .unwrap();
        fs::remove_file(broken.join("project/data/measurements.csv")).unwrap();
        fs::write(broken.join("store/units.tsv"), b"\xff").unwrap();
        let broken_before = tree_hash(&broken);
        let broken_wb = open(
            cx,
            &broken.join("project"),
            Some(&broken.join("store")),
            &temp_dir(),
        );
        barrier(cx, &broken_wb, 4);
        let broken_port = broken_wb.read_with(cx, |wb, _| wb.port.clone());
        assert_eq!(broken_port.load_state(), LoadState::Ready);
        assert!(
            broken_port.sources().is_err(),
            "unreadable inventory must surface"
        );
        assert!(
            broken_port.dataset().is_err(),
            "missing measurements must surface"
        );
        assert!(broken_port.evidence().is_err());
        let map = broken_port.map().unwrap();
        assert!(
            map.notices
                .iter()
                .any(|notice| notice.kind == "store_unavailable"),
            "unreadable store index must surface"
        );
        assert!(broken_port.exports().is_ok());
        assert_eq!(broken_port.view(Tab::Map).offset, 4);
        assert_eq!(tree_hash(&broken), broken_before);

        let invalid = fixture_copy("rawonly");
        fs::write(invalid.join("research-manifest.json"), b"{ invalid").unwrap();
        let invalid_before = tree_hash(&invalid);
        let invalid_wb =
            cx.new(|cx| Workbench::new(seams(&temp_dir()), None, Arc::new(SystemClock), cx));
        invalid_wb.update(cx, |wb, cx| wb.open_paths(invalid.clone(), None, cx));
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            cx.run_until_parked();
            if matches!(
                invalid_wb.read_with(cx, |wb, _| wb.load.clone()),
                LoadState::Failed { .. }
            ) {
                break;
            }
            assert!(Instant::now() < deadline, "invalid manifest did not fail");
            std::thread::sleep(Duration::from_millis(2));
        }
        barrier(cx, &invalid_wb, 5);
        let invalid_port = invalid_wb.read_with(cx, |wb, _| wb.port.clone());
        assert!(matches!(
            invalid_port.load_state(),
            LoadState::Failed { .. }
        ));
        assert!(invalid_port.map().is_err());
        assert!(invalid_port.sources().is_err());
        assert!(invalid_port.evidence().is_err());
        assert!(invalid_port.dataset().is_err());
        assert!(invalid_port.exports().is_ok());
        assert_eq!(invalid_port.view(Tab::Map).offset, 5);
        assert_eq!(tree_hash(&invalid), invalid_before);
    }
    fn tab_degrades_alone(cx: &mut TestAppContext, tab: Tab) {
        use axiom::desktop::{
            components::panel::PanelState,
            data::faults::{self, Fault, FaultPlan},
        };
        use std::collections::HashMap;
        let _guard = DESKTOP.lock().unwrap_or_else(|e| e.into_inner());
        cx.update(|cx| faults::install(FaultPlan::default(), cx));
        let copy = fixture_copy("rram");
        let wb = open(
            cx,
            &copy.join("project"),
            Some(&copy.join("store")),
            &temp_dir(),
        );
        barrier(cx, &wb, 31);
        let port = wb.read_with(cx, |wb, _| wb.port.clone());
        let views: Vec<_> = [
            Tab::Map,
            Tab::Sources,
            Tab::Evidence,
            Tab::Dataset,
            Tab::Exports,
        ]
        .into_iter()
        .map(|t| (t, port.view(t)))
        .collect();
        let panel = format!("research.{}.panel", tab.key());
        cx.update(|cx| faults::install(FaultPlan(HashMap::from([(panel, Fault::Corrupt)])), cx));
        wb.update(cx, |wb, _| match tab {
            Tab::Map => wb.map_feed.retry(),
            Tab::Sources => wb.sources_feed.retry(),
            Tab::Evidence => wb.evidence_feed.retry(),
            Tab::Dataset => wb.dataset_feed.retry(),
            Tab::Exports => wb.exports_feed.retry(),
        });
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            cx.executor().advance_clock(Duration::from_millis(60));
            cx.run_until_parked();
            let states = wb.read_with(cx, |wb, _| {
                [
                    wb.map_feed.snapshot().state,
                    wb.sources_feed.snapshot().state,
                    wb.evidence_feed.snapshot().state,
                    wb.dataset_feed.snapshot().state,
                    wb.exports_feed.snapshot().state,
                ]
            });
            let tabs = [
                Tab::Map,
                Tab::Sources,
                Tab::Evidence,
                Tab::Dataset,
                Tab::Exports,
            ];
            if tabs.iter().zip(&states).all(|(t, state)| {
                if *t == tab {
                    matches!(state, PanelState::Corrupt { .. })
                } else {
                    *state == PanelState::Ready
                }
            }) {
                break;
            }
            assert!(Instant::now() < deadline, "{tab:?}: {states:?}");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(port.load_state(), LoadState::Ready);
        for (t, view) in views {
            assert_eq!(port.view(t), view);
        }
        cx.update(|cx| faults::install(FaultPlan::default(), cx));
    }

    #[gpui_kit::test]
    fn tab_degrades_alone_map(cx: &mut TestAppContext) {
        tab_degrades_alone(cx, Tab::Map);
    }
    #[gpui_kit::test]
    fn tab_degrades_alone_sources(cx: &mut TestAppContext) {
        tab_degrades_alone(cx, Tab::Sources);
    }
    #[gpui_kit::test]
    fn tab_degrades_alone_evidence(cx: &mut TestAppContext) {
        tab_degrades_alone(cx, Tab::Evidence);
    }
    #[gpui_kit::test]
    fn tab_degrades_alone_dataset(cx: &mut TestAppContext) {
        tab_degrades_alone(cx, Tab::Dataset);
    }
    #[gpui_kit::test]
    fn tab_degrades_alone_exports(cx: &mut TestAppContext) {
        tab_degrades_alone(cx, Tab::Exports);
    }
}
