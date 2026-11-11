//! Opens the real shell on the demo state in a headless window, switches through every screen and
//! checks each one renders. A panic in any view's render fails the test.
use axiom::desktop::model::Screen;
use axiom::desktop::{demo, i18n, theme, Shell, Store};
use gpui_kit::component::ThemeMode;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{px, size, AppContext as _, Bounds, Point, TestAppContext, WindowBounds, WindowOptions};

const SCREENS: [Screen; 5] = [Screen::Run, Screen::Wishes, Screen::Memory, Screen::Usage, Screen::Doctor];

#[gpui_kit::test]
fn every_screen_renders_on_the_demo_state(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(ThemeMode::Dark, None, cx);
        cx.set_global(i18n::Lang::En);
        // Ephemeral, as in --demo: nothing is read from or written to ~/.axiom.
        cx.set_global(Store::open(true));
    });
    // The demo state has no backend, so nothing reads ~/.axiom or starts a process.
    let state = cx.new(|_| demo::state());
    let bounds = Bounds { origin: Point::default(), size: size(px(1440.), px(900.)) };
    let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
    let shell_state = state.clone();
    let (window, _) = cx
        .update(|cx| {
            gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(shell_state, false, window, cx)))
        })
        .expect("open the shell");

    // Visit every screen twice, so a screen is also rendered coming back from another one.
    for screen in SCREENS.into_iter().chain(SCREENS.into_iter().rev()) {
        state.update(cx, |s, cx| s.set_view(screen, cx));
        cx.update_window(window, |_, window, cx| window.render_frame(cx)).expect("render a frame");
        cx.run_until_parked();
        assert_eq!(state.read_with(cx, |s, _| s.view), screen);
        let drawn = cx.update_window(window, |_, window, _| window.elements().len()).expect("read the frame");
        assert!(drawn > 0, "{screen:?} drew no observed elements");
    }
}
