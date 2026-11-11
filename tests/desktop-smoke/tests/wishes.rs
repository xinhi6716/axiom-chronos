//! Exercises progress in the real Wishes list using only demo data.
use axiom::desktop::model::{Screen, UnitState};
use axiom::desktop::{demo, i18n, theme, Shell, Store};
use gpui_kit::component::ThemeMode;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{px, size, AppContext as _, Bounds, Point, TestAppContext, WindowBounds, WindowOptions};

#[gpui_kit::test]
fn wishes_show_fraction_and_hide_empty_progress(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(ThemeMode::Dark, None, cx);
        cx.set_global(i18n::Lang::ZhTw);
        cx.set_global(Store::open(true));
    });
    let mut initial = demo::state();
    let sample = demo::sample_run();
    assert_eq!((sample.count(UnitState::Verified), sample.units.len()), (13, 21));
    let mut zero = sample.clone();
    zero.units.truncate(3);
    for unit in &mut zero.units { unit.state = UnitState::Pending; }
    let mut full = zero.clone();
    for unit in &mut full.units { unit.state = UnitState::Verified; }
    let mut empty = sample.clone();
    empty.units.clear();
    initial.runs = vec![sample, zero, full, empty];
    let state = cx.new(|_| initial);
    let bounds = Bounds { origin: Point::default(), size: size(px(1440.), px(900.)) };
    let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
    let shell_state = state.clone();
    let (window, _) = cx.update(|cx| {
        gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(shell_state, false, window, cx)))
    }).expect("open the shell");
    state.update(cx, |s, cx| s.set_view(Screen::Wishes, cx));
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).expect("render");
    cx.run_until_parked();
    cx.update_window(window, |_, window, _| {
        let texts = gpui_kit::base::test_support::texts(window);
        for label in ["13 / 21", "0 / 3", "3 / 3"] {
            assert!(window.elements().iter().any(|element| element.label() == Some(label)), "missing {label}");
            assert!(texts.iter().any(|text| text.visible() && text.text() == label), "not painted: {label}");
        }
        for index in 0_usize..3 {
            assert!(window.try_find(("wish-progress-ring", index)).is_some());
            assert!(window.try_find(("wish-progress-count", index)).is_some());
        }
        assert!(window.try_find(("wish-menu", 3_usize)).is_some(), "empty run row is rendered");
        assert!(window.try_find(("wish-progress-ring", 3_usize)).is_none());
        assert!(window.try_find(("wish-progress-count", 3_usize)).is_none());
        assert!(window.try_find_by_label("0 / 0").is_none());
        let title = texts.iter().find(|text| text.path().iter().any(|id| *id == "wishes-page-title".into()))
            .expect("painted page title");
        assert!(title.visible());
        assert_eq!(title.text(), "願望");
        assert!(!title.text().contains('\u{200a}'));
    }).expect("query frame");
}

#[gpui_kit::test]
fn running_wish_wraps_above_its_folder(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(ThemeMode::Dark, None, cx);
        cx.set_global(i18n::Lang::ZhTw);
        cx.set_global(Store::open(true));
    });
    let title = concat!(
        "全面盤點電阻式記憶體的研究資料並建立可追溯的資料集，提出可實驗驗證的製程方案。",
        "整理研究結果、比較模型並記錄每個方案的來源與驗證方法。",
    );
    let folder = "E:\\walk\\wishes-folder";
    let long_title = title.repeat(5);
    let long_folder = "E:\\walk\\wishes-long-folder";
    let mut run = demo::sample_run();
    run.wish = title.into();
    run.cwd = Some(folder.into());
    let mut long_run = run.clone();
    long_run.session = "long-running-wish".into();
    long_run.wish = long_title.into();
    long_run.cwd = Some(long_folder.into());
    let mut initial = demo::state();
    initial.runs = vec![run, long_run];
    let state = cx.new(|_| initial);
    let bounds = Bounds { origin: Point::default(), size: size(px(1100.), px(700.)) };
    let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
    let shell_state = state.clone();
    let (window, _) = cx.update(|cx| {
        gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(shell_state, false, window, cx)))
    }).expect("open the shell");
    state.update(cx, |s, cx| s.set_view(Screen::Wishes, cx));
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).expect("render");
    cx.run_until_parked();
    cx.update_window(window, |_, window, _| {
        let texts = gpui_kit::base::test_support::texts(window);
        let wish = texts.iter().find(|text| text.path().contains(&("wish-text", 0_usize).into()));
        let subtitle = texts.iter().find(|text| text.text().starts_with(folder));
        let long_wish = texts.iter().find(|text| text.path().contains(&("wish-text", 1_usize).into()));
        let long_subtitle = texts.iter().find(|text| text.text().starts_with(long_folder));
        let fits = wish.zip(subtitle).is_some_and(|(wish, subtitle)| {
            let w = wish.bounds();
            let s = subtitle.bounds();
            wish.visible() && subtitle.visible() && wish.text() == title
                && !wish.text().ends_with('…')
                && w.size.height == px(42.)
                && w.origin.y + w.size.height <= s.origin.y
                && subtitle.text() == folder
        }) && long_wish.zip(long_subtitle).is_some_and(|(wish, subtitle)| {
            let w = wish.bounds();
            let s = subtitle.bounds();
            wish.visible() && subtitle.visible() && w.size.height == px(42.)
                && w.origin.y + w.size.height <= s.origin.y && subtitle.text() == long_folder
        });
        assert!(fits, "titles and folders must fit without overlap: wish={wish:?}, subtitle={subtitle:?}, long_wish={long_wish:?}, long_subtitle={long_subtitle:?}");
    }).expect("query frame");
}
