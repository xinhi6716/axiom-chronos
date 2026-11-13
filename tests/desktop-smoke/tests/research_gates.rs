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
