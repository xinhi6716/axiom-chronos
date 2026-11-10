//! The Chronos desktop app. Screens draw `model::AppState`; `live` fills it from disk and the session host.
mod composer;
mod control;
mod detail;
pub mod demo;
mod doctor;
mod evolve;
pub mod i18n;
mod launch;
mod live;
mod memory;
mod mesh;
pub mod model;
mod motion;
mod ornament;
mod palette;
pub mod plain_text;
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
    let prefs = store.settings.clone();
    gpui_kit::application().with_assets(ornament::Assets).run(move |cx| {
        gpui_kit::init(cx);
        let mode = if options.light { ThemeMode::Light } else { prefs.theme.resolve(cx.window_appearance()) };
        theme::apply(mode, None, cx);
        i18n::set(if options.english { i18n::Lang::En } else { prefs.language.resolve() }, cx);
        cx.set_global(store);
        let source = if ephemeral {
            plain_text::Source::Fake(std::sync::Arc::new(plain_text::DemoRunner))
        } else {
            plain_text::Source::Claude { cache: crate::base::axiom_home().join("desktop").join("plain-text.json") }
        };
        plain_text::install(cx, prefs.plain_words, source);
        settings::Store::save_on_quit(cx);
        if let Some(on) = prefs.reduce_motion {
            cx.set_reduce_motion(on);
        }
        let mut initial = demo::state();
        initial.scene = options.scene.clone();
        initial.reduce_motion = cx.reduce_motion();
        if !options.demo || options.empty {
            initial.runs.clear();
            initial.current = None;
            initial.memory.clear();
            initial.checks.clear();
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
        gpui_kit::open_window(window, cx, |window, cx| {
            cx.new(|cx| shell::Shell::new(state.clone(), may_flash, window, cx).with_launch(opening))
        })
        .expect("could not open the main window");
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
