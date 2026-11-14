use axiom::desktop::{
    demo, i18n,
    layout::{Breakpoint, Viewport},
    targets::{self, CaptureState, Opening, Target},
    theme, Shell, Store,
};
use gpui_kit::component::ThemeMode;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    px, size, AnyWindowHandle, AppContext as _, Bounds, ElementId, Pixels, Point, TestAppContext,
    WindowBounds, WindowOptions,
};

const SIZES: [(u32, u32); 4] = [(800, 600), (1100, 700), (1920, 1200), (3440, 1440)];
const SCALES: [f32; 3] = [1.0, 1.5, 2.0];

fn contains(outer: Bounds<Pixels>, inner: Bounds<Pixels>) -> bool {
    inner.origin.x >= outer.origin.x
        && inner.origin.y >= outer.origin.y
        && inner.origin.x + inner.size.width <= outer.origin.x + outer.size.width
        && inner.origin.y + inner.size.height <= outer.origin.y + outer.size.height
}

fn check_action(
    bounds: Bounds<Pixels>,
    window: Bounds<Pixels>,
    hit_is_self: bool,
) -> Result<(), String> {
    if !(bounds.size.width > px(0.) && bounds.size.height > px(0.)) {
        return Err("action has zero or invalid size".into());
    }
    if !contains(window, bounds) {
        return Err("action is outside the window".into());
    }
    if !hit_is_self {
        return Err("action centre is covered or unreachable".into());
    }
    Ok(())
}

fn check_panel(panel: Bounds<Pixels>, children: &[Bounds<Pixels>]) -> Result<(), String> {
    let mut union = *children.first().ok_or("panel has no observed children")?;
    for child in &children[1..] {
        let left = union.origin.x.min(child.origin.x);
        let top = union.origin.y.min(child.origin.y);
        let right = (union.origin.x + union.size.width).max(child.origin.x + child.size.width);
        let bottom = (union.origin.y + union.size.height).max(child.origin.y + child.size.height);
        union = Bounds {
            origin: gpui_kit::point(left, top),
            size: size(right - left, bottom - top),
        };
    }
    if contains(panel, union) {
        Ok(())
    } else {
        Err("panel clips observed children".into())
    }
}

#[test]
fn matrix_sizes_match_gates() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("release-gates.json");
    let data = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let json: serde_json::Value = serde_json::from_str(&data).expect("valid release gates JSON");
    let layout = &json["layout"];
    assert_eq!(
        layout["sizes"],
        serde_json::json!(["800x600", "1100x700", "1920x1200", "3440x1440"])
    );
    assert_eq!(layout["dpi"], serde_json::json!([100, 150, 200]));
    assert_eq!(
        SIZES.map(|(w, h)| format!("{w}x{h}")),
        ["800x600", "1100x700", "1920x1200", "3440x1440"]
    );
    assert_eq!(SCALES, [1.0, 1.5, 2.0]);
}

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
        Opening::PaperStep(step) => paper_step(step)?,
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

fn inspect(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    target: &Target,
    viewport: Viewport,
) -> Vec<String> {
    cx.update_window(window, |_, window, _| {
        let mut failures = Vec::new();
        let win = Bounds {
            origin: Point::default(),
            size: window.viewport_size(),
        };
        if window.scale_factor() != viewport.scale {
            failures.push(format!(
                "DPI scale is {}, expected {}",
                window.scale_factor(),
                viewport.scale
            ));
        }
        for id in target.actions {
            let eid = ElementId::Name((*id).into());
            match window.try_find(eid) {
                None => failures.push(format!("missing action {id}")),
                Some(action) => {
                    // Snapshots expose no corresponding hitbox identity; only
                    // validate geometry here, leaving the covered rule unit-tested.
                    if let Err(e) = check_action(action.bounds(), win, true) {
                        failures.push(format!("action {id}: {e}"));
                    }
                }
            }
        }
        for id in target.panels {
            match window.try_find(ElementId::Name((*id).into())) {
                None => failures.push(format!("missing panel {id}")),
                Some(panel) => {
                    let panel_id = ElementId::Name((*id).into());
                    let children: Vec<_> = window
                        .elements()
                        .into_iter()
                        .filter(|e| {
                            e.path().contains(&panel_id) && e.path().len() > panel.path().len()
                        })
                        .map(|e| e.bounds())
                        .collect();
                    if let Err(e) = check_panel(panel.bounds(), &children) {
                        failures.push(format!("panel {id}: {e}"));
                    }
                }
            }
        }
        failures
    })
    .expect("inspect frame")
}

fn matrix(cx: &mut TestAppContext, group: &str, id: &str) {
    let target = targets::find(id).unwrap_or_else(|| panic!("missing target {id}"));
    assert_eq!(target.group, group, "target {id} has changed group");
    let mut failures = Vec::new();
    for (width, height) in SIZES {
        for scale in SCALES {
            let viewport = Viewport {
                width: width as f32,
                height: height as f32,
                scale,
            };
            let context = format!(
                "{id} {width}x{height} scale {scale} {:?}",
                Breakpoint::of(viewport.width)
            );
            let existing = cx.windows();
            match open(cx, &target, viewport) {
                Ok(window) => {
                    failures.extend(
                        inspect(cx, window, &target, viewport)
                            .into_iter()
                            .map(|e| format!("{context}: {e}")),
                    );
                }
                Err(e) => failures.push(format!("{context}: {e}")),
            }
            for window in cx.windows().into_iter().filter(|w| !existing.contains(w)) {
                cx.update_window(window, |_, window, _| window.remove_window())
                    .expect("close window");
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// Paper steps are sections rendered on the paper page, not separate steps.
fn paper_step(step: &str) -> Result<(), String> {
    match step {
        "form" | "shortlist" | "estimate" | "progress" | "changelog" | "preview" | "readiness" => Ok(()),
        _ => Err(format!("missing public paper step hook for {step}")),
    }
}

mod rules {
    use super::*;
    use axiom::desktop::layout::persist;

    #[test]
    fn paper_step_accepts_every_page_section() {
        for step in ["form", "shortlist", "estimate", "progress", "changelog", "preview", "readiness"] {
            assert_eq!(paper_step(step), Ok(()), "{step}");
        }
        assert!(paper_step("nonsense").is_err());
    }

    #[gpui_kit::test]
    fn persist_app_api(cx: &mut TestAppContext) {
        let home = tempfile::Builder::new()
            .prefix("layout-matrix-")
            .tempdir_in(std::env::temp_dir())
            .expect("create temporary home");
        let expected = vec![240., 480.];
        cx.update(|cx| {
            persist::init(home.path(), cx);
            persist::remember("run.columns", expected.clone(), cx);
            assert_eq!(persist::sizes("run.columns", cx), Some(expected.clone()));
            persist::flush(cx);
        });
        assert_eq!(persist::load(home.path()).panels.get("run.columns"), Some(&expected));
        assert!(home.path().join("desktop/layout.json").is_file());
        home.close().expect("remove temporary home");
    }

    #[gpui_kit::test]
    fn persist_rejects_bad_input(cx: &mut TestAppContext) {
        let home = tempfile::Builder::new()
            .prefix("layout-matrix-")
            .tempdir_in(std::env::temp_dir())
            .expect("create temporary home");
        cx.update(|cx| {
            persist::init(home.path(), cx);
            for (id, sizes) in [
                ("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", vec![1.]),
                ("Run.columns", vec![1.]),
                ("bad.nan", vec![f32::NAN]),
                ("bad.negative", vec![-1.]),
                ("bad.empty", vec![]),
            ] {
                persist::remember(id, sizes, cx);
                assert_eq!(persist::sizes(id, cx), None, "accepted {id}");
            }
            persist::flush(cx);
        });
        home.close().expect("remove temporary home");
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds {
            origin: gpui_kit::point(px(x), px(y)),
            size: size(px(w), px(h)),
        }
    }
    #[test]
    fn inside_passes() {
        assert!(check_action(rect(10., 10., 20., 20.), rect(0., 0., 100., 100.), true).is_ok());
    }
    #[test]
    fn outside_fails() {
        assert!(check_action(rect(90., 10., 20., 20.), rect(0., 0., 100., 100.), true).is_err());
    }
    #[test]
    fn covered_fails() {
        assert!(check_action(rect(10., 10., 20., 20.), rect(0., 0., 100., 100.), false).is_err());
    }
    #[test]
    fn panel_clipped_child_fails() {
        assert!(check_panel(rect(0., 0., 100., 100.), &[rect(90., 10., 20., 20.)]).is_err());
    }
    #[test]
    fn panel_contained_children_pass() {
        assert!(check_panel(
            rect(0., 0., 100., 100.),
            &[rect(0., 0., 10., 10.), rect(80., 80., 20., 20.)]
        )
        .is_ok());
    }
    #[test]
    fn panel_empty_fails() {
        assert!(check_panel(rect(0., 0., 100., 100.), &[]).is_err());
    }
    #[test]
    fn zero_size_fails() {
        assert!(check_action(rect(10., 10., 0., 20.), rect(0., 0., 100., 100.), true).is_err());
    }
}

macro_rules! group {
    ($group:ident { $($name:ident => $id:literal),+ $(,)? }) => {
        pub mod $group {
            pub const IDS: &[&str] = &[$($id),+];
            $(#[gpui_kit::test] fn $name(cx: &mut gpui_kit::TestAppContext) { super::matrix(cx, stringify!($group), $id); })+
        }
    };
    ($group:ident {}) => {
        pub mod $group {
            pub const IDS: &[&str] = &[];
            #[test] fn no_targets() { assert!(super::targets::all().iter().all(|t| t.group != stringify!($group)), "group gained targets"); }
        }
    };
}
group!(root {});
group!(shell { run => "run", memory => "memory", usage => "usage" });
group!(overlays { palette => "palette", composer => "composer", dialog_close => "dialog.close", dialog_control => "dialog.control", dialog_interjection => "dialog.interjection", dialog_composer => "dialog.composer", dialog_evolve => "dialog.evolve" });
group!(run {});
group!(mesh {});
group!(wishes { wishes => "wishes" });
group!(panels {});
group!(mini { mini => "mini", focus_graph => "focus.graph", focus_checklist => "focus.checklist" });
group!(sessions_settings { settings => "settings", session => "session" });
group!(research { research => "research", research_map => "research.map", research_sources => "research.sources", research_evidence => "research.evidence", research_dataset => "research.dataset", research_exports => "research.exports" });
group!(paper { paper => "paper", paper_form => "paper.form", paper_shortlist => "paper.shortlist", paper_estimate => "paper.estimate", paper_progress => "paper.progress", paper_changelog => "paper.changelog", paper_preview => "paper.preview", paper_readiness => "paper.readiness" });
group!(modes { storage => "storage", chat => "chat", quick => "quick", plaza => "plaza", routine => "routine" });
group!(data {});
group!(layout {});
group!(components {});
group!(motion {});
group!(viz {});

#[test]
fn group_lists_match_targets() {
    let groups: &[(&str, &[&str])] = &[
        ("root", root::IDS),
        ("shell", shell::IDS),
        ("overlays", overlays::IDS),
        ("run", run::IDS),
        ("mesh", mesh::IDS),
        ("wishes", wishes::IDS),
        ("panels", panels::IDS),
        ("mini", mini::IDS),
        ("sessions_settings", sessions_settings::IDS),
        ("research", research::IDS),
        ("paper", paper::IDS),
        ("modes", modes::IDS),
        ("data", data::IDS),
        ("layout", layout::IDS),
        ("components", components::IDS),
        ("motion", motion::IDS),
        ("viz", viz::IDS),
    ];
    let mut listed = std::collections::HashSet::new();
    for (group, ids) in groups {
        for id in *ids {
            assert!(listed.insert(*id), "duplicate group listing: {id}");
            let target = targets::find(id).unwrap_or_else(|| panic!("unknown listed target {id}"));
            assert_eq!(target.group, *group, "wrong group for {id}");
        }
    }
    for target in targets::all() {
        assert!(listed.contains(target.id), "unlisted target {}", target.id);
    }
}
