//! Observe the demo graph in the real Shell at the minimum window size.
use axiom::desktop::model::{AppState, Screen};
use axiom::desktop::{demo, i18n, theme, Shell, Store};
use gpui_kit::component::{ActiveTheme as _, ThemeMode};
use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{div, font, px, size, AnyWindowHandle, AppContext as _, Bounds, Context, ElementId, Entity, ParentElement as _, Pixels, Point, Render, Styled as _, TestAppContext, TextRun, Window, WindowBounds, WindowOptions};

fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<AppState>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(ThemeMode::Dark, None, cx);
        cx.set_global(i18n::Lang::ZhTw);
        cx.set_global(Store::open(true));
    });
    let mut initial = demo::state();
    initial.view = Screen::Run;
    initial.reduce_motion = true;
    assert!(initial.backend.is_none());
    let state = cx.new(|_| initial);
    let bounds = Bounds { origin: Point::default(), size: size(px(1100.), px(700.)) };
    let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
    let shell_state = state.clone();
    let (window, _) = cx.update(|cx| gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(shell_state, false, window, cx)))).unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    cx.run_until_parked();
    (window, state)
}

fn inside(a: Bounds<Pixels>, b: Bounds<Pixels>) -> bool {
    a.left() >= b.left() - px(0.1) && a.top() >= b.top() - px(0.1)
        && a.right() <= b.right() + px(0.1) && a.bottom() <= b.bottom() + px(0.1)
}

fn overlaps(a: Bounds<Pixels>, b: Bounds<Pixels>) -> bool {
    a.left() < b.right() && b.left() < a.right() && a.top() < b.bottom() && b.top() < a.bottom()
}

fn graph_nodes(window: &mut Window) -> Vec<gpui_kit::base::test_support::ElementSnapshot> {
    let nodes: Vec<_> = window.elements().into_iter().filter(|node| matches!(node.path().last(),
        Some(ElementId::NamedInteger(name, _)) if name.as_ref() == "mesh-module-node" || name.as_ref() == "mesh-chip-node"
    )).collect();
    assert_eq!(nodes.iter().filter(|n| matches!(n.path().last(), Some(ElementId::NamedInteger(name, _)) if name.as_ref() == "mesh-module-node")).count(), demo::sample_run().modules.len());
    assert!(nodes.iter().any(|n| matches!(n.path().last(), Some(ElementId::NamedInteger(name, _)) if name.as_ref() == "mesh-chip-node")));
    nodes
}

fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    cx.run_until_parked();
}

fn first_node_in_graph(window: &mut Window) -> Bounds<Pixels> {
    let mut node = window.find(("mesh-module-node", 0_usize)).bounds();
    node.origin -= window.find("mesh-graph").bounds().origin;
    node
}

struct SizedShell {
    shell: Entity<Shell>,
    width: Pixels,
    height: Pixels,
}

impl Render for SizedShell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui_kit::IntoElement {
        div().w(self.width).h(self.height).child(self.shell.clone())
    }
}

#[gpui_kit::test]
fn zero_height_first_frame_then_real_view_is_fitted(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(ThemeMode::Dark, None, cx);
        cx.set_global(i18n::Lang::ZhTw);
        cx.set_global(Store::open(true));
    });
    let mut initial = demo::state();
    initial.view = Screen::Run;
    initial.reduce_motion = true;
    let state = cx.new(|_| initial);
    let shell_state = state.clone();
    let bounds = Bounds { origin: Point::default(), size: size(px(1100.), px(700.)) };
    let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
    let (window, wrapper) = cx.update(|cx| gpui_kit::open_window(options, cx, |window, cx| {
        let shell = cx.new(|cx| Shell::new(shell_state, false, window, cx));
        cx.new(|_| SizedShell { shell, width: px(1100.), height: px(0.) })
    })).unwrap();
    frame(cx, window);
    wrapper.update(cx, |wrapper, cx| { wrapper.height = px(700.); cx.notify(); });
    frame(cx, window);
    let first = cx.update_window(window, |_, window, _| {
        let mut canvas = window.find("mesh-graph").bounds();
        canvas.size.height -= px(58.);
        for node in graph_nodes(window) {
            assert!(inside(node.bounds(), canvas), "{:?} outside canvas {canvas:?}", node.path());
        }
        first_node_in_graph(window)
    }).unwrap();
    state.update(cx, |s, cx| s.set_view(Screen::Wishes, cx));
    frame(cx, window);
    state.update(cx, |s, cx| s.set_view(Screen::Run, cx));
    frame(cx, window);
    let fresh = cx.update_window(window, |_, window, _| first_node_in_graph(window)).unwrap();
    assert_eq!(first, fresh, "a zero-height first frame must not consume the initial fit");
}

fn open_large_host(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<SizedShell>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(ThemeMode::Dark, None, cx);
        cx.set_global(i18n::Lang::ZhTw);
        cx.set_global(Store::open(true));
    });
    let mut initial = demo::state();
    initial.view = Screen::Run;
    initial.reduce_motion = true;
    let state = cx.new(|_| initial);
    let bounds = Bounds { origin: Point::default(), size: size(px(1552.), px(1103.)) };
    let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
    let (window, wrapper) = cx.update(|cx| gpui_kit::open_window(options, cx, |window, cx| {
        let shell = cx.new(|cx| Shell::new(state, false, window, cx));
        cx.new(|_| SizedShell { shell, width: px(1552.), height: px(1103.) })
    })).unwrap();
    frame(cx, window);
    (window, wrapper)
}

fn shrink_host(cx: &mut TestAppContext, window: AnyWindowHandle, wrapper: &Entity<SizedShell>) {
    wrapper.update(cx, |wrapper, cx| { wrapper.width = px(1100.); wrapper.height = px(700.); cx.notify(); });
    frame(cx, window);
}

#[gpui_kit::test]
fn untouched_graph_refits_after_resize(cx: &mut TestAppContext) {
    let (window, wrapper) = open_large_host(cx);
    shrink_host(cx, window, &wrapper);
    cx.update_window(window, |_, window, _| {
        let mut canvas = window.find("mesh-graph").bounds();
        canvas.size.height -= px(58.);
        let controls = window.find("mesh-view-controls").bounds();
        for node in graph_nodes(window) {
            assert!(inside(node.bounds(), canvas), "{:?}: {:?} outside canvas {canvas:?}", node.path(), node.bounds());
            assert!(!overlaps(node.bounds(), controls), "{:?}: {:?} intersects {controls:?}", node.path(), node.bounds());
        }
    }).unwrap();
}

#[gpui_kit::test]
fn zoomed_graph_does_not_refit_after_resize(cx: &mut TestAppContext) {
    let (window, wrapper) = open_large_host(cx);
    cx.update_window(window, |_, window, cx| window.click("mesh-zoom-out", cx)).unwrap();
    frame(cx, window);
    let zoomed_height = cx.update_window(window, |_, window, _| first_node_in_graph(window).size.height).unwrap();
    shrink_host(cx, window, &wrapper);
    let resized_height = cx.update_window(window, |_, window, _| first_node_in_graph(window).size.height).unwrap();
    assert!((resized_height - zoomed_height).abs() < px(0.1), "zoom changed from {zoomed_height:?} to {resized_height:?}");
}

#[gpui_kit::test]
fn reopening_a_run_refits_instead_of_restoring_its_view(cx: &mut TestAppContext) {
    let (window, state) = open(cx);
    let original = cx.update_window(window, |_, window, _| first_node_in_graph(window)).unwrap();
    cx.update_window(window, |_, window, cx| window.click("mesh-zoom-in", cx)).unwrap();
    frame(cx, window);
    let zoomed = cx.update_window(window, |_, window, _| first_node_in_graph(window)).unwrap();
    assert!(zoomed.size.width > original.size.width);
    cx.update_window(window, |_, window, cx| {
        let graph = window.find("mesh-graph").bounds();
        let start = gpui_kit::point(graph.left() + px(8.), graph.top() + px(8.));
        let end = start + gpui_kit::point(px(30.), px(25.));
        window.pointer_down(start, gpui_kit::MouseButton::Left, cx);
        window.pointer_move(end, Some(gpui_kit::MouseButton::Left), cx);
        window.pointer_up(end, gpui_kit::MouseButton::Left, cx);
    }).unwrap();
    frame(cx, window);
    let panned = cx.update_window(window, |_, window, _| first_node_in_graph(window)).unwrap();
    assert_ne!(panned.origin, zoomed.origin);
    state.update(cx, |s, cx| s.set_view(Screen::Wishes, cx));
    frame(cx, window);
    state.update(cx, |s, cx| s.set_view(Screen::Run, cx));
    frame(cx, window);
    let reopened = cx.update_window(window, |_, window, _| first_node_in_graph(window)).unwrap();
    assert_eq!(reopened, original, "reopening the same run must discard its pan and zoom");
}

#[gpui_kit::test]
fn all_nodes_fit_on_open(cx: &mut TestAppContext) {
    let (window, _) = open(cx);
    cx.update_window(window, |_, window, _| {
        let mut canvas = window.find("mesh-graph").bounds();
        // The 36px controls, 10px inset and 12px gap occupy the bottom strip.
        canvas.size.height -= px(58.);
        for node in graph_nodes(window) {
            assert!(inside(node.bounds(), canvas), "{:?} outside canvas {canvas:?}", node.path());
        }
    }).unwrap();
}

#[gpui_kit::test]
fn controls_do_not_overlap_nodes(cx: &mut TestAppContext) {
    let (window, _) = open(cx);
    cx.update_window(window, |_, window, _| {
        let controls = window.find("mesh-view-controls").bounds();
        for node in graph_nodes(window) {
            assert!(!overlaps(node.bounds(), controls), "{:?}: {:?} intersects {controls:?}", node.path(), node.bounds());
            assert!(node.bounds().bottom() + px(8.) <= controls.top() + px(0.1), "{:?}: {:?} is less than 8px above {controls:?}", node.path(), node.bounds());
        }
    }).unwrap();
}

#[gpui_kit::test]
fn module_labels_are_painted_whole(cx: &mut TestAppContext) {
    let (window, _) = open(cx);
    cx.update_window(window, |_, window, cx| {
        let texts = gpui_kit::base::test_support::texts(window);
        for (index, module) in demo::sample_run().modules.iter().enumerate() {
            let label = window.try_find(("mesh-module-label", index)).unwrap_or_else(|| panic!("missing module label {}", module.id));
            let text = texts.iter().find(|text| text.path().contains(&("mesh-module-label", index).into()) && text.text() == module.id.as_ref() && text.visible())
                .unwrap_or_else(|| panic!("module label {} is not painted", module.id));
            let run = TextRun { len: module.id.len(), font: font(cx.theme().font_family.clone()), ..Default::default() };
            // Module cards are 40 graph pixels high, so their painted height gives the actual zoom.
            let zoom = f32::from(window.find(("mesh-module-node", index)).bounds().size.height) / 40.;
            let width = window.text_system().shape_line(module.id.clone(), px((13. * zoom).clamp(12., 20.)), &[run], None).width;
            assert!(text.bounds().size.width + px(0.1) >= width, "{}: painted text bounds {:?} narrower than shaped width {width:?}", module.id, text.bounds());
            assert!(inside(text.bounds(), label.bounds()), "{} painted outside its box", module.id);
            assert!(inside(label.bounds(), window.find(("mesh-module-node", index)).bounds()), "{} label clipped by its node", module.id);
        }
    }).unwrap();
}
