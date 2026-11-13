//! Fault isolation and retry contract for desktop panels.
use axiom::desktop::components::panel::{Panel, PanelState};
use axiom::desktop::data::{
    self,
    faults::{self, Fault, FaultPlan},
    Cursor, Delta, Feed, Source,
};
use axiom::desktop::{
    demo, i18n,
    layout::Viewport,
    targets::{self, CaptureState, Opening, Target},
    theme, Shell, Store,
};
use gpui_kit::component::ThemeMode;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    div, px, size, AnyWindowHandle, AppContext as _, Bounds, Context, ElementId,
    InteractiveElement as _, IntoElement, ParentElement as _, Point, Render, SharedString,
    Styled as _, TestAppContext, Window, WindowBounds, WindowOptions,
};
use std::{collections::HashMap, time::Duration};

fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.refresh();
        window.render_frame(cx);
    })
    .expect("render frame");
    cx.run_until_parked();
}
fn open(
    cx: &mut TestAppContext,
    target: &Target,
    viewport: Viewport,
) -> Result<AnyWindowHandle, String> {
    let mut state = demo::state();
    if matches!(
        target.opening,
        Opening::Page("paper") | Opening::PaperStep(_)
    ) {
        let fake = std::sync::Arc::new(axiom::desktop::ux::seams::FakePaper::new());
        let session = state
            .runs
            .first()
            .ok_or("paper needs a demo session")?
            .session
            .to_string();
        use axiom::paper::{Change, ChangeKind, PassId, PassRecord, SpendEstimate};
        fn unknown<T>() -> axiom::paper::Known<T> {
            axiom::paper::Known::Unknown {
                reason: "not supplied".into(),
            }
        }
        let shortlist = axiom::paper::Shortlist {
            requirements_hash: "requirements".into(),
            ranked: vec![axiom::paper::RankedJournal {
                journal: axiom::paper::JournalRecord {
                    id: "journal".into(),
                    title: "Example journal".into(),
                    issns: vec![],
                    issn_l: None,
                    publisher: unknown(),
                    homepage: unknown(),
                    quartiles: vec![],
                    sjr: unknown(),
                    access: unknown(),
                    apc: unknown(),
                    review_weeks: unknown(),
                    topics: vec![],
                    works_count: unknown(),
                },
                bucket: axiom::paper::Bucket::NeedsCheck,
                filters: vec![],
                fit: 0.9,
                terms: vec![],
                free_text: vec![],
            }],
            excluded: vec![],
            generated_at: "2025-03-04".into(),
            hash: "layout-list".into(),
        };
        fake.set_shortlist(&session, shortlist.clone());
        let estimate = SpendEstimate {
            per_member: Default::default(),
            priced_usd: Some(1.25),
            samples: 1,
            unpriced: vec![],
            line: "Estimated $1.25".into(),
            digest: "layout-estimate".into(),
        };
        let pdf = std::path::PathBuf::from("layout-preview.pdf");
        fake.set_preview(
            &session,
            axiom::desktop::ux::seams::Preview::Unavailable {
                reason: "Preview unavailable".into(),
                pdf: pdf.clone(),
            },
        );
        fake.set_status(
            &session,
            axiom::paper::service::PaperStatus {
                steps: vec![],
                evidence: None,
                requirements: None,
                shortlist: Some(shortlist),
                choice: None,
                estimate: Some(estimate),
                passes: vec![PassRecord {
                    pass: PassId::AstraDraft,
                    member: "Astra".into(),
                    model: None,
                    provider: "fake".into(),
                    status: axiom::council::Status::Ok,
                    evidence_hash: "evidence".into(),
                    input_hash: "input".into(),
                    output_hash: Some("output".into()),
                    usd: None,
                    tokens: 0,
                    raw_path: String::new(),
                    change_log: vec![Change {
                        location: "abstract".into(),
                        kind: ChangeKind::Edited,
                        reason: "layout".into(),
                    }],
                    validation: vec![],
                    at: "2025-03-04".into(),
                }],
                ready: None,
                pdf: Some(pdf),
                running: false,
                last_error: None,
            },
        );
        state.ux.seams.paper = Some(fake);
    }
    if matches!(target.opening, Opening::Dialog("interjection")) {
        state.ux.seams.interjections = Some(std::sync::Arc::new(
            axiom::desktop::ux::seams::FakeInterjections::new(),
        ));
    }
    // Subtargets are prepared through their public parent, then opened by UI actions.
    let parent = match target.opening {
        Opening::ResearchTab(_) => Some("research"),
        Opening::PaperStep(_) => Some("paper"),
        Opening::Dialog("control" | "interjection") => Some("run"),
        Opening::Dialog("evolve") => Some("usage"),
        _ => None,
    };
    let prepared = parent
        .map(|id| targets::find(id).expect("registered parent"))
        .unwrap_or_else(|| target.clone());
    targets::prepare(&prepared, CaptureState::Live, &mut state)
        .map_err(|e| format!("prepare {} failed: {e:?}", prepared.id))?;
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(ThemeMode::Dark, None, cx);
        i18n::set(i18n::Lang::En, cx);
        cx.set_global(Store::open(true));
    });
    let state = cx.new(|_| state);
    let bounds = Bounds {
        origin: Point::default(),
        size: size(px(viewport.width), px(viewport.height)),
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        ..Default::default()
    };
    let shell_state = state.clone();
    let (mut window, _) = cx
        .update(|cx| {
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| Shell::new(shell_state, false, window, cx))
            })
        })
        .map_err(|e| format!("open Shell: {e:?}"))?;
    cx.simulate_window_scale_factor_change(window, viewport.scale);
    for _ in 0..4 {
        frame(cx, window);
    }
    match target.opening {
        Opening::Overlay("composer") | Opening::Dialog("composer") => {
            state.update(cx, |state, cx| state.set_composing(true, cx));
        }
        Opening::Overlay("palette") => {
            let key = if cfg!(target_os = "macos") {
                "cmd-k"
            } else {
                "ctrl-k"
            };
            cx.update_window(window, |_, window, cx| window.press(key, cx))
                .map_err(|e| format!("open palette: {e:?}"))?;
        }
        Opening::Dialog("close") => {
            if gpui_kit::VisualTestContext::from_window(window, cx).simulate_close() {
                return Err("close request closed the window instead of opening the dialog".into());
            }
        }
        Opening::ResearchTab(tab) => {
            let id = ElementId::Name(format!("research-tab-{tab}").into());
            cx.update_window(window, |_, w, cx| w.click(id, cx))
                .map_err(|e| format!("open research tab {tab}: {e:?}"))?;
        }
        Opening::PaperStep(step) => match step {
            // These sections are rendered on the paper page, not separate steps.
            "form" | "shortlist" | "estimate" | "progress" | "changelog" | "preview" => {}
            "readiness" => return Err("missing public paper readiness view hook".into()),
            _ => return Err(format!("missing public paper step hook for {step}")),
        },
        Opening::Window("focus.graph" | "focus.checklist") => {
            use axiom::desktop::settings::{self, FocusPanel, WindowMode};
            let panel = if target.id == "focus.graph" {
                FocusPanel::Graph
            } else {
                FocusPanel::Checklist
            };
            cx.update(|cx| {
                settings::set_mode(WindowMode::Focus(panel), cx);
            });
        }
        Opening::Window("mini") => {
            // The test platform's minimize operation is unimplemented, after the
            // mini window has already been created (as in mini.rs).
            let site = std::sync::Arc::new(std::sync::Mutex::new(None));
            let captured = site.clone();
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                if let Some(p) = info.location() {
                    *captured.lock().unwrap() = Some((p.file().replace('\\', "/"), p.line()));
                }
            }));
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                cx.update_window(window, |_, w, cx| {
                    w.press(
                        if cfg!(target_os = "macos") {
                            "cmd-shift-m"
                        } else {
                            "ctrl-shift-m"
                        },
                        cx,
                    )
                })
                .unwrap();
            }));
            std::panic::set_hook(previous);
            if let Err(e) = result {
                let allowed = site.lock().unwrap().as_ref().is_some_and(|(file, line)| {
                    file.ends_with("/gpui-pre-0.3.11/src/platform/test/window.rs") && *line == 473
                }) && e.downcast_ref::<&str>() == Some(&"not implemented");
                if !allowed {
                    std::panic::resume_unwind(e);
                }
            }
            window = cx
                .windows()
                .into_iter()
                .find(|w| *w != window)
                .ok_or("mini transition did not create a window")?;
            cx.simulate_window_scale_factor_change(window, viewport.scale);
        }
        Opening::Dialog("control") => {
            return Err(
                "missing public control dialog opening hook (the run control bar is not a dialog)"
                    .into(),
            )
        }
        Opening::Dialog("interjection") => click_if_present(cx, window, "open-interjection")?,
        Opening::Dialog("evolve") => click_if_present(cx, window, "evolve-open")?,
        Opening::Screen(_) | Opening::Page(_) => {}
        ref opening => return Err(format!("missing public opening hook for {opening:?}")),
    }
    frame(cx, window);
    Ok(window)
}

fn click_if_present(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    id: &'static str,
) -> Result<(), String> {
    cx.update_window(window, |_, w, cx| {
        if w.try_find(id).is_none() {
            return Err(format!("public action {id} unavailable"));
        }
        w.click(id, cx);
        Ok(())
    })
    .map_err(|e| format!("public action {id}: {e:?}"))?
}

fn set_fault(cx: &mut TestAppContext, panel: &'static str, fault: Option<Fault>) {
    let mut plan = HashMap::new();
    if let Some(fault) = fault {
        plan.insert(panel.to_owned(), fault);
    }
    cx.update(|cx| faults::install(FaultPlan(plan), cx));
}
fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: &str) {
    cx.update_window(window, |_, w, cx| {
        w.click(SharedString::from(id.to_owned()), cx)
    })
    .expect("click");
}
fn has(cx: &mut TestAppContext, window: AnyWindowHandle, id: &str) -> bool {
    cx.update_window(window, |_, w, _| {
        w.try_find(ElementId::Name(id.to_owned().into())).is_some()
    })
    .unwrap()
}
fn texts(cx: &mut TestAppContext, window: AnyWindowHandle) -> Vec<String> {
    cx.update_window(window, |_, w, _| {
        gpui_kit::base::test_support::texts(w)
            .into_iter()
            .filter(|t| t.visible())
            .map(|t| t.text().to_owned())
            .collect()
    })
    .unwrap()
}
fn title(fault: &Fault) -> &'static str {
    match fault {
        Fault::Slow { .. } => "Loading",
        Fault::Corrupt => "Invalid content",
        Fault::Missing => "Nothing here yet",
        Fault::Stale { .. } => "Out of date",
        Fault::Offline => "Offline",
        Fault::Panic => "Unable to load",
    }
}
fn fault_cases() -> [Fault; 6] {
    [
        Fault::Slow { ms: 1000 },
        Fault::Corrupt,
        Fault::Missing,
        Fault::Stale { age_ms: 10000 },
        Fault::Offline,
        Fault::Panic,
    ]
}
fn contains_title(lines: &[String], fault: &Fault) -> bool {
    lines.iter().any(|s| s.contains(title(fault)))
}
fn tick(cx: &mut TestAppContext) {
    cx.executor().advance_clock(Duration::from_millis(60));
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(55));
}

mod rules {
    use super::*;
    use axiom::desktop::components::panel::RenderGuard;
    use axiom::desktop::data::feed::FeedCore;

    struct TestSource;
    impl Source<String> for TestSource {
        fn poll(&mut self, cursor: Cursor) -> axiom::base::Res<(Delta<String>, Cursor)> {
            Ok((
                Delta::Replace("healthy content".into()),
                Cursor(cursor.0 + 1),
            ))
        }
    }
    struct TestView {
        feed: Feed<String>,
    }
    impl Render for TestView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let feed = self.feed.clone();
            let panel = Panel::new("synthetic", &self.feed.snapshot().state)
                .retry(move |_, _, cx| {
                    faults::install(FaultPlan::default(), cx);
                    feed.retry();
                })
                .content(|_, _| {
                    div()
                        .id("healthy")
                        .test_support()
                        .child("healthy content")
                        .into_any_element()
                });
            div().flex().flex_col().child(panel).child(
                Panel::new("sibling", &PanelState::Ready).content(|_, _| {
                    div()
                        .id("sibling-content")
                        .test_support()
                        .child("sibling content")
                        .into_any_element()
                }),
            )
        }
    }
    fn open_test(cx: &mut TestAppContext, fault: Option<Fault>) -> (AnyWindowHandle, Feed<String>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            theme::apply(ThemeMode::Dark, None, cx);
            i18n::set(i18n::Lang::En, cx);
        });
        set_fault(cx, "synthetic", fault);
        let feed =
            cx.update(|cx| Feed::spawn("synthetic", TestSource, Duration::from_secs(60), cx));
        let f = feed.clone();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1440.), px(900.)),
            })),
            ..Default::default()
        };
        let (window, _) = cx
            .update(|cx| {
                gpui_kit::open_window(options, cx, |_, cx| {
                    cx.new(|cx| {
                        f.observe(cx);
                        TestView { feed: f }
                    })
                })
            })
            .unwrap();
        (window, feed)
    }
    fn wait_for(cx: &mut TestAppContext, window: AnyWindowHandle, predicate: impl Fn() -> bool) {
        for _ in 0..100 {
            frame(cx, window);
            if predicate() {
                return;
            }
            tick(cx);
        }
        panic!("feed did not reach expected state");
    }
    #[gpui_kit::test]
    fn each_fault_and_retry(cx: &mut TestAppContext) {
        for fault in fault_cases() {
            let (window, feed) = open_test(cx, Some(fault.clone()));
            if !matches!(fault, Fault::Slow { .. } | Fault::Panic) {
                wait_for(cx, window, || feed.snapshot().version > 0);
            } else if matches!(fault, Fault::Panic) {
                wait_for(cx, window, || feed.snapshot().state == PanelState::Ready);
            }
            frame(cx, window);
            let state = feed.snapshot().state;
            assert!(
                match fault {
                    Fault::Slow { .. } => matches!(state, PanelState::Loading),
                    Fault::Corrupt => matches!(state, PanelState::Corrupt { .. }),
                    Fault::Missing => matches!(state, PanelState::Empty),
                    Fault::Stale { .. } => matches!(state, PanelState::Stale { .. }),
                    Fault::Offline => matches!(state, PanelState::Offline { .. }),
                    Fault::Panic => matches!(state, PanelState::Ready),
                },
                "{fault:?}: wrong feed state {state:?}"
            );
            assert!(
                contains_title(&texts(cx, window), &fault),
                "{fault:?}: wrong panel state {:?}",
                texts(cx, window)
            );
            let shown_state = if matches!(fault, Fault::Panic) {
                PanelState::Failed { reason: String::new() }
            } else {
                state
            };
            let (expected_title, expected_reason) =
                axiom::desktop::components::panel::copy(i18n::Lang::En, &shown_state).unwrap();
            let visible_texts = texts(cx, window);
            for expected in [expected_title, expected_reason, "Retry"] {
                assert!(visible_texts.iter().any(|s| s == expected),
                    "{fault:?}: missing visible text {expected:?}: {visible_texts:?}");
            }
            assert!(
                has(cx, window, "retry:synthetic"),
                "{fault:?}: missing retry"
            );
            assert!(
                has(cx, window, "sibling-content"),
                "{fault:?}: sibling lost"
            );
            click(cx, window, "retry:synthetic");
            if matches!(fault, Fault::Slow { .. }) {
                cx.executor().advance_clock(Duration::from_millis(1100));
            }
            wait_for(cx, window, || feed.snapshot().state == PanelState::Ready);
            frame(cx, window);
            assert!(
                has(cx, window, "healthy"),
                "{fault:?}: retry did not recover content"
            );
            assert!(has(cx, window, "sibling-content"));
            cx.update_window(window, |_, w, _| w.remove_window())
                .unwrap();
        }
    }
    #[gpui_kit::test]
    fn no_fault_is_a_negative_control(cx: &mut TestAppContext) {
        let (window, feed) = open_test(cx, None);
        wait_for(cx, window, || feed.snapshot().state == PanelState::Ready);
        frame(cx, window);
        assert!(has(cx, window, "healthy"), "texts {:?}", texts(cx, window));
        assert_eq!(feed.snapshot().state, PanelState::Ready);
        assert!(!has(cx, window, "retry:synthetic"));
    }
    #[test]
    fn old_generation_is_rejected_and_control_detects_overwrite() {
        let core = FeedCore::new(true, None);
        let old = core.begin_poll();
        let new = core.begin_poll();
        assert!(core.apply(new, Ok((Delta::Replace("new"), Cursor(2))), 2, None));
        let before = core.snapshot();
        assert!(!core.apply(old, Ok((Delta::Replace("old"), Cursor(1))), 3, None));
        let after = core.snapshot();
        assert_eq!(after.value.as_deref(), Some(&"new"));
        assert_eq!(after.version, before.version);
        assert_eq!(after.state, before.state);
        let control = FeedCore::new(true, None);
        let a = control.begin_poll();
        let b = control.begin_poll();
        control.apply(a, Ok((Delta::Replace("new"), Cursor(2))), 2, None);
        control.apply(b, Ok((Delta::Replace("old"), Cursor(1))), 3, None);
        assert_ne!(control.snapshot().value.as_deref(), Some(&"new"));
    }
    #[test]
    fn render_io_guard_and_negative_control() {
        let path = std::env::temp_dir().join("axiom-missing-render-guard-test");
        assert!(data::io::read(&path).is_err());
        let panic = std::panic::catch_unwind(|| {
            let _guard = RenderGuard::new("synthetic");
            let _ = data::io::read(&path);
        })
        .expect_err("guard must panic");
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap();
        assert_eq!(message, "io during render synthetic");
    }
}

fn selected_targets() -> Vec<Target> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("release-gates.json");
    match std::fs::read_to_string(&path) {
        Ok(data) => {
            let json: serde_json::Value =
                serde_json::from_str(&data).expect("invalid release-gates.json");
            json["views"]
                .as_array()
                .expect("release gates must list targets")
                .iter()
                .map(|entry| {
                    let id = entry["id"].as_str().expect("target id");
                    let target = targets::find(id)
                        .unwrap_or_else(|| panic!("release target {id} is not registered"));
                    assert_eq!(
                        entry["group"].as_str(),
                        Some(target.group),
                        "release group mismatch for {id}"
                    );
                    target
                })
                .collect()
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => targets::all(),
        Err(e) => panic!("cannot read {}: {e}", path.display()),
    }
}
fn group_matrix(cx: &mut TestAppContext, group: &str) {
    let mut failures = Vec::new();
    for target in selected_targets().into_iter().filter(|t| t.group == group) {
        for &panel in target.panels {
            for fault in fault_cases() {
                let context = format!("target {} panel {} fault {:?}", target.id, panel, fault);
                let existing = cx.windows();
                set_fault(cx, panel, Some(fault.clone()));
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || -> Result<(), String> {
                        let window = open(
                            cx,
                            &target,
                            Viewport {
                                width: 1440.,
                                height: 900.,
                                scale: 1.,
                            },
                        )?;
                        // The guard also detects direct wrapped I/O outside a Panel during a frame.
                        {
                            let _guard = axiom::desktop::components::panel::RenderGuard::new(panel);
                            frame(cx, window);
                        }
                        let retry = format!("retry:{panel}");
                        if !has(cx, window, &retry) {
                            return Err(format!(
                                "no migrated Panel/retry for target {} panel {}",
                                target.id, panel
                            ));
                        }
                        if !contains_title(&texts(cx, window), &fault) {
                            return Err(format!("expected {} state", title(&fault)));
                        }
                        let other_panels: Vec<_> = targets::all()
                            .into_iter()
                            .flat_map(|t| t.panels.iter().copied())
                            .filter(|id| *id != panel && has(cx, window, id))
                            .collect();
                        let actions: Vec<_> = target
                            .actions
                            .iter()
                            .copied()
                            .filter(|id| has(cx, window, id))
                            .collect();
                        set_fault(cx, panel, None);
                        click(cx, window, &retry);
                        let mut recovered = false;
                        for _ in 0..100 {
                            frame(cx, window);
                            if !has(cx, window, &retry) {
                                recovered = true;
                                break;
                            }
                            tick(cx);
                        }
                        if !recovered {
                            return Err("retry did not recover".into());
                        }
                        for id in other_panels {
                            if !has(cx, window, id) {
                                return Err(format!("sibling panel {id} disappeared"));
                            }
                        }
                        for id in actions {
                            if !has(cx, window, id) {
                                return Err(format!("action {id} disappeared"));
                            }
                        }
                        Ok(())
                    },
                ));
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => failures.push(format!("{context}: {e}")),
                    Err(e) => {
                        let message = e
                            .downcast_ref::<String>()
                            .map(String::as_str)
                            .or_else(|| e.downcast_ref::<&str>().copied())
                            .unwrap_or("non-string panic");
                        failures.push(format!("{context}: window or render panicked: {message}"));
                    }
                }
                for window in cx.windows().into_iter().filter(|w| !existing.contains(w)) {
                    let _ = cx.update_window(window, |_, w, _| w.remove_window());
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
macro_rules! groups {
    ($($group:ident),* $(,)?) => { $(mod $group {
        #[gpui_kit::test]
        fn fault_matrix(cx: &mut gpui_kit::TestAppContext) { super::group_matrix(cx, stringify!($group)); }
    })* };
}
groups!(
    shell,
    overlays,
    run,
    mesh,
    wishes,
    panels,
    mini,
    sessions_settings,
    research,
    paper,
    modes
);
