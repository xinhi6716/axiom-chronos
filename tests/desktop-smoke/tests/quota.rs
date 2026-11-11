//! Quota meters in the real Shell and Mini, with isolated demo state.
use axiom::desktop::{demo, i18n, theme, Shell, Store};
use axiom::usage::quota::{ProviderQuota, Window, WindowKind};
use gpui_kit::component::ThemeMode;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{px, size, AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, Point, TestAppContext, WindowBounds, WindowOptions};
use std::time::{SystemTime, UNIX_EPOCH};

fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<axiom::desktop::model::AppState>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(ThemeMode::Dark, None, cx);
        i18n::set(i18n::Lang::En, cx);
        cx.set_global(Store::open(true));
    });
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
    let sample = |kind, used| Some(Window { kind, used, resets_at: if used == 1.0 { Some(now + 1200) } else { None }, observed_at: now, status: None });
    let mut initial = demo::state();
    initial.codex_slots = None; initial.codex_models.clear(); initial.live_models.clear();
    initial.quota.claude = ProviderQuota { present: [false; 2], observed_at: None, five_hour: sample(WindowKind::FiveHour, 0.25), weekly: sample(WindowKind::Weekly, 0.5) };
    initial.quota.codex = ProviderQuota { present: [false; 2], observed_at: None, five_hour: sample(WindowKind::FiveHour, 1.0), weekly: sample(WindowKind::Weekly, 0.0) };
    let state = cx.new(|_| initial);
    let bounds = Bounds { origin: Point::default(), size: size(px(1440.), px(900.)) };
    let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
    let (window, _) = cx.update(|cx| gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(state.clone(), false, window, cx)))).unwrap();
    frame(cx, window);
    (window, state)
}

fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    cx.run_until_parked();
}

fn check(cx: &mut TestAppContext, handle: AnyWindowHandle, mini: bool, zh: bool) {
    cx.update_window(handle, |_, window, cx| {
        let viewport = window.viewport_size();
        if mini { assert_eq!(viewport.width, px(380.)); }
        let texts = gpui_kit::base::test_support::texts(window);
        let row = theme::row_height(theme::density(cx));
        let strip = window.find("quota-strip").bounds();
        assert_eq!(strip.size.height, row);
        assert_eq!(strip.origin.y + strip.size.height, viewport.height);
        if mini {
            let scale = window.scale_factor();
            let back_y = f32::from(strip.origin.y - theme::space(theme::density(cx), theme::Step::Sm) - row) * scale;
            let back_height = f32::from(row) * scale;
            assert!(window.painted_quads().iter().any(|q| q.background.as_solid() == Some(cx.global::<theme::Tokens>().raised)
                && (q.bounds.origin.y.0 - back_y).abs() < 1.0
                && (q.bounds.size.height.0 - back_height).abs() < 1.0), "Back is immediately above the quota row");
        }
        let strip_id: ElementId = "quota-strip".into();
        let strip_texts: Vec<_> = texts.iter().filter(|t| t.path().contains(&strip_id) && t.visible()).collect();
        assert!(!strip_texts.is_empty());
        let top = strip_texts[0].bounds().origin.y;
        assert!(strip_texts.iter().all(|t| !t.text().contains(['\n', '\r']) && t.bounds().origin.y == top), "strip text must occupy one line without line breaks");
        if mini && !strip_texts.iter().any(|t|t.text()=="Claude") {
            assert!(strip_texts.iter().any(|t|t.text().contains(if zh {"並行"}else{"parallel"})));
            for text in strip_texts { assert!(text.bounds().right()<=viewport.width); }
            return;
        }
        let exhausted = match (mini, zh) {
            (true, true) => "0% ↻20分", (true, false) => "0% ↻20m",
            (false, true) => "0% · 20 分後重置", (false, false) => "0% · resets in 20 min",
        };
        let cases = if zh {
            [("quota-claude-five-hour", "75%", "5 小時"), ("quota-claude-weekly", "50%", "每週"),
             ("quota-codex-five-hour", exhausted, "5 小時"), ("quota-codex-weekly", "100%", "每週")]
        } else {
            [("quota-claude-five-hour", "75%", "5 hours"), ("quota-claude-weekly", "50%", "weekly"),
             ("quota-codex-five-hour", exhausted, "5 hours"), ("quota-codex-weekly", "100%", "weekly")]
        };
        for (id, expected, name) in cases {
            let bounds = window.find(id).bounds();
            assert!(bounds.origin.x >= px(0.) && bounds.origin.x + bounds.size.width <= viewport.width, "{id} meter exceeds window: {bounds:?}");
            assert!(bounds.origin.y >= strip.origin.y && bounds.origin.y + bounds.size.height <= viewport.height, "{id} meter exceeds row: {bounds:?}");
            let element_id: ElementId = id.into();
            let matching: Vec<_> = texts.iter().filter(|t| t.path().contains(&element_id) && t.visible()).collect();
            assert!(matching.iter().any(|t| t.text() == expected), "{id} must paint {expected}");
            assert_eq!(matching.iter().any(|t| t.text() == name), !mini, "{id} window label");
            assert!(matching.iter().all(|t| !t.text().contains(['\n', '\r'])), "{id} text must not contain line breaks");
            let top = matching[0].bounds().origin.y;
            assert!(matching.iter().all(|t| t.bounds().origin.y == top), "{id} text must occupy one line");
            for text in matching {
                let b = text.bounds();
                assert!(b.origin.x >= px(0.) && b.origin.x + b.size.width <= viewport.width, "{id} exceeds window: {b:?}");
                assert!(b.origin.y >= viewport.height - row && b.origin.y + b.size.height <= viewport.height, "{id} exceeds quota row: {b:?}");
            }
        }
    }).unwrap();
}

#[gpui_kit::test]
fn main_quota_meters(cx: &mut TestAppContext) {
    let (main, _) = open(cx);
    check(cx, main, false, false);
    cx.update_window(main, |_, window, _| {
        let text = gpui_kit::base::test_support::texts(window).iter().map(|t|t.text().to_string()).collect::<Vec<_>>().join(" ");
        assert!(text.contains("Codex parallel — / 48"), "{text}");
    }).unwrap();
    cx.update(|cx| { i18n::set(i18n::Lang::ZhTw, cx); theme::set_density(theme::Density::Dense, cx); });
    frame(cx, main);
    check(cx, main, false, true);
}

#[gpui_kit::test]
fn plan_windows_and_running_models_are_painted_in_strip_order(cx: &mut TestAppContext) {
    let (main, state) = open(cx);
    cx.update(|cx| state.update(cx, |state, _| {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        state.quota.codex = ProviderQuota { present: [false; 2], observed_at: None, five_hour: None, weekly: Some(Window { kind: WindowKind::Weekly, used: 0.42, resets_at: None, observed_at: now, status: None }) };
        state.codex_slots = Some(15);
        let demo=demo::state(); state.codex_models=demo.codex_models; state.live_models=demo.live_models;
    }));
    frame(cx, main);
    cx.update_window(main, |_, window, _| {
        let strip: ElementId = "quota-strip".into();
        let texts: Vec<_> = gpui_kit::base::test_support::texts(window).iter()
            .filter(|t| t.path().contains(&strip) && t.visible()).map(|t| t.text().to_string()).collect();
        let codex = texts.iter().position(|t| t == "Codex").unwrap();
        assert!(!texts[codex..].iter().any(|t| t == "5 hours"), "{texts:?}");
        let models: ElementId = "running-models".into();
        let all = gpui_kit::base::test_support::texts(window);
        let first = all.iter().find(|t| t.path().contains(&models) && t.visible()).expect("demo has running model glyph counts");
        assert!(first.bounds().origin.x > window.find("quota-codex").bounds().right());
        assert!(first.bounds().origin.x < window.find("codex-parallel").bounds().origin.x);
        let painted: Vec<_>=all.iter().filter(|t|t.path().contains(&models)&&t.visible()).map(|t|t.text()).collect();
        assert_eq!(painted,vec!["2","12","1","·","1","4","2","·","1"]);
        assert!(all.iter().filter(|t|t.path().contains(&strip)&&t.visible()).all(|t|t.bounds().right()<=window.viewport_size().width));
    }).unwrap();
}

#[gpui_kit::test]
fn fresh_absence_unseen_and_stale_windows_for_both_providers(cx: &mut TestAppContext) {
    let (main,state)=open(cx);
    for (five,weekly,stale,expected) in [(false,true,false,[false,true]),(true,false,false,[true,false]),
        (false,false,false,[true,true]),(false,true,true,[true,true])] {
        cx.update(|cx|state.update(cx,|state,_| {
            let now=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
            let sample=|kind|Some(Window { kind,used:0.42,resets_at:None,observed_at:if stale {now-2400}else{now},status:None });
            for provider in [&mut state.quota.codex,&mut state.quota.claude] {
                *provider=ProviderQuota { present: [false; 2], observed_at:None,five_hour:if five {sample(WindowKind::FiveHour)}else{None},weekly:if weekly {sample(WindowKind::Weekly)}else{None} };
            }
        }));
        frame(cx,main);
        cx.update_window(main,|_,window,_| {
            let texts=gpui_kit::base::test_support::texts(window);
            for prefix in ["quota-codex","quota-claude"] {
                for (suffix,visible) in [("five-hour",expected[0]),("weekly",expected[1])] {
                    let id:ElementId=format!("{prefix}-{suffix}").into();
                    let painted:Vec<_>=texts.iter().filter(|t|t.path().contains(&id)&&t.visible()).collect();
                    assert_eq!(!painted.is_empty(),visible,"{prefix}-{suffix}");
                    if visible && (stale || (!five&&!weekly)) { assert!(painted.iter().any(|t|t.text()=="—")); }
                }
            }
        }).unwrap();
    }
}

#[gpui_kit::test]
fn mini_quota_meters(cx: &mut TestAppContext) {
    let (main, state) = open(cx);
    // Match walk.rs: the locked test platform cannot minimize the main window.
    let site = std::sync::Arc::new(std::sync::Mutex::new(None));
    let captured = site.clone();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(loc) = info.location() { *captured.lock().unwrap() = Some((loc.file().replace('\\', "/"), loc.line())); }
    }));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.update_window(main, |_, window, cx| window.press(if cfg!(target_os = "macos") { "cmd-shift-m" } else { "ctrl-shift-m" }, cx)).unwrap();
    }));
    std::panic::set_hook(previous);
    if let Err(error) = result {
        let location = site.lock().unwrap().clone();
        let message = error.downcast_ref::<&str>().copied().or_else(|| error.downcast_ref::<String>().map(String::as_str));
        if !location.as_ref().is_some_and(|(file, line)| file.ends_with("/gpui-pre-0.3.11/src/platform/test/window.rs") && *line == 473) || message != Some("not implemented") {
            std::panic::resume_unwind(error);
        }
    }
    let mini = cx.windows().into_iter().find(|window| *window != main).expect("real Mini window");
    frame(cx, mini);
    check(cx, mini, true, false);
    cx.update(|cx| { i18n::set(i18n::Lang::ZhTw, cx); theme::set_density(theme::Density::Dense, cx); });
    frame(cx, mini);
    check(cx, mini, true, true);
    cx.update(|cx| state.update(cx, |state, _| {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let exhausted = |kind| Some(Window { kind, used: 1.0, resets_at: Some(now + 1200), observed_at: now, status: None });
        for provider in [&mut state.quota.claude, &mut state.quota.codex] {
            provider.five_hour = exhausted(WindowKind::FiveHour);
            provider.weekly = exhausted(WindowKind::Weekly);
        }
    }));
    for (lang, density) in [(i18n::Lang::ZhTw, theme::Density::Dense), (i18n::Lang::En, theme::Density::Dense),
                            (i18n::Lang::ZhTw, theme::Density::Comfortable), (i18n::Lang::En, theme::Density::Comfortable)] {
        cx.update(|cx| { i18n::set(lang, cx); theme::set_density(density, cx); });
        frame(cx, mini);
        cx.update_window(mini, |_, window, _| {
            let texts=gpui_kit::base::test_support::texts(window);
            let strip:ElementId="quota-strip".into();
            if !texts.iter().any(|t|t.path().contains(&strip)&&t.text()=="Claude") {
                assert!(texts.iter().filter(|t|t.path().contains(&strip)&&t.visible()).all(|t|t.bounds().right()<=window.viewport_size().width));
                return;
            }
            for id in ["quota-claude-five-hour", "quota-claude-weekly", "quota-codex-five-hour", "quota-codex-weekly"] {
                let b = window.find(id).bounds();
                assert!(b.origin.x >= px(0.) && b.origin.x + b.size.width <= window.viewport_size().width, "{id} exhausted countdown exceeds Mini: {b:?}");
            }
        }).unwrap();
    }
    for (kind,show) in [("one",true),("overflow",false),("zero",false)] {
        cx.update(|cx|state.update(cx,|state,_| {
            let demo=demo::state(); state.codex_models=demo.codex_models; state.live_models=demo.live_models;
            state.codex_slots=Some(15);
            match kind {
                "one"=>{state.codex_models.truncate(1);state.live_models.clear();},
                "overflow"=>for model in state.codex_models.iter_mut().chain(&mut state.live_models) {model.count=u64::MAX;},
                _=>for model in state.codex_models.iter_mut().chain(&mut state.live_models) {model.count=0;},
            }
        }));
        frame(cx,mini);
        cx.update_window(mini,|_,window,_| {
            let texts=gpui_kit::base::test_support::texts(window);
            let group:ElementId="running-models".into(); let strip:ElementId="quota-strip".into();
            assert_eq!(texts.iter().any(|t|t.path().contains(&group)&&t.visible()),show,"{kind}");
            let painted:Vec<_>=texts.iter().filter(|t|t.path().contains(&strip)&&t.visible()).collect();
            assert!(painted.iter().all(|t|t.bounds().right()<=window.viewport_size().width));
            if kind=="overflow" {assert_eq!(painted.len(),1,"only parallel total fits");}
        }).unwrap();
    }

}
