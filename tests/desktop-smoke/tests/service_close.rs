//! The local stop-and-close path, driven through the Shell with the real ServiceBackground.
mod ux_support;

use axiom::base::Res;
use axiom::desktop::{
    demo,
    i18n::Lang,
    model::{Backend, RunStatus, Screen, WishRequest},
    ux::real::background::ServiceBackground,
    ux::seams::{BackgroundService, KeepAlive},
};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use std::{cell::RefCell, rc::Rc, sync::Arc, time::Duration};

#[derive(Default)]
struct Calls {
    stopped: Vec<String>,
    quit: Vec<bool>,
}
struct Recording(Rc<RefCell<Calls>>);
impl Backend for Recording {
    fn start_wish(&self, _: WishRequest) -> Res<()> {
        Ok(())
    }
    fn interject(&self, _: &str, _: &str) -> Res<()> {
        Ok(())
    }
    fn stop(&self, session: &str) -> Res<()> {
        self.0.borrow_mut().stopped.push(session.into());
        Ok(())
    }
    fn resume(&self, _: &str, _: Option<&str>) -> Res<()> {
        Ok(())
    }
    fn delete_memory(&self, _: i64) -> Res<()> {
        Ok(())
    }
    fn correct_memory(&self, _: i64, _: &str) -> Res<()> {
        Ok(())
    }
    fn refresh(&self) -> Res<()> {
        Ok(())
    }
    fn recheck(&self) -> Res<()> {
        Ok(())
    }
    fn follow(&self, _: &str) -> Res<()> {
        Ok(())
    }
    fn quit_service(&self, stop: bool) -> Res<()> {
        self.0.borrow_mut().quit.push(stop);
        Ok(())
    }
}

fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
    cx.update_window(window, |_, window, cx| window.click(id, cx))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn stop_and_close_without_a_service_stops_local_wishes_and_never_quits(cx: &mut TestAppContext) {
    let home = std::env::temp_dir().join(format!("axiom-close-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    // No service runs at this home, so the real adapter reports the background as unavailable.
    let background = Arc::new(ServiceBackground::new(&home));
    assert!(matches!(background.status(), KeepAlive::Unavailable { .. }));

    let calls = Rc::new(RefCell::new(Calls::default()));
    let mut initial = demo::state();
    initial.backend = Some(Box::new(Recording(calls.clone())));
    initial.ux.seams.background = Some(background.clone() as Arc<dyn BackgroundService>);
    let hosted: Vec<String> = initial
        .runs
        .iter()
        .filter(|r| r.hosted && r.status == RunStatus::Active)
        .map(|r| r.session.to_string())
        .collect();
    assert!(!hosted.is_empty());

    let (window, _) = ux_support::open(cx, Lang::En, initial, Screen::Run);
    assert!(
        !gpui_kit::VisualTestContext::from_window(window, cx).simulate_close(),
        "running wishes must ask before the window closes"
    );
    cx.run_until_parked();
    ux_support::capture(cx, window);
    click(cx, window, "stop-and-close");
    assert_eq!(calls.borrow().stopped, hosted);
    assert!(calls.borrow().quit.is_empty());

    // The window closes once the stop deadline passes.
    cx.executor().advance_clock(Duration::from_secs(11));
    cx.run_until_parked();
    assert!(!cx.windows().contains(&window));
    assert!(calls.borrow().quit.is_empty(), "closing never quits the service");
    let _ = std::fs::remove_dir_all(&home);
}
