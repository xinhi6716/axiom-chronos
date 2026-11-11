//! The Chronos desktop app. Screens draw `model::AppState`; `live` fills it from disk and the session host.
mod composer;
mod control;
mod concurrency;
mod detail;
pub mod demo;
mod doctor;
mod evolve;
pub mod i18n;
mod launch;
mod live;
mod memory;
mod mesh;
mod mini;
pub mod model;
mod motion;
mod ornament;
mod palette;
pub mod plain_text;
mod quota;
mod run_view;
mod settings;
mod shell;
mod superswitch;
mod todo;
pub mod theme;
mod usage;
mod wishes;

use gpui_kit::component::ThemeMode;
use gpui_kit::*;
use model::Screen;
/// Exported so a headless test can install the same global `run` does, with `Store::open(true)`.
pub use settings::Store;
pub use shell::Shell;
use std::time::Duration;

/// How the app starts. Demo options exist so every screen can be seen and captured without a live wish.
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub demo: bool,
    pub view: Option<Screen>,
    pub light: bool,
    pub english: bool,
    /// Demo data with no runs, to see the empty states.
    pub empty: bool,
    pub scene: Option<String>,
    pub mode: Option<settings::WindowMode>,
    pub density: Option<theme::Density>,
    /// Open off-screen without taking focus, for capturing screenshots while someone uses the machine.
    pub offscreen: bool,
    pub quit_after: Option<u64>,
    /// Opening window size, to capture a screen at a chosen size.
    pub size: Option<Size<Pixels>>,
}

fn parse_size(text: &str) -> Option<Size<Pixels>> {
    let (width, height) = text.split_once('x')?;
    Some(size(px(width.parse().ok()?), px(height.parse().ok()?)))
}

fn initial_state(options: &Options) -> model::AppState {
    let mut state = demo::state();
    if !options.demo || options.empty {
        state.codex_slots = None;
        state.codex_models.clear();
        state.live_models.clear();
        state.runs.clear();
        state.current = None;
        state.memory.clear();
        state.checks.clear();
    }
    if !options.demo { state.quota = Default::default(); }
    state
}

pub fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--demo" => options.demo = true,
            "--light" => options.light = true,
            "--english" => options.english = true,
            "--empty" => options.empty = true,
            "--scene" => {
                i += 1;
                options.scene = Some(args.get(i).ok_or("--scene takes a name")?.clone());
            }
            "--mode" => {
                i += 1;
                options.mode = Some(match args.get(i).map(String::as_str) {
                    Some("full") => settings::WindowMode::Full,
                    Some("focus-graph") => settings::WindowMode::Focus(settings::FocusPanel::Graph),
                    Some("focus-checklist") => settings::WindowMode::Focus(settings::FocusPanel::Checklist),
                    Some("mini") => settings::WindowMode::Mini,
                    _ => return Err("--mode takes full, focus-graph, focus-checklist or mini".into()),
                });
            }
            "--density" => {
                i += 1;
                options.density = Some(match args.get(i).map(String::as_str) {
                    Some("comfortable") => theme::Density::Comfortable,
                    Some("dense") => theme::Density::Dense,
                    _ => return Err("--density takes comfortable or dense".into()),
                });
            }
            "--offscreen" => options.offscreen = true,
            "--size" => {
                i += 1;
                options.size = Some(
                    args.get(i)
                        .and_then(|s| parse_size(s))
                        .ok_or("--size takes WIDTHxHEIGHT")?,
                );
            }
            "--view" => {
                i += 1;
                options.view = Some(match args.get(i).map(String::as_str) {
                    Some("run") => Screen::Run,
                    Some("wishes") => Screen::Wishes,
                    Some("memory") => Screen::Memory,
                    Some("usage") => Screen::Usage,
                    Some("settings") => Screen::Doctor,
                    other => return Err(format!("unknown view: {}", other.unwrap_or(""))),
                });
            }
            "--quit-after" => {
                i += 1;
                options.quit_after = Some(
                    args.get(i)
                        .and_then(|s| s.parse().ok())
                        .ok_or("--quit-after takes seconds")?,
                );
            }
            other => return Err(format!("unknown option: {other}")),
        }
        i += 1;
    }
    Ok(options)
}

pub fn state_scene(options: &Options) -> Option<String> {
    if options.demo { options.scene.clone() } else { None }
}

pub fn run(options: Options) -> i32 {
    // Read before the event loop starts, so no frame ever waits on the disk.
    let ephemeral = options.demo || options.offscreen;
    let mut store = settings::Store::open(ephemeral);
    if ephemeral {
        store.settings.theme = if options.light { settings::ThemeChoice::Light } else { settings::ThemeChoice::Dark };
        if options.english {
            store.settings.language = settings::LanguageChoice::English;
        }
    }
    if let Some(mode) = options.mode { store.settings.mode = mode; }
    if let Some(density) = options.density { store.settings.density = density; }
    let prefs = store.settings.clone();
    gpui_kit::application().with_assets(ornament::Assets).run(move |cx| {
        gpui_kit::init(cx);
        let mode = if options.light { ThemeMode::Light } else { prefs.theme.resolve(cx.window_appearance()) };
        theme::apply(mode, None, cx);
        i18n::set(if options.english { i18n::Lang::En } else { prefs.language.resolve() }, cx);
        cx.set_global(store);
        theme::set_density(prefs.density, cx);
        let source = if ephemeral {
            plain_text::Source::Fake(std::sync::Arc::new(plain_text::DemoRunner))
        } else {
            plain_text::Source::Claude { cache: crate::base::axiom_home().join("desktop").join("plain-text.json") }
        };
        plain_text::install(cx, prefs.plain_words, source);
        // The quit hook captures the system motion setting before the stored override is applied.
        settings::Store::save_on_quit(cx);
        if let Some(on) = prefs.reduce_motion {
            cx.set_reduce_motion(on);
        }
        let mut initial = initial_state(&options);
        initial.scene = state_scene(&options);
        initial.reduce_motion = cx.reduce_motion();
        if options.demo {
            if let Some(scene) = options.scene.as_deref() {
                demo::apply_data_scene(&mut initial, scene);
            }
        }
        initial.view = options.view.unwrap_or(prefs.last_screen);
        let state = cx.new(|_| initial);
        if !options.demo {
            live::connect(state.clone(), cx);
        }
        let default_size = options.size.unwrap_or_else(|| size(px(1440.), px(900.)));
        let min_size = size(px(1100.), px(700.));
        let bounds = if options.offscreen {
            WindowBounds::Windowed(Bounds { origin: point(px(-3000.), px(-3000.)), size: default_size })
        } else {
            settings::opening_bounds(&prefs, default_size, min_size, cx)
        };
        let window = WindowOptions {
            window_bounds: Some(bounds),
            window_min_size: Some(min_size),
            titlebar: Some(component::TitleBar::title_bar_options()),
            focus: !options.offscreen,
            ..Default::default()
        };
        let may_flash = !options.offscreen;
        // The launch starts with the window; a scene that holds it freezes the motion clock there.
        let start = std::time::Instant::now();
        let scene = options.scene.as_deref();
        if let Some(ms) = launch::held_ms(scene) {
            motion::freeze(start + Duration::from_millis(ms), cx);
        }
        let opening = if launch::plays(options.offscreen, scene, cx.reduce_motion()) {
            launch::Launch::start(start)
        } else {
            launch::Launch::none()
        };
        if shell::start_mini_only(ephemeral, prefs.mode) {
                if let Err(error) = mini::open_mini_at(state.clone(), prefs.mini_on_top, options.offscreen, cx) {
                    eprintln!("{error}");
                    cx.quit();
                }
        } else {
                gpui_kit::open_window(window, cx, |window, cx| {
                    cx.new(|cx| shell::Shell::new(state.clone(), may_flash, window, cx).with_launch(opening))
                }).expect("could not open the main window");
        }
        if let Some(seconds) = options.quit_after {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_secs(seconds)).await;
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
        }
    });
    0
}

#[cfg(test)]
mod strip_startup_tests {
    use super::*;
    use std::prelude::v1::test;
    #[test]
    fn real_and_empty_starts_have_no_demo_running_counts() {
        let real=initial_state(&Options::default());
        assert_eq!(real.quota,crate::usage::quota::Quotas::default());
        assert!(real.codex_slots.is_none() && real.codex_models.is_empty() && real.live_models.is_empty());
        let empty=initial_state(&Options {demo:true,empty:true,..Default::default()});
        assert!(empty.codex_slots.is_none() && empty.codex_models.is_empty() && empty.live_models.is_empty());
        let demo=initial_state(&Options {demo:true,..Default::default()});
        assert_eq!(demo.codex_slots,Some(15)); assert!(!demo.live_models.is_empty());
    }
}
