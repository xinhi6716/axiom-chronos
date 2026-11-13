//! The Chronos desktop app. Screens draw `model::AppState`; `live` fills it from disk and the session host.
// --- research workspace (phase 9) ---
pub mod layout;
pub mod components;
pub mod viz;
pub mod data;
pub mod targets;
pub mod perf;
pub mod research;
// --- end research workspace (phase 9) ---
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
// p8: begin stub
pub mod paper;
// p8: end stub
pub mod plain_text;
mod quota;
mod run_view;
pub mod settings;
mod shell;
mod superswitch;
mod todo;
pub mod theme;
// ux: begin module
pub mod ux;
// ux: end
mod usage;
mod wishes;

use gpui_kit::component::ThemeMode;
use gpui_kit::*;
use model::Screen;
/// Exported so a headless test can install the same global `run` does, with `Store::open(true)`.
pub use settings::Store;
pub use shell::Shell;
pub use live::connect_service;
use std::time::Duration;

/// How the app starts. Demo options exist so every screen can be seen and captured without a live wish.
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub target: Option<String>,
    pub capture_state: targets::CaptureState,
    pub perf_log: Option<std::path::PathBuf>,
    pub demo: bool,
    pub confirm_quit: bool,
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
        // ux: begin initial state
        state.ux = if options.demo {
            ux::UxState::default()
        } else {
            ux::UxState {
                seams: ux::real::wiring::production(&crate::base::axiom_home(), &crate::codex::slots::home()),
                // --- research workspace (phase 9) ---
                research: research::ResearchSeams::production(&crate::base::axiom_home()),
                // --- end research workspace (phase 9) ---
                ..Default::default()
            }
        };
        // ux: end
    }
    if !options.demo { state.quota = Default::default(); }
    state.feeds.quit_requested = options.confirm_quit;
    state
}

pub fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--target" => { i += 1; let id = args.get(i).ok_or("--target takes an id")?; if targets::find(id).is_none() { return Err(format!("unknown target: {id}")); } options.target = Some(id.clone()); },
            "--state" => { i += 1; let value = args.get(i).ok_or("--state takes a value")?; options.capture_state = targets::CaptureState::from_key(value).ok_or_else(|| format!("unknown state: {value}"))?; },
            "--perf-log" => { i += 1; options.perf_log = Some(args.get(i).ok_or("--perf-log takes a path")?.into()); },
            "--demo" => options.demo = true,
            "--confirm-quit" => options.confirm_quit = true,
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

// `mini::open_mini_at` owns creation outside this module. Preserve its content and
// the required gpui-kit overlay host while inserting the instrumentation root.
fn instrument_mini(handle: AnyWindowHandle, cx: &mut App) -> gpui_kit::Result<()> {
    handle.update(cx, |_, window, cx| {
        let view = gpui_kit::base::Root::read(window, cx).view().clone();
        let instrumented = cx.new(|_| perf::Root::new(view));
        window.replace_root(cx, |window, cx| gpui_kit::base::Root::new(instrumented, window, cx));
    })
}

pub fn run(options: Options) -> i32 {
    let started = std::time::Instant::now();
    let failed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let run_failed = failed.clone();
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
    if options.target.is_some() { store.settings.mode = settings::WindowMode::Full; }
    if let Some(density) = options.density { store.settings.density = density; }
    let prefs = store.settings.clone();
    gpui_kit::application().with_assets(ornament::Assets).run(move |cx| {
        gpui_kit::init(cx);
        if let Some(path) = options.perf_log.clone() { perf::install(path, started, cx); }
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
        let capture = options.target.as_deref().and_then(targets::find);
        let state = cx.new(|_| initial);
        if !options.demo {
            live::connect(state.clone(), cx);
        }
        let default_size = options.size.unwrap_or_else(|| size(px(layout::DEFAULT_WINDOW.0), px(layout::DEFAULT_WINDOW.1)));
        let min_size = size(px(layout::MIN_WINDOW.0), px(layout::MIN_WINDOW.1));
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
        if shell::start_mini_only(ephemeral, prefs.mode, options.confirm_quit) {
                match mini::open_mini_at(state.clone(), prefs.mini_on_top, options.offscreen, cx) {
                    Ok(handle) => if let Err(error) = instrument_mini(handle, cx) {
                        eprintln!("{error}");
                        cx.quit();
                    },
                    Err(error) => { eprintln!("{error}"); cx.quit(); }
                }
        } else {
                gpui_kit::open_window(window, cx, |window, cx| {
                    let shell = cx.new(|cx| shell::Shell::new(state.clone(), may_flash, window, cx).with_launch(opening));
                    if let Some(t) = &capture {
                        let prepared = state.update(cx, |state, cx| { let result = targets::prepare(t, options.capture_state, state); cx.notify(); result });
                        let result = prepared.and_then(|()| shell.update(cx, |shell, cx| shell.apply_target(t, options.capture_state, window, cx)));
                        if let Err(error) = result { eprintln!("unregistered target: {} ({})", error.target, error.state.key()); run_failed.store(true, std::sync::atomic::Ordering::Relaxed); cx.quit(); }
                    }
                    cx.new(|_| perf::Root::new(shell.into()))
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
    if failed.load(std::sync::atomic::Ordering::Relaxed) { 1 } else { 0 }
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

    #[test]
    fn startup_uses_real_seams_only_outside_demo() {
        let real = initial_state(&Options::default());
        assert!(real.ux.seams.dual_core.is_some());
        assert!(real.ux.seams.sessions.is_none());
        let demo = initial_state(&Options { demo: true, ..Default::default() });
        assert!(demo.ux.seams.dual_core.is_some());
        assert!(demo.ux.seams.sessions.is_some());
        let empty = initial_state(&Options { demo: true, empty: true, ..Default::default() });
        assert!(empty.ux.seams.dual_core.is_none());
        assert!(empty.ux.seams.sessions.is_none());
    }

    #[test]
    fn confirm_quit_option_requests_startup_dialog() {
        for (args, expected) in [(vec![], false), (vec!["--confirm-quit".into()], true)] {
            let options = parse_options(&args).unwrap();
            assert_eq!(options.confirm_quit, expected);
            assert_eq!(initial_state(&options).feeds.quit_requested, expected);
        }
    }
}
